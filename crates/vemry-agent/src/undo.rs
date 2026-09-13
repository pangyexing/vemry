//! Undoing a turn's writes. Every write that passes the gate is journaled with what
//! it replaced (the note's previous text, or for a move the path pair and the link
//! rewrites), and "undo this turn" walks the turn's records backwards: a note whose
//! text is still what the write left it gets its previous text back (a note the
//! write created goes to the trash), a move is moved back when nothing has taken its
//! place, a rewrite (a move's relinked referrers, a block move's destination) is
//! restored when its note is untouched. Anything edited since is skipped and
//! counted — a stranger's edit is never overwritten. The journal lives
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
    /// The side writes (old → new per note): a move's link rewrites, a block move's
    /// destination and the referrers that followed the blocks.
    pub rewrites: Vec<RewriteText>,
}

/// The journal holds this many records at most (the oldest go first).
pub const JOURNAL_MAX: usize = 200;

#[derive(Clone, Debug, Default, Serialize)]
pub struct UndoReport {
    pub reverted: usize,
    pub skipped: usize,
    /// The notes the undo changed, for the vault events the glue layer raises (the
    /// watcher swallows an undo's writes as the app's own, and without an event an
    /// open buffer would keep the agent's text and autosave it straight back). Not
    /// part of the wire shape the frontend reads.
    #[serde(skip)]
    pub changes: Vec<UndoChange>,
}

/// One note an undo changed: its text put back or a side write reverted
/// (`Modified`), a created note trashed or a move's destination vacated
/// (`Removed`), a move's source restored (`Added`). Only notes — a moved file
/// raises no note event (the glue layer's IndexReady refreshes the tree).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UndoChange {
    Modified(NoteId),
    Removed(NoteId),
    Added(NoteId),
}

impl UndoChange {
    /// The vault event that announces this change to the UI.
    pub fn event(&self) -> vemry_core::VaultEvent {
        match self {
            UndoChange::Modified(id) => vemry_core::VaultEvent::NoteModified { id: id.clone() },
            UndoChange::Removed(id) => vemry_core::VaultEvent::NoteRemoved { id: id.clone() },
            UndoChange::Added(id) => vemry_core::VaultEvent::NoteAdded { id: id.clone() },
        }
    }
}

/// The note id of a journaled path, when it is a note (a moved file's path is not).
fn note_at(vault: &Vault, path: &str) -> Option<NoteId> {
    path.ends_with(".md").then(|| vault.note_id(path).ok()).flatten()
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
    if let Some(to) = &rec.to {
        // A move: back only when the destination is still there and the source is free
        let root = vault.root().to_path_buf();
        if root.join(to).exists() && !root.join(&rec.path).exists() && vault.rename_path(to, &rec.path).is_ok() {
            report.reverted += 1;
            if let (Some(vacated), Some(restored)) = (note_at(vault, to), note_at(vault, &rec.path)) {
                report.changes.push(UndoChange::Removed(vacated));
                report.changes.push(UndoChange::Added(restored));
            }
        } else {
            report.skipped += 1;
        }
    } else {
        undo_note(vault, rec, report);
    }
    // The side writes landed after the main note, whatever the record kind (a move's
    // relinked referrers, a block move's destination), and go back alongside it
    for r in &rec.rewrites {
        if r.apply(vault, true) {
            report.reverted += 1;
            report.changes.extend(note_at(vault, &r.path).map(UndoChange::Modified));
        } else {
            report.skipped += 1;
        }
    }
}

/// The main note of a non-move record: its previous text back when it still holds
/// what the write left; a note the write created goes to the trash.
fn undo_note(vault: &mut Vault, rec: &WriteRecord, report: &mut UndoReport) {
    let (Ok(id), Some(new)) = (vault.note_id(&rec.path), &rec.new) else {
        report.skipped += 1;
        return;
    };
    let done = match (vault.read_note(&id), &rec.old) {
        (Ok(cur), Some(old)) if cur == *new => vault.write_note_as(&id, old, vemry_core::Writer::Agent).is_ok().then_some(UndoChange::Modified(id)),
        // The write created the note: it goes to the trash (recoverable), never deleted outright
        (Ok(cur), None) if cur == *new => vault.delete_path(id.as_str()).is_ok().then_some(UndoChange::Removed(id)),
        _ => None,
    };
    match done {
        Some(change) => {
            report.reverted += 1;
            report.changes.push(change);
        }
        None => report.skipped += 1,
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
        // newest first: the move back (two ends) and its rewrite, the creation, the edit
        assert_eq!(
            r.changes,
            vec![
                UndoChange::Removed(NoteId::new("moved/A.md")),
                UndoChange::Added(NoteId::new("A.md")),
                UndoChange::Modified(NoteId::new("Ref.md")),
                UndoChange::Removed(NoteId::new("New.md")),
                UndoChange::Modified(NoteId::new("A.md")),
            ]
        );
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

    /// A block move's record has no destination path of its own: the destination note
    /// and the relinked referrers travel as rewrites, and the undo puts them back too.
    #[test]
    fn undo_reverts_the_rewrites_of_a_block_move() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Src.md"), "keep\n\nmoved ^m1\n").unwrap();
        std::fs::write(dir.path().join("Dst.md"), "# Dst\n").unwrap();
        std::fs::write(dir.path().join("Ref.md"), "see [[Src#^m1]]\n").unwrap();
        let mut vault = Vault::open(dir.path()).unwrap();
        vault.write_note(&NoteId::new("Src.md"), "keep\n").unwrap();
        vault.write_note(&NoteId::new("Dst.md"), "# Dst\n\nmoved ^m1\n").unwrap();
        vault.write_note(&NoteId::new("Ref.md"), "see [[Dst#^m1]]\n").unwrap();
        let mut journal = vec![];
        record(&mut journal, WriteRecord {
            turn: Some("t1".into()), path: "Src.md".into(), old: Some("keep\n\nmoved ^m1\n".into()), new: Some("keep\n".into()), to: None,
            rewrites: vec![
                RewriteText { path: "Dst.md".into(), old: "# Dst\n".into(), new: "# Dst\n\nmoved ^m1\n".into() },
                RewriteText { path: "Ref.md".into(), old: "see [[Src#^m1]]\n".into(), new: "see [[Dst#^m1]]\n".into() },
            ],
        });
        let r = undo_turn(&mut vault, &mut journal, "t1");
        assert_eq!((r.reverted, r.skipped), (3, 0), "{r:?}");
        assert_eq!(std::fs::read_to_string(dir.path().join("Src.md")).unwrap(), "keep\n\nmoved ^m1\n");
        assert_eq!(std::fs::read_to_string(dir.path().join("Dst.md")).unwrap(), "# Dst\n");
        assert_eq!(std::fs::read_to_string(dir.path().join("Ref.md")).unwrap(), "see [[Src#^m1]]\n");
        assert_eq!(
            r.changes,
            vec![UndoChange::Modified(NoteId::new("Src.md")), UndoChange::Modified(NoteId::new("Dst.md")), UndoChange::Modified(NoteId::new("Ref.md"))]
        );
    }
}
