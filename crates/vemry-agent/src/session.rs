//! Session audit log: every user message, model output, tool call, diff and
//! permission decision is appended to `.vemry/sessions/<id>.jsonl` in the
//! vault. Plain text, git-diffable — the data source for the Agent review
//! projection (v0.2).
//!
//! Since v0.5.1 a "session" spans turns: one jsonl file per session, appended to
//! turn by turn (an existing log from the days of one file per turn reads
//! naturally as a single-turn session). Beside it, the `<id>.history.json`
//! sidecar is written whole and holds `Vec<ChatMessage>` — the exact state of the
//! model context. It is not rebuilt from the jsonl, because the jsonl is the
//! audit view (a tool_result keeps only a preview, and extra_items never reach
//! disk).

use crate::types::ChatMessage;
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// ids are timestamps we generated; refuse anything path-like.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && !id.contains('/') && !id.contains('\\') && !id.contains("..")
}

/// The one spelling of `.vemry/sessions/<id>.jsonl` — a change to the directory
/// layout or the suffix happens here alone.
fn session_path(meta_dir: &Path, id: &str) -> PathBuf {
    meta_dir.join("sessions").join(format!("{id}.jsonl"))
}

/// One truncation rule for titles: a manual rename (`rename_session`) and a
/// derived title (`derive_title`) share it, and the two must not drift apart.
const TITLE_MAX: usize = 80;

pub struct SessionLog {
    pub id: String,
    path: PathBuf,
}

impl SessionLog {
    /// `meta_dir` is the vault's `.vemry` directory.
    ///
    /// The id is a millisecond timestamp, and **two within one millisecond
    /// collide**: a fork created right after its source session, or two turns
    /// opened back to back, can land in the same millisecond, and a collision
    /// means two sessions sharing one jsonl (the forked ledger picking up the
    /// source session's later turns). So `create_new` claims the file atomically:
    /// whoever loses retries with a `-n` suffix, and both the id and the file are
    /// guaranteed unique. The suffix does not disturb the list ordering
    /// (`stamp-1` sorts after `stamp` and before the next millisecond), and
    /// `valid_id` accepts it.
    pub fn create(meta_dir: &Path) -> std::io::Result<SessionLog> {
        let dir = meta_dir.join("sessions");
        fs::create_dir_all(&dir)?;
        let stamp = Utc::now().format("%Y%m%d-%H%M%S%.3f").to_string();
        for n in 0..1000 {
            let id = if n == 0 { stamp.clone() } else { format!("{stamp}-{n}") };
            let path = session_path(meta_dir, &id);
            match OpenOptions::new().create_new(true).write(true).open(&path) {
                Ok(_) => return Ok(SessionLog { id, path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "every session id within this millisecond is taken"))
    }

    /// Reopen an existing session for appending the next turn.
    pub fn open(meta_dir: &Path, id: &str) -> std::io::Result<SessionLog> {
        if !valid_id(id) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad session id"));
        }
        let dir = meta_dir.join("sessions");
        fs::create_dir_all(&dir)?;
        Ok(SessionLog { id: id.to_string(), path: session_path(meta_dir, id) })
    }

    pub fn append(&self, kind: &str, mut payload: Value) {
        if let Some(obj) = payload.as_object_mut() {
            obj.insert("ts".into(), json!(Utc::now().to_rfc3339()));
            obj.insert("kind".into(), json!(kind));
        }
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(f, "{payload}");
        }
    }
}

pub fn list_sessions(meta_dir: &Path) -> Vec<String> {
    let dir = meta_dir.join("sessions");
    let mut ids: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.strip_suffix(".jsonl").map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids.reverse();
    ids
}

pub fn read_session(meta_dir: &Path, id: &str) -> Option<String> {
    if !valid_id(id) {
        return None;
    }
    fs::read_to_string(session_path(meta_dir, id)).ok()
}

/// The session's source table, rebuilt from its audit jsonl (`sources` lines, with
/// retry / edit_resend markers applied — see `sources::read_sources`). Missing or
/// unreadable → empty.
pub fn read_session_sources(meta_dir: &Path, id: &str) -> Vec<crate::sources::Source> {
    read_session(meta_dir, id).map(|text| crate::sources::read_sources(&text)).unwrap_or_default()
}

fn history_path(meta_dir: &Path, id: &str) -> PathBuf {
    meta_dir.join("sessions").join(format!("{id}.history.json"))
}

/// Write the model-context sidecar whole at the end of a turn. Failure is silent
/// (the same policy as append: auditing and persistence never block the
/// conversation). `extra_items` is serde(skip) on ChatMessage (to keep it out of
/// a Chat Completions request body), so it is added to the sidecar by hand here —
/// otherwise the Responses search context could not be resumed after a restart.
/// `turn` and `context_refs` are the same story (skip_serializing keeps them out
/// of the request body); reading back uses serde's default deserialisation, so no
/// symmetric handwork is needed.
pub fn write_history(meta_dir: &Path, id: &str, history: &[ChatMessage]) {
    if !valid_id(id) {
        return;
    }
    let items: Vec<Value> = history
        .iter()
        .map(|m| {
            let mut v = serde_json::to_value(m).unwrap_or(Value::Null);
            if let Some(obj) = v.as_object_mut() {
                if !m.extra_items.is_empty() {
                    obj.insert("extra_items".into(), Value::Array(m.extra_items.clone()));
                }
                if let Some(t) = &m.turn {
                    obj.insert("turn".into(), json!(t));
                }
                if !m.context_refs.is_empty() {
                    obj.insert("context_refs".into(), json!(m.context_refs));
                }
            }
            v
        })
        .collect();
    if let Ok(text) = serde_json::to_string(&items) {
        let _ = fs::write(history_path(meta_dir, id), text);
    }
}

/// sidecar → model context; missing (an existing old single-turn file) or corrupt
/// → None, and the caller continues with an empty context while the transcript is
/// still replayable.
pub fn read_history(meta_dir: &Path, id: &str) -> Option<Vec<ChatMessage>> {
    if !valid_id(id) {
        return None;
    }
    let text = fs::read_to_string(history_path(meta_dir, id)).ok()?;
    let vals: Vec<Value> = serde_json::from_str(&text).ok()?;
    let mut out = Vec::with_capacity(vals.len());
    for mut v in vals {
        let extra = v.as_object_mut().and_then(|o| o.remove("extra_items"));
        let mut msg: ChatMessage = serde_json::from_value(v).ok()?;
        if let Some(Value::Array(items)) = extra {
            msg.extra_items = items;
        }
        out.push(msg);
    }
    Some(out)
}

/// Fork a new session off an existing one (the research tree): copy every line of
/// the audit jsonl before the fork point's user message (keeping the original ts)
/// and open the new session with a fork marker line. The fork point is located by
/// turn id first (`turn`, the one the runner stamps on the user line) — an edit
/// and re-send leaves an abandoned user line in the jsonl, and counting ordinals
/// would cut in the wrong place; when `turn` is None (an existing old session with
/// no ids) it falls back to counting to the `before_user_turn`-th user message
/// (0-based). The model-context sidecar is written separately by the caller after
/// truncating at `history_fork_cut` — the jsonl is the audit view and never
/// reconstructs the model context.
pub fn fork_session(
    meta_dir: &Path,
    from_id: &str,
    turn: Option<&str>,
    before_user_turn: usize,
) -> std::io::Result<SessionLog> {
    let text = read_session(meta_dir, from_id)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "session not found"))?;
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let mut user_seen = 0usize;
    let mut found = turn.is_none();
    let mut cut = lines.len();
    let mut copied = String::new();
    for (idx, line) in lines.iter().enumerate() {
        let v = serde_json::from_str::<Value>(line).unwrap_or(Value::Null);
        if v["kind"] == "user" {
            let hit = match turn {
                Some(t) => v["turn"].as_str() == Some(t),
                None => user_seen == before_user_turn,
            };
            if hit {
                found = true;
                cut = idx;
                break;
            }
            user_seen += 1;
        }
        copied.push_str(line);
        copied.push('\n');
    }
    // An id that was given but not found = the timeline and the audit record are
    // out of step (which should not happen); an honest error beats copying the
    // whole file into a complete duplicate that forked nothing
    if !found {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "fork point not in session log"));
    }
    let log = SessionLog::create(meta_dir)?;
    let mut marker = json!({ "from": from_id, "before_turn": before_user_turn });
    if let Some(t) = turn {
        marker["turn"] = json!(t);
    }
    log.append("fork", marker);
    if !copied.is_empty() {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&log.path) {
            let _ = f.write_all(copied.as_bytes());
        }
    }
    // A manual title follows the fork: a title line before the cut comes along
    // with the prefix; a rename made after the fork point is picked up as the last
    // one after the cut and appended to the end of the new ledger (last one wins,
    // so appending overwrites an earlier rename in the prefix). A forked session
    // therefore shows "⑂ + the manual title" rather than the derived one.
    // Filter out non-title lines on the raw bytes first: a session that was never
    // renamed (the vast majority) therefore parses no JSON at all
    if let Some(t) = lines[cut..].iter().rev().filter(|l| l.contains(r#""kind":"title""#)).find_map(|l| {
        let v = serde_json::from_str::<Value>(l).ok()?;
        if v["kind"] == "title" { v["title"].as_str().map(str::to_string) } else { None }
    }) {
        log.append("title", json!({ "title": t }));
    }
    Ok(log)
}

/// The fork cut point in the model history (sidecar or memory). Returns (the cut
/// index, whether the fork point has already been swallowed by second-level
/// compaction into a summary). The rules:
/// 1. `turn` matches an envelope message → cut exactly before it (the prefix
///    holds any earlier compaction summary — which is precisely this session's
///    real model context before that turn);
/// 2. `turn` was given but not found → the fork point has been swallowed into a
///    synthesised summary by second-level compaction. Every message that still
///    carries an id lies after the fork point (compaction eats the prefix), so
///    the cut goes before the first message with an id: the prefix is nothing but
///    the summary, which covers more than the fork point did, and true is
///    returned so the caller can tell the user;
/// 3. `turn` is None (an existing old session) → count user ordinals, as before.
pub fn history_fork_cut(
    history: &[ChatMessage],
    turn: Option<&str>,
    before_user_turn: usize,
) -> (usize, bool) {
    if let Some(t) = turn {
        if let Some(i) = history.iter().position(|m| m.turn.as_deref() == Some(t)) {
            return (i, false);
        }
        if let Some(i) = history.iter().position(|m| m.turn.is_some()) {
            return (i, true);
        }
    }
    let mut seen = 0usize;
    for (i, m) in history.iter().enumerate() {
        if m.role == "user" {
            if seen == before_user_turn {
                return (i, false);
            }
            seen += 1;
        }
    }
    (history.len(), false)
}

/// Rename a session by hand: append one title override line to the audit jsonl —
/// the ledger is append-only and **the last line wins** (renaming repeatedly is
/// fine, and the history keeps the trace); a derived title (the first user
/// message) and the id fallback remain the behaviour when there is no override
/// line. The truncation matches derive_title at 80 characters.
/// Being purely an append, it does not touch the history sidecar and is allowed
/// while the agent is running.
pub fn rename_session(meta_dir: &Path, id: &str, title: &str) -> Result<(), String> {
    let t: String = title.trim().chars().take(TITLE_MAX).collect();
    if t.is_empty() {
        return Err("the title cannot be empty".into());
    }
    if !valid_id(id) {
        return Err("session not found".into());
    }
    // Opening (without create) doubles as the existence check: it does not read
    // the whole ledger into memory (a long session's jsonl can run to hundreds of
    // kilobytes, and renaming is allowed mid-run), and it leaves no window for
    // "deleted concurrently after the stat, and append's create(true) resurrects
    // the session as a ghost file holding nothing but a title line". The write
    // does not follow SessionLog::append's silent policy — this one line is the
    // entire effect of a rename, so a failure must be reported.
    let mut f = OpenOptions::new().append(true).open(session_path(meta_dir, id)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "session not found".to_string()
        } else {
            format!("rename failed: {e}")
        }
    })?;
    let line = json!({ "ts": Utc::now().to_rfc3339(), "kind": "title", "title": t });
    writeln!(f, "{line}").map_err(|e| format!("rename failed: {e}"))?;
    Ok(())
}

/// Delete a session's audit log and its history sidecar.
pub fn delete_session(meta_dir: &Path, id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err("bad session id".into());
    }
    let jsonl = session_path(meta_dir, id);
    fs::remove_file(&jsonl).map_err(|e| format!("failed to delete the session: {e}"))?;
    let _ = fs::remove_file(history_path(meta_dir, id)); // an old file may have no sidecar
    Ok(())
}

/// A session list entry: the data behind a session row in the research panel.
#[derive(Debug, Clone, Serialize)]
pub struct SessionMeta {
    pub id: String,
    /// The first line of the first user message, cut to 80 characters. (Web
    /// context is the structured `page` field, so content is the user's own text
    /// and no envelope has to be stripped.)
    pub title: String,
    /// How many user messages there are.
    pub turns: usize,
    /// The timestamp of the last audit line (RFC3339); empty when the file holds
    /// no valid line.
    pub last_ts: String,
    /// Having a history sidecar means the model context can be resumed. The false
    /// ones are the existing fragments from before v0.5.1, when each turn was its
    /// own file (or leftovers of an interruption); the frontend folds them into
    /// "older records".
    pub resumable: bool,
    /// Forked off another session (the jsonl opens with a fork marker line). A
    /// fork copies the audit prefix, so its first user message matches the source
    /// session's → the derived title is identical, and the list UI tells them
    /// apart by this flag.
    pub forked: bool,
}

/// A derived session title: the first line, capped at `TITLE_MAX` characters.
fn derive_title(content: &str) -> String {
    let line = content.lines().next().unwrap_or("").trim();
    line.chars().take(TITLE_MAX).collect()
}

/// One session jsonl → SessionMeta (the whole file read and parsed line by line;
/// the incremental path is `SessionMetaCache`, which only lets a file through
/// here when its fingerprint changed).
fn parse_meta(meta_dir: &Path, id: &str) -> SessionMeta {
    let mut title = String::new();
    let mut turns = 0;
    let mut last_ts = String::new();
    let mut forked = false;
    if let Some(text) = read_session(meta_dir, id) {
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let Ok(obj) = serde_json::from_str::<Value>(line) else { continue };
            if obj["kind"] == "user" {
                turns += 1;
                if title.is_empty() {
                    title = derive_title(obj["content"].as_str().unwrap_or(""));
                }
            }
            // A manual rename's title line, last one winning; the is_empty guard in
            // the user branch stops a derived title appearing later from overwriting it.
            if obj["kind"] == "title" {
                if let Some(t) = obj["title"].as_str().filter(|t| !t.trim().is_empty()) {
                    title = t.to_string();
                }
            }
            if obj["kind"] == "fork" {
                forked = true;
            }
            if let Some(ts) = obj["ts"].as_str() {
                last_ts = ts.to_string();
            }
        }
    }
    if title.is_empty() {
        title = id.to_string(); // manual title > derived title > the id as a fallback
    }
    let resumable = history_path(meta_dir, id).exists();
    SessionMeta { id: id.to_string(), title, turns, last_ts, resumable, forked }
}

pub fn list_session_meta(meta_dir: &Path) -> Vec<SessionMeta> {
    list_sessions(meta_dir).iter().map(|id| parse_meta(meta_dir, id)).collect()
}

/// The incremental cache behind `list_session_meta`: while a jsonl's fingerprint
/// (mtime, len) is unchanged the previous parse is reused, and only files that
/// were appended to are re-read. The session list refreshes at the end of every
/// turn and should not slow down linearly with the total history. The key is the
/// full path, so switching vaults cannot collide; each list keeps only the
/// entries that still exist, which evicts deleted sessions and the cache of an
/// old vault along with them. resumable is checked fresh every time (a sidecar
/// can appear or vanish independently of the jsonl).
#[derive(Default)]
pub struct SessionMetaCache {
    map: std::collections::HashMap<PathBuf, (std::time::SystemTime, u64, SessionMeta)>,
}

impl SessionMetaCache {
    pub fn list(&mut self, meta_dir: &Path) -> Vec<SessionMeta> {
        let mut next = std::collections::HashMap::new();
        let out = list_sessions(meta_dir)
            .into_iter()
            .map(|id| {
                let path = session_path(meta_dir, &id);
                let fp = fs::metadata(&path)
                    .ok()
                    .and_then(|m| m.modified().ok().map(|mtime| (mtime, m.len())));
                let mut meta = match (fp, self.map.remove(&path)) {
                    (Some(fp), Some((mtime, len, cached))) if (mtime, len) == fp => {
                        next.insert(path, (mtime, len, cached.clone()));
                        cached
                    }
                    _ => {
                        let parsed = parse_meta(meta_dir, &id);
                        if let Some((mtime, len)) = fp {
                            next.insert(path, (mtime, len, parsed.clone()));
                        }
                        parsed
                    }
                };
                meta.resumable = history_path(meta_dir, &meta.id).exists();
                meta
            })
            .collect();
        self.map = next;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_appends_to_same_file_and_meta_counts_turns() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::create(dir.path()).unwrap();
        log.append("user", json!({ "content": "第一问\n第二行" }));
        let reopened = SessionLog::open(dir.path(), &log.id).unwrap();
        reopened.append("user", json!({ "content": "追问" }));

        let metas = list_session_meta(dir.path());
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].id, log.id);
        assert_eq!(metas[0].turns, 2);
        assert_eq!(metas[0].title, "第一问");
        assert!(!metas[0].last_ts.is_empty());
        // No sidecar (the shape of an existing old fragment) → not resumable; once written → resumable
        assert!(!metas[0].resumable);
        write_history(dir.path(), &log.id, &[ChatMessage::user("x")]);
        assert!(list_session_meta(dir.path())[0].resumable);
    }

    #[test]
    fn title_is_first_line_capped() {
        assert_eq!(derive_title("总结这页\n第二行"), "总结这页");
        assert_eq!(derive_title(&"长".repeat(100)), "长".repeat(80));
    }

    /// The cache: an unchanged fingerprint is reused (a line carrying a page field
    /// does not disturb the parse), an append is re-read, a deleted entry is
    /// evicted, and a change in the sidecar shows up in resumable immediately.
    #[test]
    fn meta_cache_reuses_and_invalidates() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::create(dir.path()).unwrap();
        log.append("user", json!({ "content": "首问", "page": { "url": "https://a.b", "title": "T" } }));

        let mut cache = SessionMetaCache::default();
        let first = cache.list(dir.path());
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].title, "首问");
        assert!(!first[0].resumable);

        // Fingerprint unchanged: the same result (served from cache); a sidecar appearing shows up at once
        write_history(dir.path(), &log.id, &[ChatMessage::user("x")]);
        let second = cache.list(dir.path());
        assert_eq!(second[0].title, "首问");
        assert!(second[0].resumable);

        // An append changes the fingerprint → re-parsed into a new turn count
        SessionLog::open(dir.path(), &log.id).unwrap().append("user", json!({ "content": "追问" }));
        let third = cache.list(dir.path());
        assert_eq!(third[0].turns, 2);

        delete_session(dir.path(), &log.id).unwrap();
        assert!(cache.list(dir.path()).is_empty());
        assert!(cache.map.is_empty(), "a deleted session must be evicted from the cache");
    }

    #[test]
    fn history_sidecar_roundtrip_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::create(dir.path()).unwrap();
        let mut assistant = ChatMessage::assistant(Some("there".into()), None);
        // A serde(skip) field must be added to the sidecar by hand, or the Responses search context is cut off
        assistant.extra_items = vec![json!({ "type": "web_search_call", "id": "ws_1" })];
        let mut user = ChatMessage::user("hi");
        user.turn = Some("t1".into()); // skip_serializing likewise: lose the turn id and forking degrades to counting ordinals
        // context_refs likewise: lose the metadata and scrub is left with the attribute-scan fallback
        user.context_refs = vec![crate::types::StoredRef { kind: "note".into(), path: "a/b.md".into() }];
        let history = vec![user, assistant];
        write_history(dir.path(), &log.id, &history);
        let loaded = read_history(dir.path(), &log.id).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].role, "user");
        assert_eq!(loaded[0].turn.as_deref(), Some("t1"));
        assert_eq!(loaded[0].context_refs, vec![crate::types::StoredRef { kind: "note".into(), path: "a/b.md".into() }]);
        assert_eq!(loaded[1].turn, None);
        assert!(loaded[1].context_refs.is_empty());
        assert_eq!(loaded[1].extra_items.len(), 1);
        assert_eq!(loaded[1].extra_items[0]["id"], "ws_1");
        // An existing old session has no sidecar → None, and no panic
        assert!(read_history(dir.path(), "20200101-000000.000").is_none());
    }

    /// Images enter the sidecar as paths alone: the file holds image_ref and no
    /// image_url or base64.
    #[test]
    fn history_sidecar_keeps_image_paths_not_bytes() {
        use crate::types::{Content, Part};
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::create(dir.path()).unwrap();
        let user = ChatMessage::user_content(Content::Parts(vec![
            Part::Text { text: "看图".into() },
            Part::ImageRef { path: "attachments/a/pic.png".into() },
        ]));
        write_history(dir.path(), &log.id, &[user]);
        let raw = std::fs::read_to_string(history_path(dir.path(), &log.id)).unwrap();
        assert!(raw.contains("\"image_ref\""), "{raw}");
        assert!(!raw.contains("image_url") && !raw.contains("base64,"), "{raw}");
        let loaded = read_history(dir.path(), &log.id).unwrap();
        assert_eq!(loaded[0].text(), Some("看图"));
        assert_eq!(loaded[0].content.as_ref().unwrap().parts().unwrap().len(), 2);
    }

    #[test]
    fn sidecar_not_listed_as_session_and_delete_removes_both() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::create(dir.path()).unwrap();
        log.append("user", json!({ "content": "x" }));
        write_history(dir.path(), &log.id, &[ChatMessage::user("x")]);
        assert_eq!(list_sessions(dir.path()), vec![log.id.clone()]);

        delete_session(dir.path(), &log.id).unwrap();
        assert!(list_sessions(dir.path()).is_empty());
        assert!(read_history(dir.path(), &log.id).is_none());
    }

    /// fork (the ordinal fallback, for an existing old session with no ids): the
    /// audit prefix is copied up to the user turn boundary (keeping the original
    /// ts), the fork marker line comes first, turns past the boundary do not come
    /// along, and the source session is untouched.
    #[test]
    fn fork_copies_audit_prefix_up_to_turn() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::create(dir.path()).unwrap();
        log.append("user", json!({ "content": "第一问" }));
        log.append("assistant", json!({ "content": "第一答" }));
        log.append("user", json!({ "content": "第二问" }));
        log.append("assistant", json!({ "content": "第二答" }));

        // Fork before the second user message (ordinal 1): only the first turn comes along
        let forked = fork_session(dir.path(), &log.id, None, 1).unwrap();
        assert_ne!(forked.id, log.id);
        let text = read_session(dir.path(), &forked.id).unwrap();
        assert!(text.lines().next().unwrap().contains(r#""kind":"fork""#));
        assert!(text.contains("第一问"));
        assert!(text.contains("第一答"));
        assert!(!text.contains("第二问"), "turns past the fork point must not be copied");
        // The source session is intact
        let orig = read_session(dir.path(), &log.id).unwrap();
        assert!(orig.contains("第二答"));
        // The forked session's title matches the source's (same first user message); the list tells them apart by the forked flag
        let metas = list_session_meta(dir.path());
        assert!(metas.iter().find(|m| m.id == forked.id).unwrap().forked);
        assert!(!metas.iter().find(|m| m.id == log.id).unwrap().forked);
        // A source session that does not exist is an error
        assert!(fork_session(dir.path(), "20200101-000000.000", None, 0).is_err());
    }

    /// Renaming: the last title override line wins; trim plus the 80-character cut
    /// matches derive_title; a title line is not a turn; and the new title comes
    /// back through SessionMetaCache's fingerprint-invalidation path.
    #[test]
    fn rename_overrides_derived_title_and_last_wins() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::create(dir.path()).unwrap();
        log.append("user", json!({ "content": "sdf" }));

        let mut cache = SessionMetaCache::default();
        assert_eq!(cache.list(dir.path())[0].title, "sdf");

        rename_session(dir.path(), &log.id, "  DeepSeek 模型对比  ").unwrap();
        let metas = cache.list(dir.path());
        assert_eq!(metas[0].title, "DeepSeek 模型对比", "trimmed on the way to disk; the cache fingerprint goes stale and it re-parses");
        assert_eq!(metas[0].turns, 1, "a title line is not a turn");

        rename_session(dir.path(), &log.id, "改第二次").unwrap();
        assert_eq!(cache.list(dir.path())[0].title, "改第二次", "the last one wins");

        rename_session(dir.path(), &log.id, &"长".repeat(100)).unwrap();
        assert_eq!(cache.list(dir.path())[0].title, "长".repeat(80), "truncated the same way as derive_title");
    }

    #[test]
    fn rename_rejects_bad_id_and_empty() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::create(dir.path()).unwrap();
        log.append("user", json!({ "content": "x" }));
        assert!(rename_session(dir.path(), &log.id, "   ").is_err(), "an empty title is refused");
        assert!(rename_session(dir.path(), "../evil", "t").is_err(), "a path-like id is refused");
        assert!(rename_session(dir.path(), "20200101-000000.000", "t").is_err(), "a session that does not exist is refused");
        assert_eq!(list_session_meta(dir.path())[0].title, "x", "a refusal leaves no trace");
    }

    /// A fork inherits the manual title: a rename after the fork point (its title
    /// line outside the prefix) is picked up as the last one after the cut; one
    /// inside the prefix comes along with the copy. The forked flag survives in
    /// both cases, and the list shows "⑂ + the manual title" rather than the
    /// derived one.
    #[test]
    fn fork_inherits_manual_title_with_forked_flag() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::create(dir.path()).unwrap();
        log.append("user", json!({ "content": "第一问", "turn": "t1" }));
        log.append("assistant", json!({ "content": "第一答" }));
        log.append("user", json!({ "content": "第二问", "turn": "t2" }));
        // The title line comes after the t2 line — when forking before t2 it lies outside the prefix
        rename_session(dir.path(), &log.id, "手动名").unwrap();
        let forked = fork_session(dir.path(), &log.id, Some("t2"), 1).unwrap();
        let metas = list_session_meta(dir.path());
        let f = metas.iter().find(|m| m.id == forked.id).unwrap();
        assert_eq!(f.title, "手动名");
        assert!(f.forked);

        // The in-prefix path: the rename precedes the later turn, so the title line is copied with the prefix
        let log2 = SessionLog::create(dir.path()).unwrap();
        log2.append("user", json!({ "content": "甲", "turn": "a1" }));
        rename_session(dir.path(), &log2.id, "前缀内的名").unwrap();
        log2.append("user", json!({ "content": "乙", "turn": "a2" }));
        let forked2 = fork_session(dir.path(), &log2.id, Some("a2"), 1).unwrap();
        let metas2 = list_session_meta(dir.path());
        let f2 = metas2.iter().find(|m| m.id == forked2.id).unwrap();
        assert_eq!(f2.title, "前缀内的名");
        assert!(f2.forked);
    }

    /// Sessions opened back to back within one millisecond must each get their own
    /// file: the id only goes down to the millisecond, and a collision means two
    /// sessions sharing one jsonl (a fork created right after its source is the
    /// easiest way to hit it, and the Linux CI leg has actually gone red on it).
    #[test]
    fn create_never_collides_within_same_millisecond() {
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<String> =
            (0..50).map(|_| SessionLog::create(dir.path()).unwrap().id).collect();
        let uniq: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(uniq.len(), ids.len(), "session ids created within one millisecond must all differ");
        assert_eq!(list_sessions(dir.path()).len(), ids.len(), "one file per id");
        assert!(ids.iter().all(|id| valid_id(id)), "the suffixed form is still a valid id");
    }

    /// fork (by turn id): an edit and re-send leaves an abandoned user line in the
    /// jsonl, and counting ordinals would cut a turn off; locating by id is exact
    /// and unaffected. An id that is not in the record → an error, and no new file.
    #[test]
    fn fork_by_turn_id_survives_edit_resend_ghost_lines() {
        let dir = tempfile::tempdir().unwrap();
        let log = SessionLog::create(dir.path()).unwrap();
        log.append("user", json!({ "content": "第一问", "turn": "t1" }));
        log.append("assistant", json!({ "content": "第一答" }));
        log.append("user", json!({ "content": "第二问(被放弃)", "turn": "t2" }));
        log.append("edit_resend", json!({}));
        log.append("user", json!({ "content": "第二问改", "turn": "t3" }));
        log.append("assistant", json!({ "content": "第二答" }));

        // Fork before "第二问改" (t3). On the timeline it is ordinal 1 (the
        // abandoned turn is not shown), but in the jsonl it is the second user line
        // — cutting by ordinal would drag the abandoned t2 line along.
        let forked = fork_session(dir.path(), &log.id, Some("t3"), 1).unwrap();
        let text = read_session(dir.path(), &forked.id).unwrap();
        assert!(text.contains("第一问"));
        assert!(text.contains("被放弃"), "every audit line before the fork point (abandoned turns included) comes along verbatim");
        assert!(!text.contains("第二问改"));
        assert!(text.lines().next().unwrap().contains(r#""turn":"t3""#), "the fork marker records the turn id");

        let before = list_sessions(dir.path()).len();
        assert!(fork_session(dir.path(), &log.id, Some("no-such"), 0).is_err());
        assert_eq!(list_sessions(dir.path()).len(), before, "a failure must not leave half a new session");
    }

    /// The model-history cut point: a matching id cuts exactly; an id swallowed by
    /// second-level compaction cuts before the first message that has one (the
    /// prefix being nothing but the synthesised summary) and reports summarized;
    /// no id falls back to ordinals.
    #[test]
    fn history_fork_cut_by_id_ordinal_and_compacted() {
        let mut u1 = ChatMessage::user("第一问");
        u1.turn = Some("t1".into());
        let mut u2 = ChatMessage::user("第二问");
        u2.turn = Some("t2".into());
        let history = vec![
            u1.clone(),
            ChatMessage::assistant(Some("第一答".into()), None),
            u2.clone(),
        ];
        assert_eq!(history_fork_cut(&history, Some("t2"), 99), (2, false), "a matching id ignores the ordinal");
        assert_eq!(history_fork_cut(&history, None, 1), (2, false), "no id means ordinals");
        assert_eq!(history_fork_cut(&history, None, 5), (3, false), "an out-of-range ordinal cuts at the end");

        // After second-level compaction: the prefix is a synthesised summary with no id, and t1 is no longer in the history
        let compacted = vec![ChatMessage::user("(the earlier conversation was compacted…)"), u2];
        assert_eq!(
            history_fork_cut(&compacted, Some("t1"), 0),
            (1, true),
            "a fork point swallowed by compaction: cut before the first message with an id, and report summarized"
        );
        assert_eq!(history_fork_cut(&compacted, Some("t2"), 0), (1, false), "a turn that survived compaction still cuts exactly");
    }

    #[test]
    fn path_like_ids_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(SessionLog::open(dir.path(), "../etc/passwd").is_err());
        assert!(read_history(dir.path(), "a/b").is_none());
        assert!(delete_session(dir.path(), "..").is_err());
        write_history(dir.path(), "../x", &[]); // silently refused, nothing written
        assert!(!dir.path().parent().unwrap().join("x.history.json").exists());
    }
}
