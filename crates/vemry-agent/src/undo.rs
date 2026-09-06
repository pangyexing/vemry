//! Undoing a turn's writes. Every write that passes the gate is journaled with what
//! it replaced (the note's previous text, or for a move the path pair and the link
//! rewrites), and "undo this turn" walks the turn's records backwards: a note whose
//! text is still what the write left it gets its previous text back (a note the
//! write created goes to the trash), a move is moved back when nothing has taken its
//! place, a rewrite is restored when its note is untouched. Anything edited since is
//! skipped and counted — a stranger's edit is never overwritten. The journal lives
//! with the session in memory (the audit jsonl gets an `undo` marker, never the
//! texts); after a restart the button is gone, the provenance bar remains.

use serde::Serialize;
use vemry_core::{NoteId, Vault};

use crate::permission::RewriteText;

/// One landed write, with what it takes to put it back.
#[derive(Clone, Debug)]
pub struct WriteRecord {
    pub turn: Option<String>,
    /// The note (or file) written; for a move, the source path.
    pub path: String,
    /// The text before the write (None = the write created the note).
    pub old: Option<String>,
    /// The text the write left (None for a move).
    pub new: Option<String>,
    /// A move's destination.
    pub to: Option<String>,
    /// A move's link rewrites (old → new per note).
    pub rewrites: Vec<RewriteText>,
}

/// The journal holds this many records at most (the oldest go first).
pub const JOURNAL_MAX: usize = 200;

#[derive(Clone, Debug, Default, Serialize)]
pub struct UndoReport {
    pub reverted: usize,
    pub skipped: usize,
}

/// Undo every record of `turn`, newest first; the records are removed from the
/// journal whether or not they could be reverted (an undo is one shot).
pub fn undo_turn(vault: &mut Vault, journal: &mut Vec<WriteRecord>, turn: &str) -> UndoReport {
    let mut report = UndoReport::default();
    let mut i = journal.len();
    while i > 0 {
        i -= 1;
        if journal[i].turn.as_deref() != Some(turn) {
            continue;
        }
        let rec = journal.remove(i);
        undo_one(vault, &rec, &mut report);
    }
    report
}

fn undo_one(vault: &mut Vault, rec: &WriteRecord, report: &mut UndoReport) {
    let skip = |report: &mut UndoReport| report.skipped += 1;
    if let Some(to) = &rec.to {
        // A move: back only when the destination is still there and the source is free
        let root = vault.root().to_path_buf();
        if root.join(to).exists() && !root.join(&rec.path).exists() && vault.rename_path(to, &rec.path).is_ok() {
            report.reverted += 1;
        } else {
            skip(report);
        }
        for r in &rec.rewrites {
            if r.apply(vault, true) {
                report.reverted += 1;
            } else {
                skip(report);
            }
        }
        return;
    }
    let Ok(id) = vault.note_id(&rec.path) else {
        skip(report);
        return;
    };
    let Some(new) = &rec.new else {
        skip(report);
        return;
    };
    match (vault.read_note(&id), &rec.old) {
        (Ok(cur), Some(old)) if cur == *new => {
            if vault.write_note(&id, old).is_ok() {
                report.reverted += 1;
            } else {
                skip(report);
            }
        }
        // The write created the note: it goes to the trash (recoverable), never deleted outright
        (Ok(cur), None) if cur == *new => {
            if vault.delete_path(id.as_str()).is_ok() {
                report.reverted += 1;
            } else {
                skip(report);
            }
        }
        _ => skip(report),
    }
}

/// Append one record, keeping the journal bounded.
pub fn record(journal: &mut Vec<WriteRecord>, rec: WriteRecord) {
    journal.push(rec);
    if journal.len() > JOURNAL_MAX {
        let excess = journal.len() - JOURNAL_MAX;
        journal.drain(..excess);
    }
}

/// The previous text of a note about to be written (None = it does not exist yet).
pub fn previous_text(vault: &Vault, path: &str) -> Option<String> {
    let id: NoteId = vault.note_id(path).ok()?;
    let id = vault.index().canonical_id(&id).unwrap_or(id);
    vault.read_note(&id).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_restores_edits_trashes_creations_and_moves_back() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("A.md"), "one\n").unwrap();
        std::fs::write(dir.path().join("Ref.md"), "see [[A]]\n").unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();
        let mut journal = vec![];
        // an edit, a creation, a move with a rewrite, all in turn t1; one edit in t2
        vault.write_note(&NoteId::new("A.md"), "one\ntwo\n").unwrap();
        record(&mut journal, WriteRecord { turn: Some("t1".into()), path: "A.md".into(), old: Some("one\n".into()), new: Some("one\ntwo\n".into()), to: None, rewrites: vec![] });
        vault.write_note(&NoteId::new("New.md"), "fresh\n").unwrap();
        record(&mut journal, WriteRecord { turn: Some("t1".into()), path: "New.md".into(), old: None, new: Some("fresh\n".into()), to: None, rewrites: vec![] });
        vault.rename_path("A.md", "moved/A.md").unwrap();
        vault.write_note(&NoteId::new("Ref.md"), "see [[moved/A]]\n").unwrap();
        record(&mut journal, WriteRecord {
            turn: Some("t1".into()), path: "A.md".into(), old: None, new: None, to: Some("moved/A.md".into()),
            rewrites: vec![RewriteText { path: "Ref.md".into(), old: "see [[A]]\n".into(), new: "see [[moved/A]]\n".into() }],
        });
        record(&mut journal, WriteRecord { turn: Some("t2".into()), path: "Ref.md".into(), old: Some("x".into()), new: Some("y".into()), to: None, rewrites: vec![] });
        let r = undo_turn(&mut vault, &mut journal, "t1");
        assert_eq!((r.reverted, r.skipped), (4, 0), "{r:?}");
        assert_eq!(std::fs::read_to_string(dir.path().join("A.md")).unwrap(), "one\n");
        assert!(!dir.path().join("moved/A.md").exists());
        assert_eq!(std::fs::read_to_string(dir.path().join("Ref.md")).unwrap(), "see [[A]]\n");
        assert!(!dir.path().join("New.md").exists(), "a created note is trashed");
        assert_eq!(journal.len(), 1, "t2's record stays");
        // t2's note text is not what the record says: skipped, never overwritten
        let r = undo_turn(&mut vault, &mut journal, "t2");
        assert_eq!((r.reverted, r.skipped), (0, 1));
        assert!(journal.is_empty());
    }
}
