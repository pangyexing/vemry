//! Side-by-side translation, the glue: a source (a note, a web page, a PDF) is cut
//! into segments here, the job in vemry-agent does the sending, and its events
//! are relayed to the frontend as `translate://event`.
//!
//! One job at a time: opening a source cancels the previous one. Whether a
//! source may be sent at all is decided here — a note in the privacy zone is
//! refused whole, and the private blocks of a visible note are marked by the
//! segmenter and skipped by the job. The translations are a projection: they
//! live in the job (and in `.vemry/cache/translate/` for a note) until the user
//! saves them as a note through the same `note_write` path an ordinary note takes.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use vemry_agent::translate::{TranslateDoc, TranslateEvent, TranslateJob, TranslationCache};
use vemry_agent::HttpTransport;
use vemry_core::{sanitize_filename, Frontmatter, NoteId, SegKind, Segment, VemryMeta};
use vemry_clip::CLIPS_DIR;

use crate::commands::{broadcast, with_vault, with_vault_mut};
use crate::{settings, AppState};

/// Where a vault's translation caches live (under `.vemry/`, beside the PDF cache).
pub const CACHE_SUBDIR: &str = "cache/translate";

/// How many web pages' translations are kept in memory (the reader's page cache
/// holds as many).
const WEB_CACHE_CAP: usize = 32;

#[derive(Default)]
pub struct TranslateState {
    job: Mutex<Option<Arc<TranslateJob>>>,
    next: AtomicU64,
    /// What each job was opened from — the save needs the source back.
    sources: Mutex<HashMap<String, TranslateSource>>,
    /// A web page's translations, by `url + target`, kept only in memory: the
    /// projection's content never lands on disk, and neither does its translation.
    /// Newest last; the oldest is dropped past the cap.
    web_caches: Mutex<Vec<(String, TranslationCache)>>,
}

/// What is being translated. The wire shape is the frontend's `TranslateSource`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranslateSource {
    Note { path: String },
    /// A page the reading projection holds (the reader window's cache); the URL is
    /// the page's final one.
    Page { url: String },
    /// A PDF in the vault: its text page by page (the same extraction and cache the
    /// agent's read_file uses), the rows aligned by page.
    Pdf { path: String },
}

fn web_cache_key(url: &str, target: &str) -> String {
    format!("{url}\u{0}{target}")
}

impl TranslateState {
    /// Put a page's in-memory cache away for the next opening of the same page.
    fn park_web_cache(&self, job: &TranslateJob) {
        let TranslateSource::Page { url } = self.sources.lock().unwrap().get(&job.id).cloned().unwrap_or(TranslateSource::Note { path: String::new() }) else {
            return;
        };
        let Some(cache) = job.take_cache() else { return };
        if cache.is_empty() {
            return;
        }
        let key = web_cache_key(&url, &job.target);
        let mut caches = self.web_caches.lock().unwrap();
        caches.retain(|(k, _)| *k != key);
        caches.push((key, cache));
        while caches.len() > WEB_CACHE_CAP {
            caches.remove(0);
        }
    }

    fn take_web_cache(&self, url: &str, target: &str) -> Option<TranslationCache> {
        let key = web_cache_key(url, target);
        let mut caches = self.web_caches.lock().unwrap();
        let at = caches.iter().position(|(k, _)| *k == key)?;
        Some(caches.remove(at).1)
    }

    /// Drop the current job: it stops streaming, and a page's cache is parked.
    fn drop_job(&self, job: &TranslateJob) {
        job.cancel();
        self.park_web_cache(job);
        self.sources.lock().unwrap().remove(&job.id);
    }
}

/// One segment as the view renders it: the engine's segment plus the cached
/// translation, if there is one, so the row is filled before the first request.
#[derive(Serialize)]
pub struct SegmentView {
    pub id: u32,
    pub kind: SegKind,
    pub prefix: String,
    pub text: String,
    pub translate: bool,
    pub private: bool,
    pub byte_start: usize,
    /// The 1-based page of a PDF's segment; None for the other sources.
    pub page: Option<u32>,
    pub cached: Option<String>,
}

#[derive(Serialize)]
pub struct TranslateDocView {
    pub job: String,
    pub title: String,
    pub source: TranslateSource,
    pub target: String,
    pub model: String,
    pub segments: Vec<SegmentView>,
}

#[derive(Clone, Serialize)]
struct EventView {
    job: String,
    #[serde(flatten)]
    event: TranslateEvent,
}

/// The target language: the setting, or the interface language when unset.
pub fn target_of(s: &settings::Settings) -> String {
    let t = s.translate_target.trim();
    if !t.is_empty() {
        return t.to_string();
    }
    match crate::i18n::current() {
        crate::i18n::Lang::ZhCn => "zh-CN".into(),
        crate::i18n::Lang::En => "en".into(),
    }
}

/// The provider for a translation: the chat endpoint and key, with the
/// translation model when one is set.
fn provider_of(s: &settings::Settings) -> vemry_agent::ProviderConfig {
    let mut p = s.provider_config();
    let m = s.translate_model.trim();
    if !m.is_empty() {
        p.model = m.to_string();
    }
    p
}

fn t(key: &str) -> String {
    crate::i18n::t(crate::i18n::current(), key)
}

#[tauri::command]
pub async fn translate_open(
    app: AppHandle,
    state: State<'_, AppState>,
    source: TranslateSource,
) -> Result<TranslateDocView, String> {
    let s = settings::load(&app);
    let provider = provider_of(&s);
    if provider.api_key.is_empty() {
        return Err(t("ipc.noApiKey"));
    }
    let target = target_of(&s);
    let (doc, cache) = match &source {
        TranslateSource::Page { url } => {
            let (final_url, title, markdown) = state
                .reader
                .0
                .lock()
                .unwrap()
                .cached_page(url)
                .ok_or_else(|| t("translate.pageExpired"))?;
            let cut = vemry_core::segment_markdown(&markdown);
            let doc = TranslateDoc { key: vemry_agent::translate::cache_key(&final_url), title, segments: cut.segments };
            let cache = state
                .translate
                .take_web_cache(&final_url, &target)
                .unwrap_or_else(|| TranslationCache::in_memory(&provider.model));
            (doc, cache)
        }
        TranslateSource::Pdf { path } => with_vault(&state, |v| {
            // The whole file is in the privacy zone: nothing of it may be sent
            if v.is_private_rel(path, false) {
                return Err(t("translate.privateFile"));
            }
            let (key, pages) = vemry_agent::context::pdf_pages(v, path, path)?;
            let segments = vemry_core::segment_pages(&pages);
            let title = std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| path.clone());
            let doc = TranslateDoc { key, title, segments };
            let cache = TranslationCache::open(&v.meta_dir().join(CACHE_SUBDIR), &doc.key, &target, &provider.model);
            Ok((doc, cache))
        })?,
        TranslateSource::Note { path } => with_vault(&state, |v| {
            let id = v.note_id(path).map_err(crate::i18n::err)?;
            // The whole note is in the privacy zone: nothing of it may be sent
            if v.is_private(&id) {
                return Err(t("translate.privateNote"));
            }
            let raw = v.read_note(&id).map_err(crate::i18n::err)?;
            let cut = vemry_core::segment_markdown(&raw);
            let title = cut.frontmatter.title.clone().unwrap_or_else(|| id.stem().to_string());
            let doc = TranslateDoc { key: vemry_agent::translate::cache_key(id.as_str()), title, segments: cut.segments };
            let cache = TranslationCache::open(&v.meta_dir().join(CACHE_SUBDIR), &doc.key, &target, &provider.model);
            Ok((doc, cache))
        })?,
    };
    if !doc.segments.iter().any(|seg| seg.translate && !seg.private) {
        return Err(t("translate.nothingToTranslate"));
    }
    let job_id = format!("t{}", state.translate.next.fetch_add(1, Ordering::Relaxed) + 1);
    let emit_app = app.clone();
    let emit_id = job_id.clone();
    let on_event: Box<dyn Fn(TranslateEvent) + Send + Sync> = Box::new(move |event| {
        let _ = emit_app.emit("translate://event", EventView { job: emit_id.clone(), event });
    });
    let effort = s.reasoning_active().then(|| s.reasoning_effort.clone());
    let title = doc.title.clone();
    let segments: Vec<Segment> = doc.segments.clone();
    let job = TranslateJob::new(
        job_id.clone(),
        doc,
        target.clone(),
        s.translate_addendum.clone(),
        provider.clone(),
        effort,
        Arc::new(HttpTransport::new()),
        Some(cache),
        on_event,
    );
    let known = job.translations();
    // One job at a time: the previous one stops streaming, its events stop mattering
    let old = state.translate.job.lock().unwrap().replace(Arc::clone(&job));
    if let Some(old) = old {
        state.translate.drop_job(&old);
    }
    state.translate.sources.lock().unwrap().insert(job_id.clone(), source.clone());
    // Everything not yet known, in document order, as background work (the budget
    // bounds it); the view asks for what it shows, which always comes first
    let pending: Vec<u32> = segments
        .iter()
        .filter(|seg| seg.translate && !seg.private && !known.contains_key(&seg.id))
        .map(|seg| seg.id)
        .collect();
    job.queue(&pending);
    Ok(TranslateDocView {
        job: job_id,
        title,
        source,
        target,
        model: provider.model,
        segments: segments
            .into_iter()
            .map(|seg| SegmentView {
                cached: known.get(&seg.id).cloned(),
                id: seg.id,
                kind: seg.kind,
                prefix: seg.prefix,
                text: seg.text,
                translate: seg.translate,
                private: seg.private,
                byte_start: seg.span.start,
                page: seg.page,
            })
            .collect(),
    })
}

fn current_job(state: &State<'_, AppState>, job: &str) -> Result<Arc<TranslateJob>, String> {
    state
        .translate
        .job
        .lock()
        .unwrap()
        .as_ref()
        .filter(|j| j.id == job)
        .cloned()
        .ok_or_else(|| "no such translation".to_string())
}

/// The view asks for segments in the order it wants them (the visible rows
/// first); `force` retranslates ones already done.
#[tauri::command]
pub async fn translate_request(state: State<'_, AppState>, job: String, ids: Vec<u32>, force: bool) -> Result<(), String> {
    current_job(&state, &job)?.request(&ids, force);
    Ok(())
}

/// Go on with the background translation past its stop: another budget's worth,
/// or the whole document.
#[tauri::command]
pub async fn translate_continue(state: State<'_, AppState>, job: String, all: bool) -> Result<(), String> {
    let more = if all { None } else { Some(vemry_agent::translate::BACKGROUND_BUDGET_CHARS) };
    current_job(&state, &job)?.continue_background(more);
    Ok(())
}

#[tauri::command]
pub async fn translate_cancel(state: State<'_, AppState>, job: String) -> Result<(), String> {
    let taken = {
        let mut slot = state.translate.job.lock().unwrap();
        match slot.as_ref() {
            Some(j) if j.id == job => slot.take(),
            _ => None,
        }
    };
    if let Some(j) = taken {
        state.translate.drop_job(&j);
    }
    Ok(())
}

/// How the translation points back at its source: a wikilink by stem, unless the
/// stem cannot sit inside `[[…]]` (a clip named after a page title with brackets
/// in it, say), when the path is written as it is.
fn link_to(id: &NoteId) -> String {
    let stem = id.stem();
    if stem.contains(['[', ']', '|', '#']) { id.as_str().to_string() } else { format!("[[{stem}]]") }
}

/// The name a saved translation carries: the source's title plus the target
/// language, in that language.
pub fn translated_title(title: &str, target: &str) -> String {
    let tag = match target {
        "zh-CN" | "zh" => "中译",
        "zh-TW" | "zh-HK" => "中譯",
        "en" => "English",
        "ja" => "日本語訳",
        "ko" => "한국어 번역",
        other => other,
    };
    format!("{title} ({tag})")
}

/// The heading a page's translations sit under in a saved PDF translation, in the
/// target language.
pub fn page_heading(page: u32, target: &str) -> String {
    match target {
        "zh-CN" | "zh" => format!("## 第 {page} 页"),
        "zh-TW" | "zh-HK" => format!("## 第 {page} 頁"),
        "ja" => format!("## {page} ページ"),
        "ko" => format!("## {page}쪽"),
        _ => format!("## Page {page}"),
    }
}

/// The saved note's body: the translations in document order, the copied-through
/// blocks as they are, a segment not translated yet as its source, and a private
/// block left out without a trace. A PDF's segments are grouped under a heading
/// per page.
pub fn compose_translation(segments: &[Segment], translations: &HashMap<u32, String>, target: &str) -> String {
    let mut out = String::new();
    let mut page: Option<u32> = None;
    for seg in segments {
        if seg.private {
            continue;
        }
        if seg.page.is_some() && seg.page != page {
            page = seg.page;
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(&page_heading(page.unwrap_or(1), target));
        }
        let body = if seg.translate { translations.get(&seg.id).map(String::as_str).unwrap_or(&seg.text) } else { &seg.text };
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&seg.prefix);
        out.push_str(body);
    }
    out.push('\n');
    out
}

/// Save the job's translations as a note beside the source: the same path a note
/// the user writes takes (no gate — it is the user's action), formatted like a
/// clip, and linked back through `translation_of`.
#[tauri::command]
pub async fn translate_save(app: AppHandle, state: State<'_, AppState>, job: String) -> Result<String, String> {
    let j = current_job(&state, &job)?;
    let source = state.translate.sources.lock().unwrap().get(&job).cloned().ok_or("no such translation")?;
    // What is still untranslated is translated first — the saved note is complete,
    // not a mix of translation and source
    if j.has_remaining() {
        j.continue_background(None);
        j.wait_idle().await;
        if j.is_cancelled() {
            return Err("no such translation".into());
        }
    }
    let s = settings::load(&app);
    let translations = j.translations();
    let segments = j.segments();
    let body = compose_translation(&segments, &translations, &j.target);
    let title = translated_title(&j.title, &j.target);
    let mut fm = Frontmatter {
        title: Some(title.clone()),
        vemry: Some(VemryMeta { origin: Some("translation".into()), agent_session: None }),
        ..Frontmatter::default()
    };
    let (folder, of) = match &source {
        TranslateSource::Note { path } => {
            let id = NoteId::new(path);
            (id.folder().to_string(), link_to(&id))
        }
        // A page: the original is clipped first (or the clip it already has is
        // used), and the translation sits beside it in clips/, linked to it and
        // carrying the same source URL
        TranslateSource::Page { url } => {
            let existing = with_vault(&state, |v| Ok(v.clips_for_urls(std::slice::from_ref(url)).remove(url)))?;
            let clip = match existing {
                Some(rel) => rel,
                None => crate::browser::reader::reading_clip(app.clone(), state.clone(), url.clone())?,
            };
            let id = NoteId::new(&clip);
            let of = link_to(&id);
            fm.extra.insert("source".into(), url.clone().into());
            if let Some(domain) = url::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string())) {
                fm.extra.insert("domain".into(), domain.into());
            }
            (CLIPS_DIR.trim_end_matches('/').to_string(), of)
        }
        // A PDF: the translation sits beside the file, pointing at it by name
        TranslateSource::Pdf { path } => {
            let id = NoteId::new(path);
            let name = path.rsplit('/').next().unwrap_or(path);
            let of = if name.contains(['[', ']', '|', '#']) { path.clone() } else { format!("[[{name}]]") };
            (id.folder().to_string(), of)
        }
    };
    fm.extra.insert("translation_of".into(), of.into());
    fm.extra.insert("lang".into(), j.target.clone().into());
    fm.extra.insert("translator".into(), j.model().to_string().into());
    let yaml = fm.to_yaml().map_err(crate::i18n::err)?;
    let content = format!("---\n{yaml}---\n\n{body}");
    let content = settings::format_or(&content, s.clip_format());
    let (rel, event) = with_vault_mut(&state, |v| {
        let safe = sanitize_filename(&title);
        let folder = folder.trim_matches('/');
        let mut candidate = if folder.is_empty() { format!("{safe}.md") } else { format!("{folder}/{safe}.md") };
        let mut n = 1;
        while v.abs_path(&NoteId::new(&candidate)).exists() {
            candidate = if folder.is_empty() { format!("{safe} {n}.md") } else { format!("{folder}/{safe} {n}.md") };
            n += 1;
        }
        let id = NoteId::new(&candidate);
        let event = v.write_note(&id, &content).map_err(crate::i18n::err)?;
        Ok((candidate, event))
    })?;
    broadcast(&app, &event);
    Ok(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(id: u32, kind: SegKind, prefix: &str, text: &str, translate: bool, private: bool) -> Segment {
        Segment { id, kind, prefix: prefix.into(), text: text.into(), translate, private, span: 0..0, page: None }
    }

    #[test]
    fn a_saved_translation_keeps_structure_and_drops_private_blocks() {
        let segments = vec![
            seg(0, SegKind::Heading { level: 2 }, "## ", "Intro", true, false),
            seg(1, SegKind::Para, "", "Hello", true, false),
            seg(2, SegKind::Private, "", "> [!private]\n> secret", false, true),
            seg(3, SegKind::Code, "", "```\nx\n```", false, false),
            seg(4, SegKind::ListItem { ordered: false, task: true }, "- [ ] ", "todo", true, false),
            seg(5, SegKind::Para, "", "not yet", true, false),
        ];
        let mut tr = HashMap::new();
        tr.insert(0, "简介".to_string());
        tr.insert(1, "你好".to_string());
        tr.insert(4, "待办".to_string());
        assert_eq!(compose_translation(&segments, &tr, "zh-CN"), "## 简介\n\n你好\n\n```\nx\n```\n\n- [ ] 待办\n\nnot yet\n");
        // a PDF's segments go under a heading per page
        let mut p1 = seg(0, SegKind::Para, "", "one", true, false);
        p1.page = Some(1);
        let mut p2 = seg(1, SegKind::Para, "", "two", true, false);
        p2.page = Some(2);
        let mut tr = HashMap::new();
        tr.insert(0, "一".to_string());
        assert_eq!(compose_translation(&[p1, p2], &tr, "zh-CN"), "## 第 1 页\n\n一\n\n## 第 2 页\n\ntwo\n");
        assert_eq!(translated_title("Paper", "zh-CN"), "Paper (中译)");
        assert_eq!(translated_title("论文", "en"), "论文 (English)");
    }
}
