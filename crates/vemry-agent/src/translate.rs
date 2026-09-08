//! Side-by-side translation: a document's segments are sent to the model in
//! batches, the translations stream back one segment at a time, and a cache keyed
//! by the segment's text means a document is paid for once.
//!
//! This is not an agent turn. There is no session, no tool table, no ledger: the
//! request is the same tool-free single call the summary is (`tools: None`, the
//! light thinking level), made through the same transport. What the model gets is
//! decided here and only here — a private block is never enqueued, and the
//! caller decides whether the document as a whole may be sent at all.
//!
//! Wire shape: each segment goes out as `<s id=N>…</s>` and is expected back the
//! same way, so the alignment is by id rather than by position and a dropped or
//! reordered segment is detected rather than mislaid. `StreamParser` closes a
//! segment the moment its `</s>` arrives, which is what lets the right column fill
//! in as the model writes rather than when the batch ends.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};
use vemry_core::provenance::fnv1a64;
pub use vemry_core::{SegKind, Segment};

use crate::client::ChatTransport;
use crate::provider::ProviderConfig;
use crate::runner::effort_fields;
use crate::types::{ChatMessage, ChatRequest};
use crate::AgentError;

/// How much source text one request carries (in bytes of segment text). Small
/// enough that the first translation lands within seconds; large enough that a
/// page is a handful of calls.
pub const BATCH_CHARS: usize = 2000;
/// Requests in flight at once. Two keeps the model busy while the first batch's
/// text is still streaming, without burning a rate limit on a long document.
pub const MAX_IN_FLIGHT: usize = 2;

/// The document being translated: the cache key it is filed under, its title, and
/// the segments (`vemry_core::segment_markdown`, or the PDF page cutter).
#[derive(Clone, Debug)]
pub struct TranslateDoc {
    pub key: String,
    pub title: String,
    pub segments: Vec<Segment>,
}

/// What a job reports. `Segment` with `done: false` is the text so far of the
/// segment the model is writing; with `done: true` it is final (and cached).
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TranslateEvent {
    Segment { id: u32, text: String, done: bool },
    /// These segments will not arrive: the request failed (`message` is the
    /// provider's error as-is), or the model returned nothing for them twice.
    Failed { ids: Vec<u32>, message: String },
    /// Nothing pending and nothing in flight.
    Idle,
}

// ---- the cache ----

/// One document's translations into one language, keyed by the hash of each
/// segment's text: an edited paragraph misses on its own, everything else hits.
/// Holds no source text — the file is a map from hashes to translations.
#[derive(Debug)]
pub struct TranslationCache {
    /// None: an in-memory cache (a web page's — projected content never lands on disk).
    path: Option<PathBuf>,
    model: String,
    entries: HashMap<String, String>,
    dirty: bool,
}

#[derive(Serialize, Deserialize, Default)]
struct CacheFile {
    #[serde(default)]
    model: String,
    #[serde(default)]
    entries: HashMap<String, String>,
}

pub fn cache_key(text: &str) -> String {
    format!("{:016x}", fnv1a64(text.as_bytes()))
}

fn safe_name(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

impl TranslationCache {
    /// Open the cache of document `key` into `target` under `dir` (created on the
    /// first save). A missing or unreadable file is an empty cache.
    pub fn open(dir: &Path, key: &str, target: &str, model: &str) -> Self {
        let path = dir.join(format!("{}.{}.json", safe_name(key), safe_name(target)));
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<CacheFile>(&s).ok())
            .map(|f| f.entries)
            .unwrap_or_default();
        TranslationCache { path: Some(path), model: model.to_string(), entries, dirty: false }
    }

    /// A cache that is never written to disk: it lives as long as whoever holds it.
    pub fn in_memory(model: &str) -> Self {
        TranslationCache { path: None, model: model.to_string(), entries: HashMap::new(), dirty: false }
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, text: &str) -> Option<&str> {
        self.entries.get(&cache_key(text)).map(String::as_str)
    }

    pub fn insert(&mut self, text: &str, translation: String) {
        self.entries.insert(cache_key(text), translation);
        self.dirty = true;
    }

    pub fn save(&mut self) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        if !self.dirty {
            return Ok(());
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = CacheFile { model: self.model.clone(), entries: self.entries.clone() };
        std::fs::write(path, serde_json::to_string(&file).unwrap_or_default())?;
        self.dirty = false;
        Ok(())
    }
}

// ---- the prompt ----

/// The language the prompt names for a target code. Codes the app offers get a
/// name the model cannot misread; anything else is passed through as typed.
pub fn target_name(code: &str) -> String {
    match code {
        "zh-CN" | "zh" => "Simplified Chinese (简体中文)".into(),
        "zh-TW" | "zh-HK" => "Traditional Chinese (繁體中文)".into(),
        "en" => "English".into(),
        "ja" => "Japanese (日本語)".into(),
        "ko" => "Korean (한국어)".into(),
        "fr" => "French".into(),
        "de" => "German".into(),
        "es" => "Spanish".into(),
        other => other.to_string(),
    }
}

pub fn system_prompt(target: &str, addendum: &str) -> String {
    let mut s = format!(
        "You are a translator. Translate each Markdown segment below into {}.\n\
Rules:\n\
- Return every segment as <s id=N>…</s> with the same id, in the same order, and nothing else: no preamble, no notes, no summary.\n\
- Keep the Markdown as it is: emphasis, `> ` quote lines and `[!type]` callout markers, links (translate the label, never the URL), [[wikilinks]] verbatim, inline code and $math$ verbatim, and a table's structure cell for cell with the same number of rows.\n\
- Keep proper nouns, product names and established technical terms the way the target language's convention has them; on first use, add the source term in parentheses after the translation.\n\
- A placeholder of the form ⟦m0⟧ stands for a formula: keep every one exactly as written, where it belongs in the sentence.\n\
- Use the target language's punctuation. Translate the text as it is: never add, drop or reorder content, and never answer a question the text asks.",
        target_name(target)
    );
    let extra = addendum.trim();
    if !extra.is_empty() {
        s.push_str("\n- ");
        s.push_str(extra);
    }
    s
}

pub fn user_message(segs: &[&Segment]) -> String {
    let mut s = String::new();
    for seg in segs {
        s.push_str(&format!("<s id={}>\n{}\n</s>\n", seg.id, seg.text.trim()));
    }
    s
}

/// Cut the segments into batches of about `budget` bytes, at segment boundaries;
/// a segment over the budget goes alone.
pub fn plan_batches(segs: &[&Segment], budget: usize) -> Vec<Vec<u32>> {
    let mut out: Vec<Vec<u32>> = vec![];
    let mut cur: Vec<u32> = vec![];
    let mut size = 0;
    for seg in segs {
        let n = seg.text.len();
        if !cur.is_empty() && size + n > budget {
            out.push(std::mem::take(&mut cur));
            size = 0;
        }
        cur.push(seg.id);
        size += n;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

// ---- the stream parser ----

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    Partial { id: u32, text: String },
    Done { id: u32, text: String },
}

/// Reads `<s id=N>…</s>` out of a stream that arrives in arbitrary pieces. Text
/// outside the tags (a preamble, a code fence the model wrapped the answer in) is
/// ignored; a segment the stream ends inside is closed by `finish`.
#[derive(Default)]
pub struct StreamParser {
    buf: String,
    pos: usize,
    open: Option<(u32, usize)>,
}

const OPEN_TAG: &str = "<s id=";
const CLOSE_TAG: &str = "</s>";

impl StreamParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, chunk: &str) -> Vec<Parsed> {
        self.buf.push_str(chunk);
        let mut out = vec![];
        loop {
            match self.open {
                None => {
                    let Some(rel) = self.buf[self.pos..].find(OPEN_TAG) else { break };
                    let start = self.pos + rel;
                    let head = &self.buf[start + OPEN_TAG.len()..];
                    let Some(gt) = head.find('>') else { break }; // the header is still arriving
                    match head[..gt].trim().trim_matches(|c| c == '"' || c == '\'').parse::<u32>() {
                        Ok(id) => {
                            let content_start = start + OPEN_TAG.len() + gt + 1;
                            self.open = Some((id, content_start));
                            self.pos = content_start;
                        }
                        Err(_) => self.pos = start + OPEN_TAG.len(),
                    }
                }
                Some((id, start)) => {
                    if let Some(rel) = self.buf[start..].find(CLOSE_TAG) {
                        let text = self.buf[start..start + rel].trim().to_string();
                        out.push(Parsed::Done { id, text });
                        self.pos = start + rel + CLOSE_TAG.len();
                        self.open = None;
                    } else {
                        // a tail that may be the start of the closing tag is held back
                        let body = &self.buf[start..];
                        let keep = body.len() - trailing_tag_prefix(body);
                        let text = body[..keep].trim_start();
                        if !text.is_empty() {
                            out.push(Parsed::Partial { id, text: text.to_string() });
                        }
                        break;
                    }
                }
            }
        }
        out
    }

    /// The stream ended: a segment still open with text in it is taken as complete
    /// (the model forgot the closing tag).
    pub fn finish(mut self) -> Vec<Parsed> {
        if let Some((id, start)) = self.open.take() {
            let text = self.buf[start..].trim();
            if !text.is_empty() {
                return vec![Parsed::Done { id, text: text.to_string() }];
            }
        }
        vec![]
    }
}

fn trailing_tag_prefix(s: &str) -> usize {
    for p in ["</s", "</", "<"] {
        if s.ends_with(p) {
            return p.len();
        }
    }
    0
}

/// A translated table has to keep its row count, or it cannot sit beside the source.
fn table_shape_ok(source: &str, translation: &str) -> bool {
    let rows = |s: &str| s.lines().filter(|l| l.trim().starts_with('|')).count();
    rows(source) == rows(translation)
}

// ---- the job ----

#[derive(Default)]
struct Inner {
    /// Segment ids waiting to be sent, front first; `request` moves the visible
    /// ones to the front.
    pending: VecDeque<u32>,
    in_flight: usize,
    /// Ids in a batch that is being streamed.
    active: HashSet<u32>,
    /// Ids already resent alone after the model left them out — the second miss
    /// is a failure.
    retried: HashSet<u32>,
    done: HashMap<u32, String>,
    failed: HashSet<u32>,
}

pub struct TranslateJob {
    pub id: String,
    pub key: String,
    pub title: String,
    /// The segments, growable: a web page translated in place keeps producing
    /// blocks as it scrolls (`extend`); ids keep counting from where they were.
    segments: RwLock<Vec<Segment>>,
    pub target: String,
    addendum: String,
    provider: ProviderConfig,
    effort: Option<String>,
    transport: Arc<dyn ChatTransport>,
    cancel: AtomicBool,
    inner: Mutex<Inner>,
    cache: Mutex<Option<TranslationCache>>,
    on_event: Box<dyn Fn(TranslateEvent) + Send + Sync>,
}

impl TranslateJob {
    /// Build the job and take what the cache already holds. Nothing is sent until
    /// `request`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        doc: TranslateDoc,
        target: String,
        addendum: String,
        provider: ProviderConfig,
        effort: Option<String>,
        transport: Arc<dyn ChatTransport>,
        cache: Option<TranslationCache>,
        on_event: Box<dyn Fn(TranslateEvent) + Send + Sync>,
    ) -> Arc<Self> {
        let mut inner = Inner::default();
        if let Some(c) = &cache {
            for seg in doc.segments.iter().filter(|s| s.translate && !s.private) {
                if let Some(t) = c.get(&seg.text) {
                    inner.done.insert(seg.id, t.to_string());
                }
            }
        }
        Arc::new(TranslateJob {
            id,
            key: doc.key,
            title: doc.title,
            segments: RwLock::new(doc.segments),
            target,
            addendum,
            provider,
            effort,
            transport,
            cancel: AtomicBool::new(false),
            inner: Mutex::new(inner),
            cache: Mutex::new(cache),
            on_event,
        })
    }

    /// The model the job sends to (the saved note names it).
    pub fn model(&self) -> &str {
        &self.provider.model
    }

    /// The segments as they stand (a copy).
    pub fn segments(&self) -> Vec<Segment> {
        self.segments.read().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.segments.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Add segments (their ids are reassigned to continue the sequence), take
    /// what the cache holds for them, and queue the rest. Returns the ids given.
    pub fn extend(self: &Arc<Self>, more: Vec<Segment>) -> Vec<u32> {
        let ids: Vec<u32> = {
            let mut segs = self.segments.write().unwrap();
            let mut inner = self.inner.lock().unwrap();
            let cache = self.cache.lock().unwrap();
            let mut ids = vec![];
            for mut seg in more {
                seg.id = segs.len() as u32;
                if seg.translate && !seg.private {
                    if let Some(t) = cache.as_ref().and_then(|c| c.get(&seg.text)) {
                        inner.done.insert(seg.id, t.to_string());
                    }
                }
                ids.push(seg.id);
                segs.push(seg);
            }
            ids
        };
        self.request(&ids, false);
        ids
    }

    /// Every translation known now: the cache hits at open, plus what has landed.
    pub fn translations(&self) -> HashMap<u32, String> {
        self.inner.lock().unwrap().done.clone()
    }

    /// Ask for these segments next, in this order (the visible rows first). A
    /// segment already translated is skipped unless `force` (retranslate); one
    /// that is being streamed is left to finish. Private and copied-through
    /// segments are never enqueued whatever is asked.
    pub fn request(self: &Arc<Self>, ids: &[u32], force: bool) {
        {
            let mut inner = self.inner.lock().unwrap();
            let segs = self.segments.read().unwrap();
            let mut front: Vec<u32> = vec![];
            for &id in ids {
                let Some(seg) = segs.get(id as usize) else { continue };
                if !seg.translate || seg.private || inner.active.contains(&id) {
                    continue;
                }
                if force {
                    inner.done.remove(&id);
                    inner.retried.remove(&id);
                } else if inner.done.contains_key(&id) {
                    continue;
                }
                inner.failed.remove(&id);
                inner.pending.retain(|p| *p != id);
                if !front.contains(&id) {
                    front.push(id);
                }
            }
            for id in front.into_iter().rev() {
                inner.pending.push_front(id);
            }
        }
        self.pump();
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.inner.lock().unwrap().pending.clear();
    }

    /// Take the cache out of the job (an in-memory one, to keep for the next
    /// opening of the same page); the job goes on without one.
    pub fn take_cache(&self) -> Option<TranslationCache> {
        self.cache.lock().unwrap().take()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn emit(&self, e: TranslateEvent) {
        (self.on_event)(e);
    }

    /// Start batches while there is room. Called after every request and after
    /// every batch; must run inside a tokio runtime.
    fn pump(self: &Arc<Self>) {
        if self.is_cancelled() {
            return;
        }
        let idle = loop {
            let batch = {
                let mut inner = self.inner.lock().unwrap();
                if inner.in_flight >= MAX_IN_FLIGHT || inner.pending.is_empty() {
                    break inner.in_flight == 0 && inner.pending.is_empty();
                }
                let mut ids = vec![];
                let mut size = 0;
                let segs = self.segments.read().unwrap();
                while let Some(&id) = inner.pending.front() {
                    let n = segs[id as usize].text.len();
                    if !ids.is_empty() && size + n > BATCH_CHARS {
                        break;
                    }
                    inner.pending.pop_front();
                    ids.push(id);
                    size += n;
                }
                inner.in_flight += 1;
                inner.active.extend(ids.iter().copied());
                ids
            };
            let job = Arc::clone(self);
            tokio::spawn(async move { job.run_batch(batch).await });
        };
        if idle {
            self.emit(TranslateEvent::Idle);
        }
    }

    async fn run_batch(self: Arc<Self>, ids: Vec<u32>) {
        let owned: Vec<Segment> = {
            let all = self.segments.read().unwrap();
            ids.iter().map(|id| all[*id as usize].clone()).collect()
        };
        let segs: Vec<&Segment> = owned.iter().collect();
        // Thinking off: a translation is transcription, not reasoning, and on a
        // thinking model the thinking is most of the latency. Sent only where the
        // level is sent at all (a DeepSeek endpoint); elsewhere nothing is sent
        let (reasoning_effort, thinking) = effort_fields(self.effort.as_deref().map(|_| "none"));
        let request = ChatRequest {
            model: self.provider.model.clone(),
            messages: vec![
                ChatMessage::system(system_prompt(&self.target, &self.addendum)),
                ChatMessage::user(user_message(&segs)),
            ],
            tools: None,
            stream: true,
            stream_options: None,
            reasoning_effort,
            thinking,
        };
        let wanted: HashSet<u32> = ids.iter().copied().collect();
        let mut parser = StreamParser::new();
        let mut got: Vec<(u32, String)> = vec![];
        let mut reshape: Vec<u32> = vec![];
        let result = {
            let mut handle = |ev: Parsed| match ev {
                Parsed::Partial { id, text } if wanted.contains(&id) => {
                    self.emit(TranslateEvent::Segment { id, text, done: false });
                }
                Parsed::Done { id, text } if wanted.contains(&id) => {
                    let seg = owned.iter().find(|s| s.id == id).expect("a wanted id is in the batch");
                    let retried = self.inner.lock().unwrap().retried.contains(&id);
                    if seg.kind == SegKind::Table && !retried && !table_shape_ok(&seg.text, &text) {
                        reshape.push(id);
                        return;
                    }
                    got.push((id, text.clone()));
                    self.emit(TranslateEvent::Segment { id, text, done: true });
                }
                _ => {}
            };
            let mut on_chunk = |chunk: crate::types::StreamChunk| -> bool {
                for choice in chunk.choices {
                    if let Some(t) = choice.delta.content {
                        for ev in parser.push(&t) {
                            handle(ev);
                        }
                    }
                }
                true
            };
            let r = self.transport.stream_chat(&self.provider, &request, &self.cancel, &mut on_chunk).await;
            if r.is_ok() {
                for ev in parser.finish() {
                    handle(ev);
                }
            }
            r
        };

        let mut inner = self.inner.lock().unwrap();
        inner.in_flight -= 1;
        for id in &ids {
            inner.active.remove(id);
        }
        let mut cache = self.cache.lock().unwrap();
        for (id, text) in &got {
            inner.done.insert(*id, text.clone());
            if let (Some(c), Some(seg)) = (cache.as_mut(), owned.iter().find(|s| s.id == *id)) {
                c.insert(&seg.text, text.clone());
            }
        }
        if let Some(c) = cache.as_mut() {
            let _ = c.save();
        }
        drop(cache);
        let landed: HashSet<u32> = got.iter().map(|(id, _)| *id).collect();
        let missing: Vec<u32> = ids.iter().copied().filter(|id| !landed.contains(id)).collect();
        match result {
            Err(AgentError::Cancelled) => return,
            Err(e) => {
                inner.failed.extend(missing.iter().copied());
                drop(inner);
                if !missing.is_empty() {
                    self.emit(TranslateEvent::Failed { ids: missing, message: e.to_string() });
                }
            }
            Ok(()) => {
                let mut failed = vec![];
                // Left out of a multi-segment answer (or a table that lost rows): once
                // more, alone; a segment that misses twice is reported
                for id in missing.iter().rev() {
                    if (ids.len() > 1 || reshape.contains(id)) && !inner.retried.contains(id) {
                        inner.retried.insert(*id);
                        inner.pending.push_front(*id);
                    } else {
                        inner.failed.insert(*id);
                        failed.push(*id);
                    }
                }
                drop(inner);
                if !failed.is_empty() {
                    failed.reverse();
                    self.emit(TranslateEvent::Failed { ids: failed, message: "no translation returned".into() });
                }
            }
        }
        self.pump();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{finish_chunk, text_chunk, ScriptedTransport};
    use std::time::Duration;

    fn seg(id: u32, kind: SegKind, text: &str, translate: bool, private: bool) -> Segment {
        Segment { id, kind, prefix: String::new(), text: text.into(), translate, private, span: 0..0, page: None }
    }

    fn doc(segments: Vec<Segment>) -> TranslateDoc {
        TranslateDoc { key: "k".into(), title: "T".into(), segments }
    }

    #[test]
    fn parser_closes_segments_across_arbitrary_chunks() {
        let mut p = StreamParser::new();
        let mut all = vec![];
        for c in ["<s i", "d=0>Hel", "lo</", "s>\n<s id=\"1\">W", "orld"] {
            all.extend(p.push(c));
        }
        all.extend(p.finish());
        assert_eq!(
            all,
            vec![
                Parsed::Partial { id: 0, text: "Hel".into() },
                Parsed::Partial { id: 0, text: "Hello".into() },
                Parsed::Done { id: 0, text: "Hello".into() },
                Parsed::Partial { id: 1, text: "W".into() },
                Parsed::Partial { id: 1, text: "World".into() },
                Parsed::Done { id: 1, text: "World".into() },
            ]
        );
    }

    #[test]
    fn parser_ignores_text_outside_tags() {
        let mut p = StreamParser::new();
        let out = p.push("Sure! ```xml\n<s id=7>\n**bold** text\n</s>\n```\nDone.");
        assert_eq!(out, vec![Parsed::Done { id: 7, text: "**bold** text".into() }]);
    }

    #[test]
    fn batches_cut_at_segment_boundaries() {
        let a = seg(0, SegKind::Para, &"a".repeat(600), true, false);
        let b = seg(1, SegKind::Para, &"b".repeat(600), true, false);
        let c = seg(2, SegKind::Para, &"c".repeat(2500), true, false);
        let d = seg(3, SegKind::Para, "d", true, false);
        assert_eq!(plan_batches(&[&a, &b, &c, &d], 1000), vec![vec![0], vec![1], vec![2], vec![3]]);
        assert_eq!(plan_batches(&[&a, &b, &d], 2000), vec![vec![0, 1, 3]]);
    }

    fn events_channel() -> (Box<dyn Fn(TranslateEvent) + Send + Sync>, tokio::sync::mpsc::UnboundedReceiver<TranslateEvent>) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (Box::new(move |e| { let _ = tx.send(e); }), rx)
    }

    async fn drain_until_idle(rx: &mut tokio::sync::mpsc::UnboundedReceiver<TranslateEvent>) -> Vec<TranslateEvent> {
        let mut out = vec![];
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("idle").expect("open");
            let idle = matches!(ev, TranslateEvent::Idle);
            out.push(ev);
            if idle {
                return out;
            }
        }
    }

    #[tokio::test]
    async fn private_and_copied_segments_never_reach_the_wire() {
        let transport = Arc::new(ScriptedTransport::new(vec![vec![
            text_chunk("<s id=0>你好</s><s id=3>| a |\n|---|\n| 1 |</s>"),
            finish_chunk("stop"),
        ]]));
        let (on_event, mut rx) = events_channel();
        let d = doc(vec![
            seg(0, SegKind::Para, "Hello", true, false),
            seg(1, SegKind::Private, "> [!private]\n> secret", false, true),
            seg(2, SegKind::Code, "```\nx\n```", false, false),
            seg(3, SegKind::Table, "| a |\n|---|\n| 1 |", true, false),
        ]);
        let job = TranslateJob::new(
            "j".into(), d, "zh-CN".into(), "keep terms".into(), ProviderConfig::zhipu("k".into()), None,
            transport.clone(), None, on_event,
        );
        job.request(&[0, 1, 2, 3], false);
        let events = drain_until_idle(&mut rx).await;
        let done: Vec<(u32, String)> = events
            .iter()
            .filter_map(|e| match e { TranslateEvent::Segment { id, text, done: true } => Some((*id, text.clone())), _ => None })
            .collect();
        assert_eq!(done, vec![(0, "你好".into()), (3, "| a |\n|---|\n| 1 |".into())]);
        let reqs = transport.requests.lock().unwrap();
        assert_eq!(reqs.len(), 1);
        assert!(reqs[0].tools.is_none());
        let user = reqs[0].messages[1].text().unwrap().to_string();
        assert!(user.contains("<s id=0>\nHello\n</s>"));
        assert!(!user.contains("secret") && !user.contains("```"));
        let system = reqs[0].messages[0].text().unwrap().to_string();
        assert!(system.contains("Simplified Chinese") && system.ends_with("- keep terms"));
        assert_eq!(job.translations().len(), 2);
    }

    #[tokio::test]
    async fn a_segment_the_model_left_out_is_resent_alone_then_reported() {
        let transport = Arc::new(ScriptedTransport::new(vec![
            vec![text_chunk("<s id=0>A</s>"), finish_chunk("stop")],
            vec![text_chunk("nothing useful"), finish_chunk("stop")],
        ]));
        let (on_event, mut rx) = events_channel();
        let d = doc(vec![seg(0, SegKind::Para, "a", true, false), seg(1, SegKind::Para, "b", true, false)]);
        let job = TranslateJob::new(
            "j".into(), d, "en".into(), String::new(), ProviderConfig::zhipu("k".into()), None,
            transport.clone(), None, on_event,
        );
        job.request(&[0, 1], false);
        let events = drain_until_idle(&mut rx).await;
        assert!(events.iter().any(|e| matches!(e, TranslateEvent::Failed { ids, .. } if ids == &vec![1])));
        let reqs = transport.requests.lock().unwrap();
        assert_eq!(reqs.len(), 2);
        assert!(reqs[1].messages[1].text().unwrap().contains("<s id=1>"));
        assert!(!reqs[1].messages[1].text().unwrap().contains("<s id=0>"));
    }

    #[tokio::test]
    async fn a_provider_error_fails_the_batch_with_its_message() {
        struct Failing;
        #[async_trait::async_trait]
        impl ChatTransport for Failing {
            async fn stream_chat(
                &self, _c: &ProviderConfig, _r: &ChatRequest, _x: &AtomicBool,
                _o: &mut (dyn FnMut(crate::types::StreamChunk) -> bool + Send),
            ) -> crate::Result<()> {
                Err(AgentError::Status { code: 429, message: "HTTP 429: slow down".into() })
            }
        }
        let (on_event, mut rx) = events_channel();
        let d = doc(vec![seg(0, SegKind::Para, "a", true, false)]);
        let job = TranslateJob::new(
            "j".into(), d, "en".into(), String::new(), ProviderConfig::zhipu("k".into()), None,
            Arc::new(Failing), None, on_event,
        );
        job.request(&[0], false);
        let events = drain_until_idle(&mut rx).await;
        assert!(events.iter().any(|e| matches!(e, TranslateEvent::Failed { ids, message } if ids == &vec![0] && message.contains("429"))));
    }

    #[tokio::test]
    async fn the_cache_prefills_and_is_written_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut warm = TranslationCache::open(dir.path(), "doc", "zh-CN", "m");
        warm.insert("known", "已知".into());
        warm.save().unwrap();

        let transport = Arc::new(ScriptedTransport::new(vec![vec![text_chunk("<s id=1>新的</s>"), finish_chunk("stop")]]));
        let (on_event, mut rx) = events_channel();
        let d = doc(vec![seg(0, SegKind::Para, "known", true, false), seg(1, SegKind::Para, "new", true, false)]);
        let cache = TranslationCache::open(dir.path(), "doc", "zh-CN", "m");
        let job = TranslateJob::new(
            "j".into(), d, "zh-CN".into(), String::new(), ProviderConfig::zhipu("k".into()), None,
            transport.clone(), Some(cache), on_event,
        );
        assert_eq!(job.translations().get(&0).map(String::as_str), Some("已知"));
        job.request(&[0, 1], false);
        drain_until_idle(&mut rx).await;
        assert_eq!(transport.requests.lock().unwrap().len(), 1);
        let again = TranslationCache::open(dir.path(), "doc", "zh-CN", "m");
        assert_eq!(again.get("new"), Some("新的"));
        assert_eq!(again.get("known"), Some("已知"));
        // retranslating bypasses the cache
        let transport2 = Arc::new(ScriptedTransport::new(vec![vec![text_chunk("<s id=0>重译</s>"), finish_chunk("stop")]]));
        let (on_event, mut rx) = events_channel();
        let d = doc(vec![seg(0, SegKind::Para, "known", true, false)]);
        let job = TranslateJob::new(
            "j2".into(), d, "zh-CN".into(), String::new(), ProviderConfig::zhipu("k".into()), None,
            transport2.clone(), Some(TranslationCache::open(dir.path(), "doc", "zh-CN", "m")), on_event,
        );
        job.request(&[0], true);
        drain_until_idle(&mut rx).await;
        assert_eq!(job.translations().get(&0).map(String::as_str), Some("重译"));
    }
}
