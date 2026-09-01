//! Agent tools. All domain-specific, no bash: every write has typed
//! parameters so the gate can render a diff and the session log can audit it.
//! Read tools run unguarded; write tools are *prepared* here (into a
//! WriteRequest) and only executed after the permission gate approves.

use serde_json::{json, Value};
use similar::TextDiff;
use vemry_core::{BlockQuery, NoteId, Vault};

use crate::permission::{WriteKind, WriteRequest};
use crate::types::{FunctionSpec, ToolDef};

const READ_NOTE_MAX_CHARS: usize = 30_000;
/// limit is whatever the model asks for, so it needs a hard ceiling — one call
/// must not be able to blow up the context.
const LIST_NOTES_MAX_LIMIT: usize = 500;
const SEARCH_MAX_LIMIT: usize = 100;
const QUERY_BLOCKS_MAX_LIMIT: usize = 200;
/// get_links used to be entirely unbounded (outgoing links plus every backlink):
/// on a popular note a single call was hundreds of lines.
const LINKS_MAX_EACH: usize = 100;

pub fn is_write_tool(name: &str) -> bool {
    matches!(name, "write_note" | "edit_note" | "move_note")
}

pub fn tool_defs() -> Vec<ToolDef> {
    let f = |name: &str, description: &str, parameters: Value| {
        ToolDef::function(FunctionSpec { name: name.into(), description: description.into(), parameters })
    };
    vec![
        f(
            "list_notes",
            "List notes in the vault (path, title, tags). Optionally filter by folder prefix.",
            json!({
                "type": "object",
                "properties": {
                    "folder": {"type": "string", "description": "Only notes under this folder"},
                    "limit": {"type": "integer", "description": "Max results, default 100"}
                }
            }),
        ),
        f(
            "read_note",
            "Read the full content of a note by its vault-relative path.",
            json!({
                "type": "object",
                "properties": {"path": {"type": "string", "description": "e.g. 'projects/Alpha.md'"}},
                "required": ["path"]
            }),
        ),
        f(
            "search_notes",
            "Full-text search across the vault (multi-term AND, works for CJK). Returns path, line and snippet — use it to locate notes before reading.",
            json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer", "description": "Max results, default 20"}
                },
                "required": ["query"]
            }),
        ),
        f(
            "get_links",
            "Show a note's outgoing links (resolved and unresolved) and its backlinks — the local link graph.",
            json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
        ),
        f(
            "query_blocks",
            "Query typed blocks across the vault: tasks, list items, claims, evidence, code blocks, headings. Blocks carry inline `key:: value` fields you can filter on. The same projection API the UI uses.",
            json!({
                "type": "object",
                "properties": {
                    // `private` is deliberately absent: the agent's query never returns private blocks
                    "kind": {"type": "string", "enum": ["paragraph", "heading", "list_item", "task", "claim", "evidence", "code_block"]},
                    "note": {"type": "string", "description": "Restrict to one note path"},
                    "tag": {"type": "string"},
                    "text": {"type": "string", "description": "Substring filter on block content"},
                    "task_state": {"type": "string", "enum": ["todo", "doing", "done"]},
                    "due_before": {"type": "string", "description": "YYYY-MM-DD: tasks due on or before this date"},
                    "field": {"type": "string", "description": "Inline-field filter: `key` (present), `key = v`, `key != v`, `key < v`, `key > v`, `key ~ v` (contains); YYYY-MM-DD values compare as dates, numbers as numbers"},
                    "limit": {"type": "integer"}
                }
            }),
        ),
        f(
            "read_skill",
            "Read the full instructions of a skill from the Skills list in the system prompt. Call it before doing the matching task.",
            json!({
                "type": "object",
                "properties": {"name": {"type": "string", "description": "Skill name exactly as listed"}},
                "required": ["name"]
            }),
        ),
        f(
            "write_note",
            "Create a new note or fully overwrite an existing one. Requires user approval. Prefer edit_note for changes to existing notes.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Vault-relative path, e.g. 'ideas/New Idea.md'"},
                    "content": {"type": "string", "description": "Full Markdown content"}
                },
                "required": ["path", "content"]
            }),
        ),
        f(
            "edit_note",
            "Replace an exact text fragment in a note. old_str must match exactly once — include enough surrounding context. Requires user approval.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "old_str": {"type": "string", "description": "Exact existing text (unique in the note)"},
                    "new_str": {"type": "string", "description": "Replacement text"}
                },
                "required": ["path", "old_str", "new_str"]
            }),
        ),
        f(
            "move_note",
            "Move or rename a note to a new vault-relative path. Does not rewrite links (wiki-links by the old name become unresolved). Requires user approval.",
            json!({
                "type": "object",
                "properties": {
                    "from": {"type": "string", "description": "Current path, e.g. 'inbox/Idea.md'"},
                    "to": {"type": "string", "description": "New path including .md, e.g. 'projects/Idea.md'"}
                },
                "required": ["from", "to"]
            }),
        ),
    ]
}

/// The plan confirmation tool (injected only into sessions where
/// RunnerConfig.plan_confirm is on; like web_search, an addition to the tool
/// table that is frozen per session — so the prefix stays stable). Not part of
/// the nine in `tool_defs()`: it never touches the vault, and executing it means
/// the runner suspending at the plan gate rather than going through the
/// read/write funnel.
pub fn propose_plan_def() -> ToolDef {
    ToolDef::function(FunctionSpec {
        name: "propose_plan".into(),
        description: "Propose a short plan (3-7 steps) for a multi-step task and wait for the \
            user's decision before executing. Call it alone, before any other tool. The result \
            says whether the user approved, edited, or rejected the plan."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Plan steps in the user's language, one short phrase each"
                }
            },
            "required": ["items"]
        }),
    })
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, ToolError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::Other(format!("missing required argument '{key}'")))
}

fn arg_usize(args: &Value, key: &str, default: usize) -> usize {
    args.get(key).and_then(|v| v.as_u64()).map(|v| v as usize).unwrap_or(default)
}

/// Result of a read tool: the result text plus how many results the privacy
/// scope removed (audited as a count only, never as paths).
#[derive(Debug)]
pub struct ReadOutcome {
    pub text: String,
    pub filtered: usize,
}

/// Tool-level error. `Privacy` messages are word-for-word identical to the
/// real not-found/invalid-path errors — "no permission" would leak that the
/// path exists — but the variant lets the runner audit that privacy fired.
#[derive(Debug)]
pub enum ToolError {
    Privacy(String),
    Other(String),
}

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ToolError::Privacy(m) | ToolError::Other(m) => f.write_str(m),
        }
    }
}

/// Execute a read-level tool. Error text is sent back to the model as an
/// error tool result so it can adjust. Privacy-zone paths behave exactly as
/// if they did not exist.
pub fn execute_read_tool(vault: &Vault, name: &str, args: &Value) -> Result<ReadOutcome, ToolError> {
    match name {
        "list_notes" => {
            let folder = args.get("folder").and_then(|v| v.as_str()).unwrap_or("").trim_matches('/');
            let limit = arg_usize(args, "limit", 100).min(LIST_NOTES_MAX_LIMIT);
            // partition before take: `filtered` audits the whole listing, and
            // a lazy filter stops counting once the window fills
            let (visible, hidden): (Vec<_>, Vec<_>) = vault
                .index()
                .summaries()
                .into_iter()
                .filter(|s| folder.is_empty() || s.id.as_str().starts_with(&format!("{folder}/")))
                .partition(|s| !vault.is_private(&s.id));
            let filtered = hidden.len();
            let mut lines: Vec<String> = visible
                .into_iter()
                .take(limit)
                .map(|s| {
                    let tags = if s.tags.is_empty() { String::new() } else { format!("  [{}]", s.tags.join(", ")) };
                    format!("{}\t{}{}", s.id, s.title, tags)
                })
                .collect();
            if lines.is_empty() {
                lines.push(crate::prompt::NO_NOTES.into());
            }
            Ok(ReadOutcome { text: lines.join("\n"), filtered })
        }
        "read_note" => {
            let id = vault.note_id(arg_str(args, "path")?).map_err(|e| ToolError::Other(e.to_string()))?;
            if vault.is_private(&id) {
                return Err(ToolError::Privacy(format!("note not found: {id}")));
            }
            // index membership, not a bare fs read: hidden dirs the index
            // skips must not be readable through the agent either
            if !vault.index().contains(&id) {
                return Err(ToolError::Other(format!("note not found: {id}")));
            }
            // The agent's read: private blocks removed (privacy.rs), counted as filtered
            let red = vault.read_note_for_agent(&id).map_err(|e| ToolError::Other(e.to_string()))?;
            let filtered = red.count();
            let content = red.text;
            // The truncation note points at a way to read on: the block index is
            // exactly the "read a part on demand" API
            let text = match crate::truncate_chars(
                &content,
                READ_NOTE_MAX_CHARS,
                &format!(
                    "[... truncated at {READ_NOTE_MAX_CHARS} chars — use query_blocks(note=\"{id}\") to read the rest in blocks]"
                ),
            ) {
                Some(t) => t,
                None if content.is_empty() => "(empty note)".into(),
                None => content,
            };
            Ok(ReadOutcome { text, filtered })
        }
        "search_notes" => {
            let query = arg_str(args, "query")?;
            let limit = arg_usize(args, "limit", 20).min(SEARCH_MAX_LIMIT);
            // overfetch so privacy filtering still fills the window; partition
            // before take so `filtered` counts every private hit in the fetch
            // search_for_agent re-matches notes with private blocks over a
            // blanked copy, so no snippet and no AND-match comes from a private line
            let (hits, hidden): (Vec<_>, Vec<_>) = vault
                .search_for_agent(query, limit * 4 + 20)
                .into_iter()
                .partition(|h| !vault.is_private(&h.note));
            let filtered = hidden.len();
            let hits: Vec<_> = hits.into_iter().take(limit).collect();
            if hits.is_empty() {
                return Ok(ReadOutcome { text: format!("no results for '{query}'"), filtered });
            }
            let text = hits
                .into_iter()
                .map(|h| format!("{}:{}: {}", h.note, h.line, h.snippet))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(ReadOutcome { text, filtered })
        }
        "get_links" => {
            let id = vault.note_id(arg_str(args, "path")?).map_err(|e| ToolError::Other(e.to_string()))?;
            if vault.is_private(&id) {
                return Err(ToolError::Privacy(format!("note not found: {id}")));
            }
            if !vault.index().contains(&id) {
                return Err(ToolError::Other(format!("note not found: {id}")));
            }
            let mut filtered = 0;
            let mut out = String::from("Outgoing links:\n");
            let links = vault.index().links_out(&id);
            if links.is_empty() {
                out.push_str("  (none)\n");
            }
            for (edge, _) in links.iter().take(LINKS_MAX_EACH) {
                match &edge.resolved {
                    // a link into the privacy zone reads as dangling — the
                    // raw target text is already visible in the note body,
                    // but not the fact that it resolves
                    Some(t) if vault.is_private(t) => {
                        filtered += 1;
                        out.push_str(&format!("  [[{}]] -> UNRESOLVED\n", edge.raw_target));
                    }
                    Some(t) => out.push_str(&format!("  [[{}]] -> {}\n", edge.raw_target, t)),
                    None => out.push_str(&format!("  [[{}]] -> UNRESOLVED\n", edge.raw_target)),
                }
            }
            if links.len() > LINKS_MAX_EACH {
                out.push_str(&format!("  (+{} more not shown)\n", links.len() - LINKS_MAX_EACH));
            }
            out.push_str("Backlinks:\n");
            let bl: Vec<_> = vault
                .backlinks(&id)
                .into_iter()
                .filter(|b| {
                    let private = vault.is_private(&b.source);
                    filtered += private as usize;
                    !private
                })
                .collect();
            if bl.is_empty() {
                out.push_str("  (none)\n");
            }
            let bl_total = bl.len();
            for b in bl.into_iter().take(LINKS_MAX_EACH) {
                out.push_str(&format!("  {} — {}\n", b.source, b.snippet));
            }
            if bl_total > LINKS_MAX_EACH {
                out.push_str(&format!("  (+{} more not shown)\n", bl_total - LINKS_MAX_EACH));
            }
            Ok(ReadOutcome { text: out, filtered })
        }
        "query_blocks" => {
            let mut q: BlockQuery =
                serde_json::from_value(args.clone()).map_err(|e| ToolError::Other(format!("bad query: {e}")))?;
            q.limit = Some(q.limit.unwrap_or(QUERY_BLOCKS_MAX_LIMIT).min(QUERY_BLOCKS_MAX_LIMIT));
            let mut hits = vault.index().query(&q);
            let before = hits.len();
            // Whole private notes, then private blocks: both are nonexistent to
            // the agent (a `kind: private` query therefore finds nothing).
            // Positional ids are not renumbered around a removed block — one id
            // space for the UI, the editor and id minting matters more than a
            // gapless sequence
            hits.retain(|h| !vault.is_private(&h.note) && !matches!(h.block.kind, vemry_core::BlockKind::Private));
            let filtered = before - hits.len();
            if hits.is_empty() {
                return Ok(ReadOutcome { text: "(no matching blocks)".into(), filtered });
            }
            let out: Vec<Value> = hits
                .into_iter()
                .map(|h| {
                    let content: String = h.block.content.chars().take(300).collect();
                    json!({
                        "note": h.note.as_str(),
                        "block": h.block.local_id,
                        "kind": h.block.kind,
                        "content": content,
                    })
                })
                .collect();
            let text = serde_json::to_string_pretty(&out).map_err(|e| ToolError::Other(e.to_string()))?;
            Ok(ReadOutcome { text, filtered })
        }
        "read_skill" => {
            let name = arg_str(args, "name")?;
            // resolves against the scanned listing only (no path join) — the
            // narrow, deliberate exception to `.vemry` being agent-invisible
            match crate::skills::read_skill(&vault.root().join(".vemry"), name) {
                Some(text) if !text.is_empty() => Ok(ReadOutcome { text, filtered: 0 }),
                Some(_) => Ok(ReadOutcome { text: "(empty skill)".into(), filtered: 0 }),
                None => Err(ToolError::Other(format!("skill not found: {name}"))),
            }
        }
        _ => Err(ToolError::Other(format!("unknown tool: {name}"))),
    }
}

/// Gate order for a note that already exists, shared by edit_note and the source
/// side of move_note: fold the case onto the indexed id → privacy before
/// existence → anything outside the index (hidden directories, symlinks) is
/// unreachable. Both errors echo the caller's spelling back — the canonical
/// casing would confirm that a private note exists.
fn existing_note_id(vault: &Vault, path: &str) -> Result<NoteId, ToolError> {
    let requested = vault.note_id(path).map_err(|e| ToolError::Other(e.to_string()))?;
    let id = vault.index().canonical_id(&requested).unwrap_or_else(|| requested.clone());
    if vault.is_private(&id) {
        return Err(ToolError::Privacy(format!("note not found: {requested}")));
    }
    if !vault.index().contains(&id) {
        return Err(ToolError::Other(format!("note not found: {requested}")));
    }
    Ok(id)
}

/// Prepare a write tool into a WriteRequest for the permission gate. Nothing
/// touches disk here. Privacy-zone paths are rejected before any diff is
/// generated and never reach the permission dialog.
pub fn prepare_write(
    vault: &Vault,
    request_id: String,
    name: &str,
    args: &Value,
    session: Option<&str>,
) -> Result<WriteRequest, ToolError> {
    match name {
        "write_note" => {
            let path = arg_str(args, "path")?;
            let content = arg_str(args, "content")?;
            let requested = vault.note_id(path).map_err(|e| ToolError::Other(e.to_string()))?;
            // Canonicalize case-variant spellings to the indexed id: the FS
            // folds "hello.md" onto Hello.md, so the privacy check, the diff
            // baseline and the index update must all target the real note —
            // not a phantom "create" whose approval silently overwrites it.
            let id = vault.index().canonical_id(&requested).unwrap_or(requested);
            // covers creating NEW files inside a private folder: the glob
            // rules match paths that do not exist yet
            if vault.is_private(&id) {
                return Err(ToolError::Privacy(format!("invalid path: {path}")));
            }
            let is_new = !vault.index().contains(&id);
            // On disk but not indexed: a pre-existing symlink (fs::write would
            // follow it, possibly out of the vault) or a file the watcher has
            // not indexed yet. Refuse rather than blind-overwrite behind a
            // "create" diff whose baseline would read as empty.
            if is_new && std::fs::symlink_metadata(vault.abs_path(&id)).is_ok() {
                return Err(ToolError::Other(format!("invalid path: {path}")));
            }
            // Portability before the permission gate: an illegal Windows
            // destination goes straight back to the model to correct, with no diff
            // rendered and no dialog raised (move_note's destination takes the same
            // path). Only new paths are checked — an existing file stays writable
            // whatever it is called
            if is_new {
                vault.check_portable_rel(id.as_str()).map_err(|e| ToolError::Other(e.to_string()))?;
            }
            let old = if is_new { String::new() } else { vault.read_note(&id).unwrap_or_default() };
            // A note holding private blocks cannot be overwritten whole: the model
            // never saw those blocks, so its "full content" would delete them. The
            // refusal is a detectable difference (the model learns something is
            // hidden, never what) — the same class as a private path erroring
            // where a public one opens a dialog.
            if !is_new && vemry_core::has_private_blocks(&vemry_core::parser::parse_note(&old).blocks) {
                return Err(ToolError::Privacy(format!(
                    "write_note is not available for {path}; use edit_note for changes to this note"
                )));
            }
            let new_content = if is_new { with_agent_provenance(content, session) } else { content.to_string() };
            let diff = unified_diff(&old, &new_content, id.as_str());
            let kind = if is_new { WriteKind::Create } else { WriteKind::Overwrite };
            Ok(WriteRequest { id: request_id, tool: name.into(), path: id.to_string(), to_path: None, diff, kind, new_content })
        }
        "edit_note" => {
            let path = arg_str(args, "path")?;
            let old_str = arg_str(args, "old_str")?;
            let new_str = arg_str(args, "new_str")?;
            // Gate order in existing_note_id (never a bare fs read: the hidden
            // directories and symlinks the index skips are just as unreachable to
            // the agent)
            let id = existing_note_id(vault, path)?;
            let content = vault.read_note(&id).map_err(|e| ToolError::Other(e.to_string()))?;
            // old_str must match exactly once in what the model could see: an
            // occurrence inside (or straddling) a private block does not count,
            // and the same "not found" wording covers it. The replacement is
            // applied to the raw text; the diff the user sees is over the raw text.
            let blocks = vemry_core::parser::parse_note(&content).blocks;
            let redacted = vemry_core::redact_private_blocks(&content, &blocks);
            let occurrences = vemry_core::public_occurrences(&content, old_str, &redacted.removed);
            let count = occurrences.len();
            if count == 0 {
                return Err(ToolError::Other(
                    "old_str not found in note — read the note again and copy the exact text".into(),
                ));
            }
            if count > 1 {
                return Err(ToolError::Other(format!(
                    "old_str matches {count} times — include more surrounding context to make it unique"
                )));
            }
            let at = occurrences[0];
            let new_content = format!("{}{}{}", &content[..at], new_str, &content[at + old_str.len()..]);
            let diff = unified_diff(&content, &new_content, id.as_str());
            Ok(WriteRequest {
                id: request_id,
                tool: name.into(),
                path: id.to_string(),
                to_path: None,
                diff,
                kind: WriteKind::Edit,
                new_content,
            })
        }
        "move_note" => {
            let from = arg_str(args, "from")?;
            let to = arg_str(args, "to")?;
            // The source-side gate order is shared with edit_note (see existing_note_id)
            let from_id = existing_note_id(vault, from)?;
            let to_id = vault.note_id(to).map_err(|e| ToolError::Other(e.to_string()))?;
            // Privacy before existence for the destination: checking "already
            // exists" first would let a collision confirm that a private note of
            // that name exists; and the agent should not be able to move content
            // out of its own sight
            if vault.is_private(&to_id) {
                return Err(ToolError::Privacy(format!("invalid path: {to}")));
            }
            if vault.index().canonical_id(&to_id).is_some() {
                return Err(ToolError::Other(format!("already exists: {to_id}")));
            }
            // On disk but not indexed (a symlink or hidden directory): refused the
            // same way write_note refuses it
            if std::fs::symlink_metadata(vault.abs_path(&to_id)).is_ok() {
                return Err(ToolError::Other(format!("invalid path: {to}")));
            }
            // Portability before the permission gate: an illegal Windows file name
            // goes straight back to the model to correct
            vault.check_portable_rel(to_id.as_str()).map_err(|e| ToolError::Other(e.to_string()))?;
            Ok(WriteRequest {
                id: request_id,
                tool: name.into(),
                path: from_id.to_string(),
                to_path: Some(to_id.to_string()),
                diff: String::new(),
                kind: WriteKind::Move,
                new_content: String::new(),
            })
        }
        _ => Err(ToolError::Other(format!("unknown write tool: {name}"))),
    }
}

/// Execute an approved WriteRequest (possibly with user-edited content). The
/// blocks the write adds are recorded in the provenance ledger under
/// `session` (the audit log's id), pending the user's review.
pub fn execute_write(
    vault: &mut Vault,
    req: &WriteRequest,
    content_override: Option<&str>,
    session: Option<&str>,
) -> Result<String, String> {
    if req.tool == "move_note" {
        // AllowEdited means nothing for a move (there is no content to edit), so
        // the override is defensively ignored
        return execute_move(vault, req);
    }
    let id = vault.note_id(&req.path).map_err(|e| e.to_string())?;
    // Canonicalize again: the index can change during the gate await (e.g. a
    // note appearing at this path), and the write must land on the real note.
    let id = vault.index().canonical_id(&id).unwrap_or(id);
    // Re-check privacy at execute time, under the vault lock. The permission
    // gate awaits with the lock released, so the note can turn private in the
    // meantime (the user marks it private, adds a rule, or the watcher reloads
    // .vemry/privacy). The prepare-time decision is stale; never write into a
    // path that is private now — including user-edited (AllowEdited) content.
    // The error echoes req.path, never the canonical casing.
    if vault.is_private(&id) {
        return Err(format!("note not found: {}", req.path));
    }
    let old = vault.read_note(&id).unwrap_or_default();
    // The same re-check for a private block that appeared during the gate:
    // an overwrite would delete it
    if req.tool == "write_note" && vemry_core::has_private_blocks(&vemry_core::parser::parse_note(&old).blocks) {
        return Err(format!("write_note is not available for {}; use edit_note for changes to this note", req.path));
    }
    let content = content_override.unwrap_or(&req.new_content);
    vault.write_note(&id, content).map_err(|e| e.to_string())?;
    // Provenance is computed from the applied bytes (an AllowEdited override
    // included), as a multiset difference of block hashes: a block the model
    // merely moved is not new, a block it changed is
    let added = vemry_core::added_blocks(&old, content);
    if !added.is_empty() {
        vault.record_provenance(vemry_core::ProvenanceEntry::Agent {
            ts: vemry_core::provenance::now_ts(),
            session: session.map(String::from),
            path: id.to_string(),
            blocks: added,
        });
    }
    Ok(format!("wrote {id} ({} bytes)", content.len()))
}

/// move_note's execution side: both the index and privacy can change while the
/// permission gate is awaited, so both sides are re-checked (the same spirit as
/// execute_write's TOCTOU re-check); the rename itself goes through
/// `rename_path`.
fn execute_move(vault: &mut Vault, req: &WriteRequest) -> Result<String, String> {
    let to_path = req.to_path.as_deref().ok_or("missing move target")?;
    let from = vault.note_id(&req.path).map_err(|e| e.to_string())?;
    let from = vault.index().canonical_id(&from).unwrap_or(from);
    if vault.is_private(&from) {
        return Err(format!("note not found: {}", req.path));
    }
    let to = vault.note_id(to_path).map_err(|e| e.to_string())?;
    if vault.is_private(&to) {
        return Err(format!("invalid path: {to_path}"));
    }
    if vault.index().canonical_id(&to).is_some() {
        return Err(format!("already exists: {to}"));
    }
    let landed = vault.rename_path(from.as_str(), to.as_str()).map_err(|e| e.to_string())?;
    Ok(format!("moved {from} -> {landed}"))
}

/// New agent-created notes get provenance frontmatter (origin and the
/// session that made them) unless the agent already provided frontmatter
/// of its own.
fn with_agent_provenance(content: &str, session: Option<&str>) -> String {
    if content.starts_with("---\n") || content.starts_with("---\r\n") {
        return content.to_string();
    }
    match session {
        Some(s) => format!("---\nvemry:\n  origin: agent\n  agent_session: {s}\n---\n{content}"),
        None => format!("---\nvemry:\n  origin: agent\n---\n{content}"),
    }
}

pub fn unified_diff(old: &str, new: &str, path: &str) -> String {
    if old == new {
        return String::new();
    }
    TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vault with one public note, one glob-private note (`.vemry/privacy`
    /// rule `secret/`), one frontmatter-private note and one hidden-dir note.
    /// All private content carries the sentinel string TOPSECRET.
    fn fixture() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("secret")).unwrap();
        std::fs::create_dir_all(root.join(".vemry")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("Hello.md"), "# Hello\n\n参考 [[secret/Plan]] 与 [[Diary]]。 #public\n").unwrap();
        std::fs::write(root.join("secret/Plan.md"), "# Plan\n\nTOPSECRET glob content #leaktag\n").unwrap();
        std::fs::write(
            root.join("Diary.md"),
            "---\nprivate: true\ntags: [leaktag]\n---\n# Diary\n\nTOPSECRET diary, see [[Hello]]\n",
        )
        .unwrap();
        std::fs::write(root.join(".hidden/Sneaky.md"), "TOPSECRET hidden\n").unwrap();
        std::fs::write(root.join(".vemry/privacy"), "secret/\n").unwrap();
        let vault = Vault::open(root).unwrap();
        (dir, vault)
    }

    fn read(vault: &Vault, name: &str, args: Value) -> Result<ReadOutcome, ToolError> {
        execute_read_tool(vault, name, &args)
    }

    #[test]
    fn read_skill_reads_only_listed_skills() {
        let (_d, vault) = fixture();
        let skills = vault.root().join(".vemry/skills");
        std::fs::create_dir_all(&skills).unwrap();
        std::fs::write(skills.join("weekly.md"), "---\ndescription: d\n---\n周报完整流程\n").unwrap();
        std::fs::write(skills.join("off.md"), "---\ndisabled: true\n---\nHIDDEN\n").unwrap();

        let out = read(&vault, "read_skill", json!({"name": "weekly"})).unwrap();
        assert_eq!(out.text, "周报完整流程");
        // missing and disabled report identically
        let e1 = read(&vault, "read_skill", json!({"name": "off"})).unwrap_err();
        let e2 = read(&vault, "read_skill", json!({"name": "ghost"})).unwrap_err();
        assert_eq!(e1.to_string(), "skill not found: off");
        assert_eq!(e2.to_string(), "skill not found: ghost");
        // path-shaped names cannot escape the skills dir: the privacy-zone
        // sentinel stays unreachable through this tool
        for name in ["../privacy", "../../secret/Plan", "../../Diary", "skills/weekly"] {
            let err = read(&vault, "read_skill", json!({"name": name})).unwrap_err();
            assert!(err.to_string().starts_with("skill not found:"));
            assert!(!err.to_string().contains("TOPSECRET"));
        }
        // read_note still cannot reach into .vemry even though skills can be
        // read through their own tool — the exception stays narrow
        let err = read(&vault, "read_note", json!({"path": ".vemry/skills/weekly.md"})).unwrap_err();
        assert!(err.to_string().contains("note not found"));
    }

    #[test]
    fn list_notes_hides_private() {
        let (_d, vault) = fixture();
        let out = read(&vault, "list_notes", json!({})).unwrap();
        assert!(out.text.contains("Hello.md"));
        assert!(!out.text.contains("secret"));
        assert!(!out.text.contains("Diary"));
        assert_eq!(out.filtered, 2);
        // folder listing of a fully-private dir looks empty, not forbidden
        let out = read(&vault, "list_notes", json!({"folder": "secret"})).unwrap();
        assert_eq!(out.text, "(no notes)");
    }

    #[test]
    fn filtered_count_survives_limit() {
        // The audit count must cover the whole result set, not stop once the
        // visible window fills (the pre-partition pipeline under-reported).
        let (_d, vault) = fixture();
        // summaries() sorts by id: Diary.md (private) < Hello.md < secret/Plan.md
        let out = read(&vault, "list_notes", json!({"limit": 1})).unwrap();
        assert!(out.text.contains("Hello.md"));
        assert_eq!(out.filtered, 2, "private notes past the window must still be counted");

        // search: equal scores tie-break by id, so the public hit fills the
        // window before the private hit is ever reached
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("A.md"), "needle here\n").unwrap();
        std::fs::write(dir.path().join("Z.md"), "---\nprivate: true\n---\nneedle here\n").unwrap();
        let vault = Vault::open(dir.path()).unwrap();
        let out = read(&vault, "search_notes", json!({"query": "needle", "limit": 1})).unwrap();
        assert!(out.text.contains("A.md"));
        assert_eq!(out.filtered, 1, "private hit past the window must still be counted");
    }

    #[test]
    fn read_note_private_indistinguishable_from_missing() {
        let (_d, vault) = fixture();
        let glob = read(&vault, "read_note", json!({"path": "secret/Plan.md"})).unwrap_err();
        assert!(matches!(glob, ToolError::Privacy(_)));
        assert_eq!(glob.to_string(), "note not found: secret/Plan.md");
        let fm = read(&vault, "read_note", json!({"path": "Diary.md"})).unwrap_err();
        assert!(matches!(fm, ToolError::Privacy(_)));
        assert_eq!(fm.to_string(), "note not found: Diary.md");
        // a genuinely missing note produces the same wording
        let missing = read(&vault, "read_note", json!({"path": "missing/Nope.md"})).unwrap_err();
        assert_eq!(missing.to_string(), "note not found: missing/Nope.md");
        // hidden-dir bare-fs read hole is closed
        let hidden = read(&vault, "read_note", json!({"path": ".hidden/Sneaky.md"})).unwrap_err();
        assert!(hidden.to_string().starts_with("note not found:"));
    }

    #[test]
    fn search_never_surfaces_private_hits() {
        let (_d, vault) = fixture();
        let out = read(&vault, "search_notes", json!({"query": "TOPSECRET"})).unwrap();
        assert!(out.text.starts_with("no results"));
        assert_eq!(out.filtered, 2);
        // public results unaffected
        let out = read(&vault, "search_notes", json!({"query": "Hello"})).unwrap();
        assert!(out.text.contains("Hello.md"));
    }

    #[test]
    fn get_links_masks_private_targets_and_backlinks() {
        let (_d, vault) = fixture();
        let out = read(&vault, "get_links", json!({"path": "Hello.md"})).unwrap();
        assert!(out.text.contains("[[secret/Plan]] -> UNRESOLVED"));
        assert!(out.text.contains("[[Diary]] -> UNRESOLVED"));
        assert!(!out.text.contains("-> secret/Plan.md"));
        assert!(!out.text.contains("-> Diary.md"));
        assert!(!out.text.contains("Diary —"), "backlink from private note must be dropped");
        assert_eq!(out.filtered, 3); // 2 masked targets + 1 dropped backlink
        // asking for a private note's links: not found
        let err = read(&vault, "get_links", json!({"path": "Diary.md"})).unwrap_err();
        assert!(matches!(err, ToolError::Privacy(_)));
        assert_eq!(err.to_string(), "note not found: Diary.md");
    }

    /// A public note holding a `> [!private]` block (sentinel BLOCKSECRET) next
    /// to a plain one: the agent's reads never show the block.
    fn fixture_with_private_block() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(
            root.join("Mixed.md"),
            "# Mixed\n\npublic apple\n\n> [!private] BLOCKSECRET banana apple\n\ntail apple\n",
        )
        .unwrap();
        std::fs::write(root.join("Plain.md"), "plain\n").unwrap();
        let vault = Vault::open(root).unwrap();
        (dir, vault)
    }

    #[test]
    fn read_note_redacts_private_blocks() {
        let (_d, vault) = fixture_with_private_block();
        let out = read(&vault, "read_note", json!({"path": "Mixed.md"})).unwrap();
        assert_eq!(out.text, "# Mixed\n\npublic apple\n\ntail apple\n");
        assert_eq!(out.filtered, 1);
    }

    #[test]
    fn search_notes_never_snippets_private_blocks() {
        let (_d, vault) = fixture_with_private_block();
        let out = read(&vault, "search_notes", json!({"query": "banana"})).unwrap();
        assert_eq!(out.text, "no results for 'banana'");
        // the AND oracle is closed: a public term plus a private-only term finds nothing
        let out = read(&vault, "search_notes", json!({"query": "apple banana"})).unwrap();
        assert!(out.text.starts_with("no results"));
        let out = read(&vault, "search_notes", json!({"query": "apple"})).unwrap();
        assert_eq!(out.text, "Mixed.md:3: public apple");
    }

    #[test]
    fn query_blocks_drops_private_blocks() {
        let (_d, vault) = fixture_with_private_block();
        let out = read(&vault, "query_blocks", json!({"text": "BLOCKSECRET"})).unwrap();
        assert_eq!(out.text, "(no matching blocks)");
        assert_eq!(out.filtered, 1);
        let out = read(&vault, "query_blocks", json!({"kind": "private"})).unwrap();
        assert_eq!(out.text, "(no matching blocks)");
        // positional ids keep their raw numbering around the gap
        let out = read(&vault, "query_blocks", json!({"note": "Mixed.md"})).unwrap();
        assert!(out.text.contains("\"block\": \"b1\"") && out.text.contains("\"block\": \"b3\""), "{}", out.text);
        assert!(!out.text.contains("\"block\": \"b2\"") && !out.text.contains("BLOCKSECRET"));
    }

    #[test]
    fn write_note_refused_when_private_blocks_but_create_and_edit_work() {
        let (_d, mut vault) = fixture_with_private_block();
        let err =
            prepare_write(&vault, "r".into(), "write_note", &json!({"path": "Mixed.md", "content": "x"}), None).unwrap_err();
        assert!(matches!(err, ToolError::Privacy(_)));
        assert_eq!(err.to_string(), "write_note is not available for Mixed.md; use edit_note for changes to this note");
        assert!(prepare_write(&vault, "r".into(), "write_note", &json!({"path": "Plain.md", "content": "x"}), None).is_ok());
        assert!(prepare_write(&vault, "r".into(), "write_note", &json!({"path": "New.md", "content": "x"}), None).is_ok());
        // execute re-checks: a private block that appeared during the gate blocks the overwrite
        let req =
            prepare_write(&vault, "r".into(), "write_note", &json!({"path": "Plain.md", "content": "new"}), None).unwrap();
        vault.write_note(&NoteId::new("Plain.md"), "plain\n\n> [!private] late\n").unwrap();
        let err = execute_write(&mut vault, &req, None, None).unwrap_err();
        assert!(err.starts_with("write_note is not available for Plain.md"), "{err}");
        assert!(std::fs::read_to_string(vault.root().join("Plain.md")).unwrap().contains("late"));
    }

    #[test]
    fn edit_note_matches_public_text_and_applies_to_raw() {
        let (_d, vault) = fixture_with_private_block();
        // "apple" occurs three times in the raw text, twice where the model can see
        let err = prepare_write(
            &vault,
            "r".into(),
            "edit_note",
            &json!({"path": "Mixed.md", "old_str": "apple", "new_str": "pear"}),
            None,
        )
        .unwrap_err();
        assert!(err.to_string().starts_with("old_str matches 2 times"), "{err}");
        let req = prepare_write(
            &vault,
            "r".into(),
            "edit_note",
            &json!({"path": "Mixed.md", "old_str": "tail apple", "new_str": "tail pear"}),
            None,
        )
        .unwrap();
        assert!(req.new_content.contains("BLOCKSECRET"), "the raw text keeps the private block");
        assert!(req.new_content.contains("tail pear") && req.diff.contains("tail pear"));
        // a match only inside the private block, or straddling its boundary, is "not found"
        for old in ["banana", "apple\n\n> [!private]", "> [!private] BLOCKSECRET"] {
            let err = prepare_write(
                &vault,
                "r".into(),
                "edit_note",
                &json!({"path": "Mixed.md", "old_str": old, "new_str": "x"}),
                None,
            )
            .unwrap_err();
            assert!(err.to_string().starts_with("old_str not found"), "{old:?}: {err}");
        }
    }

    #[test]
    fn writes_record_provenance_under_the_session() {
        let (_d, mut vault) = fixture_with_private_block();
        let req = prepare_write(
            &vault,
            "r".into(),
            "write_note",
            &json!({"path": "New.md", "content": "fresh\n"}),
            Some("20260902-100000.000"),
        )
        .unwrap();
        assert!(req.new_content.starts_with("---\nvemry:\n  origin: agent\n  agent_session: 20260902-100000.000\n---\n"));
        execute_write(&mut vault, &req, None, Some("20260902-100000.000")).unwrap();
        let prov = vault.provenance(&NoteId::new("New.md")).unwrap();
        assert!(
            matches!(prov[0].origin, vemry_core::Origin::Agent { pending: true, session: Some(ref s), .. } if s == "20260902-100000.000"),
            "{:?}",
            prov[0].origin
        );
        // an edit records only the block it changed; the applied (overridden) bytes count
        let req = prepare_write(
            &vault,
            "r".into(),
            "edit_note",
            &json!({"path": "Plain.md", "old_str": "plain", "new_str": "plain edited"}),
            Some("s2"),
        )
        .unwrap();
        execute_write(&mut vault, &req, Some("plain edited by hand\n"), Some("s2")).unwrap();
        let prov = vault.provenance(&NoteId::new("Plain.md")).unwrap();
        assert_eq!(prov.len(), 1);
        assert!(matches!(prov[0].origin, vemry_core::Origin::Agent { pending: true, .. }));
        assert!(std::fs::read_to_string(vault.root().join(".vemry/provenance.jsonl")).unwrap().contains("\"session\":\"s2\""));
    }

    #[test]
    fn query_blocks_filters_private() {
        let (_d, vault) = fixture();
        let out = read(&vault, "query_blocks", json!({"text": "TOPSECRET"})).unwrap();
        assert_eq!(out.text, "(no matching blocks)");
        assert!(out.filtered >= 2);
        // scoping to a private note is identical to scoping to a missing one
        let scoped = read(&vault, "query_blocks", json!({"note": "Diary.md"})).unwrap();
        let missing = read(&vault, "query_blocks", json!({"note": "Missing.md"})).unwrap();
        assert_eq!(scoped.text, missing.text);
    }

    #[test]
    fn prepare_write_rejects_privacy_paths() {
        let (_d, vault) = fixture();
        // creating a NEW note inside a private folder is refused pre-diff
        let err = prepare_write(&vault, "r1".into(), "write_note", &json!({"path": "secret/New.md", "content": "x"}), None)
            .unwrap_err();
        assert!(matches!(err, ToolError::Privacy(_)));
        assert_eq!(err.to_string(), "invalid path: secret/New.md");
        // editing a frontmatter-private note reads as nonexistent
        let err = prepare_write(
            &vault,
            "r2".into(),
            "edit_note",
            &json!({"path": "Diary.md", "old_str": "a", "new_str": "b"}),
            None,
        )
        .unwrap_err();
        assert!(matches!(err, ToolError::Privacy(_)));
        assert_eq!(err.to_string(), "note not found: Diary.md");
        // public writes still prepare normally
        assert!(prepare_write(&vault, "r3".into(), "write_note", &json!({"path": "Pub.md", "content": "x"}), None).is_ok());
    }

    #[test]
    fn prepare_write_case_variant_of_private_is_blocked() {
        let (_d, vault) = fixture();
        // Diary.md is frontmatter-private. A case-variant spelling must not
        // slip past is_private — on a case-insensitive FS the bare read would
        // open Diary.md and leak its body into the diff / match-count oracle.
        // Resolution is index-based, so this holds regardless of the host FS.
        let edit = prepare_write(
            &vault,
            "r".into(),
            "edit_note",
            &json!({"path": "diary.md", "old_str": "TOPSECRET", "new_str": "x"}),
            None,
        )
        .unwrap_err();
        assert!(matches!(edit, ToolError::Privacy(_)), "edit_note case-variant must be blocked");
        let write = prepare_write(&vault, "r".into(), "write_note", &json!({"path": "diary.md", "content": "x"}), None)
            .unwrap_err();
        assert!(matches!(write, ToolError::Privacy(_)), "write_note case-variant must be blocked");
    }

    #[test]
    fn prepare_write_canonicalizes_case_variant_of_public_note() {
        let (_d, vault) = fixture();
        // "hello.md" folds onto the indexed Hello.md on a case-insensitive FS:
        // the request must target the canonical id with the real content as
        // the diff baseline — not a phantom "create" whose approval would
        // silently overwrite the note and leave a duplicate index entry.
        let req =
            prepare_write(&vault, "r".into(), "write_note", &json!({"path": "hello.md", "content": "new body"}), None)
                .unwrap();
        assert_eq!(req.path, "Hello.md");
        assert_eq!(req.kind, WriteKind::Overwrite, "must prepare as overwrite");
        assert!(req.diff.contains("# Hello"), "diff baseline must be the current content");
        assert!(!req.new_content.contains("origin: agent"), "existing note must not get provenance frontmatter");
        let req = prepare_write(
            &vault,
            "r".into(),
            "edit_note",
            &json!({"path": "hello.md", "old_str": "# Hello", "new_str": "# Hi"}),
            None,
        )
        .unwrap();
        assert_eq!(req.path, "Hello.md");
    }

    #[test]
    fn prepare_write_rejects_on_disk_but_unindexed_paths() {
        let (_d, vault) = fixture();
        // watcher lag: on disk but not yet indexed — refuse the blind
        // overwrite (the "create" diff would show an empty baseline)
        std::fs::write(vault.root().join("Fresh.md"), "not yet indexed\n").unwrap();
        let err = prepare_write(&vault, "r".into(), "write_note", &json!({"path": "Fresh.md", "content": "x"}), None)
            .unwrap_err();
        assert_eq!(err.to_string(), "invalid path: Fresh.md");
        // a pre-existing symlink is never indexed; fs::write would follow it
        // to its target, possibly outside the vault
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(vault.root().join("Diary.md"), vault.root().join("link.md")).unwrap();
            let err = prepare_write(&vault, "r".into(), "write_note", &json!({"path": "link.md", "content": "x"}), None)
                .unwrap_err();
            assert_eq!(err.to_string(), "invalid path: link.md");
        }
    }

    #[test]
    fn execute_write_rechecks_privacy_at_execute_time() {
        let (_d, mut vault) = fixture();
        // Prepare a legit overwrite of a public note (as the gate would see it).
        let req =
            prepare_write(&vault, "r".into(), "write_note", &json!({"path": "Hello.md", "content": "OVERWRITTEN"}), None)
                .unwrap();
        // The note turns private during the permission-gate await.
        vault.add_privacy_rule("Hello.md").unwrap();
        // Execute must refuse and leave the file untouched (TOCTOU closed).
        let err = execute_write(&mut vault, &req, None, None).unwrap_err();
        assert!(err.contains("not found"), "stale-approval write must be refused: {err}");
        let content = vault.read_note(&vault.note_id("Hello.md").unwrap()).unwrap();
        assert!(content.contains("# Hello"), "original content must survive");
        assert!(!content.contains("OVERWRITTEN"), "private note must not be overwritten");
    }

    #[test]
    fn prepare_move_gates_privacy_both_ways() {
        let (_d, vault) = fixture();
        // A private from: worded the same as missing (echoing the caller's spelling)
        let err =
            prepare_write(&vault, "r".into(), "move_note", &json!({"from": "secret/Plan.md", "to": "Pub.md"}), None)
                .unwrap_err();
        assert!(matches!(err, ToolError::Privacy(_)));
        assert_eq!(err.to_string(), "note not found: secret/Plan.md");
        // A to that lands in a privacy zone: worded the same as a write_note create
        let err =
            prepare_write(&vault, "r".into(), "move_note", &json!({"from": "Hello.md", "to": "secret/Hello.md"}), None)
                .unwrap_err();
        assert!(matches!(err, ToolError::Privacy(_)));
        assert_eq!(err.to_string(), "invalid path: secret/Hello.md");
        // A to that collides with a private note of the same name: privacy before
        // existence — it must never say already exists
        let err = prepare_write(&vault, "r".into(), "move_note", &json!({"from": "Hello.md", "to": "Diary.md"}), None)
            .unwrap_err();
        assert!(matches!(err, ToolError::Privacy(_)));
        assert_eq!(err.to_string(), "invalid path: Diary.md", "a collision must not confirm that a private note exists");
    }

    #[test]
    fn prepare_move_rejects_existing_target_and_bad_names() {
        let (_d, vault) = fixture();
        // A public note of the same name: reported as already existing, truthfully
        std::fs::write(vault.root().join("Other.md"), "# other\n").unwrap();
        let vault = Vault::open(vault.root()).unwrap();
        let err = prepare_write(&vault, "r".into(), "move_note", &json!({"from": "Hello.md", "to": "Other.md"}), None)
            .unwrap_err();
        assert_eq!(err.to_string(), "already exists: Other.md");
        // A case variant collides too (the index folds case)
        let err = prepare_write(&vault, "r".into(), "move_note", &json!({"from": "Hello.md", "to": "other.md"}), None)
            .unwrap_err();
        assert!(err.to_string().starts_with("already exists:"));
        // A missing source: worded the same as a private one
        let err = prepare_write(&vault, "r".into(), "move_note", &json!({"from": "Nope.md", "to": "New.md"}), None)
            .unwrap_err();
        assert_eq!(err.to_string(), "note not found: Nope.md");
        // An illegal Windows file name is refused during prepare (it never reaches
        // the permission gate, so the model can correct itself)
        let err = prepare_write(&vault, "r".into(), "move_note", &json!({"from": "Hello.md", "to": "he:llo.md"}), None)
            .unwrap_err();
        assert!(err.to_string().contains("Windows"), "{err}");
        // On disk but not indexed (a symlink) is refused
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(vault.root().join("Diary.md"), vault.root().join("link.md")).unwrap();
            let err = prepare_write(&vault, "r".into(), "move_note", &json!({"from": "Hello.md", "to": "link.md"}), None)
                .unwrap_err();
            assert_eq!(err.to_string(), "invalid path: link.md");
        }
        // happy path: the path pair is prepared and the diff is empty
        let req =
            prepare_write(&vault, "r".into(), "move_note", &json!({"from": "hello.md", "to": "moved/Hi.md"}), None)
                .unwrap();
        assert_eq!(req.path, "Hello.md", "the source folds onto the indexed casing");
        assert_eq!(req.to_path.as_deref(), Some("moved/Hi.md"));
        assert!(req.diff.is_empty() && req.new_content.is_empty());
        assert_eq!((req.kind, req.path.as_str(), req.to_path.as_deref()), (WriteKind::Move, "Hello.md", Some("moved/Hi.md")));
    }

    #[test]
    fn execute_move_rechecks_and_renames() {
        let (_d, mut vault) = fixture();
        let req = prepare_write(&vault, "r".into(), "move_note", &json!({"from": "Hello.md", "to": "moved/Hi.md"}), None)
            .unwrap();
        // The source turns private while the gate is awaited: refused, and the disk is untouched
        vault.add_privacy_rule("Hello.md").unwrap();
        let err = execute_write(&mut vault, &req, None, None).unwrap_err();
        assert_eq!(err, "note not found: Hello.md");
        assert!(vault.root().join("Hello.md").exists());
        vault.remove_privacy_rule("Hello.md").unwrap();
        // The destination turns private while the gate is awaited: refused the same way
        vault.add_privacy_rule("moved/").unwrap();
        let err = execute_write(&mut vault, &req, None, None).unwrap_err();
        assert_eq!(err, "invalid path: moved/Hi.md");
        vault.remove_privacy_rule("moved/").unwrap();
        // Allowed: a real rename plus a rebuilt index (AllowEdited's override is ignored)
        let out = execute_write(&mut vault, &req, Some("IGNORED"), None).unwrap();
        assert_eq!(out, "moved Hello.md -> moved/Hi.md");
        assert!(!vault.root().join("Hello.md").exists());
        assert!(vault.root().join("moved/Hi.md").exists());
        let id = vault.note_id("moved/Hi.md").unwrap();
        assert!(vault.index().contains(&id), "the index has been rebuilt");
        assert!(vault.read_note(&id).unwrap().contains("# Hello"), "content verbatim (the override does not apply)");
    }
}
