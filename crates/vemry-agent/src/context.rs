//! `@` context references: the notes, folders, text attachments and editor
//! selections the user picked explicitly in the composer. The frontend passes
//! only path identifiers; resolution happens here, inside the runner's vault lock
//! — every vault read path the agent can see must pass the privacy gate (design
//! principle #2). Notes and folders reuse `execute_read_tool`'s funnel verbatim:
//! identical wording for private and missing, the 30k truncation and the audit
//! filtered count all come for free; text attachments had no agent read path at
//! all before, so an equivalent gate is built here. A selection is the one vault
//! reference whose **body is handed over by the frontend**: what is selected are
//! the characters in the editor buffer (possibly unsaved, with line numbers that
//! do not match the disk), Rust cannot read that buffer, and so it has to trust
//! the text the frontend gives it — but the path still passes the same privacy
//! gate: a selection made inside a private note is dropped whole here (only the
//! filtered count is recorded), and what the model sees is worded exactly like
//! "the attachment does not exist". Web context (PageContext) does not come
//! through here — a web page is outside the privacy domain, and the frontend
//! assembling and handing it over is an existing exemption the comments endorse.

//!
//! An image reference (`Image`) is the fourth agent-visible read path, gated the
//! same way as a text attachment, but its content does **not enter the envelope
//! at resolution time**: resolution only checks the gate (so a failure surfaces
//! as this turn's "attachment unavailable" line), and what history stores is
//! `Part::ImageRef{path}`; every time the runner assembles a request,
//! `inflate_images` passes the gate again inside the vault lock, reads the bytes
//! and base64-encodes them into a data URL — base64 never enters history or the
//! sidecar (disk is the single source of truth). The side effect is that images
//! are **re-gated every turn**: an image deleted or marked private afterwards is
//! left as placeholder text in later requests, which is stricter than a text
//! reference (whose original envelope is frozen and replayed).

use std::collections::HashMap;

use base64::Engine;
use serde::Deserialize;
use serde_json::json;
use vemry_core::Vault;

use crate::tools::{execute_read_tool, ToolError};
use crate::types::{ChatMessage, Content, ImageUrl, Part};

/// The cap on @ references one message may carry (the frontend refuses beyond
/// the same number; this is a defensive truncation).
pub const MAX_CONTEXT_REFS: usize = 8;
/// The character limit for injecting a text attachment into an envelope —
/// first-level compaction cannot reach inside a user envelope (compact only
/// rewrites tool messages), so truncation is the only guard rail, at the same
/// number as read_note.
pub const FILE_CONTEXT_MAX_CHARS: usize = 30_000;
/// The byte limit for a PDF attachment: extraction runs synchronously inside the
/// vault lock, and an enormous file would block every other vault operation —
/// beyond the limit it is refused outright, with an explanation.
pub const PDF_MAX_BYTES: usize = 20 * 1024 * 1024;
/// The byte limit for one image (Zhipu GLM-4V's 5MB is the strictest among the
/// common endpoints). Nothing is scaled or transcoded — pulling in the image
/// crate is a cost this batch does not pay, so anything over the limit is refused
/// with an explanation.
pub const IMAGE_MAX_BYTES: usize = 5 * 1024 * 1024;
/// How many images one request fills in across the history at most (counting
/// back from the newest); anything older keeps a one-line placeholder —
/// re-sending every image in the history each turn is an unbounded cost.
pub const MAX_HISTORY_IMAGES: usize = 6;

/// One @ reference (deserialised straight from the IPC parameter,
/// `{"kind":"note","path":".."}`; the frontend's `ContextRef` in api.ts mirrors
/// it, and the snake_case field names change on both sides together).
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContextRef {
    Note { path: String },
    Folder { path: String },
    File { path: String },
    /// An image in the vault (png/jpg/gif/webp): only the path travels; the bytes
    /// are filled in when the request is assembled.
    Image { path: String },
    /// An editor selection: `text` is what the frontend took from the buffer (see
    /// the module comment); the line numbers are 1-based and inclusive and serve
    /// only the envelope annotation and the audit log — nothing is read back off
    /// disk from them.
    Selection { path: String, from_line: usize, to_line: usize, text: String },
    /// One block of a note, chosen in the composer's `@Note#` stage: the block's
    /// local id (`^id` explicit, `bN` positional), resolved against the note at
    /// send time — a heading hands over its section, a list item its subtree.
    Block { path: String, local_id: String },
    /// What a projection view shows right now (the Agenda, the board): the view's
    /// query, re-run here through the same privacy-filtered query API the view and
    /// the query_blocks tool use — not a new read path, the user's table made
    /// citable. `path` is the view's id (`view:agenda`), the audit's spelling.
    View { path: String, label: String, query: serde_json::Value },
    /// A skill the user chose in the composer (`/research`): `path` is the
    /// skill's listed name. Its text is injected as the method for this
    /// message — the "how" beside the other references' "what" — so it is in
    /// hand before the plan and, living in the user envelope, never compacted
    /// away with the tool results.
    Skill { path: String },
}

/// The kind of a reference. A `ResolvedRef` carries it into envelope assembly
/// (prompt.rs matches exhaustively — a new kind fails to compile there rather
/// than landing silently in the "note content" envelope); `as_str()` is the one
/// spelling used by the audit log and the frontend's `kind` field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RefKind {
    Note,
    Folder,
    File,
    Image,
    Selection,
    Block,
    View,
    Skill,
}

impl RefKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RefKind::Note => "note",
            RefKind::Folder => "folder",
            RefKind::File => "file",
            RefKind::Image => "image",
            RefKind::Selection => "selection",
            RefKind::Block => "block",
            RefKind::View => "view",
            RefKind::Skill => "skill",
        }
    }
    /// The envelope tag (always `{as_str()}-context`): assembly, scrub's locator
    /// string and the attribute scan share this one spelling. `Image` has no
    /// envelope; its value only makes the match exhaustive.
    pub fn tag(self) -> &'static str {
        match self {
            RefKind::Note => "note-context",
            RefKind::Folder => "folder-context",
            RefKind::File => "file-context",
            RefKind::Image => "image-context",
            RefKind::Selection => "selection-context",
            RefKind::Block => "block-context",
            RefKind::View => "view-context",
            RefKind::Skill => "skill-context",
        }
    }
    /// Every kind, so `parse` stays the inverse of `as_str()` by construction and a
    /// new variant is one edit instead of three.
    pub const ALL: [RefKind; 8] = [
        RefKind::Note,
        RefKind::Folder,
        RefKind::File,
        RefKind::Image,
        RefKind::Selection,
        RefKind::Block,
        RefKind::View,
        RefKind::Skill,
    ];

    /// The inverse of `as_str()`: used when reading a `StoredRef.kind` string back
    /// out of the sidecar. An unrecognised kind (older or newer) returns None and
    /// the caller decides what to do with it.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

impl ContextRef {
    pub fn kind(&self) -> RefKind {
        match self {
            ContextRef::Note { .. } => RefKind::Note,
            ContextRef::Folder { .. } => RefKind::Folder,
            ContextRef::File { .. } => RefKind::File,
            ContextRef::Image { .. } => RefKind::Image,
            ContextRef::Selection { .. } => RefKind::Selection,
            ContextRef::Block { .. } => RefKind::Block,
            ContextRef::View { .. } => RefKind::View,
            ContextRef::Skill { .. } => RefKind::Skill,
        }
    }
    pub fn path(&self) -> &str {
        match self {
            ContextRef::Note { path }
            | ContextRef::Folder { path }
            | ContextRef::File { path }
            | ContextRef::Image { path }
            | ContextRef::Selection { path, .. }
            | ContextRef::Block { path, .. }
            | ContextRef::View { path, .. }
            | ContextRef::Skill { path } => path,
        }
    }
    /// A block reference's handle as the audit names it when the block could not
    /// be resolved: `^id` for an explicit id, `bN` for a positional one (None for
    /// every other kind). A resolved block hands over its own handle instead
    /// (`Vault::block_for_agent`), which is what the envelope shows.
    pub fn block(&self) -> Option<String> {
        match self {
            ContextRef::Block { local_id, .. } => {
                let clean = local_id.trim().trim_start_matches('^');
                Some(if is_positional(clean) { clean.to_string() } else { format!("^{clean}") })
            }
            _ => None,
        }
    }
    /// A selection's line range (None for every other kind).
    pub fn lines(&self) -> Option<(usize, usize)> {
        match self {
            ContextRef::Selection { from_line, to_line, .. } => Some((*from_line, *to_line)),
            _ => None,
        }
    }
}

/// The resolution result: `body` Ok = the text injected into the envelope; Err =
/// the explanation shown to the model — private and genuinely missing are worded
/// identically ("no permission" would reveal that the path exists).
#[derive(Debug)]
pub struct ResolvedRef {
    pub kind: RefKind,
    pub path: String,
    /// A selection's line range (1-based, inclusive); it goes into the envelope
    /// attribute and the audit log's `lines` field.
    pub lines: Option<(usize, usize)>,
    /// A block reference's handle (`^id` / `bN`); it goes into the envelope's
    /// `block` attribute and the audit log's `block` field.
    pub block: Option<String>,
    /// When `kind == RefKind::Image`, Ok holds the mime type (the proof it passed
    /// the gate) and the content is not here — the image bytes are filled in by
    /// `inflate_images` by path when the request is assembled.
    pub body: Result<String, String>,
    /// This reference itself was filtered out by the privacy gate: the audit's
    /// refs list drops it accordingly (recording the count, never the path).
    /// `body.is_err()` cannot stand in for it — an Err also covers publicly
    /// recordable cases like a genuine miss or a failed PDF, and privacy and
    /// missing are deliberately worded alike, so the two cannot be told apart
    /// after the fact.
    pub private: bool,
}

/// Resolve every reference inside the vault lock (privacy can change between
/// pick and send, and only the verdict at resolution time counts — the same
/// spirit as execute_write's TOCTOU re-check). Returns (the resolutions, the
/// privacy-filtered count); the count goes into the audit's privacy line, as a
/// count and never a path.
/// `image_cache` is the runner's fill-in cache for this turn: the bytes read
/// while checking the gate are encoded and stored there in passing, so the
/// `inflate_images` that follows immediately need not read the same image again.
pub fn resolve_context_refs(
    vault: &Vault,
    refs: &[ContextRef],
    image_cache: &mut HashMap<String, Result<String, String>>,
) -> (Vec<ResolvedRef>, usize) {
    let mut filtered = 0usize;
    // The skill goes first — the method before the material — and stands
    // outside the cap: it is one short text, and cutting it for a ninth note
    // would drop the very instructions the user chose.
    let (skills, material): (Vec<&ContextRef>, Vec<&ContextRef>) =
        refs.iter().partition(|r| matches!(r, ContextRef::Skill { .. }));
    let out = skills
        .into_iter()
        .chain(material.into_iter().take(MAX_CONTEXT_REFS))
        .map(|r| {
            let before = filtered;
            let mut resolved_handle: Option<String> = None;
            let body = match r {
                ContextRef::Note { path } => run_read(vault, "read_note", json!({ "path": path }), &mut filtered),
                // A folder means a listing: paths and titles, with the content
                // read on demand by the model through read_note. Private children
                // go into filtered, and a private or non-existent folder yields
                // "(no notes)" just the same — exactly what the model gets when it
                // calls list_notes itself
                ContextRef::Folder { path } => {
                    run_read(vault, "list_notes", json!({ "folder": path, "limit": 500 }), &mut filtered)
                }
                ContextRef::File { path } => read_text_file(vault, path, &mut filtered),
                ContextRef::Image { path } => resolve_image(vault, path, &mut filtered, image_cache),
                ContextRef::Selection { path, text, .. } => gate_selection(vault, path, text, &mut filtered),
                ContextRef::Block { path, local_id } => {
                    resolve_block(vault, path, local_id, &mut filtered).map(|(body, handle)| {
                        resolved_handle = Some(handle);
                        body
                    })
                }
                ContextRef::View { label, query, .. } => resolve_view(vault, label, query, &mut filtered),
                // Not a vault read: the skills directory is the agent-visible
                // exception, and a built-in is not a file at all — the name is
                // resolved exactly as the read_skill tool resolves it
                ContextRef::Skill { path } => crate::skills::read_skill(&vault.meta_dir(), path)
                    .ok_or_else(|| format!("skill not found: {path}")),
            };
            // Whether the reference itself was filtered by the gate: for the
            // non-folder kinds any increase in filtered can only come from this
            // one hitting the gate; for a folder the increase means private
            // children, and the folder itself is judged separately
            // A selection that was delivered with private lines cut still keeps
            // its audit path (the count went up, the body is Ok) — only a
            // reference that failed the gate outright is private here
            let private = match r {
                ContextRef::Folder { path } => vault.is_private_rel(path.trim().trim_matches('/'), true),
                _ => body.is_err() && filtered > before,
            };
            // A selection that lies inside one block is citable as that block (the
            // block menu's "ask the AI about this block" arrives as a selection)
            let block = match r {
                ContextRef::Selection { path, from_line, to_line, .. } if body.is_ok() => {
                    selection_block(vault, path, *from_line, *to_line)
                }
                _ => resolved_handle.or_else(|| r.block()),
            };
            ResolvedRef { kind: r.kind(), path: r.path().to_string(), lines: r.lines(), block, body, private }
        })
        .collect();
    (out, filtered)
}

/// The cap on rows a view reference hands over (a view can hold thousands of tasks; the model gets the head and a count).
const VIEW_CONTEXT_MAX_ROWS: usize = 200;

/// A view reference: the view's query re-run through `query_for_agent` (the
/// privacy-filtered query every tool and view goes through), rendered one block
/// per line with its citation handle, state and dates — what the user is looking
/// at, made citable.
fn resolve_view(vault: &Vault, label: &str, query: &serde_json::Value, filtered: &mut usize) -> Result<String, String> {
    let mut q: vemry_core::BlockQuery = serde_json::from_value(query.clone()).map_err(|e| format!("bad view query: {e}"))?;
    q.limit = Some(q.limit.unwrap_or(VIEW_CONTEXT_MAX_ROWS).min(VIEW_CONTEXT_MAX_ROWS));
    let (hits, hidden) = vault.query_for_agent(&q);
    *filtered += hidden;
    if hits.is_empty() {
        return Ok(format!("({label}: no blocks)"));
    }
    let mut out = String::new();
    for h in &hits {
        let note = h.note.as_str().strip_suffix(".md").unwrap_or(h.note.as_str());
        let mut meta = String::new();
        if let vemry_core::BlockKind::Task { state, due, scheduled, start, priority, depends_on, .. } = &h.block.kind {
            meta.push_str(match state {
                vemry_core::TaskState::Todo => " [todo]",
                vemry_core::TaskState::Doing => " [doing]",
                vemry_core::TaskState::Done => " [done]",
                vemry_core::TaskState::Cancelled => " [cancelled]",
            });
            if !priority.is_normal() {
                meta.push_str(&format!(" priority {}", vemry_core::taskattrs::priority_name(*priority)));
            }
            if let Some(d) = start {
                meta.push_str(&format!(" start {d}"));
            }
            if let Some(d) = scheduled {
                meta.push_str(&format!(" scheduled {d}"));
            }
            if let Some(d) = due {
                meta.push_str(&format!(" due {d}"));
            }
            if !depends_on.is_empty() {
                meta.push_str(&format!(" blocked-by {}", depends_on.join(",")));
            }
        }
        let first: String = h.block.content.lines().next().unwrap_or("").chars().take(160).collect();
        out.push_str(&format!("[[{note}#{}]]{meta}\t{first}\n", h.block.handle()));
    }
    if hits.len() >= VIEW_CONTEXT_MAX_ROWS {
        out.push_str(&format!("(showing the first {VIEW_CONTEXT_MAX_ROWS} rows; use query_blocks with a narrower filter for the rest)\n"));
    }
    Ok(out.trim_end().to_string())
}

fn run_read(vault: &Vault, tool: &str, args: serde_json::Value, filtered: &mut usize) -> Result<String, String> {
    match execute_read_tool(vault, tool, &args) {
        Ok(o) => {
            *filtered += o.filtered;
            Ok(o.text)
        }
        Err(ToolError::Privacy(msg)) => {
            *filtered += 1;
            Err(msg)
        }
        Err(ToolError::Other(msg)) => Err(msg),
    }
}

/// The privacy gate: the three read paths built outside the `execute_read_tool`
/// funnel (text attachments, selections, images) share this one. The order is not
/// negotiable — the privacy verdict comes before existence, and the error wording
/// **is produced by the same constructor** as `Vault::read_file`'s missing case,
/// so the two are equal byte for byte ("no permission" would reveal that a path
/// exists; writing the literal twice would let the two quietly diverge the next
/// time the wording changes, which is exactly how the Chinese and English
/// versions came apart in 2026-08). The rel returned matches the trim
/// validate_rel does inside read_file, and everything after the gate shares that
/// one wording; the privacy verdict and the read that follows must both use this
/// rel — judging privacy on the raw path and then reading the trimmed one would
/// let a path with leading whitespace walk around the gate.
/// Path escapes (`..`, absolute paths) are refused by read_file.
fn gate_private<'a>(vault: &Vault, path: &'a str, filtered: &mut usize) -> Result<&'a str, String> {
    let rel = path.trim().trim_matches('/');
    if vault.is_private_rel(rel, false) {
        *filtered += 1;
        return Err(vemry_core::reject(vemry_core::PathReject::NotFound, rel).to_string());
    }
    Ok(rel)
}

/// The block a selection lies in, as a citation handle (`^id` / `bN`): only when
/// its first and last lines fall inside one block that is not private — a passage
/// spanning blocks, or one the disk no longer holds, gets no handle. The lines are
/// the editor's (1-based, over the whole file) and the mapping is against the disk
/// copy, so an unsaved buffer may miss; the frontend's content guard catches a
/// handle that went stale.
fn selection_block(vault: &Vault, path: &str, from_line: usize, to_line: usize) -> Option<String> {
    let id = vault.note_id(path).ok()?;
    if vault.is_private(&id) {
        return None;
    }
    let raw = vault.read_note(&id).ok()?;
    let mut offsets = Vec::new();
    let mut off = 0;
    for l in raw.split_inclusive('\n') {
        offsets.push(off);
        off += l.len();
    }
    let start = *offsets.get(from_line.checked_sub(1)?)?;
    let end = *offsets.get(to_line.checked_sub(1)?)?;
    let parsed = vemry_core::parser::parse_note(&raw);
    let i = vemry_core::locate::find_block_at(&parsed.blocks, start)?;
    if vemry_core::locate::find_block_at(&parsed.blocks, end)? != i {
        return None;
    }
    let b = &parsed.blocks[i];
    if matches!(b.kind, vemry_core::BlockKind::Private) {
        return None;
    }
    Some(b.handle())
}

/// `bN`: the index's positional id (never written to disk; valid until the note changes).
fn is_positional(id: &str) -> bool {
    id.len() > 1 && id.starts_with('b') && id[1..].chars().all(|c| c.is_ascii_digit())
}

/// A block reference: the privacy verdict first (worded like a missing note, by
/// the same constructor as every other gated read), then the block by its local
/// id through `Vault::block_for_agent` (a heading = its section, a list item = its
/// subtree, private blocks removed from the slice, a private block nonexistent).
/// A missing block is an ordinary miss, not a privacy filter: the id came from
/// the UI's index a moment ago, and its wording names the id so the user sees
/// what went stale. An id with characters outside a block id's alphabet is a
/// miss too (it can never be in a note, and it must not reach an attribute).
fn resolve_block(vault: &Vault, path: &str, local_id: &str, filtered: &mut usize) -> Result<(String, String), String> {
    let rel = gate_private(vault, path, filtered)?;
    let id = vault.note_id(rel).map_err(|e| e.to_string())?;
    let clean = local_id.trim().trim_start_matches('^');
    // The parser's own acceptance rule: an id it would never mint can never be in
    // a note, and must not reach an envelope attribute.
    if !vemry_core::blockid::is_valid_block_id(clean) {
        return Err(format!("block not found: {id}#{clean}"));
    }
    match vault.block_for_agent(&id, clean) {
        Ok(Some((hit, handle))) => Ok((hit.content, handle)),
        Ok(None) => Err(format!("block not found: {id}#{clean}")),
        // a note that cannot be read is worded exactly like a gated one
        Err(_) => Err(vemry_core::reject(vemry_core::PathReject::NotFound, rel).to_string()),
    }
}

/// Truncation for what is injected into an envelope: the limit and the marker are
/// shared by both self-built read paths (a test asserts this sentence byte for
/// byte).
fn truncate_ctx(text: &str) -> Option<String> {
    crate::truncate_chars(text, FILE_CONTEXT_MAX_CHARS, &format!("[... truncated at {FILE_CONTEXT_MAX_CHARS} chars]"))
}

/// Reading a text attachment: an agent-visible read path, gated by `gate_private`.
/// A note sent as a file is read as a note — `read_file` knows nothing of
/// private blocks, so a `.md` path must take the note path that cuts them.
pub(crate) fn read_text_file(vault: &Vault, path: &str, filtered: &mut usize) -> Result<String, String> {
    let rel = gate_private(vault, path, filtered)?;
    if rel.to_ascii_lowercase().ends_with(".md") {
        return run_read(vault, "read_note", json!({ "path": rel }), filtered);
    }
    let pdf = is_pdf(rel);
    let content =
        if pdf { read_pdf_text(vault, rel, rel)? } else { vault.read_file(rel).map_err(|e| e.to_string())? };
    match truncate_ctx(&content) {
        Some(t) => Ok(t),
        None if content.is_empty() && pdf => Ok("(no extractable text in this PDF; it may be a scan)".into()),
        None if content.is_empty() => Ok("(empty file)".into()),
        None => Ok(content),
    }
}

pub(crate) fn is_pdf(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case("pdf"))
}

/// PDF → text (pdf-extract, pure Rust): a model cannot take PDF bytes, and the
/// extraction route works with any text-only model. The privacy gate is passed by
/// the caller first; this only handles size, parse failures (including a panic
/// inside the library — a malformed PDF must not take down the whole turn) and
/// whitespace normalisation. Layout is necessarily lost, which the first line of
/// the envelope says.
pub(crate) fn read_pdf_text(vault: &Vault, path: &str, rel: &str) -> Result<String, String> {
    let (_, pages) = pdf_pages(vault, path, rel)?;
    let text = pages_text(&pages);
    if text.is_empty() {
        return Ok(text);
    }
    Ok(format!("(the text below was extracted from a PDF; its layout may be lost; [p. N] marks where page N starts)\n\n{text}"))
}

/// The flat text of a PDF: the pages one after another, each opened by a `[p. N]`
/// marker so a reader (the model, a search hit) can name the page.
fn pages_text(pages: &[String]) -> String {
    let mut out = String::new();
    for (i, page) in pages.iter().enumerate() {
        if page.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&format!("[p. {}]\n{}", i + 1, page));
    }
    out
}

/// A PDF's text page by page (pdf-extract, pure Rust), with the bytes' hash it is
/// filed under. The extraction is cached under .vemry/cache/pdf by that hash: a PDF
/// read once (attached, through read_file, or for translation) costs nothing the
/// next time. Three files per PDF: `<key>.pages.json` (the pages), `<key>.txt` (the
/// flat text search_notes searches) and `<key>.path` (where it came from).
/// The privacy gate is passed by the caller first; this only handles size, parse
/// failures (including a panic inside the library — a malformed PDF must not take
/// down the whole turn) and whitespace normalisation.
pub fn pdf_pages(vault: &Vault, path: &str, rel: &str) -> Result<(String, Vec<String>), String> {
    let bytes = vault.read_file_bytes(path).map_err(|e| e.to_string())?;
    if bytes.len() > PDF_MAX_BYTES {
        return Err(format!("PDF too large (>20MB): {rel}"));
    }
    let key = format!("{:016x}", vemry_core::provenance::fnv1a64(&bytes));
    let cache_dir = vault.meta_dir().join(PDF_CACHE_SUBDIR);
    let cached = cache_dir.join(format!("{key}.pages.json"));
    if let Some(pages) = std::fs::read_to_string(&cached).ok().and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()) {
        return Ok((key, pages));
    }
    let extracted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pdf_extract::extract_text_from_mem_by_pages(&bytes)))
        .map_err(|_| format!("PDF parse failed: {rel}"))?
        .map_err(|e| format!("PDF parse failed: {rel} ({e})"))?;
    let pages: Vec<String> = extracted.iter().map(|p| collapse_blank_lines(p)).collect();
    if std::fs::create_dir_all(&cache_dir).is_ok() {
        let _ = std::fs::write(&cached, serde_json::to_string(&pages).unwrap_or_default());
        let _ = std::fs::write(cache_dir.join(format!("{key}.txt")), pages_text(&pages));
        let _ = std::fs::write(cache_dir.join(format!("{key}.path")), rel);
    }
    Ok((key, pages))
}

/// Where extracted PDF texts live under the vault's meta directory (`<key>.txt` = the
/// text, `<key>.path` = the vault-relative path it came from).
pub const PDF_CACHE_SUBDIR: &str = "cache/pdf";

/// The cached PDF extractions: (vault-relative path, the cached text file), private
/// ones filtered out and counted. The text itself stays on disk — a caller that
/// stops early (search, once its window is full) never pays to read it. A PDF
/// appears here once it has been read (attached with @, or through read_file);
/// until then search does not see it — the inventory is list_files.
pub fn cached_pdf_files(vault: &Vault, filtered: &mut usize) -> Vec<(String, std::path::PathBuf)> {
    let dir = vault.meta_dir().join(PDF_CACHE_SUBDIR);
    let Ok(rd) = std::fs::read_dir(&dir) else { return vec![] };
    let mut out = Vec::new();
    for e in rd.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "path") {
            continue;
        }
        let Ok(rel) = std::fs::read_to_string(&p) else { continue };
        let rel = rel.trim().to_string();
        // the file may have moved or been deleted since: a stale cache entry is skipped
        if !vault.root().join(&rel).is_file() {
            continue;
        }
        if vault.is_private_rel(&rel, false) {
            *filtered += 1;
            continue;
        }
        let txt = p.with_extension("txt");
        if txt.is_file() {
            out.push((rel, txt));
        }
    }
    out.sort();
    out
}

/// Whitespace normalisation of an extraction: trailing whitespace stripped, runs
/// of blank lines collapsed to one, and the whole thing trimmed.
fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0usize;
    for line in s.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            blank += 1;
            continue;
        }
        if !out.is_empty() {
            out.push_str(if blank > 0 { "\n\n" } else { "\n" });
        }
        blank = 0;
        out.push_str(line);
    }
    out.trim().to_string()
}

/// A selection: the body comes from the frontend, so this only applies the
/// privacy gate and the truncation. The gate matches `read_text_file`'s (private
/// and missing worded alike), but **existence is not checked** — what was
/// selected are characters in a buffer, so a newly created, never-saved note can
/// be referenced too; the path serves only the privacy verdict (both a glob and
/// a frontmatter `private` go through `is_private_rel`, the same function that
/// filters the current note).
fn gate_selection(vault: &Vault, path: &str, text: &str, filtered: &mut usize) -> Result<String, String> {
    let rel = gate_private(vault, path, filtered)?;
    // Block-level privacy, two layers. (1) Lines equal to a line of one of the
    // note's private blocks on disk are dropped — by line content, not line
    // number, so an unsaved edit above the selection cannot shift the cut, and
    // a selection that starts in the middle of a private callout (no head line
    // to parse) is still caught. (2) The text is segmented like a note and its
    // private blocks removed. The residual gap — a never-saved private block
    // selected from its middle — is closed by the editor, which refuses such a
    // selection; this stays the real gate.
    let mut body = text.to_string();
    let mut dropped = false;
    // the index knows whether the note has a private block at all: no disk read for the common case
    if let Some(id) = vault.note_id(rel).ok().filter(|id| vault.has_private_blocks(id)) {
        if let Ok(raw) = vault.read_note(&id) {
            let blocks = vemry_core::parser::parse_note(&raw).blocks;
            let private_lines = vemry_core::privacy::private_lines(&raw, &blocks);
            if !private_lines.is_empty() {
                let kept: Vec<&str> = body
                    .lines()
                    .filter(|l| {
                        let hit = !l.trim().is_empty() && private_lines.contains(l.trim());
                        dropped |= hit;
                        !hit
                    })
                    .collect();
                if dropped {
                    body = kept.join("\n");
                }
            }
        }
    }
    let blocks = vemry_core::parser::parse_blocks(&body, 0);
    if vemry_core::has_private_blocks(&blocks) {
        body = vemry_core::redact_private_blocks(&body, &blocks).text;
        dropped = true;
    }
    if dropped {
        *filtered += 1;
    }
    match truncate_ctx(&body) {
        Some(t) => Ok(t),
        None if body.trim().is_empty() => Ok("(empty selection)".into()),
        None => Ok(body),
    }
}

/// Image extension → mime (only the four every vision API accepts; the frontend's
/// `AGENT_IMAGE_EXTS` mirrors this and the two change together). svg/tiff/bmp are
/// not among them: the TIFF the macOS clipboard falls back to when there is no
/// PNG is still stored in the vault, but it cannot be sent as a reference.
pub fn image_mime(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (_, ext) = name.rsplit_once('.')?;
    match ext.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Resolving an image reference: check the gate, and encode the bytes read while
/// doing so straight into this turn's fill-in cache — `inflate_images` wants that
/// very copy next, so the disk is not read twice (for a 5MB image that saves a
/// megabyte-scale read plus a base64 pass per turn). Ok holds the mime type; the
/// content still stays out of the envelope. A failure is not cached: a reference
/// whose body is Err never produces an `ImageRef` at all.
fn resolve_image(
    vault: &Vault,
    path: &str,
    filtered: &mut usize,
    cache: &mut HashMap<String, Result<String, String>>,
) -> Result<String, String> {
    let (mime, bytes) = read_image(vault, path, filtered)?;
    cache.insert(path.to_string(), Ok(image_data_url(mime, &bytes)));
    Ok(mime.to_string())
}

/// Bytes → data URL (the resolution-time warm-up and the fill-in share one
/// spelling).
fn image_data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Reading an image: the gate order matches `read_text_file` — privacy before
/// everything else (worded byte for byte like the missing case, counted into
/// filtered), and only then format, existence and size. Resolution and the
/// per-turn fill-in share this one function, so the gate cannot drift with the
/// moment it is called.
fn read_image(vault: &Vault, path: &str, filtered: &mut usize) -> Result<(&'static str, Vec<u8>), String> {
    let rel = gate_private(vault, path, filtered)?;
    let mime = image_mime(rel).ok_or_else(|| format!("unsupported image format (png/jpg/gif/webp only): {rel}"))?;
    let bytes = vault.read_file_bytes(rel).map_err(|e| e.to_string())?;
    if bytes.len() > IMAGE_MAX_BYTES {
        return Err(format!("image too large (>5MB): {rel}"));
    }
    Ok((mime, bytes))
}

/// Before a request is assembled, fill the `ImageRef`s in the copy of the history
/// into data URLs (only the copy changes; the history itself always holds paths).
/// Counting back from the newest message, anything beyond `MAX_HISTORY_IMAGES`
/// becomes a placeholder; a failed read (deleted, turned private, over the limit)
/// becomes an "attachment unavailable" line — the model and the user see the same
/// sentence. `cache` keys results by path within one `run()` (the resolution at
/// the head of the turn has already warmed this turn's new attachments into it),
/// so repeated iterations do not re-read the disk. The privacy counts of the
/// fill-in stage are no longer written to the audit log: the resolution at the
/// head of the turn already recorded them, and a duplicate privacy line would only
/// mislead.
pub fn inflate_images(vault: &Vault, messages: &mut [ChatMessage], cache: &mut HashMap<String, Result<String, String>>) {
    let mut budget = MAX_HISTORY_IMAGES;
    for msg in messages.iter_mut().rev() {
        if msg.role != "user" {
            continue;
        }
        let Some(Content::Parts(parts)) = msg.content.as_mut() else { continue };
        for part in parts.iter_mut() {
            let Part::ImageRef { path } = &*part else { continue };
            let path = path.clone();
            *part = if budget == 0 {
                Part::Text { text: format!("(the image {path} was omitted from the context; attach it again if you need it)") }
            } else {
                budget -= 1;
                let url = cache.entry(path.clone()).or_insert_with(|| {
                    let mut ignored = 0usize;
                    read_image(vault, &path, &mut ignored).map(|(mime, bytes)| image_data_url(mime, &bytes))
                });
                match url {
                    Ok(url) => Part::ImageUrl { image_url: ImageUrl { url: url.clone() } },
                    Err(err) => Part::Text { text: crate::prompt::ref_unavailable_line(&path, err) },
                }
            };
        }
    }
}

/// When a request is assembled at the head of a turn, cut the envelope of a text
/// reference that is **private now** out of the message as a whole and replace it
/// with something worded byte for byte like the resolution-time output — the
/// per-turn re-gating of text references, dual to `inflate_images`'s re-gating of
/// images (the fifth privacy choke point). Idempotent and deterministic: with the
/// privacy state unchanged the output bytes are unchanged, so the prefix cache
/// still hits. Locating goes by the `context_refs` metadata (fence_attr is a lossy
/// escaping, and only the metadata knows the real path of a pathological file
/// name), with a second pass over the envelope attributes as a fallback for an
/// existing sidecar that has no metadata. `ImageRef` is not handled here; a
/// reference that already failed at resolution time is a one-line "unavailable"
/// (private and missing worded alike) and needs no cutting.
pub fn scrub_private_refs(vault: &Vault, messages: &mut [ChatMessage]) {
    for msg in messages.iter_mut() {
        if msg.role != "user" {
            continue;
        }
        let Some(text) = msg.content.as_mut().and_then(Content::text_mut) else { continue };
        // One cheap probe rules out the vast majority of messages (ordinary
        // conversation with no @ reference): all four tags share the
        // `-context path="` suffix, and without it only the turn tail is left to look at
        if msg.context_refs.is_empty() && !text.contains("-context path=\"") {
            scrub_note_tail(vault, text);
            continue;
        }
        // Scan the real block structure linearly first (see scan_blocks): both
        // candidates and replacements only recognise top-level blocks, so a
        // literal open tag left in the body (a pasted conversation log, say) is
        // not taken at face value
        let blocks = scan_blocks(text);
        // Candidates = metadata ∪ block attributes (the attr stands in for the
        // path: for an ordinary file name the two are identical; for a
        // pathological one the attr misses the privacy verdict and the metadata
        // catches it)
        let mut candidates: Vec<(RefKind, String)> = msg
            .context_refs
            .iter()
            .filter_map(|r| RefKind::parse(&r.kind).map(|k| (k, r.path.clone())))
            .collect();
        for b in &blocks {
            if let Some(kind) = b.kind {
                candidates.push((kind, b.attr.clone()));
            }
        }
        candidates.sort();
        candidates.dedup();
        // (kind, attr) → the replacement text for the whole block. A folder's
        // line-level re-gating does not enter this table: it ignores the
        // candidates, and every folder block has to go through it (see
        // filter_folder_block)
        let mut replace: Vec<(RefKind, String, String)> = Vec::new();
        for (kind, path) in &candidates {
            // fence_attr is computed only for the one actually being cut (four
            // replaces, each allocating its own)
            match kind {
                RefKind::Note => {
                    let Ok(id) = vault.note_id(path) else { continue };
                    if !vault.is_private(&id) {
                        continue;
                    }
                    let attr = crate::prompt::fence_attr(path);
                    let line = crate::prompt::ref_unavailable_line(&attr, &format!("note not found: {id}"));
                    replace.push((*kind, attr, line));
                }
                // The gate and the wording reuse the ones from resolution time (equal byte for byte)
                RefKind::File | RefKind::Selection | RefKind::Block => {
                    let mut ignored = 0;
                    let Err(err) = gate_private(vault, path, &mut ignored) else { continue };
                    let attr = crate::prompt::fence_attr(path);
                    let line = crate::prompt::ref_unavailable_line(&attr, &err);
                    replace.push((*kind, attr, line));
                }
                RefKind::Folder => {
                    let rel = path.trim().trim_matches('/');
                    if !vault.is_private_rel(rel, true) {
                        continue;
                    }
                    // Resolving a private folder yields "envelope kept, body (no
                    // notes)" (matching what the model gets from list_notes), so
                    // the whole block is rebuilt rather than cut
                    let attr = crate::prompt::fence_attr(path);
                    let replacement = crate::prompt::ref_envelope(
                        kind.tag(),
                        &attr,
                        "",
                        crate::prompt::FOLDER_INTRO,
                        crate::prompt::NO_NOTES,
                    );
                    replace.push((*kind, attr, replacement));
                }
                // image: inflate_images re-gates every turn, so nothing happens here
                // An image has no envelope; a view's rows already went through the
                // privacy-filtered query at resolution time and name no single note;
                // a skill is outside the privacy domain altogether
                RefKind::Image | RefKind::View | RefKind::Skill => {}
            }
        }
        // Replace back to front: the ranges of the earlier blocks do not drift with
        // a replacement, so nothing needs re-scanning and termination is guaranteed
        for b in blocks.iter().rev() {
            let Some(kind) = b.kind else { continue };
            let rep = match replace.iter().find(|(k, a, _)| *k == kind && *a == b.attr) {
                Some((.., r)) => Some(r.clone()),
                None if kind == RefKind::Folder => filter_folder_block(vault, &text[b.start..b.end], &b.attr),
                None if kind == RefKind::Note => rebuild_note_block(vault, &b.attr),
                None if kind == RefKind::Selection => scrub_selection_block(vault, &text[b.start..b.end], &b.attr),
                None if kind == RefKind::Block => rebuild_block_block(vault, &text[b.start..b.end], &b.attr),
                None => None,
            };
            if let Some(rep) = rep {
                text.replace_range(b.start..b.end, &rep);
            }
        }
        scrub_note_tail(vault, text);
    }
}

/// One top-level envelope block in a message (see `scan_blocks`).
struct EnvBlock {
    /// The four reference envelopes carrying a path attribute; None = a web page
    /// envelope — it takes no part in the scrubbing (a web page is outside the
    /// privacy domain) but must still hold its place: its body can hide a literal
    /// open tag just as easily.
    kind: Option<RefKind>,
    /// The `path="…"` attribute as written (already through fence_attr at assembly
    /// time; empty for a web page block).
    attr: String,
    start: usize,
    end: usize,
}

/// Scan the top-level envelope blocks of a message linearly. Envelopes never
/// nest, and a real closer inside a body is neutralised by fence_body, so "scan
/// from the start, take the earliest open tag, jump to the first closer of the
/// same name" recovers exactly the block structure assembly produced; a literal
/// open tag left in a body (a user pasting an agent conversation into a note, of
/// which only the closer is neutralised at injection time) falls inside some real
/// block's [start,end) and is skipped. Without this, the text
/// `<note-context path="X">` appearing inside a public envelope's body would be
/// enough for scrub to pick up a bogus candidate and eat from the middle of the
/// body all the way to the outer envelope's real closer — deleting public content
/// and leaving an unpaired open tag in the request.
fn scan_blocks(text: &str) -> Vec<EnvBlock> {
    let opens: Vec<(Option<RefKind>, String)> =
        [RefKind::Note, RefKind::Folder, RefKind::File, RefKind::Selection, RefKind::Block]
        .into_iter()
        .map(|k| (Some(k), format!("<{} path=\"", k.tag())))
        .chain([(None, "<web-page-context ".to_string())])
        .collect();
    let mut blocks = Vec::new();
    let mut from = 0;
    while from < text.len() {
        let Some((start, kind, open)) = opens
            .iter()
            .filter_map(|(k, open)| text[from..].find(open.as_str()).map(|p| (from + p, *k, open)))
            .min_by_key(|(p, ..)| *p)
        else {
            break;
        };
        let tag = kind.map_or("web-page-context", RefKind::tag);
        let close = format!("</{tag}>");
        let Some(end_rel) = text[start..].find(&close) else { break };
        let end = start + end_rel + close.len();
        // A context block's attribute is the `path="…"` the opener already consumed;
        // a web page block's is its `source="…"`, the first attribute prompt::page_envelope
        // writes. Only `Some(kind)` blocks become scrub candidates, so filling this in
        // for the web block changes nothing there — it is what page_context_urls reads.
        let attr = {
            let (from, quoted) = match kind {
                Some(_) => (start + open.len(), true),
                None => match text[start..end].find("source=\"") {
                    Some(a) => (start + a + "source=\"".len(), true),
                    None => (0, false),
                },
            };
            match quoted.then(|| text[from..end].find('"')).flatten() {
                Some(q) => text[from..from + q].to_string(),
                None => String::new(),
            }
        };
        blocks.push(EnvBlock { kind, attr, start, end });
        from = end;
    }
    blocks
}

/// The URLs of the web pages attached to a turn, read back off its envelope
/// (`<web-page-context source="…"`, assembled by compose_user_message with
/// fence_attr applied, which never touches a URL's characters). Structured by
/// `scan_blocks`, so an open tag sitting inside another envelope's body — a note
/// holding a pasted agent conversation, of which only the closer is neutralised —
/// is skipped instead of being read as a real attachment: a URL taken from one
/// would be marked verified in the source table without the user ever opening it.
pub fn page_context_urls(envelope: &str) -> std::collections::HashSet<String> {
    scan_blocks(envelope)
        .into_iter()
        .filter(|b| b.kind.is_none() && !b.attr.is_empty())
        .map(|b| b.attr)
        .collect()
}

/// Line-level re-gating of a folder envelope: the frozen body is list_notes's
/// `{id}\t{title}` listing, and judging the directory as a whole cannot cut the
/// line of a note that turned private after it was attached — that note's path,
/// title and tags would keep going out with every request, while the same note
/// attached as a note reference would be cleaned. So each line is re-gated by id;
/// cutting them all falls back to "(no notes)", the same shape list_notes would
/// have produced had those notes been private at resolution time. None = there was
/// nothing to cut (the bytes are untouched, so the prefix cache still hits).
fn filter_folder_block(vault: &Vault, block: &str, attr: &str) -> Option<String> {
    let mut kept: Vec<&str> = Vec::new();
    let mut removed = false;
    for line in block.lines() {
        // A body line carries a \t (list_notes's shape); the scaffolding lines
        // (tag, intro, blank lines, "(no notes)") do not, and are regenerated when
        // ref_envelope rebuilds the block
        let Some((id, _)) = line.split_once('\t') else { continue };
        if vault.note_id(id).is_ok_and(|id| vault.is_private(&id)) {
            removed = true;
        } else {
            kept.push(line);
        }
    }
    if !removed {
        return None;
    }
    let body = if kept.is_empty() { crate::prompt::NO_NOTES.to_string() } else { kept.join("\n") };
    Some(crate::prompt::ref_envelope(RefKind::Folder.tag(), attr, "", crate::prompt::FOLDER_INTRO, &body))
}

/// A note envelope whose note now holds private blocks is rebuilt from the
/// same read the resolution used (`read_note` through the redacting gate),
/// so a block that turned private after the note was attached leaves the
/// request from the next turn on. Deterministic for an unchanged disk; a
/// note without private blocks is left byte for byte as it was.
fn rebuild_note_block(vault: &Vault, attr: &str) -> Option<String> {
    let id = vault.note_id(attr).ok()?;
    if !vault.has_private_blocks(&id) {
        return None;
    }
    let mut ignored = 0;
    let body = run_read(vault, "read_note", json!({ "path": attr }), &mut ignored).ok()?;
    let tag = RefKind::Note.tag();
    Some(crate::prompt::ref_envelope(tag, attr, "", crate::prompt::NOTE_INTRO, &crate::prompt::fence_body(&body, tag)))
}

/// A block envelope whose note now holds private blocks is resolved again
/// through the same read the resolution used (`block_for_agent`: private
/// blocks cut from a section, a block that turned private itself
/// nonexistent), so it equals a fresh resolution byte for byte — or becomes
/// the one-line "unavailable" the resolution would have produced. The handle
/// is the envelope's own `block="…"` attribute.
fn rebuild_block_block(vault: &Vault, block: &str, attr: &str) -> Option<String> {
    let id = vault.note_id(attr).ok()?;
    if !vault.has_private_blocks(&id) {
        return None;
    }
    let head = &block[..block.find('>')?];
    let handle = head.rsplit_once(" block=\"")?.1.trim_end_matches('"');
    let mut ignored = 0;
    Some(match resolve_block(vault, attr, handle, &mut ignored) {
        Ok((body, handle)) => {
            let tag = RefKind::Block.tag();
            crate::prompt::ref_envelope(
                tag,
                attr,
                &format!(" block=\"{handle}\""),
                &crate::prompt::block_intro(attr, &handle),
                &crate::prompt::fence_body(&body, tag),
            )
        }
        Err(err) => crate::prompt::ref_unavailable_line(attr, &err),
    })
}

/// A frozen selection envelope has no disk source to rebuild from: when the
/// note now holds a private block and any body line equals one of that
/// block's lines, the whole envelope becomes the one-line "unavailable" —
/// a passage edited *and* wrapped after it was attached is not detected
/// (the same class of limitation as summaries not being re-scrubbed).
fn scrub_selection_block(vault: &Vault, block: &str, attr: &str) -> Option<String> {
    let id = vault.note_id(attr).ok()?;
    if !vault.has_private_blocks(&id) {
        return None;
    }
    let raw = vault.read_note(&id).ok()?;
    let blocks = vemry_core::parser::parse_note(&raw).blocks;
    let private_lines = vemry_core::privacy::private_lines(&raw, &blocks);
    let hit = block.lines().any(|l| !l.trim().is_empty() && private_lines.contains(l.trim()));
    hit.then(|| crate::prompt::ref_unavailable_line(attr, "selection no longer available"))
}

/// The path items of the turn tail (`Currently open note: P`, `Working note: F`):
/// when the note is private now, the whole item is removed — the same shape as
/// the output for a note that was already private at resolution time (where the
/// item never appears). The tail is always at the end of the message and closes
/// with `)`; items are separated by TAIL_SEP.
fn scrub_note_tail(vault: &Vault, text: &mut String) {
    use crate::prompt::{CUR_NOTE_LABEL, FOLLOW_LABEL, TAIL_OPEN};
    if !text.ends_with(')') {
        return;
    }
    // The tail is always the last `\n\n(` (nothing follows it): search from there
    // rather than scanning back over the whole envelope body — a body can run to
    // hundreds of kilobytes, and this runs for every message every turn
    for label in [CUR_NOTE_LABEL, FOLLOW_LABEL] {
        let Some(tail) = text.rfind(TAIL_OPEN) else { return };
        scrub_tail_item(vault, text, tail, label);
    }
}

/// Remove one labelled path item from the tail starting at `tail` when its note
/// is private now: `(A · L: P · B)` → `(A · B)`, `(L: P · B)` → `(B)`, `(A · L: P)`
/// → `(A)`, and a tail holding nothing else disappears entirely.
fn scrub_tail_item(vault: &Vault, text: &mut String, tail: usize, label: &str) {
    use crate::prompt::{TAIL_OPEN, TAIL_SEP};
    let Some(pos) = text[tail..].find(label).map(|i| tail + i) else { return };
    let p_start = pos + label.len();
    let close = text.len() - 1;
    let end = text[p_start..close].find(TAIL_SEP).map(|i| p_start + i).unwrap_or(close);
    let p = text[p_start..end].trim();
    if p.is_empty() || !vault.is_private_rel(p, false) {
        return;
    }
    if text[..pos].ends_with(TAIL_SEP) {
        text.replace_range(pos - TAIL_SEP.len()..end, "");
    } else if end < close && text[end..].starts_with(TAIL_SEP) {
        text.replace_range(pos..end + TAIL_SEP.len(), "");
    } else if text[..pos].ends_with(TAIL_OPEN) {
        text.replace_range(pos - TAIL_OPEN.len().., "");
    }
}

/// A summary request carries no images: an `ImageRef` becomes a one-line
/// "(image path)" — a summary is text, and it is enough for the model to know
/// that an image was there; re-sending it is not worth the cost.
pub fn strip_images(messages: &mut [ChatMessage]) {
    for msg in messages.iter_mut() {
        let Some(Content::Parts(parts)) = msg.content.as_mut() else { continue };
        for part in parts.iter_mut() {
            let text = match &*part {
                Part::ImageRef { path } => format!("(image {path})"),
                Part::ImageUrl { .. } => "(image)".to_string(),
                Part::Text { .. } => continue,
            };
            *part = Part::Text { text };
        }
    }
}

#[cfg(test)]
mod tests {
    /// The one not-found wording, from the same constructor production uses —
    /// asserting against a literal here is exactly how the privacy parity broke.
    fn not_found(rel: &str) -> String {
        vemry_core::reject(vemry_core::PathReject::NotFound, rel).to_string()
    }

    use super::*;

    /// The same privacy fixture as prompt.rs: a glob-private directory plus a
    /// frontmatter-private note.
    fn fixture() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("secret")).unwrap();
        std::fs::create_dir_all(root.join(".vemry")).unwrap();
        std::fs::write(root.join("Hello.md"), "# Hello\n\nworld content\n").unwrap();
        std::fs::write(root.join("secret/Plan.md"), "# Plan\n\nTOPSECRET\n").unwrap();
        std::fs::write(root.join("Diary.md"), "---\nprivate: true\n---\nTOPSECRET\n").unwrap();
        std::fs::write(root.join("notes.txt"), "plain text attachment\n").unwrap();
        std::fs::write(root.join("secret/keys.txt"), "SECRETKEY\n").unwrap();
        std::fs::write(root.join(".vemry/privacy"), "secret/\n").unwrap();
        let vault = Vault::open(root).unwrap();
        (dir, vault)
    }

    fn one(vault: &Vault, r: ContextRef) -> (ResolvedRef, usize) {
        let (mut v, filtered) = resolve_context_refs(vault, &[r], &mut HashMap::new());
        (v.remove(0), filtered)
    }

    #[test]
    fn block_ref_hands_over_block_section_or_subtree() {
        let (_d, vault) = fixture();
        // Hello.md: b0 = the heading, b1 = the paragraph (positional ids count from 0)
        let (para, f) = one(&vault, ContextRef::Block { path: "Hello.md".into(), local_id: "b1".into() });
        assert_eq!(para.kind, RefKind::Block);
        assert_eq!(para.block.as_deref(), Some("b1"));
        assert_eq!(f, 0);
        let para_body = para.body.unwrap();
        assert!(!para_body.is_empty() && !para_body.starts_with('#'), "{para_body}");
        // the heading hands over its whole section, the paragraph included
        let (section, _) = one(&vault, ContextRef::Block { path: "Hello.md".into(), local_id: "b0".into() });
        let body = section.body.unwrap();
        assert!(body.starts_with("# ") && body.contains(&para_body), "{body}");
        // an explicit id is echoed with its caret in the handle
        std::fs::write(vault.root().join("Ids.md"), "first ^k1\n\n- parent ^k2\n  - child\n").unwrap();
        let (first, _) = one(&vault, ContextRef::Block { path: "Ids.md".into(), local_id: "^k1".into() });
        assert_eq!(first.block.as_deref(), Some("^k1"));
        assert_eq!(first.body.as_deref().unwrap(), "first ^k1");
        let (item, _) = one(&vault, ContextRef::Block { path: "Ids.md".into(), local_id: "k2".into() });
        assert!(item.body.as_deref().unwrap().contains("child"));
    }

    #[test]
    fn block_ref_private_note_worded_like_missing_and_stale_id_is_a_plain_miss() {
        let (_d, vault) = fixture();
        let (private_note, f1) = one(&vault, ContextRef::Block { path: "Diary.md".into(), local_id: "b0".into() });
        let (missing_note, f2) = one(&vault, ContextRef::Block { path: "Nope.md".into(), local_id: "b0".into() });
        assert!(private_note.private && f1 == 1);
        // the same sentence a missing note gets (privacy and missing are indistinguishable)
        assert_eq!(
            private_note.body.unwrap_err().replace("Diary.md", "X"),
            missing_note.body.unwrap_err().replace("Nope.md", "X")
        );
        assert!(!missing_note.private && f2 == 0);
        let (stale, f3) = one(&vault, ContextRef::Block { path: "Hello.md".into(), local_id: "b9".into() });
        assert_eq!(stale.body.unwrap_err(), "block not found: Hello.md#b9");
        assert!(!stale.private && f3 == 0);
        let (bad, _) = one(&vault, ContextRef::Block { path: "Hello.md".into(), local_id: "x\"y".into() });
        assert!(bad.body.unwrap_err().starts_with("block not found"));
    }

    #[test]
    fn block_ref_redacts_private_blocks_and_hides_a_private_block_itself() {
        let (_d, vault) = fixture();
        std::fs::write(vault.root().join("Sec.md"), "# H\n\n> [!private] SECRET\n\npublic\n").unwrap();
        // b0 = the heading, b1 = the private block, b2 = the paragraph
        let (section, _) = one(&vault, ContextRef::Block { path: "Sec.md".into(), local_id: "b0".into() });
        let body = section.body.unwrap();
        assert!(body.contains("public") && !body.contains("SECRET"), "{body}");
        let (hidden, f) = one(&vault, ContextRef::Block { path: "Sec.md".into(), local_id: "b1".into() });
        assert_eq!(hidden.body.unwrap_err(), "block not found: Sec.md#b1");
        assert_eq!(f, 0);
    }

    #[test]
    fn note_ref_reads_body() {
        let (_d, vault) = fixture();
        let (r, filtered) = one(&vault, ContextRef::Note { path: "Hello.md".into() });
        assert_eq!(r.kind, RefKind::Note);
        assert!(r.body.unwrap().contains("world content"));
        assert_eq!(filtered, 0);
    }

    /// A private note's error is equal byte for byte to a genuine miss — the
    /// literal test of "private means nonexistent".
    #[test]
    fn private_note_err_identical_to_missing() {
        let (_d, vault) = fixture();
        let (private_glob, f1) = one(&vault, ContextRef::Note { path: "secret/Plan.md".into() });
        let (private_fm, f2) = one(&vault, ContextRef::Note { path: "Diary.md".into() });
        let (missing, f3) = one(&vault, ContextRef::Note { path: "secret/Nope.md".into() });
        let missing_err = missing.body.unwrap_err();
        assert_eq!(private_glob.body.unwrap_err(), missing_err.replace("Nope", "Plan"));
        assert_eq!(private_fm.body.unwrap_err(), missing_err.replace("secret/Nope", "Diary"));
        assert_eq!((f1, f2), (1, 1));
        // Does a genuine miss count into filtered? No: the glob check runs before
        // is_private, so a missing path inside a privacy zone is refused by the
        // read gate on privacy grounds (count 1); a public path that simply does
        // not exist goes to Other and is not counted. secret/Nope.md is inside the
        // zone → counted as 1 all the same.
        assert_eq!(f3, 1);
    }

    #[test]
    fn folder_ref_is_manifest_without_bodies() {
        let (_d, vault) = fixture();
        let (r, _f) = one(&vault, ContextRef::Folder { path: "".into() });
        let body = r.body.unwrap();
        assert!(body.contains("Hello.md"), "the listing holds the visible note paths");
        assert!(!body.contains("world content"), "the listing holds no content");
        assert!(!body.contains("secret"), "private children do not appear");
        assert!(!body.contains("Diary"), "a frontmatter-private note does not appear");
    }

    #[test]
    fn private_folder_manifest_indistinguishable_from_empty() {
        let (_d, vault) = fixture();
        let (private_dir, f1) = one(&vault, ContextRef::Folder { path: "secret".into() });
        let (missing_dir, f2) = one(&vault, ContextRef::Folder { path: "nothere".into() });
        assert_eq!(private_dir.body.unwrap(), missing_dir.body.unwrap());
        assert_eq!(f1, 1, "private children are counted");
        assert_eq!(f2, 0);
    }

    /// The gate must judge privacy on the trimmed rel: validate_rel still reads the
    /// real file for a path with leading whitespace, and a privacy check that looked
    /// at the raw string would be walked around (File/Selection/Image share one
    /// gate, so testing File tests all three).
    #[test]
    fn whitespace_padded_path_cannot_bypass_privacy_gate() {
        let (_d, vault) = fixture();
        let (glob, f1) = one(&vault, ContextRef::File { path: " secret/keys.txt".into() });
        assert_eq!(glob.body.unwrap_err(), not_found("secret/keys.txt"));
        assert_eq!(f1, 1);
        let (fm, f2) = one(&vault, ContextRef::File { path: " Diary.md ".into() });
        assert_eq!(fm.body.unwrap_err(), not_found("Diary.md"));
        assert_eq!(f2, 1);
    }

    /// A note sent as a file takes the note read: read_file knows nothing of
    /// private blocks, and the frontend's kind is not what keeps them out.
    #[test]
    fn file_ref_to_a_note_cuts_its_private_blocks() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Mixed.md"), "public\n\n> [!private] BLOCKSECRET\n").unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let (r, filtered) = one(&vault, ContextRef::File { path: "Mixed.md".into() });
        let body = r.body.unwrap();
        assert!(body.contains("public") && !body.contains("BLOCKSECRET"), "{body}");
        assert_eq!(filtered, 1);
    }

    #[test]
    fn file_ref_reads_text_and_gates_privacy_before_existence() {
        let (_d, vault) = fixture();
        let (ok, f0) = one(&vault, ContextRef::File { path: "notes.txt".into() });
        assert!(ok.body.unwrap().contains("plain text attachment"));
        assert_eq!(f0, 0);
        // A private file is worded like a missing one
        let (private_f, f1) = one(&vault, ContextRef::File { path: "secret/keys.txt".into() });
        let (missing_f, f2) = one(&vault, ContextRef::File { path: "secret/nope.txt".into() });
        assert_eq!(
            private_f.body.unwrap_err(),
            missing_f.body.unwrap_err().replace("nope", "keys")
        );
        assert_eq!((f1, f2), (1, 1));
    }

    #[test]
    fn file_ref_truncates_oversized_text() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("big.txt"), "字".repeat(FILE_CONTEXT_MAX_CHARS + 100)).unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let (r, _f) = one(&vault, ContextRef::File { path: "big.txt".into() });
        let body = r.body.unwrap();
        assert!(body.contains("[... truncated at"));
        assert!(body.chars().count() < FILE_CONTEXT_MAX_CHARS + 200);
    }

    fn sel(path: &str, text: &str) -> ContextRef {
        ContextRef::Selection { path: path.into(), from_line: 3, to_line: 4, text: text.into() }
    }

    #[test]
    fn selection_ref_uses_frontend_text_and_carries_lines() {
        let (_d, vault) = fixture();
        // The body comes from the frontend rather than the disk: Hello.md holds no
        // such text, and the envelope carries it regardless
        let (r, filtered) = one(&vault, sel("Hello.md", "buffer 里未落盘的字"));
        assert_eq!(r.kind, RefKind::Selection);
        assert_eq!(r.lines, Some((3, 4)));
        assert_eq!(r.body.unwrap(), "buffer 里未落盘的字");
        assert_eq!(filtered, 0);
        // Existence is not checked: a newly created, never-saved note can carry a selection too
        let (fresh, f) = one(&vault, sel("Untitled.md", "刚敲的"));
        assert_eq!(fresh.body.unwrap(), "刚敲的");
        assert_eq!(f, 0);
    }

    /// A selection inside a private note is dropped whole: both glob and
    /// frontmatter privacy hit, the wording matches a missing file, and not one
    /// character of the body enters the envelope.
    #[test]
    fn selection_inside_one_block_carries_its_handle() {
        let (_d, vault) = fixture();
        // Hello.md: line 1 the heading (b0), line 3 the paragraph (b1)
        let sel = |from: usize, to: usize| ContextRef::Selection { path: "Hello.md".into(), from_line: from, to_line: to, text: "x".into() };
        assert_eq!(one(&vault, sel(3, 3)).0.block.as_deref(), Some("b1"));
        assert_eq!(one(&vault, sel(1, 1)).0.block.as_deref(), Some("b0"));
        // across two blocks, or past the end: no handle
        assert_eq!(one(&vault, sel(1, 3)).0.block, None);
        assert_eq!(one(&vault, sel(40, 41)).0.block, None);
        // an explicit id keeps its caret; a private block gives none
        std::fs::write(vault.root().join("Ids.md"), "first ^k1\n\n> [!private] SECRET\n").unwrap();
        let (r, _) = one(&vault, ContextRef::Selection { path: "Ids.md".into(), from_line: 1, to_line: 1, text: "first".into() });
        assert_eq!(r.block.as_deref(), Some("^k1"));
        let (r, _) = one(&vault, ContextRef::Selection { path: "Ids.md".into(), from_line: 3, to_line: 3, text: "x".into() });
        assert_eq!(r.block, None);
    }

    #[test]
    fn selection_in_private_note_is_dropped_like_missing() {
        let (_d, vault) = fixture();
        let (glob, f1) = one(&vault, sel("secret/Plan.md", "TOPSECRET 选区"));
        let (fm, f2) = one(&vault, sel("Diary.md", "TOPSECRET 选区"));
        let (missing_file, _) = one(&vault, ContextRef::File { path: "secret/Plan.md".into() });
        assert_eq!(glob.body.unwrap_err(), missing_file.body.unwrap_err());
        assert_eq!(fm.body.unwrap_err(), not_found("Diary.md"));
        assert_eq!((f1, f2), (1, 1));
    }

    #[test]
    fn selection_truncates_and_names_empty() {
        let (_d, vault) = fixture();
        let big = "字".repeat(FILE_CONTEXT_MAX_CHARS + 100);
        let (r, _) = one(&vault, sel("Hello.md", &big));
        let body = r.body.unwrap();
        assert!(body.contains("[... truncated at"));
        assert!(body.chars().count() < FILE_CONTEXT_MAX_CHARS + 200);
        let (e, _) = one(&vault, sel("Hello.md", "  \n"));
        assert_eq!(e.body.unwrap(), "(empty selection)");
    }

    #[test]
    fn refs_capped_at_max() {
        let (_d, vault) = fixture();
        let refs: Vec<ContextRef> =
            (0..MAX_CONTEXT_REFS + 3).map(|i| ContextRef::Note { path: format!("n{i}.md") }).collect();
        let (out, _f) = resolve_context_refs(&vault, &refs, &mut HashMap::new());
        assert_eq!(out.len(), MAX_CONTEXT_REFS);
    }

    /// Build a one-page PDF with text on the fly (the standard Courier font, so
    /// nothing has to be embedded).
    fn one_page_pdf(text: &str) -> Vec<u8> {
        use lopdf::content::{Content, Operation};
        use lopdf::{dictionary, Document, Object, Stream};
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Courier",
        });
        let resources_id = doc.add_object(dictionary! { "Font" => dictionary! { "F1" => font_id } });
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 24.into()]),
                Operation::new("Td", vec![72.into(), 700.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
        });
        let pages = dictionary! {
            "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1,
            "Resources" => resources_id,
            "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
        };
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog_id);
        let mut buf = Vec::new();
        doc.save_to(&mut buf).unwrap();
        buf
    }

    /// A PDF attachment travels as a file reference: the extracted text enters the
    /// envelope; a private PDF is worded like a missing one; a malformed PDF reports
    /// a parse failure instead of taking down the whole turn.
    #[test]
    fn file_ref_extracts_pdf_text_and_gates_privacy() {
        let (_d, vault) = fixture();
        std::fs::write(vault.root().join("paper.PDF"), one_page_pdf("Hello PDF attachment")).unwrap();
        std::fs::write(vault.root().join("secret/paper.pdf"), one_page_pdf("TOPSECRET")).unwrap();
        std::fs::write(vault.root().join("broken.pdf"), b"%PDF-1.5 not really").unwrap();
        let (ok, f0) = one(&vault, ContextRef::File { path: "paper.PDF".into() });
        let body = ok.body.unwrap();
        assert!(body.starts_with("(the text below was extracted from a PDF"), "{body}");
        assert!(body.contains("Hello PDF attachment"), "{body}");
        assert_eq!(f0, 0);
        // The extraction is cached under .vemry/cache/pdf (text + the path it came from); the
        // cache feeds search_notes, and a private PDF's entry is skipped and counted there
        let cache = vault.meta_dir().join(PDF_CACHE_SUBDIR);
        let texts = std::fs::read_dir(&cache).unwrap().filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "txt")).count();
        assert_eq!(texts, 1, "one PDF read, one cached text");
        let mut filtered = 0;
        let cached = cached_pdf_files(&vault, &mut filtered);
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].0, "paper.PDF");
        assert!(std::fs::read_to_string(&cached[0].1).unwrap().contains("Hello PDF attachment"));
        let out = crate::tools::execute_read_tool(&vault, "search_notes", &json!({"query": "hello attachment"})).unwrap();
        // line 1 of the cached text is the page marker, the hit is on line 2
        assert!(out.text.contains("paper.PDF:2:") && out.text.contains("(PDF; read_file for the text)"), "{}", out.text);
        let out = crate::tools::execute_read_tool(&vault, "read_file", &json!({"path": "paper.PDF"})).unwrap();
        assert!(out.text.contains("[p. 1]\nHello PDF attachment"), "{}", out.text);
        // the pages are cached beside the flat text
        assert!(cache.join(format!("{}.pages.json", cached[0].1.file_stem().unwrap().to_string_lossy())).is_file());
        let (private_pdf, f1) = one(&vault, ContextRef::File { path: "secret/paper.pdf".into() });
        let (missing_pdf, f2) = one(&vault, ContextRef::File { path: "secret/nope.pdf".into() });
        assert_eq!(private_pdf.body.unwrap_err(), missing_pdf.body.unwrap_err().replace("nope", "paper"));
        assert_eq!((f1, f2), (1, 1));
        let (broken, _) = one(&vault, ContextRef::File { path: "broken.pdf".into() });
        let err = broken.body.unwrap_err();
        assert!(err.contains("PDF parse failed"), "{err}");
        assert!(!err.contains("does not exist"));
    }

    #[test]
    fn collapse_blank_lines_normalizes_extraction() {
        assert_eq!(collapse_blank_lines("a  \n\n\n\nb\nc\n\n"), "a\n\nb\nc");
        assert_eq!(collapse_blank_lines("  \n \n"), "");
    }

    // ---- image references ----

    /// One public image, one private, one svg and one over the limit; the bytes are
    /// never decoded, so the content is arbitrary.
    fn image_fixture() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("secret")).unwrap();
        std::fs::create_dir_all(root.join(".vemry")).unwrap();
        std::fs::write(root.join("Hello.md"), "# Hello\n").unwrap();
        std::fs::write(root.join("pic.PNG"), b"\x89PNG fake bytes").unwrap();
        std::fs::write(root.join("secret/pic.png"), b"\x89PNG secret").unwrap();
        std::fs::write(root.join("secret/pic.svg"), b"<svg/>").unwrap();
        std::fs::write(root.join("pic.svg"), b"<svg/>").unwrap();
        std::fs::write(root.join("big.png"), vec![0u8; IMAGE_MAX_BYTES + 1]).unwrap();
        std::fs::write(root.join(".vemry/privacy"), "secret/\n").unwrap();
        let vault = Vault::open(root).unwrap();
        (dir, vault)
    }

    fn img(path: &str) -> ContextRef {
        ContextRef::Image { path: path.into() }
    }

    #[test]
    fn image_ref_validates_and_reports_mime_case_insensitively() {
        let (_d, vault) = image_fixture();
        let (r, filtered) = one(&vault, img("pic.PNG"));
        assert_eq!(r.kind, RefKind::Image);
        assert_eq!(r.body.unwrap(), "image/png");
        assert_eq!(filtered, 0);
        assert_eq!(image_mime("a/b.JPEG"), Some("image/jpeg"));
        assert_eq!(image_mime("dir.png/noext"), None, "the extension comes from the file name, not the path");
    }

    /// A private image is worded like a missing one; the format and size gates both
    /// come after the privacy gate.
    #[test]
    fn image_ref_gates_privacy_before_format_and_existence() {
        let (_d, vault) = image_fixture();
        let (private_img, f1) = one(&vault, img("secret/pic.png"));
        let (missing_img, f2) = one(&vault, img("secret/nope.png"));
        assert_eq!(private_img.body.unwrap_err(), missing_img.body.unwrap_err().replace("nope", "pic"));
        assert_eq!((f1, f2), (1, 1));
        // An svg inside a privacy zone: the privacy gate refuses first and does not
        // even say "unsupported format"
        let (private_svg, f3) = one(&vault, img("secret/pic.svg"));
        assert_eq!(private_svg.body.unwrap_err(), not_found("secret/pic.svg"));
        assert_eq!(f3, 1);
        // Outside a zone: the format gate, a miss and the size limit each have their
        // own wording, and none of them counts into filtered
        let (svg, f4) = one(&vault, img("pic.svg"));
        let err = svg.body.unwrap_err();
        assert!(err.contains("unsupported image format"), "{err}");
        assert!(!err.contains("does not exist"));
        let (missing, f5) = one(&vault, img("nope.png"));
        assert!(missing.body.unwrap_err().contains("does not exist"));
        let (big, f6) = one(&vault, img("big.png"));
        assert!(big.body.unwrap_err().contains("image too large"));
        assert_eq!((f4, f5, f6), (0, 0, 0));
    }

    fn user_with_image(path: &str) -> ChatMessage {
        ChatMessage::user_content(Content::Parts(vec![
            Part::Text { text: format!("看 {path}") },
            Part::ImageRef { path: path.into() },
        ]))
    }

    fn part_kind(p: &Part) -> &'static str {
        match p {
            Part::Text { .. } => "text",
            Part::ImageRef { .. } => "image_ref",
            Part::ImageUrl { .. } => "image_url",
        }
    }

    /// Filling in only changes the copy: a readable image becomes a data URL, a
    /// private one becomes "attachment unavailable", and a non-user message is left
    /// alone.
    #[test]
    fn inflate_images_fills_data_url_and_names_failures() {
        let (_d, vault) = image_fixture();
        let mut messages = vec![
            user_with_image("pic.PNG"),
            ChatMessage::tool_result("c1", "tool text"),
            user_with_image("secret/pic.png"),
        ];
        let mut cache = HashMap::new();
        inflate_images(&vault, &mut messages, &mut cache);
        let ok = messages[0].content.as_ref().unwrap().parts().unwrap();
        assert_eq!(part_kind(&ok[0]), "text");
        match &ok[1] {
            Part::ImageUrl { image_url } => {
                assert!(image_url.url.starts_with("data:image/png;base64,"), "{}", image_url.url);
                assert!(image_url.url.len() > "data:image/png;base64,".len());
            }
            other => panic!("expected image_url, got {}", part_kind(other)),
        }
        assert_eq!(messages[1].text(), Some("tool text"));
        let bad = messages[2].content.as_ref().unwrap().parts().unwrap();
        match &bad[1] {
            Part::Text { text } => assert_eq!(*text, format!("(attachment secret/pic.png unavailable: {})", not_found("secret/pic.png"))),
            other => panic!("expected placeholder, got {}", part_kind(other)),
        }
        assert_eq!(cache.len(), 2);
    }

    /// At most 6 images across the history: filled in from the newest backwards,
    /// with only placeholders further back; one path is read once.
    #[test]
    fn inflate_images_caps_history_and_caches_reads() {
        let (_d, vault) = image_fixture();
        let mut messages: Vec<ChatMessage> = (0..MAX_HISTORY_IMAGES + 2).map(|_| user_with_image("pic.PNG")).collect();
        let mut cache = HashMap::new();
        inflate_images(&vault, &mut messages, &mut cache);
        let kinds: Vec<&str> =
            messages.iter().map(|m| part_kind(&m.content.as_ref().unwrap().parts().unwrap()[1])).collect();
        assert_eq!(kinds[0], "text", "the oldest is omitted");
        assert_eq!(kinds[1], "text");
        assert!(kinds[2..].iter().all(|k| *k == "image_url"));
        match &messages[0].content.as_ref().unwrap().parts().unwrap()[1] {
            Part::Text { text } => assert!(text.contains("was omitted from the context"), "{text}"),
            _ => unreachable!(),
        }
        assert_eq!(cache.len(), 1, "one path is read once");
    }

    #[test]
    fn strip_images_leaves_only_text_parts() {
        let mut messages = vec![user_with_image("pic.png"), ChatMessage::user("plain")];
        strip_images(&mut messages);
        let parts = messages[0].content.as_ref().unwrap().parts().unwrap();
        assert!(parts.iter().all(|p| part_kind(p) == "text"));
        match &parts[1] {
            Part::Text { text } => assert_eq!(text, "(image pic.png)"),
            _ => unreachable!(),
        }
        assert_eq!(messages[1].text(), Some("plain"));
    }

    /// Build one envelope message carrying metadata (the same attaching logic the
    /// runner uses).
    fn envelope_msg(vault: &Vault, refs: &[ContextRef], ctx: &crate::prompt::TurnContext) -> ChatMessage {
        let (resolved, _f) = resolve_context_refs(vault, refs, &mut HashMap::new());
        let text = crate::prompt::compose_user_message("看看".into(), None, &resolved, ctx);
        let mut msg = ChatMessage::user(text);
        msg.context_refs = resolved
            .iter()
            .filter(|r| r.body.is_ok())
            .map(|r| crate::types::StoredRef { kind: r.kind.as_str().into(), path: r.path.clone() })
            .collect();
        msg
    }

    #[test]
    fn selection_drops_private_lines_by_disk_and_by_structure() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Mix.md"), "public line\n\n> [!private] head SECRET1\n> body SECRET2\n\nafter\n").unwrap();
        let vault = Vault::open(root).unwrap();
        // (1) a selection starting in the middle of the callout: lines equal to
        // the block's lines on disk are dropped, and the ref keeps its audit path
        let (r, filtered) = one(
            &vault,
            ContextRef::Selection { path: "Mix.md".into(), from_line: 4, to_line: 6, text: "> body SECRET2\n\nafter".into() },
        );
        let body = r.body.unwrap();
        assert!(!body.contains("SECRET") && body.contains("after"), "{body:?}");
        assert_eq!(filtered, 1);
        assert!(!r.private);
        // (2) a private block typed into the selection text itself (not on disk yet)
        let (r, filtered) = one(
            &vault,
            ContextRef::Selection { path: "Mix.md".into(), from_line: 1, to_line: 5, text: "x\n\n> [!private] typed SECRET3\n\ny".into() },
        );
        assert_eq!(r.body.unwrap(), "x\n\ny");
        assert_eq!(filtered, 1);
        // (3) a public selection passes untouched and counts nothing
        let (r, filtered) =
            one(&vault, ContextRef::Selection { path: "Mix.md".into(), from_line: 1, to_line: 1, text: "public line".into() });
        assert_eq!(r.body.unwrap(), "public line");
        assert_eq!(filtered, 0);
        // (4) entirely private: the same empty-selection wording, still delivered
        let (r, _) =
            one(&vault, ContextRef::Selection { path: "Mix.md".into(), from_line: 3, to_line: 4, text: "> [!private] head SECRET1\n> body SECRET2".into() });
        assert_eq!(r.body.unwrap(), "(empty selection)");
    }

    #[test]
    fn scrub_rebuilds_note_and_drops_selection_when_a_block_turns_private() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Doc.md"), "# Doc\n\nkeep\n\nSOON-PRIVATE\n").unwrap();
        let mut vault = Vault::open(root).unwrap();
        let refs = [
            ContextRef::Note { path: "Doc.md".into() },
            ContextRef::Selection { path: "Doc.md".into(), from_line: 5, to_line: 5, text: "SOON-PRIVATE".into() },
            ContextRef::Block { path: "Doc.md".into(), local_id: "b2".into() },
            ContextRef::Block { path: "Doc.md".into(), local_id: "b0".into() },
        ];
        let ctx = crate::prompt::TurnContext::default();
        let mut msgs = vec![envelope_msg(&vault, &refs, &ctx)];
        assert!(msgs[0].text().unwrap().contains("SOON-PRIVATE"));
        assert_eq!(msgs[0].text().unwrap().matches("<block-context ").count(), 2);
        // nothing private yet: the bytes stay untouched
        let before = msgs[0].text().unwrap().to_string();
        scrub_private_refs(&vault, &mut msgs);
        assert_eq!(msgs[0].text().unwrap(), before);
        // the user wraps the line into a private block
        vault.write_note(&vemry_core::NoteId::new("Doc.md"), "# Doc\n\nkeep\n\n> [!private] SOON-PRIVATE\n").unwrap();
        scrub_private_refs(&vault, &mut msgs);
        let text = msgs[0].text().unwrap().to_string();
        assert!(!text.contains("SOON-PRIVATE") && text.contains("keep"), "{text}");
        assert!(text.contains("(attachment Doc.md unavailable: selection no longer available)"));
        // the block that turned private is gone the way a private block is at resolution
        assert!(text.contains("(attachment Doc.md unavailable: block not found: Doc.md#b2)"), "{text}");
        // the rebuilt note and block envelopes equal a fresh resolution byte for byte
        let fresh = envelope_msg(&vault, &refs, &ctx).text().unwrap().to_string();
        let env = |t: &str, tag: &str| {
            let a = t.find(&format!("<{tag}")).unwrap();
            let b = t.find(&format!("</{tag}>")).unwrap() + tag.len() + 3;
            t[a..b].to_string()
        };
        assert_eq!(env(&text, "note-context"), env(&fresh, "note-context"));
        assert_eq!(env(&text, "block-context"), env(&fresh, "block-context"));
        assert!(env(&text, "block-context").contains("# Doc"));
        // idempotent
        scrub_private_refs(&vault, &mut msgs);
        assert_eq!(msgs[0].text().unwrap(), text);
    }

    /// A scrubbed message is equal byte for byte to what compose produces when the
    /// note was private at resolution time — marking something private afterwards
    /// and having it private from the start must send the model the same bytes; and
    /// it is idempotent.
    #[test]
    fn scrub_matches_resolution_time_output_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("zone")).unwrap();
        std::fs::write(root.join("zone/Doc.md"), "# Doc\n\nSENTINEL-BODY\n").unwrap();
        std::fs::write(root.join("zone/att.txt"), "SENTINEL-FILE\n").unwrap();
        let mut vault = Vault::open(root).unwrap();
        let refs = [
            ContextRef::Note { path: "zone/Doc.md".into() },
            ContextRef::Folder { path: "zone".into() },
            ContextRef::File { path: "zone/att.txt".into() },
            ContextRef::Selection { path: "zone/Doc.md".into(), from_line: 1, to_line: 2, text: "SENTINEL-SEL".into() },
        ];
        let ctx = crate::prompt::TurnContext {
            date: "2026-08-26 周三".into(),
            current_note: Some("zone/Doc.md".into()),
            follow_note: None,
        };
        let mut msgs = vec![envelope_msg(&vault, &refs, &ctx)];
        assert!(msgs[0].text().unwrap().contains("SENTINEL-BODY"));

        vault.add_privacy_rule("zone/").unwrap();
        // What it looks like when it was private from the start: every reference
        // "unavailable", and the current-note item absent
        let ctx_private = crate::prompt::TurnContext { date: "2026-08-26 周三".into(), current_note: None, follow_note: None };
        let expect = envelope_msg(&vault, &refs, &ctx_private).text().unwrap().to_string();

        scrub_private_refs(&vault, &mut msgs);
        assert_eq!(msgs[0].text().unwrap(), expect);
        assert!(!msgs[0].text().unwrap().contains("SENTINEL"));
        // Idempotent: running it again leaves the bytes unchanged
        let once = msgs[0].text().unwrap().to_string();
        scrub_private_refs(&vault, &mut msgs);
        assert_eq!(msgs[0].text().unwrap(), once);
    }

    /// fence_attr is lossy (`"` → `'`): the attribute scan misses the real path, and
    /// the metadata catches it. Every character fence_attr rewrites (`"`, `<`, `>`,
    /// newline) is illegal in a Windows file name, so the file cannot exist there and
    /// the case is skipped, like the other legacy-name tests.
    #[cfg(not(windows))]
    #[test]
    fn scrub_handles_pathological_filename_via_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("se\"cret.md"), "# T\n\nSENTINEL-PATHO\n").unwrap();
        let mut vault = Vault::open(root).unwrap();
        let refs = [ContextRef::Note { path: "se\"cret.md".into() }];
        let mut msgs = vec![envelope_msg(&vault, &refs, &crate::prompt::TurnContext::default())];
        assert!(msgs[0].text().unwrap().contains("SENTINEL-PATHO"));
        let id = vault.note_id("se\"cret.md").unwrap();
        vault.set_note_private(&id, true).unwrap();
        scrub_private_refs(&vault, &mut msgs);
        assert!(!msgs[0].text().unwrap().contains("SENTINEL-PATHO"), "{}", msgs[0].text().unwrap());
    }

    /// Only user messages are scrubbed: identical-looking text in an assistant
    /// message (the model quoting it back, say) is left alone.
    #[test]
    fn scrub_leaves_non_user_messages_alone() {
        let (_d, vault) = fixture();
        let fake = "<note-context path=\"secret/Plan.md\">\n(x)\n\nECHOED\n</note-context>";
        let mut msgs = vec![ChatMessage::assistant(Some(fake.into()), None)];
        scrub_private_refs(&vault, &mut msgs);
        assert_eq!(msgs[0].text(), Some(fake));
    }

    /// An existing sidecar (with no metadata) falls back to the attribute scan: an
    /// ordinary file name is still cleaned.
    #[test]
    fn scrub_falls_back_to_attr_scan_without_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("Doc.md"), "# Doc\n\nSENTINEL-LEGACY\n").unwrap();
        let mut vault = Vault::open(root).unwrap();
        let refs = [ContextRef::Note { path: "Doc.md".into() }];
        let mut msg = envelope_msg(&vault, &refs, &crate::prompt::TurnContext::default());
        msg.context_refs = vec![]; // simulate reading back an old sidecar
        let mut msgs = vec![msg];
        let id = vault.note_id("Doc.md").unwrap();
        vault.set_note_private(&id, true).unwrap();
        scrub_private_refs(&vault, &mut msgs);
        assert!(!msgs[0].text().unwrap().contains("SENTINEL-LEGACY"), "{}", msgs[0].text().unwrap());
    }

    /// Line-level re-gating of a folder envelope: a note that turns private after it
    /// was attached has its line (path, title, tags) cut, byte for byte like the
    /// output for a note already private at resolution time; cutting them all falls
    /// back to "(no notes)".
    #[test]
    fn scrub_folder_envelope_drops_newly_private_note_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("projects")).unwrap();
        std::fs::write(root.join("projects/Open.md"), "# Open\n\npublic\n").unwrap();
        std::fs::write(root.join("projects/Secret.md"), "# Secret\n\nhidden\n").unwrap();
        let mut vault = Vault::open(root).unwrap();
        let refs = [ContextRef::Folder { path: "projects".into() }];
        let ctx = crate::prompt::TurnContext::default();
        let mut msgs = vec![envelope_msg(&vault, &refs, &ctx)];
        assert!(msgs[0].text().unwrap().contains("projects/Secret.md"));

        let id = vault.note_id("projects/Secret.md").unwrap();
        vault.set_note_private(&id, true).unwrap();
        let expect = envelope_msg(&vault, &refs, &ctx).text().unwrap().to_string();
        scrub_private_refs(&vault, &mut msgs);
        assert_eq!(msgs[0].text().unwrap(), expect);
        assert!(!msgs[0].text().unwrap().contains("Secret"));
        assert!(msgs[0].text().unwrap().contains("projects/Open.md"));

        // The other one turns private too: everything is cut, matching the shape of
        // an empty listing at resolution time, and it stays idempotent
        let id = vault.note_id("projects/Open.md").unwrap();
        vault.set_note_private(&id, true).unwrap();
        let expect = envelope_msg(&vault, &refs, &ctx).text().unwrap().to_string();
        scrub_private_refs(&vault, &mut msgs);
        assert_eq!(msgs[0].text().unwrap(), expect);
        assert!(msgs[0].text().unwrap().contains(crate::prompt::NO_NOTES));
        let once = msgs[0].text().unwrap().to_string();
        scrub_private_refs(&vault, &mut msgs);
        assert_eq!(msgs[0].text().unwrap(), once);
    }

    /// A literal open tag left in a public envelope's body (a pasted agent
    /// conversation, of which only the closer is neutralised at injection time) is
    /// not taken at face value: it becomes no candidate, and it does not let scrub
    /// tear from the middle of the body to the outer real closer — the message bytes
    /// are untouched.
    #[test]
    fn scrub_ignores_literal_open_tag_inside_envelope_body() {
        let (_d, mut vault) = fixture();
        std::fs::write(
            vault.root().join("Outer.md"),
            "# Outer\n\nbefore\n<note-context path=\"secret/Plan.md\">\nafter\n",
        )
        .unwrap();
        vault.rescan().unwrap();
        let refs = [ContextRef::Note { path: "Outer.md".into() }];
        let mut msgs = vec![envelope_msg(&vault, &refs, &crate::prompt::TurnContext::default())];
        let before = msgs[0].text().unwrap().to_string();
        assert!(before.contains("after"));
        scrub_private_refs(&vault, &mut msgs);
        assert_eq!(msgs[0].text().unwrap(), before);
    }
}
