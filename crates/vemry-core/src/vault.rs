//! The Vault: orchestrates disk <-> parser <-> index <-> search.
//! Disk is the single source of truth; the index is always derived.

use rayon::prelude::*;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};
use walkdir::WalkDir;

use crate::block::{BlockKind, WikiLink};
use crate::blockid::{BlockSelector, PlanTarget};
use crate::event::VaultEvent;
use crate::{reject, PathReject};
use crate::index::BlockIndex;
use crate::note::{Note, NoteId};
use crate::parser::{extract_frontmatter, parse_note};
use crate::privacy::{PrivacyScope, PRIVACY_FILE};
use crate::provenance::{self, BlockProvenance, NoteFacts, Origin, ProvenanceEntry, ProvenanceLedger};
use crate::search::{InMemorySearch, SearchHit, SearchProvider};
use crate::{Result, VemryError};

/// How long after our own write we ignore watcher events for that path.
const SELF_WRITE_WINDOW: Duration = Duration::from_secs(2);

/// User scripts live here (vault-relative). Edits under it emit
/// `VaultEvent::ScriptsChanged` so the script host can hot-reload.
pub const SCRIPTS_DIR: &str = ".vemry/scripts";

/// Block types live here (vault-relative). Edits under it emit
/// `VaultEvent::TypesChanged`.
pub const TYPES_DIR: &str = ".vemry/types";

/// Text files above this open with the OS default app instead of the editor.
const MAX_TEXT_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// Preview files (images/PDF) above this open with the OS default app.
const MAX_PREVIEW_FILE_BYTES: u64 = 50 * 1024 * 1024;

/// Section names `memory_append` will append into, in the order it looks for
/// them. Lookup tries **every** name vemry has ever written so that a vault
/// created in one language and used in another still finds its section —
/// only creating the section uses the caller's current-language name.
/// The wording itself lives in `locales/*.json` (`memoryFile.*`); this list is
/// a fact about the file format, which is why it stays in core.
pub const MEMORY_SECTIONS: &[&str] = &["约定", "Conventions"];

#[derive(Clone, Debug, Serialize)]
pub struct BacklinkEntry {
    pub source: NoteId,
    pub source_title: String,
    pub snippet: String,
}

/// What a wikilink target points at inside a note.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RefTarget {
    Note,
    Heading { text: String, level: u8 },
    Block { local_id: String, block_kind: BlockKind },
}

/// A resolved reference: the note, the target inside it, and the raw slice
/// it denotes (the body for a note, the heading line through the end of its
/// section, a block's own lines — or, for a list item, its subtree).
#[derive(Clone, Debug, Serialize)]
pub struct RefHit {
    pub note: NoteId,
    pub note_title: String,
    pub target: RefTarget,
    /// Byte range in the note's raw text (`Block.span` convention).
    pub span: std::ops::Range<usize>,
    pub content: String,
}

/// Outcome of [`Vault::resolve_ref`]. `AnchorMissing` = the note exists
/// but the heading or `^id` does not (a positional `bN` never resolves).
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RefResolution {
    Resolved(RefHit),
    NoteMissing,
    AnchorMissing { note: NoteId, note_title: String },
}

/// Where [`Vault::insert_note`] actually landed the text. `Tail` with a
/// requested section = the heading was missing and the caller should say so.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InsertLanded {
    Section,
    Tail,
}

pub struct Vault {
    root: PathBuf,
    index: BlockIndex,
    search: Box<dyn SearchProvider>,
    self_writes: HashMap<NoteId, Instant>,
    /// Same suppression for non-Markdown `write_file` calls, keyed by rel
    /// path — a script's own file write must not bounce back as
    /// `FilesChanged` and re-trigger workflows.
    self_file_writes: HashMap<String, Instant>,
    privacy: PrivacyScope,
    /// Who wrote which block (see provenance.rs); loaded at open, appended on writes.
    provenance: ProvenanceLedger,
}

impl Vault {
    pub fn open(root: impl Into<PathBuf>) -> Result<Vault> {
        let root: PathBuf = root.into();
        if !root.is_dir() {
            return Err(reject(PathReject::NotADirectory, root.display().to_string()));
        }
        // dunce: on Windows std canonicalize returns the verbatim `\\?\D:\…`
        // form, which leaks all the way into the recent list and the UI; dunce
        // keeps the prefix only when it is required (very long or reserved names).
        let root = dunce::canonicalize(&root)?;

        let rel_paths: Vec<PathBuf> = WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !is_hidden(e.file_name().to_string_lossy().as_ref()) || e.depth() == 0)
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "md"))
            .filter_map(|e| e.path().strip_prefix(&root).ok().map(|p| p.to_path_buf()))
            .collect();

        let notes: Vec<Note> = rel_paths
            .par_iter()
            .filter_map(|rel| load_note(&root, rel).ok())
            .collect();

        let mut vault = Vault {
            privacy: PrivacyScope::load(&root),
            provenance: ProvenanceLedger::empty(&root),
            root,
            index: BlockIndex::new(),
            search: Box::new(InMemorySearch::new()),
            self_writes: HashMap::new(),
            self_file_writes: HashMap::new(),
        };
        for note in &notes {
            vault.index.upsert(note);
            vault.search.index_note(&note.id, note.effective_title(), &note.raw);
        }
        // The live hash set is only computed when the ledger is large enough to compact
        vault.provenance = ProvenanceLedger::load(&vault.root, || {
            notes.iter().flat_map(|n| provenance::live_hashes(&n.raw, &n.blocks)).collect()
        });
        Ok(vault)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn index(&self) -> &BlockIndex {
        &self.index
    }

    pub fn abs_path(&self, id: &NoteId) -> PathBuf {
        self.root.join(id.as_str())
    }

    /// Validated vault-relative id from a user/agent-supplied path string.
    /// Rejects escapes ("../") and absolute paths.
    pub fn note_id(&self, path: &str) -> Result<NoteId> {
        let p = validate_rel(path)?;
        let with_ext = if p.ends_with(".md") { p.to_string() } else { format!("{p}.md") };
        Ok(NoteId::new(&with_ext))
    }

    /// All folders in the vault (relative paths), including empty ones.
    pub fn list_folders(&self) -> Vec<String> {
        let mut v: Vec<String> = WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !is_hidden(e.file_name().to_string_lossy().as_ref()) || e.depth() == 0)
            .filter_map(|e| e.ok())
            .filter(|e| e.depth() > 0 && e.file_type().is_dir())
            .filter_map(|e| e.path().strip_prefix(&self.root).ok().map(rel_key))
            .collect();
        v.sort();
        v
    }

    /// Portability pre-check for a new path (see `validate_portable_rel`): an
    /// agent write path is rejected during prepare, before it ever reaches the
    /// permission gate; the executing rename/write still hits the same real gate.
    pub fn check_portable_rel(&self, path: &str) -> Result<()> {
        validate_portable_rel(path).map(|_| ())
    }

    pub fn create_folder(&self, path: &str) -> Result<String> {
        let rel = validate_portable_rel(path)?;
        fs::create_dir_all(self.root.join(rel))?;
        Ok(rel.to_string())
    }

    /// All files in the vault (relative paths, hidden dirs excluded), any type.
    pub fn list_files(&self) -> Vec<String> {
        let mut v: Vec<String> = WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !is_hidden(e.file_name().to_string_lossy().as_ref()) || e.depth() == 0)
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.path().strip_prefix(&self.root).ok().map(rel_key))
            .collect();
        v.sort();
        v
    }

    /// Create an empty file with the exact given name (explicit-extension
    /// semantics: the
    /// extension you type is what you get). `.md` files are indexed as notes.
    pub fn create_file(&mut self, path: &str) -> Result<String> {
        let rel = validate_portable_rel(path)?.to_string();
        let abs = self.root.join(&rel);
        if abs.exists() {
            return Err(reject(PathReject::Exists, rel));
        }
        if rel.ends_with(".md") {
            let id = NoteId::new(&rel);
            self.write_note(&id, "")?;
        } else {
            if let Some(parent) = abs.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&abs, "")?;
        }
        Ok(rel)
    }

    /// Read a non-Markdown vault file as UTF-8 text. Errors on binary or
    /// oversized content so callers can fall back to the OS default app.
    pub fn read_file(&self, path: &str) -> Result<String> {
        let rel = validate_rel(path)?;
        let abs = self.root.join(rel);
        let meta = fs::metadata(&abs).map_err(|_| reject(PathReject::NotFound, rel))?;
        if !meta.is_file() {
            return Err(reject(PathReject::NotAFile, rel));
        }
        if meta.len() > MAX_TEXT_FILE_BYTES {
            return Err(reject(PathReject::TooLarge, rel));
        }
        let bytes = fs::read(&abs)?;
        String::from_utf8(bytes).map_err(|_| reject(PathReject::NotUtf8, rel))
    }

    /// Read a vault file as raw bytes (image/PDF preview). Errors on oversized
    /// content so callers can fall back to the OS default app.
    pub fn read_file_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let rel = validate_rel(path)?;
        let abs = self.root.join(rel);
        let meta = fs::metadata(&abs).map_err(|_| reject(PathReject::NotFound, rel))?;
        if !meta.is_file() {
            return Err(reject(PathReject::NotAFile, rel));
        }
        if meta.len() > MAX_PREVIEW_FILE_BYTES {
            return Err(reject(PathReject::TooLarge, rel));
        }
        Ok(fs::read(&abs)?)
    }

    /// Save pasted/dropped bytes under the suggested relative path, deduping
    /// the filename (`-1`, `-2`, …) if taken. Returns the final relative path.
    pub fn save_attachment(&self, suggested: &str, bytes: &[u8]) -> Result<String> {
        let rel = validate_rel(suggested)?;
        if rel.ends_with(".md") {
            return Err(reject(PathReject::AttachmentIsMd, rel));
        }
        let candidate = self.dedup_rel(rel);
        let abs = self.root.join(&candidate);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&abs, bytes)?;
        Ok(candidate)
    }

    /// First free path for `rel`, appending `-1`, `-2`, … before the extension.
    /// Pub: clipping to disk (clips/<domain>/<title>.md) reuses this same
    /// same-name deduplication.
    pub fn dedup_rel(&self, rel: &str) -> String {
        let ext = Path::new(rel)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let stem = rel.strip_suffix(ext.as_str()).unwrap_or(rel);
        let mut candidate = rel.to_string();
        let mut n = 1;
        while self.root.join(&candidate).exists() {
            candidate = format!("{stem}-{n}{ext}");
            n += 1;
        }
        candidate
    }

    /// Import external files/folders (absolute paths — clipboard paste or a
    /// future drag&drop) into a vault folder by copying. `dest` is a
    /// vault-relative folder ("" = root). Names dedupe like `save_attachment`;
    /// folders copy recursively; any `.md` content gets indexed via a rescan.
    /// With `reuse_in_vault` (paste-into-note semantics: the caller wants a
    /// reference, not a duplicate) sources already inside the vault return
    /// their existing relative path instead of being copied.
    /// Returns the vault-relative paths created (or reused), in source order.
    pub fn import_files(&mut self, sources: &[PathBuf], dest: &str, reuse_in_vault: bool) -> Result<Vec<String>> {
        let dest_prefix = if dest.trim().trim_matches('/').is_empty() {
            String::new()
        } else {
            format!("{}/", validate_rel(dest)?)
        };
        let root_canon = dunce::canonicalize(&self.root).unwrap_or_else(|_| self.root.clone());
        let mut created = Vec::new();
        let mut needs_rescan = false;
        for src in sources {
            if self.root.starts_with(src) {
                return Err(reject(PathReject::IsTheVaultItself, src.display().to_string()));
            }
            if reuse_in_vault {
                let src_canon = dunce::canonicalize(src).unwrap_or_else(|_| src.clone());
                if let Ok(rel) = src_canon.strip_prefix(&root_canon) {
                    created.push(rel_key(rel));
                    continue;
                }
            }
            let Some(name) = src.file_name().map(|n| n.to_string_lossy().to_string()) else { continue };
            let Ok(meta) = fs::metadata(src) else { continue }; // vanished or unreadable — skip
            let rel = self.dedup_rel(&format!("{dest_prefix}{name}"));
            let abs = self.root.join(&rel);
            if let Some(parent) = abs.parent() {
                fs::create_dir_all(parent)?;
            }
            if meta.is_dir() {
                if abs.starts_with(src) {
                    return Err(reject(PathReject::FolderIntoItself, name));
                }
                copy_dir_recursive(src, &abs)?;
                needs_rescan = true; // may contain notes
            } else {
                fs::copy(src, &abs)?;
                needs_rescan |= rel.ends_with(".md");
            }
            created.push(rel);
        }
        if needs_rescan {
            self.rescan()?;
        }
        Ok(created)
    }

    /// Overwrite a non-Markdown vault file (no indexing). `.md` must go through
    /// `write_note` so the index stays consistent. Registers self-write
    /// suppression like the note paths do — without it, a script's own
    /// `write_file` would come back through the watcher as `FilesChanged` and
    /// re-trigger the very workflow that wrote it (infinite cascade).
    pub fn write_file(&mut self, path: &str, content: &str) -> Result<()> {
        let rel = validate_rel(path)?;
        if rel.ends_with(".md") {
            return Err(reject(PathReject::MdNeedsNoteApi, rel));
        }
        let abs = self.root.join(rel);
        if !abs.exists() {
            validate_portable_rel(rel)?;
        }
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&abs, content)?;
        self.self_file_writes.insert(rel.to_string(), Instant::now());
        Ok(())
    }

    /// Rename/move a file or folder, then rebuild the index (links pointing at
    /// an old note name become Unresolved — auto-refactoring is a future,
    /// not-yet-scheduled feature).
    pub fn rename_path(&mut self, from: &str, to: &str) -> Result<String> {
        let from_rel = validate_rel(from)?.to_string();
        let to_rel = validate_rel(to)?.to_string();
        if from_rel == to_rel {
            return Ok(to_rel);
        }
        // The portability gate only stops *newly appearing* names: a changed
        // last segment, a newly created intermediate folder. A pure move that
        // keeps the name, and a move into a folder that already exists, are let
        // through — otherwise an existing pathological name (`?`/`:` and friends,
        // legal on mac) could never be moved again once it existed, and even
        // undoing a move would fail and strand the file halfway, breaking
        // validate_portable_rel's promise that existing files stay readable and
        // movable.
        let from_name = from_rel.rsplit('/').next().unwrap_or(&from_rel);
        let segs: Vec<&str> = to_rel.split('/').collect();
        let mut prefix = self.root.clone();
        for (i, seg) in segs.iter().enumerate() {
            prefix.push(seg);
            let keeps_name = i == segs.len() - 1 && *seg == from_name;
            if !keeps_name && !prefix.exists() && !portable_seg(seg) {
                return Err(reject(PathReject::ReservedName, to));
            }
        }
        let from_abs = self.root.join(&from_rel);
        let to_abs = self.root.join(&to_rel);
        if !from_abs.exists() {
            return Err(reject(PathReject::NotFound, from_rel));
        }
        if to_abs.exists() {
            return Err(reject(PathReject::Exists, to_rel));
        }
        if let Some(parent) = to_abs.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&from_abs, &to_abs)?;
        self.rescan()?;
        Ok(to_rel)
    }

    /// Delete a file or folder by moving it to the OS trash (recoverable, so
    /// no confirmation dialog is needed), then
    /// rebuild the index.
    pub fn delete_path(&mut self, path: &str) -> Result<()> {
        let rel = validate_rel(path)?;
        let abs = self.root.join(rel);
        if !abs.exists() {
            return Err(reject(PathReject::NotFound, rel));
        }
        trash::delete(&abs).map_err(|e| VemryError::Io(std::io::Error::other(format!("failed to move to the trash: {e}"))))?;
        self.rescan()
    }

    /// Full index/search rebuild from disk. O(vault) but parse is parallel and
    /// vaults in the tens of thousands of notes rescan in well under a second.
    pub fn rescan(&mut self) -> Result<()> {
        let rel_paths: Vec<PathBuf> = WalkDir::new(&self.root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| !is_hidden(e.file_name().to_string_lossy().as_ref()) || e.depth() == 0)
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file() && e.path().extension().is_some_and(|x| x == "md"))
            .filter_map(|e| e.path().strip_prefix(&self.root).ok().map(|p| p.to_path_buf()))
            .collect();
        let notes: Vec<Note> = rel_paths.par_iter().filter_map(|rel| load_note(&self.root, rel).ok()).collect();
        self.index = BlockIndex::new();
        self.search = Box::new(InMemorySearch::new());
        for note in &notes {
            self.index.upsert(note);
            self.search.index_note(&note.id, note.effective_title(), &note.raw);
        }
        self.reload_privacy();
        Ok(())
    }

    pub fn read_note(&self, id: &NoteId) -> Result<String> {
        let path = self.abs_path(id);
        fs::read_to_string(&path).map_err(|_| VemryError::NoteNotFound(id.to_string()))
    }

    /// Attachment-bucket migration: `attachments/<stem>/` →
    /// `attachments/<note path minus .md>/`. Only buckets whose stem belongs to
    /// exactly one note move; mixed buckets, orphans and target collisions stay
    /// put and land in the report (the pure logic and the rules live in
    /// attach_migrate.rs). dry_run computes without touching the disk.
    /// Link rewriting scans **every** note (a Markdown link is a literal
    /// root-relative reference, so the referrer need not be the owning note) and
    /// goes through write_note (self-write suppression comes for free); the
    /// watcher echo of the directory moves is covered by the single rescan +
    /// broadcast the caller finishes with, rather than a broadcast per bucket.
    pub fn migrate_attachment_buckets(&mut self, dry_run: bool) -> Result<crate::attach_migrate::MigrationReport> {
        use crate::attach_migrate::{bucket_for, owner_of, rewrite_links, MigrationReport, Owner};
        let mut report = MigrationReport::default();
        let attachments = self.root.join("attachments");
        if !attachments.is_dir() {
            return Ok(report);
        }
        let note_ids: Vec<String> = self.index.summaries().into_iter().map(|s| s.id.to_string()).collect();
        // Only the immediate children matter: an old-style bucket is named after
        // a stem. A multi-level structure is either already new-style or something
        // the user built, so leave it alone
        let mut moves: Vec<(String, String)> = Vec::new();
        // Duplicate targets within the plan (case-insensitive): attachments/Note
        // and attachments/note resolve through the case-insensitive owner_of to
        // the same owner and compute the same new_rel, and the exists() check runs
        // at planning time, so it cannot catch a collision inside the plan — on the
        // real run the second rename would land on the non-empty directory the
        // first one just moved in, erroring halfway and leaving "directories moved,
        // not one link rewritten". Of two identical targets only the first in sort
        // order moves; the later one becomes a conflict
        let mut planned: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut entries: Vec<String> = fs::read_dir(&attachments)?
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        entries.sort(); // deterministic report and execution order
        for stem in entries {
            let old_rel = format!("attachments/{stem}");
            // Directory name = a folder in the vault → this mirrors the parent of
            // a new-style bucket (attachments/projects/…), not an old-style bucket:
            // skip the orphan verdict; should it also collide with a note of the
            // same name, moving it would drag the new-style bucket along, so it
            // becomes a conflict and stays put
            let folder_prefix = format!("{stem}/");
            let mirrors_folder = note_ids.iter().any(|id| id.starts_with(&folder_prefix));
            match owner_of(&stem, &note_ids) {
                Owner::Mixed => report.mixed.push(old_rel),
                Owner::None => {
                    if !mirrors_folder {
                        report.orphaned.push(old_rel);
                    }
                }
                Owner::Unique(owner) => {
                    if mirrors_folder {
                        report.conflicts.push(old_rel);
                        continue;
                    }
                    let new_rel = bucket_for(owner);
                    // A root-level note has the same old and new path (compared
                    // case-insensitively: on APFS, renaming to a differently cased
                    // spelling of itself is meaningless)
                    if new_rel.to_lowercase() == old_rel.to_lowercase() {
                        continue;
                    }
                    if self.root.join(&new_rel).exists() {
                        report.conflicts.push(old_rel);
                        continue;
                    }
                    if !planned.insert(new_rel.to_lowercase()) {
                        report.conflicts.push(old_rel);
                        continue;
                    }
                    moves.push((old_rel, new_rel));
                }
            }
        }
        // Preview of the rewrite set (computed under dry_run too, so the report
        // gives an exact count of rewritten notes). Read note by note from disk
        // rather than from the index cache: the migration is a one-off, and disk
        // is the source of truth
        let mut rewrites: Vec<(NoteId, String)> = Vec::new();
        for id_str in &note_ids {
            let id = NoteId::new(id_str);
            let Ok(raw) = self.read_note(&id) else { continue };
            // One substring scan rules out the vast majority of notes, saving N
            // full-text rewrites per note (N = number of moves)
            if !raw.contains("attachments/") {
                continue;
            }
            let mut cur: Option<String> = None;
            for (old, new) in &moves {
                if let Some(next) = rewrite_links(cur.as_deref().unwrap_or(&raw), old, new) {
                    cur = Some(next);
                }
            }
            if let Some(content) = cur {
                rewrites.push((id, content));
            }
        }
        report.rewritten_notes = rewrites.len();
        report.moved = moves.clone();
        if dry_run {
            return Ok(report);
        }
        // Move directories first, rewrite links second: a failed rewrite only
        // leaves links pointing at the old location (the files are already in the
        // new one and can be re-linked by hand), while the reverse order would
        // write out dead links first
        for (old, new) in &moves {
            let to = self.root.join(new);
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(self.root.join(old), &to)?;
        }
        for (id, content) in &rewrites {
            self.write_note(id, content)?;
        }
        Ok(report)
    }

    /// Write full raw content and re-index the note.
    pub fn write_note(&mut self, id: &NoteId, content: &str) -> Result<VaultEvent> {
        let path = self.abs_path(id);
        if !path.exists() {
            validate_portable_rel(id.as_str())?;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let existed = self.index.contains(id);
        fs::write(&path, content)?;
        self.self_writes.insert(id.clone(), Instant::now());
        self.reindex_note(id)?;
        Ok(if existed { VaultEvent::NoteModified { id: id.clone() } } else { VaultEvent::NoteAdded { id: id.clone() } })
    }

    /// Positioned insert (clip / quick capture): land at the end of
    /// `section`'s heading section when given and present, else at the
    /// note's normalized tail. Missing notes are created (append
    /// semantics). The caller surfaces the `Tail` fallback to the user
    /// when a section was requested — never silent, never an error.
    pub fn insert_note(
        &mut self,
        id: &NoteId,
        text: &str,
        section: Option<&str>,
    ) -> Result<(VaultEvent, InsertLanded)> {
        // Locate against the content read on this call; never trust a stale span
        // from an earlier one
        let existing = self.read_note(id).unwrap_or_default();
        let offset = section
            .and_then(|h| crate::insert::section_insert_offset(&parse_note(&existing).blocks, &existing, h));
        let landed = if offset.is_some() { InsertLanded::Section } else { InsertLanded::Tail };
        let content = crate::insert::insert_at(
            &existing,
            offset.unwrap_or_else(|| crate::insert::content_end_offset(&existing)),
            text,
        );
        let event = self.write_note(id, &content)?;
        Ok((event, landed))
    }

    /// Give a block a stable `^id` if it has none, and return the id. The
    /// note is re-read and re-parsed on this call (never a stale span);
    /// `local_id` comes from a query hit (explicit or positional) and
    /// `guard` is that hit's content, so an edited note refuses instead of
    /// tagging the wrong block. An explicit id is returned without a write.
    pub fn ensure_block_id(
        &mut self,
        id: &NoteId,
        local_id: &str,
        guard: Option<&str>,
    ) -> Result<(String, Option<VaultEvent>)> {
        let raw = self.read_note(id)?;
        let selector = BlockSelector::Id { local_id: local_id.to_string(), guard: guard.map(String::from) };
        let plan = crate::blockid::plan_block_id(&raw, &selector)?;
        let block_id = match plan.target {
            PlanTarget::Block { id } => id,
            PlanTarget::Heading { .. } => return Err(VemryError::BlockNotIdable),
        };
        match plan.insert {
            None => Ok((block_id, None)),
            Some(ins) => {
                let mut content = raw;
                content.insert_str(ins.offset, &ins.text);
                let event = self.write_note(id, &content)?;
                Ok((block_id, Some(event)))
            }
        }
    }

    /// Resolve the inside of a `[[…]]` (`Note`, `Note#Heading`, `Note#^id`,
    /// `#^id`; an alias part is ignored) to the note and the slice it
    /// denotes. The note part goes through the index's resolver; the
    /// fragment is looked up in a fresh parse of the file. `from` supplies
    /// the note for an empty target.
    pub fn resolve_ref(&self, target: &str, from: Option<&NoteId>) -> Result<RefResolution> {
        let link = WikiLink::parse(target, 0..0);
        let note_id = if link.target.is_empty() {
            match from {
                Some(f) => f.clone(),
                None => return Ok(RefResolution::NoteMissing),
            }
        } else {
            let fallback = NoteId::new("_.md");
            match self.index.resolve_target(&link.target, from.unwrap_or(&fallback)) {
                Some(id) => id,
                None => return Ok(RefResolution::NoteMissing),
            }
        };
        let Ok(raw) = self.read_note(&note_id) else {
            return Ok(RefResolution::NoteMissing);
        };
        let parsed = parse_note(&raw);
        let note_title =
            self.index.summary(&note_id).map(|s| s.title).unwrap_or_else(|| note_id.stem().to_string());
        let missing = || RefResolution::AnchorMissing { note: note_id.clone(), note_title: note_title.clone() };
        let (target, span) = if let Some(id) = &link.block_ref {
            match parsed.blocks.iter().position(|b| b.explicit_id && b.local_id == *id) {
                Some(i) => {
                    let b = &parsed.blocks[i];
                    (
                        RefTarget::Block { local_id: b.local_id.clone(), block_kind: b.kind.clone() },
                        crate::locate::item_subtree_span(&parsed.blocks, &raw, i),
                    )
                }
                None => return Ok(missing()),
            }
        } else if let Some(h) = link.heading.as_deref().filter(|h| !h.trim().is_empty()) {
            match crate::locate::find_heading(&parsed.blocks, h) {
                Some(i) => {
                    let b = &parsed.blocks[i];
                    let level = match b.kind {
                        BlockKind::Heading { level } => level,
                        _ => 0,
                    };
                    (
                        RefTarget::Heading { text: crate::locate::heading_text(b).to_string(), level },
                        crate::locate::section_span_at(&parsed.blocks, &raw, i),
                    )
                }
                None => return Ok(missing()),
            }
        } else {
            let end = raw.trim_end().len().max(parsed.body_offset);
            (RefTarget::Note, parsed.body_offset..end)
        };
        Ok(RefResolution::Resolved(RefHit {
            note: note_id,
            note_title,
            target,
            content: raw[span.clone()].to_string(),
            span,
        }))
    }

    // ---- provenance: who authored each block (provenance.rs) ----

    /// The origin of every block of a note: the ledger first, then the marks
    /// the text carries, then the note's frontmatter, else the user.
    pub fn provenance(&self, id: &NoteId) -> Result<Vec<BlockProvenance>> {
        let raw = self.read_note(id)?;
        let parsed = parse_note(&raw);
        let fm = &parsed.frontmatter;
        let facts = NoteFacts {
            origin: fm.vemry.as_ref().and_then(|v| v.origin.as_deref()),
            agent_session: fm.vemry.as_ref().and_then(|v| v.agent_session.as_deref()),
            source: fm.extra.get("source").and_then(|v| v.as_str()),
            clipped: fm.extra.get("clipped").and_then(|v| v.as_str()),
        };
        let known = self.provenance.has_path(id.as_str());
        Ok(parsed
            .blocks
            .iter()
            .map(|b| {
                let slice = &raw[b.span.clone()];
                let hash = provenance::block_hash(slice, &b.kind);
                let record = hash.as_deref().and_then(|h| self.provenance.lookup(h));
                BlockProvenance {
                    local_id: b.local_id.clone(),
                    span: b.span.clone(),
                    origin: provenance::derive_origin(slice, record, &facts, known),
                    hash: hash.unwrap_or_default(),
                }
            })
            .collect())
    }

    /// Agent-written blocks of a note still awaiting review.
    pub fn pending_count(&self, id: &NoteId) -> usize {
        self.provenance(id)
            .map(|v| v.iter().filter(|p| matches!(p.origin, Origin::Agent { pending: true, .. })).count())
            .unwrap_or(0)
    }

    /// Every note with pending blocks, with the count (only notes the ledger
    /// ever recorded are candidates; a pending block moved into another note
    /// keeps its state but escapes this count).
    pub fn pending_notes(&self) -> Vec<(NoteId, usize)> {
        let mut v: Vec<(NoteId, usize)> = self
            .provenance
            .paths()
            .map(NoteId::new)
            .filter(|id| self.index.contains(id))
            .map(|id| {
                let n = self.pending_count(&id);
                (id, n)
            })
            .filter(|(_, n)| *n > 0)
            .collect();
        v.sort();
        v
    }

    /// Append one ledger entry (an agent write, or text the user placed).
    pub fn record_provenance(&mut self, entry: ProvenanceEntry) {
        self.provenance.append(entry);
    }

    /// Accept the pending agent blocks of a note — one by local id, or all of
    /// them — and return how many are still pending.
    pub fn accept_provenance(&mut self, id: &NoteId, local_id: Option<&str>) -> Result<usize> {
        let prov = self.provenance(id)?;
        let pending = |p: &BlockProvenance| matches!(p.origin, Origin::Agent { pending: true, .. });
        let hashes: Vec<String> = match local_id {
            None => prov.iter().filter(|p| pending(p)).map(|p| p.hash.clone()).collect(),
            Some(lid) => {
                let p = prov
                    .iter()
                    .find(|p| p.local_id == lid)
                    .ok_or_else(|| VemryError::BlockNotFound { local_id: lid.to_string() })?;
                if pending(p) {
                    vec![p.hash.clone()]
                } else {
                    vec![]
                }
            }
        };
        if !hashes.is_empty() {
            self.provenance.append(ProvenanceEntry::Accept { ts: provenance::now_ts(), hashes });
        }
        Ok(self.pending_count(id))
    }

    /// Append text (with separating newline) — used by quick capture.
    pub fn append_note(&mut self, id: &NoteId, text: &str) -> Result<VaultEvent> {
        let existing = self.read_note(id).unwrap_or_default();
        let mut content = existing;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(text);
        if !content.ends_with('\n') {
            content.push('\n');
        }
        self.write_note(id, &content)
    }

    /// Create an empty note with a unique, sanitized filename.
    pub fn create_note(&mut self, title: &str, folder: Option<&str>) -> Result<NoteId> {
        let safe = sanitize_filename(title);
        let folder = folder.unwrap_or("").trim_matches('/');
        let mut candidate = if folder.is_empty() { format!("{safe}.md") } else { format!("{folder}/{safe}.md") };
        let mut n = 1;
        while self.abs_path(&NoteId::new(&candidate)).exists() {
            candidate = if folder.is_empty() { format!("{safe} {n}.md") } else { format!("{folder}/{safe} {n}.md") };
            n += 1;
        }
        let id = NoteId::new(&candidate);
        self.write_note(&id, "")?;
        Ok(id)
    }

    pub fn delete_note(&mut self, id: &NoteId) -> Result<VaultEvent> {
        let path = self.abs_path(id);
        fs::remove_file(&path).map_err(|_| VemryError::NoteNotFound(id.to_string()))?;
        self.self_writes.insert(id.clone(), Instant::now());
        self.index.remove(id);
        self.search.remove_note(id);
        Ok(VaultEvent::NoteRemoved { id: id.clone() })
    }

    fn reindex_note(&mut self, id: &NoteId) -> Result<()> {
        let rel = PathBuf::from(id.as_str());
        let note = load_note(&self.root, &rel)?;
        self.search.index_note(&note.id, note.effective_title(), &note.raw);
        self.index.upsert(&note);
        Ok(())
    }

    /// Apply debounced watcher paths. Returns events for changes that were NOT
    /// caused by our own writes.
    pub fn handle_fs_change(&mut self, paths: &[PathBuf]) -> Vec<VaultEvent> {
        let mut events = Vec::new();
        let mut files_changed = false;
        let mut scripts_changed = false;
        let mut types_changed = false;
        for path in paths {
            let Ok(rel) = path.strip_prefix(&self.root) else { continue };
            // The rules file lives in a hidden dir, so it must be handled
            // before the hidden-segment skip below.
            if rel == Path::new(PRIVACY_FILE) {
                self.reload_privacy();
                events.push(VaultEvent::PrivacyChanged);
                continue;
            }
            // Same for user scripts: `.vemry/scripts/*.js` edits must reach
            // the script host (hot reload) despite living in a hidden dir.
            // The dir itself counts too: deleting/renaming `.vemry/scripts`
            // reports only the directory path, and stale contexts must not
            // keep running scripts whose files are gone.
            if rel == Path::new(SCRIPTS_DIR)
                || (rel.starts_with(SCRIPTS_DIR) && rel.extension().is_some_and(|x| x == "js"))
            {
                scripts_changed = true;
                continue;
            }
            if rel == Path::new(TYPES_DIR)
                || (rel.starts_with(TYPES_DIR) && rel.extension().is_some_and(|x| x == "md"))
            {
                types_changed = true;
                continue;
            }
            if rel.components().any(|c| is_hidden(c.as_os_str().to_string_lossy().as_ref())) {
                continue;
            }
            if path.extension().map(|x| x != "md").unwrap_or(true) {
                // our own write_file bouncing back — same window as notes.
                // The registered key is validate_rel's "/" form, so a lookup must
                // go through the same rel_key normalisation (on Windows a rel path
                // renders with backslashes, and without normalising, suppression
                // misses every time — measured in rc.1 CI).
                let key = rel_key(rel);
                if let Some(t) = self.self_file_writes.get(&key) {
                    if t.elapsed() < SELF_WRITE_WINDOW {
                        continue;
                    }
                    self.self_file_writes.remove(&key);
                }
                // non-note file or folder: the tree still has to pick it up
                files_changed = true;
                continue;
            }
            let id = NoteId::from_rel_path(rel);
            if let Some(t) = self.self_writes.get(&id) {
                if t.elapsed() < SELF_WRITE_WINDOW {
                    continue;
                }
                self.self_writes.remove(&id);
            }
            if path.exists() {
                let existed = self.index.contains(&id);
                if self.reindex_note(&id).is_ok() {
                    events.push(if existed {
                        VaultEvent::NoteModified { id }
                    } else {
                        VaultEvent::NoteAdded { id }
                    });
                }
            } else if self.index.contains(&id) {
                self.index.remove(&id);
                self.search.remove_note(&id);
                events.push(VaultEvent::NoteRemoved { id });
            }
        }
        if files_changed {
            events.push(VaultEvent::FilesChanged);
        }
        if scripts_changed {
            events.push(VaultEvent::ScriptsChanged);
        }
        if types_changed {
            events.push(VaultEvent::TypesChanged);
        }
        events
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        self.search.search(query, limit)
    }

    // ---- the agent's reads: every one of these strips `> [!private]`
    // blocks (see privacy.rs). Named rather than flagged so the choke points
    // can be grepped, and so the raw `read_note` its many UI callers use can
    // never be handed to the agent by a wrong default. ----

    /// Whether the note holds a private block (index state; a cheap trigger).
    pub fn has_private_blocks(&self, id: &NoteId) -> bool {
        self.index.blocks(id).is_some_and(crate::privacy::has_private_blocks)
    }

    /// The note as the agent may read it: disk-fresh, re-parsed on this
    /// call (index spans can lag the watcher), private blocks removed.
    pub fn read_note_for_agent(&self, id: &NoteId) -> Result<crate::privacy::Redacted> {
        let raw = self.read_note(id)?;
        let blocks = parse_note(&raw).blocks;
        Ok(crate::privacy::redact_private_blocks(&raw, &blocks))
    }

    /// Full-text search as the agent may see it. A hit in a note that holds
    /// private blocks is re-matched over a blanked copy of the note: the
    /// snippet can never come from a private line, and a query whose terms
    /// are only satisfied together with private text drops out — otherwise
    /// the AND matcher would confirm private words one at a time.
    pub fn search_for_agent(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let terms = crate::search::query_terms(query);
        let mut hits: Vec<SearchHit> = self
            .search(query, limit)
            .into_iter()
            .filter_map(|h| {
                if !self.has_private_blocks(&h.note) {
                    return Some(h);
                }
                let raw = self.read_note(&h.note).ok()?;
                let blocks = parse_note(&raw).blocks;
                let blanked = crate::privacy::blank_private_blocks(&raw, &blocks);
                let (score, snippet, line) = crate::search::match_doc(&h.title, &blanked, &terms)?;
                Some(SearchHit { note: h.note, title: h.title, snippet, line, score })
            })
            .collect();
        crate::search::sort_hits(&mut hits);
        hits
    }

    /// VEMRY.md as the agent may read it (private blocks removed).
    pub fn memory_file_for_agent(&self) -> Option<String> {
        let raw = self.memory_file()?;
        let blocks = parse_note(&raw).blocks;
        Some(crate::privacy::redact_private_blocks(&raw, &blocks).text)
    }

    /// `resolve_ref` as the agent may use it: a private block is an anchor
    /// that does not exist, and a section or subtree is handed over with its
    /// private blocks removed.
    pub fn resolve_ref_for_agent(&self, target: &str, from: Option<&NoteId>) -> Result<RefResolution> {
        match self.resolve_ref(target, from)? {
            RefResolution::Resolved(hit) => {
                if matches!(hit.target, RefTarget::Block { block_kind: BlockKind::Private, .. }) {
                    return Ok(RefResolution::AnchorMissing { note: hit.note, note_title: hit.note_title });
                }
                let blocks = crate::parser::parse_blocks(&hit.content, 0);
                let content = crate::privacy::redact_private_blocks(&hit.content, &blocks).text;
                Ok(RefResolution::Resolved(RefHit { content, ..hit }))
            }
            other => Ok(other),
        }
    }

    /// One block of a note by its local id (`^id` explicit or `bN` positional —
    /// explicit wins a clash), as the agent may see it: a heading hands over its
    /// section, a list item or task its subtree, anything else the block alone,
    /// with private blocks removed from the slice; a private block itself does
    /// not exist. None = no such block.
    pub fn block_for_agent(&self, id: &NoteId, local_id: &str) -> Result<Option<RefHit>> {
        let raw = self.read_note(id)?;
        let parsed = parse_note(&raw);
        let Some(b) = crate::locate::find_block(&parsed.blocks, local_id) else {
            return Ok(None);
        };
        let Some(i) = parsed.blocks.iter().position(|x| std::ptr::eq(x, b)) else {
            return Ok(None);
        };
        let (target, span) = match b.kind {
            BlockKind::Private => return Ok(None),
            BlockKind::Heading { level } => (
                RefTarget::Heading { text: crate::locate::heading_text(b).to_string(), level },
                crate::locate::section_span_at(&parsed.blocks, &raw, i),
            ),
            _ => (
                RefTarget::Block { local_id: b.local_id.clone(), block_kind: b.kind.clone() },
                crate::locate::item_subtree_span(&parsed.blocks, &raw, i),
            ),
        };
        let slice = &raw[span.clone()];
        let blocks = crate::parser::parse_blocks(slice, 0);
        let content = crate::privacy::redact_private_blocks(slice, &blocks).text;
        let note_title = self.index.summary(id).map(|s| s.title).unwrap_or_else(|| id.stem().to_string());
        Ok(Some(RefHit { note: id.clone(), note_title, target, content, span }))
    }

    /// Backlinks with a context snippet from the linking block.
    pub fn backlinks(&self, id: &NoteId) -> Vec<BacklinkEntry> {
        self.index
            .backlinks_of(id)
            .into_iter()
            .filter_map(|source| {
                let summary = self.index.summary(&source)?;
                let blocks = self.index.blocks(&source)?;
                let snippet = blocks
                    .iter()
                    .find(|b| {
                        b.links.iter().any(|l| {
                            self.index.resolve_target(&l.target, &source).as_ref() == Some(id)
                        })
                    })
                    .map(|b| truncate_chars(b.content.trim(), 200))
                    .unwrap_or_default();
                Some(BacklinkEntry { source, source_title: summary.title, snippet })
            })
            .collect()
    }

    /// Read VEMRY.md if present (the vault constitution injected into the
    /// agent system prompt).
    pub fn memory_file(&self) -> Option<String> {
        fs::read_to_string(self.root.join("VEMRY.md")).ok()
    }


    /// Append one convention bullet to VEMRY.md, landing at the end of its
    /// conventions section (`## 约定` / `## Conventions`); the file or the
    /// section is scaffolded when missing. User-initiated only (the `#`
    /// quick-memory in chat) — the agent's own path to VEMRY.md stays edit_note
    /// through the permission gate.
    /// `header` and `section` are the current-language VEMRY.md scaffolding
    /// (`locales/*.json`, injected by the glue layer — core stays copy-free).
    pub fn memory_append(&mut self, text: &str, header: &str, section: &str) -> Result<VaultEvent> {
        let mut lines = text.trim().lines();
        let first = lines.next().unwrap_or_default().trim();
        if first.is_empty() {
            return Err(reject(PathReject::Empty, "memory entry"));
        }
        let mut entry = format!("- {first}");
        for line in lines {
            entry.push_str("\n  ");
            entry.push_str(line.trim());
        }
        let id = self.note_id("VEMRY.md")?;
        // The section to append into: whichever known name this file already
        // has. Only when the file has none do we create the caller's.
        let mut target = section;
        match self.read_note(&id) {
            Err(_) => {
                entry = format!("{header}\n## {section}\n{entry}");
            }
            Ok(existing) => {
                let blocks = parse_note(&existing).blocks;
                match MEMORY_SECTIONS
                    .iter()
                    .find(|h| crate::insert::section_insert_offset(&blocks, &existing, h).is_some())
                {
                    Some(found) => target = found,
                    None => entry = format!("## {section}\n{entry}"),
                }
            }
        }
        let (event, _) = self.insert_note(&id, &entry, Some(target))?;
        Ok(event)
    }

    /// Vault-level config directory (.vemry) — a **pure path accessor that does
    /// not create anything**. Read paths (listing sessions/skills, reading config)
    /// must not leave a .vemry side effect in a directory that is merely being
    /// browsed: otherwise the "a directory opened by mistake does not enter the
    /// recent list" check would, on the second open, be fooled by the traces it
    /// left itself, and every folder ever glanced at would grow a stray directory.
    /// Write paths call create_dir_all themselves (creating a subdirectory creates
    /// its parent too).
    pub fn meta_dir(&self) -> PathBuf {
        self.root.join(".vemry")
    }

    pub fn privacy(&self) -> &PrivacyScope {
        &self.privacy
    }

    /// Re-read `.vemry/privacy` from disk.
    pub fn reload_privacy(&mut self) {
        self.privacy = PrivacyScope::load(&self.root);
    }

    /// Is this note in the privacy zone (glob rules or frontmatter flag)?
    /// The frontmatter half resolves case-insensitively: a case-variant
    /// spelling the filesystem folds onto a `private: true` note must not
    /// slip past (the glob half is already case-insensitive by construction).
    pub fn is_private(&self, id: &NoteId) -> bool {
        self.privacy.matches_rel(id.as_str(), false)
            || self
                .index
                .canonical_id(id)
                .and_then(|cid| self.index.summary(&cid))
                .is_some_and(|s| s.private)
    }

    /// Privacy check for an arbitrary vault-relative path (any file type or
    /// folder). Normalizes like `NoteId` so agent-supplied spellings match.
    pub fn is_private_rel(&self, rel: &str, is_dir: bool) -> bool {
        let normalized = NoteId::new(rel);
        if self.privacy.matches_rel(normalized.as_str(), is_dir) {
            return true;
        }
        !is_dir
            && self
                .index
                .canonical_id(&normalized)
                .and_then(|cid| self.index.summary(&cid))
                .is_some_and(|s| s.private)
    }

    /// Set/clear the frontmatter `private` flag in place, preserving every
    /// other field. Goes through `write_note` so index and search update and
    /// the watcher self-write suppression applies.
    pub fn set_note_private(&mut self, id: &NoteId, private: bool) -> Result<VaultEvent> {
        let raw = self.read_note(id)?;
        let (mut fm, body_offset) = extract_frontmatter(&raw);
        fm.private = private;
        let body = &raw[body_offset..];
        let content = if fm.is_empty() {
            body.to_string()
        } else {
            format!("---\n{}---\n{}", fm.to_yaml()?, body)
        };
        self.write_note(id, &content)
    }

    /// Append a rule line to `.vemry/privacy` (created on demand), then
    /// reload. Duplicate lines are not added twice.
    pub fn add_privacy_rule(&mut self, line: &str) -> Result<()> {
        let line = line.trim();
        if line.is_empty() {
            return Err(reject(PathReject::Empty, "privacy rule"));
        }
        let dir = self.meta_dir();
        fs::create_dir_all(&dir)?;
        let path = dir.join("privacy");
        let mut content = fs::read_to_string(&path).unwrap_or_default();
        if !content.lines().any(|l| l.trim() == line) {
            if !content.is_empty() && !content.ends_with('\n') {
                content.push('\n');
            }
            content.push_str(line);
            content.push('\n');
            fs::write(&path, content)?;
        }
        self.reload_privacy();
        Ok(())
    }

    /// Remove the exact rule line from `.vemry/privacy`, then reload. A path
    /// still covered by a broader rule stays private.
    pub fn remove_privacy_rule(&mut self, line: &str) -> Result<()> {
        let line = line.trim();
        let path = self.root.join(PRIVACY_FILE);
        let Ok(content) = fs::read_to_string(&path) else { return Ok(()) };
        let kept: Vec<&str> = content.lines().filter(|l| l.trim() != line).collect();
        let mut out = kept.join("\n");
        if !out.is_empty() {
            out.push('\n');
        }
        fs::write(&path, out)?;
        self.reload_privacy();
        Ok(())
    }
}

fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// The key form of a vault-relative path: "/" separated everywhere (the form
/// validate_rel registers, normalised the same way as NoteId). On Windows a Path
/// renders with backslashes, so it must pass through here before entering any
/// map or list — patching each consumer separately is bound to miss one sooner or
/// later (measured on the rc.1 watcher suppression).
fn rel_key(rel: &Path) -> String {
    rel.to_string_lossy().replace('\\', "/")
}

/// Copy a directory tree (files and subdirs; symlinks skipped).
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    for entry in WalkDir::new(src).follow_links(false) {
        let entry = entry.map_err(|e| VemryError::Io(std::io::Error::other(e)))?;
        let rel = entry.path().strip_prefix(src).expect("walkdir yields children of src");
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Characters a file name may not contain: `sanitize_filename` substitutes on
/// this list and `validate_portable_rel` rejects on it — the producer and the
/// gate share this table and `is_reserved_stem`, which is what makes a sanitized
/// name certain to pass the gate.
/// `/` is a separator on the gate's side, so it is listed separately.
const BAD_NAME_CHARS: [char; 8] = ['\\', ':', '*', '?', '"', '<', '>', '|'];

fn is_bad_name_char(c: char) -> bool {
    BAD_NAME_CHARS.contains(&c) || c.is_control()
}

/// Windows reserved device names (the stem is the first `.`-separated segment of
/// the file name, compared case-insensitively; a name with an extension counts as
/// reserved too). Gate and producer share one copy — see the note on
/// `BAD_NAME_CHARS`.
fn is_reserved_stem(stem: &str) -> bool {
    let stem = stem.to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || matches!(stem.as_bytes(), [b'C', b'O', b'M', n] | [b'L', b'P', b'T', n] if n.is_ascii_digit() && *n != b'0')
}

/// Title → safe filename component: forbidden filesystem chars become spaces,
/// empty falls back to "Untitled". Shared by create_note and clip landing.
pub fn sanitize_filename(title: &str) -> String {
    let safe: String = title
        .chars()
        .map(|c| if c == '/' || is_bad_name_char(c) { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_string();
    if safe.is_empty() {
        return "Untitled".to_string();
    }
    // Reserved device names are judged on the stem, and appending `.md` does not
    // save them ("con.figure.md" still has the stem "con"): a leading "_" empties
    // the stem, so a sanitized name always passes the gate.
    if is_reserved_stem(safe.split('.').next().unwrap_or_default()) {
        return format!("_{safe}");
    }
    safe
}

/// Reject empty, absolute, `~` and `..`-escaping paths; return trimmed rel path.
/// Backslashes are split on as separators too: normalisation (NoteId::new's
/// `\`→`/`) happens after validation, so splitting on `/` alone would let an
/// escape of the form `..\..\x.md` through.
fn validate_rel(path: &str) -> Result<&str> {
    let p = path.trim().trim_matches('/');
    if p.is_empty()
        || path.trim().starts_with('/')
        || path.trim().starts_with('\\')
        || p.starts_with('~')
        || p.split(['/', '\\']).any(|seg| seg == "..")
    {
        return Err(reject(PathReject::Escapes, path));
    }
    Ok(p)
}

/// Portability check for a create/rename destination: on every platform, reject
/// the file names Windows does not allow (illegal characters and `\`, reserved
/// device names, trailing spaces or dots) — otherwise a file written on mac is a
/// file that cannot be opened once it syncs to Windows. Applied to new paths
/// only: existing files stay readable and movable.
fn validate_portable_rel(path: &str) -> Result<&str> {
    let p = validate_rel(path)?;
    if p.split('/').any(|seg| !portable_seg(seg)) {
        return Err(reject(PathReject::ReservedName, path));
    }
    Ok(p)
}

/// The single-segment verdict of `validate_portable_rel`: `rename_path` reuses
/// the same rule when exempting a segment.
fn portable_seg(seg: &str) -> bool {
    !seg.chars().any(is_bad_name_char)
        && !is_reserved_stem(seg.split('.').next().unwrap_or_default())
        && !seg.ends_with(' ')
        && !seg.ends_with('.')
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}

fn load_note(root: &Path, rel: &Path) -> Result<Note> {
    let abs = root.join(rel);
    let raw = fs::read_to_string(&abs)?;
    let mtime = fs::metadata(&abs).and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
    let parsed = parse_note(&raw);
    let id = NoteId::from_rel_path(rel);
    let title = parsed.frontmatter.title.clone().unwrap_or_else(|| id.stem().to_string());
    Ok(Note {
        id,
        title,
        frontmatter: parsed.frontmatter,
        content: raw[parsed.body_offset..].to_string(),
        raw,
        blocks: parsed.blocks,
        mtime,
    })
}

#[cfg(test)]
mod tests {
    /// The glue layer supplies the real header from the catalog; tests only need
    /// something stable to assert against.
    const TEST_MEMORY_HEADER: &str = "# VEMRY.md\n";

    use super::*;
    use crate::block::BlockKind;
    use crate::query::{BlockKindFilter, BlockQuery};

    fn fixture_vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("projects")).unwrap();
        fs::write(
            root.join("Sleep.md"),
            "---\ntitle: 睡眠研究\naliases: [sleep-research]\ntags: [research]\n---\n# 睡眠\n\n> [!claim] 睡眠影响记忆 ^c1\n\n- [ ] 阅读 Walker 的书 📅 2026-08-20 #reading\n\n链接到 [[projects/Alpha|阿尔法]]。\n",
        )
        .unwrap();
        fs::write(
            root.join("projects/Alpha.md"),
            "# Alpha\n\n参考 [[Sleep]] 与 [[sleep-research]] 以及 [[Missing Note]]。\n\n> [!evidence] 实验数据 supports:[[Sleep#^c1]]\n",
        )
        .unwrap();
        fs::write(root.join(".vemry-ignored.md"), "should not matter").unwrap();
        let vault = Vault::open(root).unwrap();
        (dir, vault)
    }

    #[test]
    fn query_list_items_and_stable_limit_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (name, body) in [("b.md", "- b1\n- b2\n"), ("a.md", "- a1\n  - a1a\n\npara\n"), ("c.md", "- c1\n")] {
            fs::write(root.join(name), body).unwrap();
        }
        let vault = Vault::open(root).unwrap();
        let items = vault.index().query(&BlockQuery { kind: Some(BlockKindFilter::ListItem), ..Default::default() });
        assert_eq!(items.len(), 5);
        assert!(items.iter().all(|h| matches!(h.block.kind, BlockKind::ListItem { .. })));
        let paras = vault.index().query(&BlockQuery { kind: Some(BlockKindFilter::Paragraph), ..Default::default() });
        assert_eq!(paras.len(), 1, "a bullet list is no longer one paragraph");
        // A limit cuts the note-id-ordered prefix, run after run
        for _ in 0..5 {
            let two = vault.index().query(&BlockQuery { limit: Some(2), ..Default::default() });
            let got: Vec<(String, String)> =
                two.iter().map(|h| (h.note.as_str().to_string(), h.block.content.clone())).collect();
            assert_eq!(got, vec![("a.md".into(), "a1".into()), ("a.md".into(), "a1a".into())]);
        }
    }

    #[test]
    fn query_field_filters_use_block_fields_then_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("book.md"),
            "---\nauthor: Le Guin\nrating: 4\ncreated: 2026-08-01\ntags: [x]\n---\nintro\n\n- 再读 [rating:: 5] [status:: done]\n- [ ] 写笔记 due::2026-09-01\n",
        )
        .unwrap();
        fs::write(root.join("other.md"), "Author:: Someone\n").unwrap();
        let vault = Vault::open(root).unwrap();
        let q = |f: &str| {
            vault
                .index()
                .query(&BlockQuery { field: vec![crate::query::parse_field_filter(f).unwrap()], ..Default::default() })
        };
        // block field wins over the note's frontmatter
        assert_eq!(q("rating = 5").len(), 1);
        // blocks without the key fall back to frontmatter (intro, task + the two of book.md… and other.md has none)
        assert_eq!(q("rating = 4").len(), 2);
        assert_eq!(q("author").len(), 4, "3 blocks of book.md via frontmatter + other.md's own field");
        assert_eq!(q("AUTHOR ~ guin").len(), 3);
        assert_eq!(q("due < 2026-09-10").len(), 1);
        assert_eq!(q("created > 2026-07-31").len(), 3);
        assert_eq!(q("status != done").len(), 0);
        let both = vault.index().query(&BlockQuery {
            field: vec![
                crate::query::parse_field_filter("author ~ guin").unwrap(),
                crate::query::parse_field_filter("rating > 4").unwrap(),
            ],
            ..Default::default()
        });
        assert_eq!(both.len(), 1);
        assert_eq!(vault.index().frontmatter_field(&NoteId::new("book.md"), "Rating").collect::<Vec<_>>(), vec!["4"]);
    }

    #[test]
    fn ensure_block_id_mints_once_and_guards() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("n.md"), "first\n\nsecond para\n\n```\ncode\n```\n").unwrap();
        let mut vault = Vault::open(root).unwrap();
        let id = NoteId::new("n.md");
        let (minted, ev) = vault.ensure_block_id(&id, "b1", Some("second para")).unwrap();
        assert!(matches!(ev, Some(VaultEvent::NoteModified { .. })));
        let on_disk = fs::read_to_string(root.join("n.md")).unwrap();
        assert_eq!(on_disk, format!("first\n\nsecond para ^{minted}\n\n```\ncode\n```\n"));
        // the index saw the write: the block now carries the explicit id
        let b = vault.index().find_block(&id, &minted).unwrap();
        assert!(b.explicit_id);
        // asking again returns the same id and writes nothing
        let (again, ev) = vault.ensure_block_id(&id, &minted, None).unwrap();
        assert_eq!(again, minted);
        assert!(ev.is_none());
        // a guard that no longer matches refuses and leaves the file untouched
        assert!(matches!(
            vault.ensure_block_id(&id, "b0", Some("not first")),
            Err(VemryError::BlockChanged { .. })
        ));
        assert_eq!(fs::read_to_string(root.join("n.md")).unwrap(), on_disk);
        assert!(matches!(vault.ensure_block_id(&id, "b2", None), Err(VemryError::BlockNotIdable)));
    }

    #[test]
    fn resolve_ref_shapes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("Sleep.md"),
            "---\ntitle: 睡眠研究\naliases: [sleep-research]\n---\n# 睡眠\n\n## 方法\n段落\n\n### 子标题\n更多\n\n## 结论\n- 项 ^i1\n  - 子项\n- 另一项\n\n> [!claim] 断言 ^c1\n",
        )
        .unwrap();
        fs::write(root.join("Other.md"), "see [[Sleep]]\n").unwrap();
        let vault = Vault::open(root).unwrap();
        let other = NoteId::new("Other.md");
        let res = |t: &str, from: Option<&NoteId>| vault.resolve_ref(t, from).unwrap();
        // whole note: the body without frontmatter
        match res("sleep-research", None) {
            RefResolution::Resolved(h) => {
                assert!(matches!(h.target, RefTarget::Note));
                assert!(h.content.starts_with("# 睡眠"));
                assert_eq!(h.note_title, "睡眠研究");
            }
            other => panic!("{other:?}"),
        }
        // a heading: its section, sub-headings inside, up to the next heading of the same level
        match res("Sleep#方法", None) {
            RefResolution::Resolved(h) => {
                assert!(matches!(h.target, RefTarget::Heading { level: 2, .. }));
                assert_eq!(h.content, "## 方法\n段落\n\n### 子标题\n更多");
            }
            other => panic!("{other:?}"),
        }
        // case-insensitive heading, an alias target, `Note#` = the note
        assert!(matches!(res("Sleep#方法", None), RefResolution::Resolved(_)));
        assert!(matches!(res("Sleep#", None), RefResolution::Resolved(RefHit { target: RefTarget::Note, .. })));
        // a block: a list item resolves to its subtree
        match res("Sleep#^i1", None) {
            RefResolution::Resolved(h) => {
                assert!(matches!(h.target, RefTarget::Block { .. }));
                assert_eq!(h.content, "- 项 ^i1\n  - 子项");
            }
            other => panic!("{other:?}"),
        }
        match res("Sleep#^c1", None) {
            RefResolution::Resolved(h) => assert_eq!(h.content, "> [!claim] 断言 ^c1"),
            other => panic!("{other:?}"),
        }
        // `#^c1` needs a from note
        assert!(matches!(res("#^c1", Some(&NoteId::new("Sleep.md"))), RefResolution::Resolved(_)));
        assert!(matches!(res("#^c1", None), RefResolution::NoteMissing));
        assert!(matches!(res("#^c1", Some(&other)), RefResolution::AnchorMissing { .. }));
        // missing anchors: unknown id, positional id, unknown heading; missing note
        assert!(matches!(res("Sleep#^nope", None), RefResolution::AnchorMissing { .. }));
        assert!(matches!(res("Sleep#^b1", None), RefResolution::AnchorMissing { .. }));
        assert!(matches!(res("Sleep#没有", None), RefResolution::AnchorMissing { .. }));
        assert!(matches!(res("Nowhere#^c1", None), RefResolution::NoteMissing));
    }

    #[test]
    fn private_block_tags_links_excluded_everywhere() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("A.md"), "public #pub [[B]]\n\n> [!private] secret #hid [[B]] [[C]]\n").unwrap();
        fs::write(root.join("B.md"), "b\n").unwrap();
        fs::write(root.join("C.md"), "c\n").unwrap();
        let vault = Vault::open(root).unwrap();
        let a = NoteId::new("A.md");
        assert_eq!(vault.index().summary(&a).unwrap().tags, vec!["pub"]);
        assert_eq!(vault.index().links_out(&a).len(), 1);
        assert!(vault.index().backlinks_of(&NoteId::new("C.md")).is_empty());
        assert_eq!(vault.index().graph().edges.len(), 1);
        let bl = vault.backlinks(&NoteId::new("B.md"));
        assert_eq!(bl.len(), 1);
        assert!(!bl[0].snippet.contains("secret"));
        assert!(vault.has_private_blocks(&a));
        assert!(!vault.has_private_blocks(&NoteId::new("B.md")));
    }

    #[test]
    fn agent_reads_never_see_private_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("N.md"),
            "# Head\n\npublic apple\n\n> [!private] secret banana apple\n\n## Sub\n- item ^i1\n  > [!private] hidden\n  - child\n\n> [!private] gone ^p1\n",
        )
        .unwrap();
        fs::write(root.join("VEMRY.md"), "rule\n\n> [!private] memory secret\n").unwrap();
        let vault = Vault::open(root).unwrap();
        let n = NoteId::new("N.md");
        let red = vault.read_note_for_agent(&n).unwrap();
        assert!(!red.text.contains("secret") && !red.text.contains("hidden") && !red.text.contains("gone"));
        assert_eq!(red.count(), 3);
        assert!(red.text.contains("public apple") && red.text.contains("- child"));
        // search: a private-only word finds nothing; the AND oracle is closed; a public hit keeps its raw line number
        assert!(vault.search_for_agent("banana", 10).is_empty());
        assert!(!vault.search("banana", 10).is_empty(), "the UI still finds it");
        assert!(vault.search_for_agent("apple banana", 10).is_empty());
        let hits = vault.search_for_agent("apple", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].line, hits[0].snippet.as_str()), (3, "public apple"));
        // memory injection
        assert_eq!(vault.memory_file_for_agent().unwrap(), "rule\n");
        // references: a private target is a missing anchor; a subtree is redacted
        assert!(matches!(vault.resolve_ref_for_agent("N#^p1", None).unwrap(), RefResolution::AnchorMissing { .. }));
        assert!(matches!(vault.resolve_ref("N#^p1", None).unwrap(), RefResolution::Resolved(_)), "the UI resolves it");
        match vault.resolve_ref_for_agent("N#^i1", None).unwrap() {
            RefResolution::Resolved(h) => assert_eq!(h.content, "- item ^i1\n  - child"),
            other => panic!("{other:?}"),
        }
        match vault.resolve_ref_for_agent("N#Sub", None).unwrap() {
            RefResolution::Resolved(h) => assert!(!h.content.contains("hidden") && !h.content.contains("gone")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn provenance_ledger_and_derived_origins() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("n.md"), "mine\n\n- memo *(2026-09-02 10:30)*\n\n> q\n> — [T](https://u) · 2026-09-02\n").unwrap();
        fs::write(root.join("clip.md"), "---\nsource: https://s\nclipped: 2026-09-01T00:00:00Z\nvemry:\n  origin: clip\n---\nbody\n").unwrap();
        fs::write(root.join("old.md"), "---\nvemry:\n  origin: agent\n---\nlegacy\n").unwrap();
        let mut vault = Vault::open(root).unwrap();
        let n = NoteId::new("n.md");
        let origins = |v: &Vault, id: &NoteId| v.provenance(id).unwrap().into_iter().map(|p| p.origin).collect::<Vec<_>>();
        assert!(matches!(origins(&vault, &n)[..], [Origin::You, Origin::Capture { ts: Some(_) }, Origin::Clip { url: Some(_), .. }]));
        assert!(matches!(origins(&vault, &NoteId::new("clip.md"))[0], Origin::Clip { url: Some(_), ts: Some(_) }));
        assert!(matches!(origins(&vault, &NoteId::new("old.md"))[0], Origin::Agent { pending: false, session: None, .. }));
        // an agent write lands pending
        let old = vault.read_note(&n).unwrap();
        let new = format!("{old}\nagent added\n");
        vault.write_note(&n, &new).unwrap();
        let added = crate::provenance::added_blocks(&old, &new);
        assert_eq!(added.len(), 1);
        vault.record_provenance(ProvenanceEntry::Agent { ts: "t".into(), session: Some("s1".into()), path: "n.md".into(), blocks: added });
        let prov = vault.provenance(&n).unwrap();
        assert!(matches!(prov[3].origin, Origin::Agent { pending: true, session: Some(ref s), .. } if s == "s1"));
        assert_eq!(vault.pending_count(&n), 1);
        assert_eq!(vault.pending_notes(), vec![(n.clone(), 1)]);
        // the same block, turned into a list item elsewhere in the note, keeps its record
        vault.write_note(&n, &format!("- agent added\n\n{old}")).unwrap();
        assert!(matches!(vault.provenance(&n).unwrap()[0].origin, Origin::Agent { pending: true, .. }));
        // accepting one by local id, then reloading the vault keeps the state
        let remaining = vault.accept_provenance(&n, Some("b0")).unwrap();
        assert_eq!(remaining, 0);
        assert!(matches!(vault.accept_provenance(&n, Some("b99")), Err(VemryError::BlockNotFound { .. })));
        let reopened = Vault::open(root).unwrap();
        assert!(matches!(reopened.provenance(&n).unwrap()[0].origin, Origin::Agent { pending: false, .. }));
        assert!(reopened.pending_notes().is_empty());
        // editing the words turns the block into the user's
        let mut reopened = reopened;
        reopened.write_note(&n, &format!("- agent edited\n\n{old}")).unwrap();
        assert_eq!(reopened.provenance(&n).unwrap()[0].origin, Origin::You);
        assert!(fs::read_to_string(root.join(".vemry/provenance.jsonl")).unwrap().lines().count() >= 2);
    }

    #[test]
    fn open_scans_and_indexes() {
        let (_d, vault) = fixture_vault();
        assert_eq!(vault.index().note_count(), 2);
        let sleep = NoteId::new("Sleep.md");
        assert_eq!(vault.index().summary(&sleep).unwrap().title, "睡眠研究");
    }

    /// Frontmatter `source:` reaches the index (the clips_lookup reverse lookup
    /// no longer re-reads the disk).
    #[test]
    fn frontmatter_source_is_indexed() {
        let (_d, mut vault) = fixture_vault();
        fs::create_dir_all(vault.root().join("clips")).unwrap();
        fs::write(
            vault.root().join("clips/文章.md"),
            "---\nsource: https://example.com/a\n---\n正文\n",
        )
        .unwrap();
        vault.handle_fs_change(&[vault.root().join("clips/文章.md")]);
        let pairs: Vec<(String, String)> = vault
            .index()
            .sources()
            .map(|(id, s)| (id.to_string(), s.to_string()))
            .collect();
        assert_eq!(pairs, vec![("clips/文章.md".into(), "https://example.com/a".into())]);
        // A note without a source does not show up in sources()
        assert!(vault.index().summary(&NoteId::new("Sleep.md")).unwrap().source.is_none());
    }

    #[test]
    fn backlinks_resolve_via_name_and_alias() {
        let (_d, vault) = fixture_vault();
        let sleep = NoteId::new("Sleep.md");
        let alpha = NoteId::new("projects/Alpha.md");
        let bl = vault.backlinks(&sleep);
        assert_eq!(bl.len(), 1);
        assert_eq!(bl[0].source, alpha);
        assert!(!bl[0].snippet.is_empty());
        // Alpha is linked from Sleep by path
        let bl_alpha = vault.backlinks(&alpha);
        assert_eq!(bl_alpha.len(), 1);
        assert_eq!(bl_alpha[0].source, sleep);
    }

    #[test]
    fn unresolved_links_tracked() {
        let (_d, vault) = fixture_vault();
        let alpha = NoteId::new("projects/Alpha.md");
        let links = vault.index().links_out(&alpha);
        let unresolved: Vec<_> = links
            .iter()
            .filter(|(e, _)| e.resolved.is_none())
            .map(|(e, _)| e.raw_target.clone())
            .collect();
        assert_eq!(unresolved, vec!["Missing Note"]);
    }

    #[test]
    fn graph_aggregates_resolved_links() {
        let (_d, vault) = fixture_vault();
        let g = vault.index().graph();
        assert_eq!(g.nodes.len(), 2);
        assert_eq!(g.nodes[0].id.as_str(), "Sleep.md", "nodes sorted by id");
        // Sleep -> Alpha once; Alpha -> Sleep three times ([[Sleep]], the
        // alias, [[Sleep#^c1]]) aggregated into one weighted edge; the
        // unresolved [[Missing Note]] contributes nothing
        assert_eq!(g.edges.len(), 2);
        let alpha_sleep = g.edges.iter().find(|e| e.source.as_str() == "projects/Alpha.md").unwrap();
        assert_eq!(alpha_sleep.target.as_str(), "Sleep.md");
        assert_eq!(alpha_sleep.count, 3);
        let sleep_alpha = g.edges.iter().find(|e| e.source.as_str() == "Sleep.md").unwrap();
        assert_eq!(sleep_alpha.target.as_str(), "projects/Alpha.md");
        assert_eq!(sleep_alpha.count, 1);
    }

    #[test]
    fn query_blocks_by_kind() {
        let (_d, vault) = fixture_vault();
        let claims = vault.index().query(&BlockQuery { kind: Some(BlockKindFilter::Claim), ..Default::default() });
        assert_eq!(claims.len(), 1);
        assert_eq!(claims[0].block.local_id, "c1");

        let tasks = vault.index().query(&BlockQuery { kind: Some(BlockKindFilter::Task), ..Default::default() });
        assert_eq!(tasks.len(), 1);
        match &tasks[0].block.kind {
            BlockKind::Task { due, .. } => assert_eq!(due.unwrap().to_string(), "2026-08-20"),
            k => panic!("expected task, got {k:?}"),
        }

        let evidence = vault.index().query(&BlockQuery { kind: Some(BlockKindFilter::Evidence), ..Default::default() });
        assert_eq!(evidence.len(), 1);
        match &evidence[0].block.kind {
            BlockKind::Evidence { supports } => assert_eq!(supports, &vec!["Sleep#^c1".to_string()]),
            k => panic!("expected evidence, got {k:?}"),
        }
    }

    #[test]
    fn chinese_search_works() {
        let (_d, vault) = fixture_vault();
        let hits = vault.search("记忆", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].note, NoteId::new("Sleep.md"));
        assert!(hits[0].snippet.contains("记忆"));
        // multi-term AND
        assert_eq!(vault.search("记忆 不存在词", 10).len(), 0);
    }

    #[test]
    fn write_note_updates_index_and_backlinks() {
        let (_d, mut vault) = fixture_vault();
        let alpha = NoteId::new("projects/Alpha.md");
        vault.write_note(&alpha, "no links anymore\n").unwrap();
        let sleep = NoteId::new("Sleep.md");
        assert_eq!(vault.backlinks(&sleep).len(), 0);
        assert_eq!(vault.search("anymore", 5).len(), 1);
    }

    #[test]
    fn create_note_unique_names() {
        let (_d, mut vault) = fixture_vault();
        let a = vault.create_note("New Idea", None).unwrap();
        let b = vault.create_note("New Idea", None).unwrap();
        assert_eq!(a.as_str(), "New Idea.md");
        assert_eq!(b.as_str(), "New Idea 1.md");
        assert!(vault.index().contains(&a));
    }

    #[test]
    fn fs_change_external_edit_and_remove() {
        let (_d, mut vault) = fixture_vault();
        let root = vault.root().to_path_buf();
        // external create
        fs::write(root.join("External.md"), "hello [[Sleep]]\n").unwrap();
        let events = vault.handle_fs_change(&[root.join("External.md")]);
        assert!(matches!(events[0], VaultEvent::NoteAdded { .. }));
        assert_eq!(vault.backlinks(&NoteId::new("Sleep.md")).len(), 2);
        // external remove
        fs::remove_file(root.join("External.md")).unwrap();
        let events = vault.handle_fs_change(&[root.join("External.md")]);
        assert!(matches!(events[0], VaultEvent::NoteRemoved { .. }));
        assert_eq!(vault.backlinks(&NoteId::new("Sleep.md")).len(), 1);
    }

    #[test]
    fn self_write_suppressed_in_fs_change() {
        let (_d, mut vault) = fixture_vault();
        let sleep = NoteId::new("Sleep.md");
        vault.write_note(&sleep, "rewritten\n").unwrap();
        let events = vault.handle_fs_change(&[vault.abs_path(&sleep)]);
        assert!(events.is_empty(), "self write must not produce events");
    }

    #[test]
    fn fs_change_reports_non_note_files() {
        let (_d, mut vault) = fixture_vault();
        let root = vault.root().to_path_buf();
        // a new PDF dropped into the vault must surface as FilesChanged
        fs::write(root.join("paper.pdf"), b"%PDF-1.4").unwrap();
        let events = vault.handle_fs_change(&[root.join("paper.pdf")]);
        assert!(matches!(events[..], [VaultEvent::FilesChanged]));
        // hidden paths stay silent
        let events = vault.handle_fs_change(&[root.join(".obsidian/cache.pdf")]);
        assert!(events.is_empty());
        // md changes keep their typed events, with FilesChanged appended only
        // when the batch also touches other files
        fs::write(root.join("New.md"), "# new\n").unwrap();
        let events = vault.handle_fs_change(&[root.join("New.md"), root.join("paper.pdf")]);
        assert!(matches!(events[..], [VaultEvent::NoteAdded { .. }, VaultEvent::FilesChanged]));
    }

    #[test]
    fn note_id_rejects_escapes() {
        let (_d, vault) = fixture_vault();
        assert!(vault.note_id("../outside").is_err());
        assert!(vault.note_id("/abs/path").is_err());
        assert!(vault.note_id("ok/inner").is_ok());
        // Backslash form: normalisation happens after validation, so validation
        // has to recognise `\` as a separator itself
        assert!(vault.note_id("..\\..\\x.md").is_err());
        assert!(vault.note_id("a\\..\\b.md").is_err());
        assert!(vault.note_id("\\srv\\share").is_err());
    }

    #[test]
    fn portable_rel_rejects_windows_reserved() {
        let (_d, mut vault) = fixture_vault();
        // Illegal characters
        assert!(vault.create_file("a:b.md").is_err());
        assert!(vault.create_file("plan?.md").is_err());
        assert!(vault.create_file("a|b.txt").is_err());
        // Reserved device names (with an extension too), case-insensitive
        assert!(vault.create_file("CON.md").is_err());
        assert!(vault.create_file("con.md").is_err());
        assert!(vault.create_file("COM1.txt").is_err());
        assert!(vault.create_folder("nul").is_err());
        // COM0 is not among the reserved names
        assert!(vault.create_file("COM0.md").is_ok());
        // Trailing space/dot; an intermediate segment that matches is rejected too
        assert!(vault.create_folder("bad.").is_err());
        assert!(vault.create_folder("a:b/sub").is_err());
        // A normal name is let through
        assert!(vault.create_file("普通 名字.md").is_ok());
        // A rename destination gets the same check
        assert!(vault.rename_path("Sleep.md", "sle:ep.md").is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn rename_existing_colon_file_away_is_allowed() {
        // An existing mac file containing `:`: readable and movable, it just
        // cannot be used as a new destination
        let (_d, mut vault) = fixture_vault();
        fs::write(vault.root().join("a:b.md"), "# legacy\n").unwrap();
        vault.rescan().unwrap();
        assert_eq!(vault.rename_path("a:b.md", "ab.md").unwrap(), "ab.md");
        assert!(vault.root().join("ab.md").exists());
    }

    #[cfg(not(windows))]
    #[test]
    fn rename_moves_legacy_bad_name_keeping_it() {
        // A pure move of an existing pathological name (file-tree drag or
        // cut/paste keeps the name): allowed, and so is the undo direction — the
        // gate only stops names that newly appear
        let (_d, mut vault) = fixture_vault();
        fs::write(vault.root().join("怎么办?.md"), "# legacy\n").unwrap();
        vault.create_folder("archive").unwrap();
        vault.rescan().unwrap();
        assert_eq!(vault.rename_path("怎么办?.md", "archive/怎么办?.md").unwrap(), "archive/怎么办?.md");
        assert_eq!(vault.rename_path("archive/怎么办?.md", "怎么办?.md").unwrap(), "怎么办?.md");
        // A newly created intermediate folder still passes the gate; with a legal
        // name the file goes through with it
        assert!(vault.rename_path("怎么办?.md", "new?dir/怎么办?.md").is_err());
        assert_eq!(vault.rename_path("怎么办?.md", "newdir/怎么办?.md").unwrap(), "newdir/怎么办?.md");
        // Changing it to a different pathological name (the last segment changed):
        // still rejected
        assert!(vault.rename_path("newdir/怎么办?.md", "newdir/怎么办!?.md").is_err());
    }

    #[test]
    fn sanitize_reserved_title_passes_gate() {
        // A reserved device name as a title: the sanitized result must pass the
        // gate (create_note → write_note's validate_portable_rel) rather than
        // producing a name that cannot be written
        let (_d, mut vault) = fixture_vault();
        assert_eq!(vault.create_note("NUL", None).unwrap().as_str(), "_NUL.md");
        assert_eq!(vault.create_note("con.figure", None).unwrap().as_str(), "_con.figure.md");
        // A non-reserved name is left alone
        assert_eq!(sanitize_filename("Console"), "Console");
    }

    #[test]
    fn folders_listed_including_empty_and_created() {
        let (_d, vault) = fixture_vault();
        assert_eq!(vault.list_folders(), vec!["projects"]);
        // create nested + empty folder
        let created = vault.create_folder("research/papers").unwrap();
        assert_eq!(created, "research/papers");
        assert_eq!(vault.list_folders(), vec!["projects", "research", "research/papers"]);
        // hidden dirs excluded
        std::fs::create_dir_all(vault.root().join(".obsidian/plugins")).unwrap();
        assert!(!vault.list_folders().iter().any(|f| f.contains(".obsidian")));
        // escapes rejected
        assert!(vault.create_folder("../outside").is_err());
        assert!(vault.create_folder("/abs").is_err());
    }

    #[test]
    fn create_file_any_extension() {
        let (_d, mut vault) = fixture_vault();
        // non-md: created on disk, not indexed
        let rel = vault.create_file("data/notes.csv").unwrap();
        assert_eq!(rel, "data/notes.csv");
        assert!(vault.root().join("data/notes.csv").exists());
        assert!(!vault.index().contains(&NoteId::new("data/notes.csv")));
        assert!(vault.list_files().contains(&"data/notes.csv".to_string()));
        // md: indexed as a note
        vault.create_file("ideas/thought.md").unwrap();
        assert!(vault.index().contains(&NoteId::new("ideas/thought.md")));
        // duplicates rejected
        assert!(vault.create_file("data/notes.csv").is_err());
    }

    #[test]
    fn read_write_text_file() {
        let (_d, mut vault) = fixture_vault();
        vault.create_file("data/notes.csv").unwrap();
        vault.write_file("data/notes.csv", "a,b\n1,2\n").unwrap();
        assert_eq!(vault.read_file("data/notes.csv").unwrap(), "a,b\n1,2\n");
        // md must go through the note APIs
        assert!(vault.write_file("Sleep.md", "x").is_err());
        // binary content is refused so the caller can fall back to the OS app
        let png = [0x89u8, 0x50, 0x4e, 0x47, 0xff, 0xfe];
        fs::write(vault.root().join("img.png"), png).unwrap();
        assert!(vault.read_file("img.png").is_err());
        // …but the byte reader (image/PDF preview) returns it verbatim
        assert_eq!(vault.read_file_bytes("img.png").unwrap(), png);
        assert!(vault.read_file_bytes("../outside.png").is_err());
        // attachments: parent dirs created, name deduped, .md refused
        assert_eq!(vault.save_attachment("attachments/shot.png", &png).unwrap(), "attachments/shot.png");
        assert_eq!(vault.save_attachment("attachments/shot.png", &png).unwrap(), "attachments/shot-1.png");
        assert_eq!(vault.read_file_bytes("attachments/shot-1.png").unwrap(), png);
        assert!(vault.save_attachment("attachments/x.md", &png).is_err());
        // escapes / missing files rejected
        assert!(vault.read_file("../outside.txt").is_err());
        assert!(vault.read_file("missing.txt").is_err());
    }

    #[test]
    fn import_external_files() {
        let (_d, mut vault) = fixture_vault();
        // external staging area with a file, a note, and a folder holding a note
        let ext = tempfile::tempdir().unwrap();
        fs::write(ext.path().join("paper.pdf"), b"%PDF-fake").unwrap();
        fs::write(ext.path().join("Imported.md"), "# 导入的笔记\n").unwrap();
        fs::create_dir_all(ext.path().join("bundle/inner")).unwrap();
        fs::write(ext.path().join("bundle/readme.md"), "# bundle\n").unwrap();
        fs::write(ext.path().join("bundle/inner/data.csv"), "a,b\n").unwrap();

        // file into a folder: copied and named as-is
        let created = vault.import_files(&[ext.path().join("paper.pdf")], "projects", false).unwrap();
        assert_eq!(created, vec!["projects/paper.pdf"]);
        assert!(vault.root().join("projects/paper.pdf").exists());
        // again: dedupes
        let created = vault.import_files(&[ext.path().join("paper.pdf")], "projects", false).unwrap();
        assert_eq!(created, vec!["projects/paper-1.pdf"]);
        // .md into the root gets indexed
        let created = vault.import_files(&[ext.path().join("Imported.md")], "", false).unwrap();
        assert_eq!(created, vec!["Imported.md"]);
        assert!(vault.index().contains(&NoteId::new("Imported.md")));
        // folders copy recursively, nested notes indexed
        let created = vault.import_files(&[ext.path().join("bundle")], "projects", false).unwrap();
        assert_eq!(created, vec!["projects/bundle"]);
        assert!(vault.root().join("projects/bundle/inner/data.csv").exists());
        assert!(vault.index().contains(&NoteId::new("projects/bundle/readme.md")));
        // importing the vault root itself is refused
        assert!(vault.import_files(&[vault.root().to_path_buf()], "", false).is_err());
        // vanished sources are skipped, not fatal
        assert_eq!(vault.import_files(&[ext.path().join("gone.txt")], "", false).unwrap(), Vec::<String>::new());
        // reuse_in_vault: a source already inside the vault is referenced, not copied
        let created = vault.import_files(&[vault.root().join("projects/paper.pdf")], "attachments/x", true).unwrap();
        assert_eq!(created, vec!["projects/paper.pdf"]);
        assert!(!vault.root().join("attachments/x/paper.pdf").exists());
        // …but without the flag it duplicates (tree-paste semantics)
        let created = vault.import_files(&[vault.root().join("projects/paper.pdf")], "attachments/x", false).unwrap();
        assert_eq!(created, vec!["attachments/x/paper.pdf"]);
    }

    #[test]
    fn rename_note_and_folder() {
        let (_d, mut vault) = fixture_vault();
        // rename a note: new id indexed, old gone, stale links become unresolved
        vault.rename_path("Sleep.md", "睡眠总览.md").unwrap();
        assert!(!vault.index().contains(&NoteId::new("Sleep.md")));
        assert!(vault.index().contains(&NoteId::new("睡眠总览.md")));
        let alpha = NoteId::new("projects/Alpha.md");
        let unresolved: Vec<_> = vault
            .index()
            .links_out(&alpha)
            .into_iter()
            .filter(|(e, _)| e.resolved.is_none())
            .map(|(e, _)| e.raw_target)
            .collect();
        // [[Sleep]] by filename now dangles; [[sleep-research]] still resolves via alias frontmatter
        assert!(unresolved.contains(&"Sleep".to_string()));
        // rename a folder: children follow
        vault.rename_path("projects", "archive").unwrap();
        assert!(vault.index().contains(&NoteId::new("archive/Alpha.md")));
        assert!(!vault.index().contains(&alpha));
        // target collision rejected
        vault.create_file("a.md").unwrap();
        vault.create_file("b.md").unwrap();
        assert!(vault.rename_path("a.md", "b.md").is_err());
    }

    #[test]
    fn append_note_creates_and_appends() {
        let (_d, mut vault) = fixture_vault();
        let inbox = vault.note_id("inbox").unwrap();
        vault.append_note(&inbox, "- first thought").unwrap();
        vault.append_note(&inbox, "- second thought").unwrap();
        let content = vault.read_note(&inbox).unwrap();
        assert_eq!(content, "- first thought\n- second thought\n");
    }

    #[test]
    fn insert_note_section_and_fallback() {
        let (_d, mut vault) = fixture_vault();
        let id = vault.note_id("盒子").unwrap();
        vault.write_note(&id, "# 盒子\n\n## 收集箱\n- 一\n\n## 已整理\n内容\n").unwrap();
        // Section hit: lands at the end of the section, list packed against list
        let (_, landed) = vault.insert_note(&id, "- 二", Some("收集箱")).unwrap();
        assert_eq!(landed, InsertLanded::Section);
        assert_eq!(vault.read_note(&id).unwrap(), "# 盒子\n\n## 收集箱\n- 一\n- 二\n\n## 已整理\n内容\n");
        // Section missing: falls back to the end of the file, landed tells the caller to say so
        let (_, landed) = vault.insert_note(&id, "> 引用", Some("不存在")).unwrap();
        assert_eq!(landed, InsertLanded::Tail);
        assert!(vault.read_note(&id).unwrap().ends_with("内容\n\n> 引用\n"));
        // No section given = normalised append; a note that does not exist is created (append semantics)
        let inbox = vault.note_id("inbox2").unwrap();
        let (_, landed) = vault.insert_note(&inbox, "- 想法", None).unwrap();
        assert_eq!(landed, InsertLanded::Tail);
        assert_eq!(vault.read_note(&inbox).unwrap(), "- 想法\n");
    }

    #[test]
    /// A vault created in one language and used in another must keep appending
    /// into the section it already has — otherwise memos silently start landing
    /// at the end of the file.
    fn memory_append_finds_a_section_written_in_another_language() {
        let (_d, mut vault) = fixture_vault();
        let id = vault.note_id("VEMRY.md").unwrap();
        vault.write_note(&id, "# VEMRY.md\n\n## 约定\n- 旧的一条\n").unwrap();
        // English UI: creation would say "Conventions", but the file has 约定
        vault.memory_append("a new one", TEST_MEMORY_HEADER, "Conventions").unwrap();
        let out = vault.read_note(&id).unwrap();
        assert_eq!(out, "# VEMRY.md\n\n## 约定\n- 旧的一条\n- a new one\n");
        assert!(!out.contains("## Conventions"), "must not open a second section");
    }

    #[test]
    fn memory_append_scaffolds_and_packs() {
        let (_d, mut vault) = fixture_vault();
        // File missing: the whole scaffold (its header shares one source with the
        // full template) plus the first convention
        vault.memory_append("周报放 Weekly/ 下", TEST_MEMORY_HEADER, "约定").unwrap();
        assert_eq!(
            vault.read_note(&NoteId::new("VEMRY.md")).unwrap(),
            format!("{TEST_MEMORY_HEADER}\n## 约定\n- 周报放 Weekly/ 下\n")
        );
        // Section already there: a single-line bullet packs into the list
        vault.memory_append("综述放 reviews/ 目录", TEST_MEMORY_HEADER, "约定").unwrap();
        assert!(vault
            .read_note(&NoteId::new("VEMRY.md"))
            .unwrap()
            .ends_with("## 约定\n- 周报放 Weekly/ 下\n- 综述放 reviews/ 目录\n"));
        // Multi-line input folds into indented continuations; a multi-line payload
        // follows the block rule and keeps one blank line from what precedes it (insert.rs)
        vault.memory_append("综述先出大纲\n再逐节展开", TEST_MEMORY_HEADER, "约定").unwrap();
        assert!(vault
            .read_note(&NoteId::new("VEMRY.md"))
            .unwrap()
            .ends_with("- 综述放 reviews/ 目录\n\n- 综述先出大纲\n  再逐节展开\n"));
        // File is there but has no conventions section: append the section at the end
        let id = vault.note_id("VEMRY.md").unwrap();
        vault.write_note(&id, "# 本知识库的约定\n\n## 结构\n- inbox 是收集箱\n").unwrap();
        vault.memory_append("链接用 [[wikilink]]", TEST_MEMORY_HEADER, "约定").unwrap();
        assert_eq!(
            vault.read_note(&id).unwrap(),
            "# 本知识库的约定\n\n## 结构\n- inbox 是收集箱\n\n## 约定\n- 链接用 [[wikilink]]\n"
        );
        // Empty input is rejected
        assert!(vault.memory_append("  \n ", TEST_MEMORY_HEADER, "约定").is_err());
    }

    #[test]
    fn privacy_rules_and_frontmatter_flag() {
        let (_d, mut vault) = fixture_vault();
        let root = vault.root().to_path_buf();
        fs::create_dir_all(root.join("secret")).unwrap();
        fs::write(root.join("secret/Plan.md"), "# plan\n").unwrap();
        vault.rescan().unwrap();
        let plan = NoteId::new("secret/Plan.md");
        assert!(!vault.is_private(&plan));

        // glob rule marks the folder and everything below it
        vault.add_privacy_rule("secret/").unwrap();
        assert!(vault.is_private(&plan));
        assert!(vault.is_private_rel("secret", true));
        assert!(vault.is_private_rel("Secret/Plan.md", false)); // APFS case folding
        // duplicate lines are not appended twice
        vault.add_privacy_rule("secret/").unwrap();
        assert_eq!(vault.privacy().rules().len(), 1);
        vault.remove_privacy_rule("secret/").unwrap();
        assert!(!vault.is_private(&plan));

        // frontmatter flag: set, surfaced in the index, cleared
        let sleep = NoteId::new("Sleep.md");
        vault.set_note_private(&sleep, true).unwrap();
        assert!(vault.is_private(&sleep));
        assert!(vault.index().summary(&sleep).unwrap().private);
        let raw = vault.read_note(&sleep).unwrap();
        assert!(raw.contains("private: true"));
        assert!(raw.contains("title: 睡眠研究"), "other frontmatter must survive");
        assert!(raw.contains("sleep-research"), "aliases must survive");
        vault.set_note_private(&sleep, false).unwrap();
        assert!(!vault.is_private(&sleep));
        assert!(!vault.read_note(&sleep).unwrap().contains("private"));
    }

    #[test]
    fn is_private_frontmatter_is_case_insensitive() {
        let (_d, mut vault) = fixture_vault();
        // A case-insensitive FS opens `Sleep.md` for `sleep.md`; the frontmatter
        // privacy flag must be recognized through any such case variant, or a
        // write tool could reach the private note under a spelling is_private
        // misses. The index is keyed by exact NoteId, so this exercises the
        // canonical (by_path) resolution — independent of the host FS.
        let sleep = NoteId::new("Sleep.md");
        vault.set_note_private(&sleep, true).unwrap();
        assert!(vault.is_private(&sleep));
        assert!(vault.is_private(&NoteId::new("sleep.md")));
        assert!(vault.is_private(&NoteId::new("SLEEP.md")));
        // folder segment case too
        let alpha = NoteId::new("projects/Alpha.md");
        vault.set_note_private(&alpha, true).unwrap();
        assert!(vault.is_private(&NoteId::new("Projects/alpha.md")));
        assert!(vault.is_private_rel("Projects/ALPHA.md", false));
        // a genuinely different note stays public
        assert!(!vault.is_private(&NoteId::new("nonexistent.md")));
    }

    #[test]
    fn non_note_file_does_not_inherit_privacy_from_same_stem_note() {
        let (_d, mut vault) = fixture_vault();
        let root = vault.root().to_path_buf();
        // by_path keys strip ".md", so the plain file's path collides with the
        // note's key; canonical folding must apply to .md paths only
        fs::write(root.join("data.csv.md"), "---\nprivate: true\n---\nnote\n").unwrap();
        fs::write(root.join("data.csv"), "a,b\n").unwrap();
        vault.rescan().unwrap();
        assert!(vault.is_private_rel("data.csv.md", false));
        assert!(!vault.is_private_rel("data.csv", false), "plain file must not inherit the note's privacy");
    }

    #[test]
    fn set_note_private_without_existing_frontmatter() {
        let (_d, mut vault) = fixture_vault();
        let alpha = NoteId::new("projects/Alpha.md");
        vault.set_note_private(&alpha, true).unwrap();
        let raw = vault.read_note(&alpha).unwrap();
        assert!(raw.starts_with("---\nprivate: true\n---\n# Alpha"));
        // clearing the only field removes the frontmatter block entirely
        vault.set_note_private(&alpha, false).unwrap();
        let raw = vault.read_note(&alpha).unwrap();
        assert!(raw.starts_with("# Alpha"));
        assert!(!raw.contains("---"));
    }

    #[test]
    fn privacy_file_change_reloads_scope() {
        let (_d, mut vault) = fixture_vault();
        let root = vault.root().to_path_buf();
        fs::create_dir_all(root.join(".vemry")).unwrap();
        fs::write(root.join(".vemry/privacy"), "projects/\n").unwrap();
        // hidden path, but the rules file itself must be picked up
        let events = vault.handle_fs_change(&[root.join(".vemry/privacy")]);
        assert!(matches!(events[..], [VaultEvent::PrivacyChanged]));
        assert!(vault.is_private(&NoteId::new("projects/Alpha.md")));
        // other hidden paths stay silent
        assert!(vault.handle_fs_change(&[root.join(".vemry/config.json")]).is_empty());
    }

    #[test]
    fn types_dir_change_emits_types_changed() {
        let (_d, mut vault) = fixture_vault();
        let root = vault.root().to_path_buf();
        fs::create_dir_all(root.join(".vemry/types")).unwrap();
        fs::write(root.join(".vemry/types/book.md"), "---\nname: book\n---\n").unwrap();
        let events = vault.handle_fs_change(&[root.join(".vemry/types/book.md")]);
        assert!(matches!(events[..], [VaultEvent::TypesChanged]));
        // a stray non-md file under the directory stays silent
        assert!(vault.handle_fs_change(&[root.join(".vemry/types/notes.txt")]).is_empty());
    }

    #[test]
    fn scripts_dir_change_emits_scripts_changed() {
        let (_d, mut vault) = fixture_vault();
        let root = vault.root().to_path_buf();
        fs::create_dir_all(root.join(".vemry/scripts")).unwrap();
        fs::write(root.join(".vemry/scripts/hello.js"), "// hi\n").unwrap();
        let events = vault.handle_fs_change(&[root.join(".vemry/scripts/hello.js")]);
        assert!(matches!(events[..], [VaultEvent::ScriptsChanged]));
        // one event per batch, even for several script files
        fs::write(root.join(".vemry/scripts/two.js"), "// 2\n").unwrap();
        let events = vault.handle_fs_change(&[
            root.join(".vemry/scripts/hello.js"),
            root.join(".vemry/scripts/two.js"),
        ]);
        assert!(matches!(events[..], [VaultEvent::ScriptsChanged]));
        // non-js files under the dir (scripts.json is host-written state) stay silent
        let events = vault.handle_fs_change(&[root.join(".vemry/scripts.json")]);
        assert!(events.is_empty());
        let events = vault.handle_fs_change(&[root.join(".vemry/scripts/state.json")]);
        assert!(events.is_empty());
        // the directory itself (deleted/renamed dirs report only the dir path)
        let events = vault.handle_fs_change(&[root.join(".vemry/scripts")]);
        assert!(matches!(events[..], [VaultEvent::ScriptsChanged]));
    }

    #[test]
    fn write_file_suppresses_own_watcher_echo() {
        let (_d, mut vault) = fixture_vault();
        let root = vault.root().to_path_buf();
        vault.write_file("data/out.csv", "a\n").unwrap();
        // the watcher echo of our own write is swallowed — a files_changed
        // workflow writing a file must not re-trigger itself
        assert!(vault.handle_fs_change(&[root.join("data/out.csv")]).is_empty());
        // an unrelated external file still reports
        fs::write(root.join("data/other.csv"), "x\n").unwrap();
        let events = vault.handle_fs_change(&[root.join("data/other.csv")]);
        assert!(matches!(events[..], [VaultEvent::FilesChanged]));
    }

    #[test]
    fn privacy_marks_across_rename() {
        let (_d, mut vault) = fixture_vault();
        // frontmatter mark follows the file
        let sleep = NoteId::new("Sleep.md");
        vault.set_note_private(&sleep, true).unwrap();
        vault.rename_path("Sleep.md", "Moved.md").unwrap();
        assert!(vault.is_private(&NoteId::new("Moved.md")));
        // glob rules do not follow (documented limitation)
        vault.add_privacy_rule("projects/Alpha.md").unwrap();
        vault.rename_path("projects/Alpha.md", "projects/Beta.md").unwrap();
        assert!(!vault.is_private(&NoteId::new("projects/Beta.md")));
    }

    /// Attachment-bucket migration end to end: uniquely owned buckets move and
    /// links are rewritten vault-wide, mixed buckets and orphans stay put, a
    /// root-level note is a no-op, and dry_run touches no disk.
    #[test]
    fn attachment_bucket_migration() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("projects")).unwrap();
        fs::create_dir_all(root.join("a")).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        // Unique owner: projects/plan.md ← attachments/plan/
        fs::write(root.join("projects/plan.md"), "![img](attachments/plan/a.png)\n").unwrap();
        // The referrer need not be the owning note: a root-level note links to plan's attachment too
        fs::write(root.join("ref.md"), "见 [附件](attachments/plan/a.png)\n```\n![x](attachments/plan/a.png)\n```\n").unwrap();
        // Mixed bucket: a/index.md and b/index.md share a stem
        fs::write(root.join("a/index.md"), "![i](attachments/index/i.png)\n").unwrap();
        fs::write(root.join("b/index.md"), "").unwrap();
        // Root-level note: old and new path are the same, nothing to migrate
        fs::write(root.join("inbox.md"), "![n](attachments/inbox/n.png)\n").unwrap();
        for (bucket, file) in [("plan", "a.png"), ("index", "i.png"), ("inbox", "n.png"), ("ghost", "g.png")] {
            fs::create_dir_all(root.join("attachments").join(bucket)).unwrap();
            fs::write(root.join("attachments").join(bucket).join(file), b"x").unwrap();
        }
        let mut vault = Vault::open(root).unwrap();

        // dry_run: a complete report and no disk writes
        let dry = vault.migrate_attachment_buckets(true).unwrap();
        assert_eq!(dry.moved, vec![("attachments/plan".into(), "attachments/projects/plan".into())]);
        assert_eq!(dry.mixed, vec!["attachments/index".to_string()]);
        assert_eq!(dry.orphaned, vec!["attachments/ghost".to_string()]);
        assert_eq!(dry.rewritten_notes, 2); // projects/plan.md + ref.md
        assert!(root.join("attachments/plan/a.png").exists());
        assert!(!fs::read_to_string(root.join("ref.md")).unwrap().contains("attachments/projects"));

        // The real run: directories move plus two notes rewritten (nothing inside a
        // code fence), while mixed, orphan and root-level stay put
        let report = vault.migrate_attachment_buckets(false).unwrap();
        assert_eq!(report.moved.len(), 1);
        assert!(root.join("attachments/projects/plan/a.png").exists());
        assert!(!root.join("attachments/plan").exists());
        assert!(root.join("attachments/index/i.png").exists());
        assert!(root.join("attachments/ghost/g.png").exists());
        assert!(root.join("attachments/inbox/n.png").exists());
        assert_eq!(
            fs::read_to_string(root.join("projects/plan.md")).unwrap(),
            "![img](attachments/projects/plan/a.png)\n"
        );
        let re = fs::read_to_string(root.join("ref.md")).unwrap();
        assert!(re.contains("[附件](attachments/projects/plan/a.png)"));
        assert!(re.contains("```\n![x](attachments/plan/a.png)\n```")); // untouched inside a fence
        // Run it again: nothing to do (idempotent)
        let again = vault.migrate_attachment_buckets(false).unwrap();
        assert!(again.moved.is_empty());
        assert_eq!(again.rewritten_notes, 0);
    }
}
