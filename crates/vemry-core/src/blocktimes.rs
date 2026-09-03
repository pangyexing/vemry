//! When each block's current content appeared — the nearest thing a Markdown
//! line has to a modification time of its own.
//!
//! A note has a modification time; a block does not, and a task line carries no
//! creation or completion date. So the index remembers, per note, a key of every
//! block's text and the note's mtime at the first indexing in which that key
//! appeared; a later reindex keeps the stamp of every block whose key is still
//! there and stamps only the changed ones with the note's new mtime. Using the
//! note's mtime rather than the clock means a change made while the app was
//! closed is dated when the file actually changed, not when it was next seen.
//!
//! The stamps are observations, not state: `.vemry/block-times.json` is a cache,
//! and without it every block simply falls back to its note's mtime (what the
//! views showed before there was one). Nothing else may read it as truth.
//!
//! The key is deliberately not provenance's identity hash — that one strips the
//! checkbox so a block keeps its authorship across a toggle, whereas here a toggle
//! is exactly the change to notice. Only whitespace and a trailing ` ^id` are
//! ignored: minting an id is bookkeeping, not an edit, and moving or re-indenting
//! a block does not touch its words.
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::note::Note;
use crate::provenance::fnv1a64;

/// Where the cache lives, relative to the vault root.
pub const FILE: &str = ".vemry/block-times.json";

/// The change key of one block: FNV-1a over its lines, each trimmed, with a
/// trailing ` ^id` on the last line dropped.
pub fn change_key(raw_slice: &str) -> u64 {
    let mut lines: Vec<&str> = raw_slice.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if let Some(last) = lines.last_mut() {
        if let Some(at) = last.rfind(" ^") {
            let id = &last[at + 2..];
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                *last = last[..at].trim_end();
            }
        }
    }
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for l in lines {
        h ^= fnv1a64(l.as_bytes());
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The per-vault table: note id → (change key → the note's mtime, in ms, when
/// that key first appeared).
#[derive(Default)]
pub struct BlockTimes {
    path: PathBuf,
    notes: BTreeMap<String, BTreeMap<u64, u64>>,
    dirty: bool,
}

impl BlockTimes {
    /// Read the cache; a missing or unreadable file is an empty table (every
    /// block then gets its note's mtime at the next stamping).
    pub fn load(root: &Path) -> BlockTimes {
        let path = root.join(FILE);
        let notes = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<BTreeMap<String, BTreeMap<String, u64>>>(&s).ok())
            .map(|raw| {
                raw.into_iter()
                    .map(|(id, keys)| {
                        (id, keys.into_iter().filter_map(|(k, ms)| u64::from_str_radix(&k, 16).ok().map(|k| (k, ms))).collect())
                    })
                    .collect()
            })
            .unwrap_or_default();
        BlockTimes { path, notes, dirty: false }
    }

    /// Stamp every block of a freshly parsed note: a block whose key was seen
    /// before keeps its stamp, a new one gets the note's mtime. The note's
    /// table is replaced by the keys present now, so a block that is gone is
    /// forgotten with it.
    pub fn stamp(&mut self, note: &mut Note) {
        let mtime_ms = note.mtime.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
        let prev = self.notes.get(note.id.as_str());
        let mut next: BTreeMap<u64, u64> = BTreeMap::new();
        for b in &mut note.blocks {
            let key = change_key(&note.raw[b.span.clone()]);
            let ms = *next.entry(key).or_insert_with(|| prev.and_then(|m| m.get(&key).copied()).unwrap_or(mtime_ms));
            b.modified_ms = ms;
        }
        if prev != Some(&next) {
            if next.is_empty() {
                self.notes.remove(note.id.as_str());
            } else {
                self.notes.insert(note.id.as_str().to_string(), next);
            }
            self.dirty = true;
        }
    }

    pub fn remove(&mut self, id: &str) {
        if self.notes.remove(id).is_some() {
            self.dirty = true;
        }
    }

    /// Carry the stamps of a moved note, or of every note under a moved
    /// folder, over to the new path — a rename touches no content, and the
    /// blocks should not all look freshly edited.
    pub fn rename(&mut self, from: &str, to: &str) {
        let prefix = format!("{from}/");
        let moved: Vec<(String, BTreeMap<u64, u64>)> = self
            .notes
            .iter()
            .filter(|(id, _)| id.as_str() == from || id.starts_with(&prefix))
            .map(|(id, m)| (format!("{to}{}", &id[from.len()..]), m.clone()))
            .collect();
        if moved.is_empty() {
            return;
        }
        self.notes.retain(|id, _| id != from && !id.starts_with(&prefix));
        self.notes.extend(moved);
        self.dirty = true;
    }

    /// Drop the notes that no longer exist (after a full rescan).
    pub fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        let before = self.notes.len();
        self.notes.retain(|id, _| keep(id));
        if self.notes.len() != before {
            self.dirty = true;
        }
    }

    /// Write the cache if anything changed. `create` allows making `.vemry/`
    /// for it: the vault's own writes may, an external change seen by the
    /// watcher may not — a directory merely being browsed must not grow a
    /// `.vemry` (see `Vault::meta_dir`). A failed write is not an error: the
    /// table is a cache and the next successful save carries everything.
    pub fn save(&mut self, create: bool) {
        if !self.dirty {
            return;
        }
        let Some(dir) = self.path.parent() else { return };
        if !dir.is_dir() {
            if !create {
                return;
            }
            if fs::create_dir_all(dir).is_err() {
                return;
            }
        }
        let raw: BTreeMap<&str, BTreeMap<String, u64>> = self
            .notes
            .iter()
            .map(|(id, m)| (id.as_str(), m.iter().map(|(k, ms)| (format!("{k:016x}"), *ms)).collect()))
            .collect();
        if let Ok(s) = serde_json::to_string(&raw) {
            if fs::write(&self.path, s).is_ok() {
                self.dirty = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::NoteId;
    use crate::parser::parse_note;
    use std::time::{Duration, SystemTime};

    fn note(id: &str, raw: &str, secs: u64) -> Note {
        let parsed = parse_note(raw);
        Note {
            id: NoteId::new(id),
            title: id.into(),
            frontmatter: parsed.frontmatter,
            content: raw.into(),
            raw: raw.into(),
            blocks: parsed.blocks,
            mtime: UNIX_EPOCH + Duration::from_secs(secs),
        }
    }

    #[test]
    fn key_ignores_whitespace_and_block_id_but_not_the_checkbox() {
        assert_eq!(change_key("- [ ] one"), change_key("  - [ ] one   "));
        assert_eq!(change_key("- [ ] one"), change_key("- [ ] one ^abc-1"));
        assert_ne!(change_key("- [ ] one"), change_key("- [x] one"));
        assert_ne!(change_key("- [ ] one"), change_key("- [ ] one 📅 2026-01-01"));
        assert_eq!(change_key("para\nline two"), change_key("para\n\n  line two"));
        assert_ne!(change_key("a ^x"), change_key("a ^x ^y"), "only one trailing id is dropped");
    }

    #[test]
    fn stamps_keep_the_old_time_for_unchanged_blocks() {
        let mut t = BlockTimes::default();
        let mut n = note("n.md", "- [ ] one\n- [ ] two\n", 100);
        t.stamp(&mut n);
        assert_eq!(n.blocks.iter().map(|b| b.modified_ms).collect::<Vec<_>>(), vec![100_000, 100_000]);
        let mut n2 = note("n.md", "- [ ] one ^id\n- [x] two\n", 200);
        t.stamp(&mut n2);
        assert_eq!(n2.blocks.iter().map(|b| b.modified_ms).collect::<Vec<_>>(), vec![100_000, 200_000]);
        // a block that came back after being gone is new again
        let mut n3 = note("n.md", "- [ ] one\n", 300);
        t.stamp(&mut n3);
        let mut n4 = note("n.md", "- [ ] one\n- [x] two\n", 400);
        t.stamp(&mut n4);
        assert_eq!(n4.blocks[1].modified_ms, 400_000);
        assert!(t.dirty);
    }

    #[test]
    fn rename_moves_a_note_and_a_folder() {
        let mut t = BlockTimes::default();
        for id in ["a/x.md", "a/b/y.md", "ab/z.md", "a.md"] {
            t.stamp(&mut note(id, "text\n", 1));
        }
        t.rename("a", "moved");
        let ids: Vec<&String> = t.notes.keys().collect();
        assert_eq!(ids, vec!["a.md", "ab/z.md", "moved/b/y.md", "moved/x.md"]);
        t.rename("a.md", "b.md");
        assert!(t.notes.contains_key("b.md") && !t.notes.contains_key("a.md"));
    }

    #[test]
    fn save_and_load_round_trip_and_the_browse_guard() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = BlockTimes::load(dir.path());
        t.stamp(&mut note("n.md", "- [ ] one\n", 7));
        t.save(false);
        assert!(!dir.path().join(FILE).exists(), "a watcher-side save must not create .vemry");
        t.save(true);
        assert!(dir.path().join(FILE).exists());
        let mut again = BlockTimes::load(dir.path());
        let mut n = note("n.md", "- [ ] one\n- [ ] two\n", 9);
        again.stamp(&mut n);
        assert_eq!(n.blocks.iter().map(|b| b.modified_ms).collect::<Vec<_>>(), vec![7_000, 9_000]);
        let _ = SystemTime::now();
    }
}
