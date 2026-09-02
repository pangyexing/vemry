//! Block-level vault index: note table, block table, link graph (with derived
//! backlinks), name-resolution table and tag table. Every projection — UI
//! views, scripts, agent tools — reads from here.

use serde::Serialize;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::UNIX_EPOCH;

use crate::block::BlockKind;
use crate::block::Block;
use crate::note::{Note, NoteId};

#[derive(Clone, Debug, Serialize)]
pub struct NoteSummary {
    pub id: NoteId,
    pub title: String,
    pub aliases: Vec<String>,
    pub tags: Vec<String>,
    pub mtime_ms: u64,
    /// Frontmatter `created` as written (free-form date string).
    pub created: Option<String>,
    /// Frontmatter `private: true` — the note belongs to the privacy zone.
    pub private: bool,
    /// Frontmatter `source:` as written — the external origin URL of a
    /// clipped note. Indexed so a clip reverse lookup never re-reads disk.
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphNode {
    pub id: NoteId,
    pub title: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphEdge {
    pub source: NoteId,
    pub target: NoteId,
    /// Number of resolved links source -> target (parallel links aggregate).
    pub count: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ResolvedLink {
    Resolved { target: NoteId },
    Unresolved { target: String },
}

#[derive(Clone, Debug, Serialize)]
pub struct LinkEdge {
    /// The note part of the wikilink target as written, e.g. "folder/Name".
    pub raw_target: String,
    pub resolved: Option<NoteId>,
    pub from_block: String,
}

/// One inline-field key across the vault (see `BlockIndex::field_stats`).
#[derive(Debug, Clone, serde::Serialize)]
pub struct FieldStat {
    pub key: String,
    /// Notes carrying the key (in a block or in frontmatter).
    pub count: usize,
    /// The most frequent values, most frequent first.
    pub values: Vec<String>,
}

pub(crate) struct NoteEntry {
    pub(crate) summary: NoteSummary,
    pub(crate) blocks: Vec<Block>,
    pub(crate) edges: Vec<LinkEdge>,
    /// Frontmatter scalars (plus `created`) as note-level fields: the
    /// fallback a field filter consults when a block has no field of that key.
    pub(crate) fm_fields: Vec<(String, String)>,
}

/// Flatten frontmatter into (key, value) strings: scalars as written,
/// scalar sequences joined with ", "; mappings and nulls are skipped.
/// `title`/`tags`/`aliases`/`private`/`vemry` are metadata, not fields.
fn frontmatter_fields(note: &Note) -> Vec<(String, String)> {
    fn scalar(v: &serde_yaml::Value) -> Option<String> {
        match v {
            serde_yaml::Value::String(s) => Some(s.clone()),
            serde_yaml::Value::Number(n) => Some(n.to_string()),
            serde_yaml::Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    }
    let mut out = Vec::new();
    if let Some(c) = &note.frontmatter.created {
        out.push(("created".to_string(), c.clone()));
    }
    for (k, v) in &note.frontmatter.extra {
        let Some(key) = k.as_str() else { continue };
        let value = match v {
            serde_yaml::Value::Sequence(items) => {
                let parts: Vec<String> = items.iter().filter_map(scalar).collect();
                if parts.is_empty() {
                    continue;
                }
                parts.join(", ")
            }
            other => match scalar(other) {
                Some(s) => s,
                None => continue,
            },
        };
        out.push((key.to_string(), value));
    }
    out
}

#[derive(Default)]
pub struct BlockIndex {
    notes: HashMap<NoteId, NoteEntry>,
    /// lowercase stem/alias -> candidate notes
    by_name: HashMap<String, BTreeSet<NoteId>>,
    /// lowercase full path without ".md" -> note
    by_path: HashMap<String, NoteId>,
    tags: HashMap<String, HashSet<NoteId>>,
    backlinks: HashMap<NoteId, HashSet<NoteId>>,
}

impl BlockIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&mut self, note: &Note) {
        self.remove_contributions(&note.id);
        let mtime_ms = note
            .mtime
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let mut tags: Vec<String> = note.frontmatter.tags.clone();
        for b in &note.blocks {
            for t in &b.tags {
                if !tags.contains(t) {
                    tags.push(t.clone());
                }
            }
        }
        let edges: Vec<LinkEdge> = note
            .blocks
            .iter()
            .flat_map(|b| {
                b.links.iter().map(|l| LinkEdge {
                    raw_target: l.target.clone(),
                    resolved: None,
                    from_block: b.local_id.clone(),
                })
            })
            .collect();
        let summary = NoteSummary {
            id: note.id.clone(),
            title: note.effective_title().to_string(),
            aliases: note.frontmatter.aliases.clone(),
            tags: tags.clone(),
            mtime_ms,
            created: note.frontmatter.created.clone(),
            private: note.frontmatter.private,
            source: note
                .frontmatter
                .extra
                .get("source")
                .and_then(|v| v.as_str())
                .map(String::from),
        };

        self.by_name.entry(note.id.stem().to_lowercase()).or_default().insert(note.id.clone());
        for alias in &note.frontmatter.aliases {
            self.by_name.entry(alias.to_lowercase()).or_default().insert(note.id.clone());
        }
        let path_key = note.id.as_str().strip_suffix(".md").unwrap_or(note.id.as_str()).to_lowercase();
        self.by_path.insert(path_key, note.id.clone());
        for t in &tags {
            self.tags.entry(t.clone()).or_default().insert(note.id.clone());
        }

        self.notes.insert(
            note.id.clone(),
            NoteEntry { summary, blocks: note.blocks.clone(), edges, fm_fields: frontmatter_fields(note) },
        );
        self.resolve_all();
    }

    pub fn remove(&mut self, id: &NoteId) {
        self.remove_contributions(id);
        self.resolve_all();
    }

    fn remove_contributions(&mut self, id: &NoteId) {
        if self.notes.remove(id).is_none() {
            return;
        }
        self.by_name.retain(|_, set| {
            set.remove(id);
            !set.is_empty()
        });
        self.by_path.retain(|_, v| v != id);
        self.tags.retain(|_, set| {
            set.remove(id);
            !set.is_empty()
        });
    }

    /// Re-resolve every edge and rebuild backlinks. O(total links); fine for
    /// vaults in the tens of thousands of notes, and trivially correct after
    /// renames/creations that can flip unresolved links anywhere in the vault.
    fn resolve_all(&mut self) {
        let mut backlinks: HashMap<NoteId, HashSet<NoteId>> = HashMap::new();
        let sources: Vec<NoteId> = self.notes.keys().cloned().collect();
        for source in sources {
            let raw_targets: Vec<String> = self.notes[&source].edges.iter().map(|e| e.raw_target.clone()).collect();
            let resolved: Vec<Option<NoteId>> =
                raw_targets.iter().map(|t| self.resolve_target(t, &source)).collect();
            for (i, r) in resolved.iter().enumerate() {
                if let Some(t) = r {
                    if *t != source {
                        backlinks.entry(t.clone()).or_default().insert(source.clone());
                    }
                }
                if let Some(entry) = self.notes.get_mut(&source) {
                    entry.edges[i].resolved = r.clone();
                }
            }
        }
        self.backlinks = backlinks;
    }

    /// Wikilink resolution: empty target = self; path targets match
    /// by full relative path; bare names match stem/alias (case-insensitive),
    /// shortest path wins on ambiguity.
    pub fn resolve_target(&self, target: &str, from: &NoteId) -> Option<NoteId> {
        if target.is_empty() {
            return Some(from.clone());
        }
        let lower = target.to_lowercase();
        let lower = lower.strip_suffix(".md").unwrap_or(&lower);
        if target.contains('/') {
            return self.by_path.get(lower).cloned();
        }
        if let Some(hit) = self.by_path.get(lower) {
            return Some(hit.clone());
        }
        let candidates = self.by_name.get(lower)?;
        candidates
            .iter()
            .min_by_key(|id| (id.as_str().len(), id.as_str().to_string()))
            .cloned()
    }

    pub fn note_count(&self) -> usize {
        self.notes.len()
    }

    pub fn contains(&self, id: &NoteId) -> bool {
        self.notes.contains_key(id)
    }

    /// Resolve a possibly-miscased note id to its canonical indexed id. A
    /// case-insensitive filesystem (APFS/NTFS) opens `Diary.md` for `diary.md`,
    /// but the note table is keyed by the exact `NoteId`; privacy checks must
    /// see through that folding. Returns the exact id when it is itself
    /// indexed, otherwise the case-insensitive `by_path` match, if any.
    /// Only `.md` paths fold: `by_path` keys strip the extension, so a plain
    /// file like `data.csv` must not resolve to a note named `data.csv.md`.
    pub fn canonical_id(&self, id: &NoteId) -> Option<NoteId> {
        if self.notes.contains_key(id) {
            return Some(id.clone());
        }
        let key = id.as_str().strip_suffix(".md")?.to_lowercase();
        self.by_path.get(&key).cloned()
    }

    pub fn summaries(&self) -> Vec<NoteSummary> {
        let mut v: Vec<NoteSummary> = self.notes.values().map(|e| e.summary.clone()).collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// (id, frontmatter `source:`) for notes that carry one — the zero-copy
    /// entry point for clip reverse lookup: no cloning every summary just to
    /// resolve a handful of URLs.
    pub fn sources(&self) -> impl Iterator<Item = (&NoteId, &str)> {
        self.notes
            .values()
            .filter_map(|e| e.summary.source.as_deref().map(|s| (&e.summary.id, s)))
    }

    /// Whole-vault link graph: every note is a node; resolved note-to-note
    /// links aggregate into weighted edges (self-links and unresolved targets
    /// skipped). Sorted for deterministic output.
    pub fn graph(&self) -> GraphData {
        let mut nodes: Vec<GraphNode> = self
            .notes
            .values()
            .map(|e| GraphNode { id: e.summary.id.clone(), title: e.summary.title.clone() })
            .collect();
        nodes.sort_by(|a, b| a.id.cmp(&b.id));
        let mut counts: HashMap<(&NoteId, &NoteId), u32> = HashMap::new();
        for (source, entry) in &self.notes {
            for edge in &entry.edges {
                if let Some(target) = &edge.resolved {
                    if target != source {
                        *counts.entry((source, target)).or_insert(0) += 1;
                    }
                }
            }
        }
        let mut edges: Vec<GraphEdge> = counts
            .into_iter()
            .map(|((source, target), count)| GraphEdge { source: source.clone(), target: target.clone(), count })
            .collect();
        edges.sort_by(|a, b| a.source.cmp(&b.source).then_with(|| a.target.cmp(&b.target)));
        GraphData { nodes, edges }
    }

    pub fn summary(&self, id: &NoteId) -> Option<NoteSummary> {
        self.notes.get(id).map(|e| e.summary.clone())
    }

    pub fn blocks(&self, id: &NoteId) -> Option<&[Block]> {
        self.notes.get(id).map(|e| e.blocks.as_slice())
    }

    /// One block of one note by local id (`locate::find_block` rules: an
    /// explicit `^id` beats a positional `bN` of the same spelling).
    pub fn find_block(&self, id: &NoteId, local_id: &str) -> Option<&Block> {
        crate::locate::find_block(self.blocks(id)?, local_id)
    }

    pub fn links_out(&self, id: &NoteId) -> Vec<(LinkEdge, ResolvedLink)> {
        match self.notes.get(id) {
            Some(e) => e
                .edges
                .iter()
                .map(|edge| {
                    let r = match &edge.resolved {
                        Some(t) => ResolvedLink::Resolved { target: t.clone() },
                        None => ResolvedLink::Unresolved { target: edge.raw_target.clone() },
                    };
                    (edge.clone(), r)
                })
                .collect(),
            None => vec![],
        }
    }

    pub fn backlinks_of(&self, id: &NoteId) -> Vec<NoteId> {
        let mut v: Vec<NoteId> = self.backlinks.get(id).map(|s| s.iter().cloned().collect()).unwrap_or_default();
        v.sort();
        v
    }

    /// Every inline-field key in use (blocks' fields plus frontmatter scalars),
    /// with how many notes carry it and its most frequent values — the editor's
    /// `key::` value completion and the views' group-by menus read this. `skip`
    /// drops notes the caller must not expose (a privacy zone); keys compare
    /// case-insensitively and keep their first-seen spelling.
    pub fn field_stats(&self, skip: impl Fn(&NoteId) -> bool, top: usize) -> Vec<FieldStat> {
        struct Acc {
            key: String,
            notes: usize,
            values: HashMap<String, usize>,
        }
        let mut acc: HashMap<String, Acc> = HashMap::new();
        for (id, e) in &self.notes {
            if skip(id) {
                continue;
            }
            let mut seen_here: Vec<String> = Vec::new();
            let mut bump = |key: &str, value: &str| {
                let lower = key.to_lowercase();
                let a = acc.entry(lower.clone()).or_insert_with(|| Acc { key: key.to_string(), notes: 0, values: HashMap::new() });
                if !seen_here.contains(&lower) {
                    seen_here.push(lower);
                    a.notes += 1;
                }
                *a.values.entry(value.to_string()).or_insert(0) += 1;
            };
            for b in &e.blocks {
                if matches!(b.kind, BlockKind::Private) {
                    continue;
                }
                for f in &b.fields {
                    bump(&f.key, &f.value);
                }
            }
            for (k, v) in &e.fm_fields {
                bump(k, v);
            }
        }
        let mut out: Vec<FieldStat> = acc
            .into_values()
            .map(|a| {
                let mut values: Vec<(String, usize)> = a.values.into_iter().collect();
                values.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
                FieldStat { key: a.key, count: a.notes, values: values.into_iter().take(top).map(|(v, _)| v).collect() }
            })
            .collect();
        out.sort_by(|a, b| b.count.cmp(&a.count).then(a.key.cmp(&b.key)));
        out
    }

    pub fn tags(&self) -> Vec<(String, usize)> {
        let mut v: Vec<(String, usize)> = self.tags.iter().map(|(t, s)| (t.clone(), s.len())).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    /// All note stems + aliases, for wikilink completion.
    pub fn link_targets(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .notes
            .values()
            .flat_map(|e| {
                std::iter::once(e.summary.id.stem().to_string()).chain(e.summary.aliases.iter().cloned())
            })
            .collect();
        v.sort();
        v.dedup();
        v
    }

    /// Note entries in note-id order, so a query that stops at its limit
    /// returns a deterministic prefix instead of whatever the hash map yields.
    pub(crate) fn entries_sorted(&self) -> Vec<(&NoteId, &NoteEntry)> {
        let mut v: Vec<_> = self.notes.iter().collect();
        v.sort_by(|a, b| a.0.cmp(b.0));
        v
    }

    /// Values of a note-level field (frontmatter scalar or `created`), key
    /// compared case-insensitively.
    pub fn frontmatter_field<'a>(&'a self, id: &NoteId, key: &str) -> impl Iterator<Item = &'a str> + 'a {
        let key = key.to_lowercase();
        self.notes
            .get(id)
            .into_iter()
            .flat_map(|e| e.fm_fields.iter())
            .filter(move |(k, _)| k.to_lowercase() == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn mtime_of(&self, id: &NoteId) -> Option<u64> {
        self.notes.get(id).map(|e| e.summary.mtime_ms)
    }
}
