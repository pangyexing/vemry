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
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use vemry_agent::translate::{TranslateDoc, TranslateEvent, TranslateJob, TranslationCache};
use vemry_agent::HttpTransport;
use vemry_core::{sanitize_filename, Frontmatter, NoteId, SegKind, Segment, VemryMeta};

use crate::commands::{broadcast, with_vault, with_vault_mut};
use crate::{settings, AppState};

/// Where a vault's translation caches live (under `.vemry/`, beside the PDF cache).
pub const CACHE_SUBDIR: &str = "cache/translate";

#[derive(Default)]
pub struct TranslateState {
    job: Mutex<Option<Arc<TranslateJob>>>,
    next: AtomicU64,
    /// What each job was opened from — the save needs the source back.
    sources: Mutex<HashMap<String, TranslateSource>>,
}

/// What is being translated. The wire shape is the frontend's `TranslateSource`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranslateSource {
    Note { path: String },
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
    let (doc, cache_dir) = match &source {
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
            Ok((doc, Some(v.meta_dir().join(CACHE_SUBDIR))))
        })?,
    };
    if !doc.segments.iter().any(|seg| seg.translate && !seg.private) {
        return Err(t("translate.nothingToTranslate"));
    }
    let cache = cache_dir.as_ref().map(|dir: &PathBuf| TranslationCache::open(dir, &doc.key, &target, &provider.model));
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
        cache,
        on_event,
    );
    let known = job.translations();
    // One job at a time: the previous one stops streaming, its events stop mattering
    if let Some(old) = state.translate.job.lock().unwrap().replace(Arc::clone(&job)) {
        old.cancel();
        state.translate.sources.lock().unwrap().remove(&old.id);
    }
    state.translate.sources.lock().unwrap().insert(job_id.clone(), source.clone());
    // Everything not yet known, in document order; the view moves what it shows to the front
    let pending: Vec<u32> = segments
        .iter()
        .filter(|seg| seg.translate && !seg.private && !known.contains_key(&seg.id))
        .map(|seg| seg.id)
        .collect();
    job.request(&pending, false);
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

#[tauri::command]
pub async fn translate_cancel(state: State<'_, AppState>, job: String) -> Result<(), String> {
    let mut slot = state.translate.job.lock().unwrap();
    if let Some(j) = slot.as_ref().filter(|j| j.id == job) {
        j.cancel();
        state.translate.sources.lock().unwrap().remove(&j.id);
        *slot = None;
    }
    Ok(())
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

/// The saved note's body: the translations in document order, the copied-through
/// blocks as they are, a segment not translated yet as its source, and a private
/// block left out without a trace.
pub fn compose_translation(segments: &[Segment], translations: &HashMap<u32, String>) -> String {
    let mut out = String::new();
    for seg in segments {
        if seg.private {
            continue;
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
    let s = settings::load(&app);
    let translations = j.translations();
    let body = compose_translation(&j.doc.segments, &translations);
    let title = translated_title(&j.doc.title, &j.target);
    let mut fm = Frontmatter {
        title: Some(title.clone()),
        vemry: Some(VemryMeta { origin: Some("translation".into()), agent_session: None }),
        ..Frontmatter::default()
    };
    let (folder, of) = match &source {
        TranslateSource::Note { path } => {
            let id = NoteId::new(path);
            (id.folder().to_string(), format!("[[{}]]", id.stem()))
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
        Segment { id, kind, prefix: prefix.into(), text: text.into(), translate, private, span: 0..0 }
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
        assert_eq!(compose_translation(&segments, &tr), "## 简介\n\n你好\n\n```\nx\n```\n\n- [ ] 待办\n\nnot yet\n");
        assert_eq!(translated_title("Paper", "zh-CN"), "Paper (中译)");
        assert_eq!(translated_title("论文", "en"), "论文 (English)");
    }
}
