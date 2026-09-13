//! The text of a note before each write, kept on disk so it outlives the session
//! that wrote over it.
//!
//! An approved AI overwrite, an autosave landing a stale buffer over an agent's
//! edit, a sync client or a checkout replacing a note: in every case the previous
//! text is gone from the file, and the session's undo journal (undo.rs, in memory)
//! goes with the session. `.vemry/history/<note path>/` holds that previous text,
//! one file per write, named by the moment and by who wrote next:
//! `20260914-081522-417-agent.md`. Under `.vemry` it is never indexed, never watched
//! as a note and never visible to the agent.
//!
//! Every write that is not the user's own typing keeps an exact snapshot. The
//! editor's autosave lands about once a second while the user types, so its
//! snapshots are coalesced: a new one only when the newest is older than
//! `COALESCE` — unless the text it replaces was written by someone else (an agent,
//! a script, a program outside the app), which is the stale-buffer case and is
//! always kept exact. A note keeps at most `PER_NOTE` snapshots, and those older
//! than `KEEP` are pruned when the vault opens.
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::note::NoteId;

/// Where the snapshots live, relative to the vault root.
pub const DIR: &str = ".vemry/history";

/// The editor's own writes are coalesced to one snapshot per this interval.
const COALESCE_MS: i64 = 5 * 60 * 1000;
/// Snapshots older than this are pruned at open.
const KEEP_DAYS: i64 = 30;
/// A note keeps at most this many snapshots; the oldest go first.
const PER_NOTE: usize = 60;
/// A note larger than this is not snapshotted (a pathological file, not a note).
const MAX_BYTES: usize = 8 * 1024 * 1024;

const STAMP_FORMAT: &str = "%Y%m%d-%H%M%S-%3f";

/// Who is writing the note — recorded in the snapshot's name so the history can
/// say "before an AI write" rather than only "before".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Writer {
    /// The editor buffer (autosave, ⌘S, a checkbox or field edit in a view).
    Editor,
    /// The agent's tools, their undo and a move's link rewrites.
    Agent,
    /// A user script.
    Script,
    /// A clip or a quick capture landing.
    Clip,
    /// A restore from this history.
    Restore,
    /// Everything else the app writes (a template, a translation, a migration).
    App,
    /// A program outside the app, seen by the watcher.
    External,
}

impl Writer {
    fn name(self) -> &'static str {
        match self {
            Writer::Editor => "editor",
            Writer::Agent => "agent",
            Writer::Script => "script",
            Writer::Clip => "clip",
            Writer::Restore => "restore",
            Writer::App => "app",
            Writer::External => "external",
        }
    }

    fn parse(s: &str) -> Option<Writer> {
        Some(match s {
            "editor" => Writer::Editor,
            "agent" => Writer::Agent,
            "script" => Writer::Script,
            "clip" => Writer::Clip,
            "restore" => Writer::Restore,
            "app" => Writer::App,
            "external" => Writer::External,
            _ => return None,
        })
    }
}

/// One kept state of a note.
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    /// The file stem — the handle `read` and `restore` take.
    pub id: String,
    /// When it was taken, ms since the epoch (UTC).
    pub ts_ms: i64,
    /// Who wrote over it.
    pub writer: Writer,
    pub bytes: u64,
}

pub struct History {
    dir: PathBuf,
    /// Who last wrote each note in this process: the editor coalesces its
    /// snapshots only over its own writes.
    last_writer: HashMap<NoteId, Writer>,
}

impl History {
    pub fn new(root: &Path) -> History {
        History { dir: root.join(DIR), last_writer: HashMap::new() }
    }

    fn note_dir(&self, id: &NoteId) -> PathBuf {
        self.dir.join(id.as_str())
    }

    /// Keep `previous` — the note as it is on disk now — before `writer` replaces
    /// it. Returns whether a snapshot was written (an editor write inside the
    /// coalescing interval, an empty note, a note over the size cap: no).
    pub fn keep(&mut self, id: &NoteId, previous: &str, writer: Writer) -> bool {
        let last = self.last_writer.insert(id.clone(), writer);
        if previous.is_empty() || previous.len() > MAX_BYTES {
            return false;
        }
        let dir = self.note_dir(id);
        let mut at = Utc::now();
        let newest = newest_ms(&dir);
        let exact = writer != Writer::Editor || last != Some(Writer::Editor);
        if !exact && newest.is_some_and(|ms| at.timestamp_millis() - ms < COALESCE_MS) {
            return false;
        }
        if fs::create_dir_all(&dir).is_err() {
            return false;
        }
        // The stamps of a note strictly increase: two writes inside one millisecond
        // (or a clock set back) would otherwise tie, and a tie has no order
        if let Some(ms) = newest.filter(|ms| at.timestamp_millis() <= *ms) {
            at = DateTime::from_timestamp_millis(ms + 1).unwrap_or(at);
        }
        let path = dir.join(stem(at, writer)).with_extension("md");
        let tmp = dir.join(format!(".{}.tmp", path.file_name().unwrap_or_default().to_string_lossy()));
        if fs::write(&tmp, previous).is_err() || fs::rename(&tmp, &path).is_err() {
            let _ = fs::remove_file(&tmp);
            return false;
        }
        let mut all = snapshots_in(&dir);
        if all.len() > PER_NOTE {
            all.sort_by_key(|s| std::cmp::Reverse(s.ts_ms));
            for old in &all[PER_NOTE..] {
                let _ = fs::remove_file(dir.join(&old.id).with_extension("md"));
            }
        }
        true
    }

    /// The kept states of a note, newest first.
    pub fn list(&self, id: &NoteId) -> Vec<Snapshot> {
        let mut all = snapshots_in(&self.note_dir(id));
        all.sort_by_key(|s| std::cmp::Reverse(s.ts_ms));
        all
    }

    /// The text of one kept state. `snapshot` is a stem from `list` — anything
    /// with a path separator or an unparsable name is refused rather than read.
    pub fn read(&self, id: &NoteId, snapshot: &str) -> Option<String> {
        parse_stem(snapshot)?;
        fs::read_to_string(self.note_dir(id).join(snapshot).with_extension("md")).ok()
    }

    /// A note or a folder moved: its snapshots travel with it. A destination that
    /// already has snapshots keeps them, and the source's stay where they are (age
    /// prunes them) — merging two histories would interleave two notes' pasts.
    pub fn rename(&mut self, from_rel: &str, to_rel: &str) {
        let from = self.dir.join(from_rel);
        let to = self.dir.join(to_rel);
        if from.is_dir() && !to.exists() {
            if let Some(parent) = to.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::rename(&from, &to);
        }
        let moved: Vec<(NoteId, Writer)> = self
            .last_writer
            .iter()
            .filter(|(id, _)| id.as_str() == from_rel || id.as_str().starts_with(&format!("{from_rel}/")))
            .map(|(id, w)| (id.clone(), *w))
            .collect();
        for (id, w) in moved {
            self.last_writer.remove(&id);
            let rest = &id.as_str()[from_rel.len()..];
            self.last_writer.insert(NoteId::new(&format!("{to_rel}{rest}")), w);
        }
    }

    /// Drop snapshots older than `KEEP_DAYS` and the directories that empties.
    /// Only where a history exists already: opening never creates one.
    pub fn prune_old(&self) {
        if !self.dir.is_dir() {
            return;
        }
        let cutoff = Utc::now().timestamp_millis() - KEEP_DAYS * 24 * 3600 * 1000;
        for entry in WalkDir::new(&self.dir).contents_first(true).min_depth(1).into_iter().filter_map(|e| e.ok()) {
            let p = entry.path();
            if entry.file_type().is_dir() {
                // remove_dir refuses a non-empty directory, which is exactly the check
                let _ = fs::remove_dir(p);
            } else if let Some(snap) = snapshot_of(p) {
                if snap.ts_ms < cutoff {
                    let _ = fs::remove_file(p);
                }
            }
        }
    }
}

fn stem(at: DateTime<Utc>, writer: Writer) -> String {
    format!("{}-{}", at.format(STAMP_FORMAT), writer.name())
}

/// `20260914-081522-417-agent` → (ms, writer).
fn parse_stem(stem: &str) -> Option<(i64, Writer)> {
    let (ts, writer) = stem.rsplit_once('-')?;
    let writer = Writer::parse(writer)?;
    let ms = NaiveDateTime::parse_from_str(ts, STAMP_FORMAT).ok()?.and_utc().timestamp_millis();
    Some((ms, writer))
}

fn snapshot_of(path: &Path) -> Option<Snapshot> {
    if path.extension().is_none_or(|e| e != "md") {
        return None;
    }
    let stem = path.file_stem()?.to_str()?;
    let (ts_ms, writer) = parse_stem(stem)?;
    let bytes = fs::metadata(path).ok()?.len();
    Some(Snapshot { id: stem.to_string(), ts_ms, writer, bytes })
}

fn snapshots_in(dir: &Path) -> Vec<Snapshot> {
    let Ok(rd) = fs::read_dir(dir) else { return vec![] };
    rd.filter_map(|e| e.ok()).filter(|e| e.file_type().is_ok_and(|t| t.is_file())).filter_map(|e| snapshot_of(&e.path())).collect()
}

fn newest_ms(dir: &Path) -> Option<i64> {
    snapshots_in(dir).into_iter().map(|s| s.ts_ms).max()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history() -> (tempfile::TempDir, History) {
        let dir = tempfile::tempdir().unwrap();
        let h = History::new(dir.path());
        (dir, h)
    }

    #[test]
    fn stem_round_trips() {
        let at = DateTime::from_timestamp_millis(1_789_373_722_417).unwrap();
        let s = stem(at, Writer::Agent);
        assert_eq!(s, "20260914-081522-417-agent");
        assert_eq!(parse_stem(&s), Some((1_789_373_722_417, Writer::Agent)));
        assert_eq!(parse_stem("20260914-081522-417-nobody"), None);
        assert_eq!(parse_stem("../x-agent"), None);
    }

    #[test]
    fn a_non_editor_write_is_kept_exactly_and_read_back() {
        let (_d, mut h) = history();
        let id = NoteId::new("a/b.md");
        assert!(h.keep(&id, "one\n", Writer::Agent));
        assert!(h.keep(&id, "two\n", Writer::Agent));
        let list = h.list(&id);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].writer, Writer::Agent);
        assert_eq!(h.read(&id, &list[0].id).as_deref(), Some("two\n"));
        assert_eq!(h.read(&id, &list[1].id).as_deref(), Some("one\n"));
        assert!(h.read(&id, "../../b").is_none());
        assert!(!h.keep(&id, "", Writer::Agent), "an empty note keeps nothing");
    }

    #[test]
    fn editor_writes_coalesce_but_not_over_someone_elses_text() {
        let (_d, mut h) = history();
        let id = NoteId::new("n.md");
        assert!(h.keep(&id, "v1\n", Writer::Editor), "the first is kept: nothing to coalesce with");
        assert!(!h.keep(&id, "v2\n", Writer::Editor), "the second editor write is inside the interval");
        assert!(h.keep(&id, "v3\n", Writer::Agent), "an agent write is always exact");
        assert!(h.keep(&id, "agent text\n", Writer::Editor), "the editor over the agent's text: the stale-buffer case is exact");
        assert!(!h.keep(&id, "v5\n", Writer::Editor));
        let texts: Vec<String> = h.list(&id).iter().map(|s| h.read(&id, &s.id).unwrap()).collect();
        assert_eq!(texts, vec!["agent text\n", "v3\n", "v1\n"]);
    }

    #[test]
    fn rename_moves_the_snapshots() {
        let (_d, mut h) = history();
        let id = NoteId::new("old/n.md");
        assert!(h.keep(&id, "x\n", Writer::Agent));
        h.rename("old", "new");
        assert!(h.list(&id).is_empty());
        let moved = h.list(&NoteId::new("new/n.md"));
        assert_eq!(moved.len(), 1);
        assert_eq!(h.last_writer.get(&NoteId::new("new/n.md")), Some(&Writer::Agent));
    }

    #[test]
    fn prune_drops_old_snapshots_and_empty_dirs() {
        let (d, mut h) = history();
        let id = NoteId::new("n.md");
        assert!(h.keep(&id, "x\n", Writer::Agent));
        let dir = d.path().join(DIR).join("n.md");
        let old = dir.join("20200101-000000-000-agent.md");
        fs::write(&old, "ancient\n").unwrap();
        fs::create_dir_all(d.path().join(DIR).join("gone.md")).unwrap();
        h.prune_old();
        assert!(!old.exists());
        assert_eq!(h.list(&id).len(), 1);
        assert!(!d.path().join(DIR).join("gone.md").exists());
    }
}
