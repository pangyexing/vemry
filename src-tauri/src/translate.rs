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
    web_caches: ParkedCaches,
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

/// A page's translations have no file to live in, so the cache is kept in memory and
/// put away when the page closes: opening the same page again finds it. The most
/// recent WEB_CACHE_CAP pages, oldest out first. Two pools use this — the reading
/// view's and the in-page pane's — because the two hash different text (Markdown
/// blocks against the DOM's own), so an entry of one would never match the other.
#[derive(Default)]
pub struct ParkedCaches(Mutex<Vec<(String, TranslationCache)>>);

impl ParkedCaches {
    fn key(url: &str, target: &str) -> String {
        format!("{url}\u{0}{target}")
    }

    pub fn park(&self, url: &str, target: &str, cache: TranslationCache) {
        if cache.is_empty() {
            return;
        }
        let key = Self::key(url, target);
        let mut caches = self.0.lock().unwrap();
        caches.retain(|(k, _)| *k != key);
        caches.push((key, cache));
        while caches.len() > WEB_CACHE_CAP {
            caches.remove(0);
        }
    }

    pub fn take(&self, url: &str, target: &str) -> Option<TranslationCache> {
        let key = Self::key(url, target);
        let mut caches = self.0.lock().unwrap();
        let at = caches.iter().position(|(k, _)| *k == key)?;
        Some(caches.remove(at).1)
    }
}

impl TranslateState {
    /// Put a page's in-memory cache away for the next opening of the same page.
    fn park_web_cache(&self, job: &TranslateJob) {
        let Some(TranslateSource::Page { url }) = self.sources.lock().unwrap().get(&job.id).cloned() else {
            return;
        };
        let Some(cache) = job.take_cache() else { return };
        self.web_caches.park(&url, &job.target, cache);
    }

    fn take_web_cache(&self, url: &str, target: &str) -> Option<TranslationCache> {
        self.web_caches.take(url, target)
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
    crate::i18n::current().tag().to_string()
}

/// The provider for a translation: the translation model when one is chosen, the
/// chat's otherwise — each resolved against its own provider, so a translation can
/// go to another endpoint than the chat (or through vemry hosted while the chat
/// does not).
pub(crate) fn provider_of(s: &settings::Settings) -> Result<vemry_agent::ProviderConfig, String> {
    let r = s.translation();
    match r.unready {
        Some(u) => Err(u.message()),
        None => Ok(r.config),
    }
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
    let provider = provider_of(&s)?;
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
            // The verdict and the read must be taken on one spelling: the read trims and
            // NoteId::new does not, so judging the raw path and opening the trimmed one
            // would let a leading space walk around the gate
            let rel = NoteId::new(path.trim());
            let rel = rel.as_str();
            // The whole file is in the privacy zone: nothing of it may be sent
            if v.is_private_rel(rel, false) {
                return Err(t("translate.privateFile"));
            }
            let (key, pages) = vemry_agent::context::pdf_pages(v, rel, rel)?;
            // No text at all is a scan, not an empty document: say which it is
            if pages.iter().all(|p| p.trim().is_empty()) {
                return Err(t("translate.pdfEmpty"));
            }
            let segments = vemry_core::segment_pages(&pages);
            let title = std::path::Path::new(rel).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| rel.to_string());
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
    let hosted = provider.is_hosted();
    let on_event: Box<dyn Fn(TranslateEvent) + Send + Sync> = Box::new(move |event| {
        // A relay error in the user's words (and a 401 clearing the token), as for a chat turn
        let event = match event {
            TranslateEvent::Failed { ids, message } if hosted => {
                let message = crate::account::localize_relay_error(&emit_app, &message).unwrap_or(message);
                TranslateEvent::Failed { ids, message }
            }
            e => e,
        };
        let _ = emit_app.emit("translate://event", EventView { job: emit_id.clone(), event });
    });
    let effort = provider.reasoning_supported().then(|| s.reasoning_effort.clone());
    let title = doc.title.clone();
    let segments: Vec<Segment> = doc.segments.clone();
    let job = TranslateJob::new(
        job_id.clone(),
        doc,
        target.clone(),
        s.translate_addendum.clone(),
        provider.clone(),
        effort,
        vemry_agent::transport_for(&provider, false),
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
        .ok_or_else(|| t("translate.noSuchJob"))
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
    let more = if all { None } else { Some(vemry_agent::translate::BACKGROUND_BUDGET_BYTES) };
    current_job(&state, &job)?.continue_background(more);
    Ok(())
}

#[tauri::command]
pub async fn translate_cancel(state: State<'_, AppState>, job: String) -> Result<(), String> {
    let taken = state.translate.job.lock().unwrap().take_if(|j| j.id == job);
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
    let tag = words_of(target).map(|w| w.0).unwrap_or(target);
    format!("{title} ({tag})")
}

/// The heading a page's translations sit under in a saved PDF translation, in the
/// target language.
pub fn page_heading(page: u32, target: &str) -> String {
    let heading = words_of(target).map(|w| w.1).unwrap_or("Page {n}");
    format!("## {}", heading.replace("{n}", &page.to_string()))
}

/// What a saved translation is called in the target language, and its page heading
/// with `{n}` for the page number (the word order differs per language): one table,
/// so a new language is one row rather than two match arms.
fn words_of(target: &str) -> Option<(&'static str, &'static str)> {
    match target {
        "zh-CN" | "zh" => Some(("中译", "第 {n} 页")),
        "zh-TW" | "zh-HK" => Some(("中譯", "第 {n} 頁")),
        "en" => Some(("English", "Page {n}")),
        "ja" => Some(("日本語訳", "{n} ページ")),
        "ko" => Some(("한국어 번역", "{n} 쪽")),
        "fr" => Some(("Traduction française", "Page {n}")),
        "de" => Some(("Deutsche Übersetzung", "Seite {n}")),
        "es" => Some(("Traducción al español", "Página {n}")),
        _ => None,
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
    let source = state.translate.sources.lock().unwrap().get(&job).cloned().ok_or_else(|| t("translate.noSuchJob"))?;
    // What is still untranslated is translated first — the saved note is complete,
    // not a mix of translation and source
    if j.has_remaining() {
        j.continue_background(None);
        j.wait_idle().await;
        if j.is_cancelled() {
            return Err(t("translate.noSuchJob"));
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
            // stem() strips only `.md`, so a PDF's stem is its file name — link_to's rule
            (id.folder().to_string(), link_to(&id))
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
        // the vault's own "a name already taken gets a suffix" rule, as clips use it
        let candidate = v.dedup_rel(&if folder.is_empty() { format!("{safe}.md") } else { format!("{folder}/{safe}.md") });
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
        assert_eq!(translated_title("论文", "es"), "论文 (Traducción al español)");
        assert_eq!(translated_title("论文", "pt"), "论文 (pt)");
        assert_eq!(page_heading(3, "en"), "## Page 3");
        assert_eq!(page_heading(3, "ja"), "## 3 ページ");
        assert_eq!(page_heading(3, "es"), "## Página 3");
        assert_eq!(page_heading(3, "pt"), "## Page 3");
    }
}
