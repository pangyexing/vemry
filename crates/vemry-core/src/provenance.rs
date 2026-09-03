//! Block provenance: who authored the current text of a block — the user, an
//! agent session (with a pending → accepted review state), a web clip or a
//! quick capture. No syntax is added to the notes; the record is a ledger in
//! `.vemry/provenance.jsonl`, written at the moment text lands (an agent
//! write through the permission gate, a chat answer inserted, a clip, a
//! capture), and a block is identified by a hash of its normalized text —
//! so provenance survives a block being moved, indented or turned into
//! another kind, and is dropped the moment its words change.
//!
//! The permission gate is consent before a write; provenance is review after
//! it. "Allow for this session", a turn that writes five notes, an "edit then
//! apply" where one line of fifty was read — those land on disk without being
//! read line by line. A pending mark on the block says so until the user
//! accepts it.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::block::{Block, BlockKind};
use crate::blockid::is_valid_block_id;
use crate::parser::{parse_list_marker, parse_note};

/// Vault-relative path of the ledger (inside `.vemry`, so never indexed and
/// never visible to the agent).
pub const LEDGER_FILE: &str = ".vemry/provenance.jsonl";

/// The ledger's timestamp format (RFC 3339, UTC — the audit log's too).
pub fn now_ts() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Above this many lines the ledger is compacted on load: only records whose
/// hash still exists in some note survive.
const COMPACT_ABOVE_LINES: usize = 5000;

/// FNV-1a, 64-bit. Implemented here rather than through `DefaultHasher`,
/// whose output is not stable across Rust versions — a ledger must hash the
/// same text to the same id for years.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The text a block's identity is computed from: markers, checkbox, callout
/// head, trailing ` ^id` and capture stamps stripped line by line, whitespace
/// folded, blank lines dropped. A fenced code block keeps its lines verbatim
/// (a `#` or `-` inside code is code) and only loses the fence lines.
/// `None` when nothing is left.
pub fn normalize_block(raw_slice: &str, kind: &BlockKind) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    if matches!(kind, BlockKind::CodeBlock { .. }) {
        let all: Vec<&str> = raw_slice.lines().collect();
        let inner = if all.len() >= 2 { &all[1..all.len() - 1] } else { &all[..0] };
        for l in inner {
            lines.push(l.trim_end().to_string());
        }
    } else {
        for l in raw_slice.lines() {
            let t = normalize_line(l);
            if !t.is_empty() {
                lines.push(t);
            }
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn normalize_line(line: &str) -> String {
    let mut t = line.trim();
    // quote markers, repeatedly
    while let Some(r) = t.strip_prefix('>') {
        t = r.trim_start();
    }
    // heading marks
    let hashes = t.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
        t = t[hashes..].trim_start();
    }
    // a bare marker (`-`, `1.`) is an empty item
    if parse_list_marker(&format!("{t} ")).is_some_and(|(_, _, off)| off > t.len()) {
        return String::new();
    }
    // list marker + checkbox
    if let Some((_, _, off)) = parse_list_marker(t) {
        t = t[off..].trim_start();
        let mut chars = t.chars();
        if chars.next() == Some('[') {
            chars.next();
            if chars.next() == Some(']') {
                let rest = chars.as_str();
                if rest.is_empty() || rest.starts_with(' ') {
                    t = rest.trim_start();
                }
            }
        }
    }
    // callout head
    if let Some(r) = t.strip_prefix("[!") {
        if let Some(close) = r.find(']') {
            let mut rest = &r[close + 1..];
            rest = rest.strip_prefix('+').or_else(|| rest.strip_prefix('-')).unwrap_or(rest);
            t = rest.trim_start();
        }
    }
    // trailing anchor
    if let Some(p) = t.rfind(" ^") {
        if is_valid_block_id(&t[p + 2..]) {
            t = t[..p].trim_end();
        }
    }
    let unstamped = strip_stamps(t);
    unstamped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Remove every quick-capture stamp `*(YYYY-MM-DD HH:MM)*` (the shape
/// `vault.capture` writes).
fn strip_stamps(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find("*(") {
        let after = &rest[p + 2..];
        if is_stamp_body(after) {
            out.push_str(&rest[..p]);
            rest = &after[18..];
        } else {
            out.push_str(&rest[..p + 2]);
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// `YYYY-MM-DD HH:MM)*` — 16 characters of stamp, then the closer.
fn is_stamp_body(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 18 {
        return false;
    }
    let digit = |i: usize| b[i].is_ascii_digit();
    (0..4).all(digit)
        && b[4] == b'-'
        && digit(5)
        && digit(6)
        && b[7] == b'-'
        && digit(8)
        && digit(9)
        && b[10] == b' '
        && digit(11)
        && digit(12)
        && b[13] == b':'
        && digit(14)
        && digit(15)
        && &b[16..18] == b")*"
}

/// Whether a block's raw text carries a capture stamp, and the stamp itself.
pub fn capture_stamp(raw_slice: &str) -> Option<String> {
    let mut rest = raw_slice;
    while let Some(p) = rest.find("*(") {
        let after = &rest[p + 2..];
        if is_stamp_body(after) {
            return Some(after[..16].to_string());
        }
        rest = after;
    }
    None
}

/// A selection clip's credit line: `> — [title](url) · YYYY-MM-DD` as the
/// block's last line → (url, date).
pub fn clip_credit(raw_slice: &str) -> Option<(String, String)> {
    let last = raw_slice.lines().last()?.trim();
    let body = last.strip_prefix('>')?.trim_start().strip_prefix("— [")?;
    let close = body.find("](")?;
    let after = &body[close + 2..];
    let paren = after.find(')')?;
    let url = after[..paren].to_string();
    let date = after[paren + 1..].trim().strip_prefix("· ")?.trim();
    if date.len() != 10 || !date.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() }) {
        return None;
    }
    Some((url, date.to_string()))
}

/// 16 hex characters of FNV-1a over the normalized text.
pub fn block_hash(raw_slice: &str, kind: &BlockKind) -> Option<String> {
    normalize_block(raw_slice, kind).map(|n| format!("{:016x}", fnv1a64(n.as_bytes())))
}

fn kind_label(kind: &BlockKind) -> &'static str {
    match kind {
        BlockKind::Paragraph => "paragraph",
        BlockKind::Heading { .. } => "heading",
        BlockKind::ListItem { .. } => "list_item",
        BlockKind::Task { .. } => "task",
        BlockKind::Claim => "claim",
        BlockKind::Evidence { .. } => "evidence",
        BlockKind::CodeBlock { .. } => "code_block",
        BlockKind::Private => "private",
    }
}

/// One block as the ledger names it: its hash and kind, never its text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockStamp {
    pub hash: String,
    pub kind: String,
}

/// One changed region of an agent write, kept with the ledger line so the
/// write can be put back block by block: `old` and `new` are the lines the
/// write removed and added (each line with its break), `before` and `after`
/// up to three unchanged lines on either side, to find the region again by
/// its surroundings once the note has moved on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hunk {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub before: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub old: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub new: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub after: String,
}

/// One replacement of a revert plan: a byte range of the text the plan was
/// made for, and what goes in its place.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Replacement {
    pub from: usize,
    pub to: usize,
    pub insert: String,
}

/// How to undo the agent writes behind a note's pending blocks, as
/// replacements in the text handed to `plan_revert` — the editor's buffer,
/// which may be ahead of disk. `missed` counts pending blocks whose write
/// could not be located: a ledger line older than the hunks, or surroundings
/// that have moved on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RevertPlan {
    pub replacements: Vec<Replacement>,
    pub missed: usize,
}

/// Where a hunk's added lines sit in `text` now: with their context when the
/// context still matches, the added lines alone when the user edited nearby;
/// None when they are gone or occur more than once. A hunk that only removed
/// lines is the point where its context meets (an empty range), found by that
/// context alone.
pub fn locate_hunk(text: &str, h: &Hunk) -> Option<std::ops::Range<usize>> {
    if h.new.is_empty() {
        let join = format!("{}{}", h.before, h.after);
        let r = unique_match(text, &join)?;
        let at = r.start + h.before.len();
        return Some(at..at);
    }
    let full = format!("{}{}{}", h.before, h.new, h.after);
    if let Some(r) = unique_match(text, &full) {
        return Some(r.start + h.before.len()..r.end - h.after.len());
    }
    unique_match(text, &h.new)
}

fn unique_match(text: &str, needle: &str) -> Option<std::ops::Range<usize>> {
    let mut it = text.match_indices(needle);
    let (at, _) = it.next()?;
    if it.next().is_some() {
        return None;
    }
    Some(at..at + needle.len())
}

/// The hunk of a pending record that produced the block at `span` in `text`:
/// each of the record's hunks located in the text, the one touching the block.
/// A block the write merely extended is matched by the lines it added, not by
/// the block as a whole. None for a record older than the hunks, or when no
/// hunk can be found exactly once.
pub fn hunk_of_block<'a>(
    text: &str,
    rec: &'a ProvRecord,
    span: &std::ops::Range<usize>,
) -> Option<(std::ops::Range<usize>, &'a Hunk)> {
    rec.hunks.iter().find_map(|h| {
        let r = locate_hunk(text, h)?;
        let touches =
            if r.is_empty() { r.start > span.start && r.start < span.end } else { r.start < span.end && r.end > span.start };
        touches.then_some((r, h))
    })
}

/// The revert plan for `text`: the pending block with `local_id`, or every
/// pending block when None. A block's write is the ledger hunk whose added
/// lines hold the block; the plan puts that hunk back to what it replaced, so
/// blocks that arrived in one hunk go back together. Replacements come sorted
/// and disjoint; a hunk that cannot be found once is missed, never guessed at.
pub fn plan_revert<'a>(
    text: &str,
    lookup: impl Fn(&str) -> Option<&'a ProvRecord>,
    local_id: Option<&str>,
) -> crate::Result<RevertPlan> {
    let parsed = parse_note(text);
    let targets: Vec<&Block> = match local_id {
        Some(lid) => vec![parsed
            .blocks
            .iter()
            .find(|b| b.local_id == lid)
            .ok_or_else(|| crate::VemryError::BlockNotFound { local_id: lid.to_string() })?],
        None => parsed.blocks.iter().collect(),
    };
    let mut plan = RevertPlan::default();
    for b in targets {
        let slice = &text[b.span.start.min(text.len())..b.span.end.min(text.len())];
        let Some(hash) = block_hash(slice, &b.kind) else { continue };
        let Some(rec) = lookup(&hash) else { continue };
        if rec.accepted || !matches!(rec.source, ProvSource::Agent { .. }) {
            continue;
        }
        let Some((range, hunk)) = hunk_of_block(text, rec, &b.span) else {
            plan.missed += 1;
            continue;
        };
        if plan.replacements.iter().any(|r| r.from == range.start && r.to == range.end) {
            continue;
        }
        plan.replacements.push(Replacement { from: range.start, to: range.end, insert: hunk.old.clone() });
    }
    plan.replacements.sort_by_key(|r| r.from);
    let mut end = 0;
    plan.replacements.retain(|r| {
        let keep = r.from >= end;
        if keep {
            end = r.to;
        }
        keep
    });
    Ok(plan)
}

/// Where an inserted text came from, for writes that do not pass the
/// permission gate (they are the user's own act, so they land accepted).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertOrigin {
    Agent,
    Clip,
    Capture,
}

/// One ledger line. Append-only; a later line overrides an earlier one for
/// the same hash, and an `accept` flips the state of the hashes it names.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProvenanceEntry {
    /// An agent write that passed the permission gate: pending until accepted.
    Agent {
        ts: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
        path: String,
        blocks: Vec<BlockStamp>,
        /// The write's changed regions, so it can be put back (`plan_revert`);
        /// absent on lines older than the field.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        hunks: Vec<Hunk>,
    },
    /// Text the user put in place themselves (chat insert, clip, capture).
    Insert {
        ts: String,
        origin: InsertOrigin,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        path: String,
        blocks: Vec<BlockStamp>,
    },
    Accept {
        ts: String,
        hashes: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvSource {
    Agent { session: Option<String> },
    Clip { url: Option<String> },
    Capture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvRecord {
    pub source: ProvSource,
    pub ts: String,
    pub accepted: bool,
    /// The write's changed regions (an `agent` line's; empty otherwise).
    pub hunks: Vec<Hunk>,
}

/// The ledger in memory: the latest record per block hash, plus which notes
/// ever had a record (the candidate set for a pending count).
pub struct ProvenanceLedger {
    path: PathBuf,
    by_hash: HashMap<String, ProvRecord>,
    by_path: HashMap<String, HashSet<String>>,
}

impl ProvenanceLedger {
    pub fn empty(vault_root: &Path) -> ProvenanceLedger {
        ProvenanceLedger { path: vault_root.join(LEDGER_FILE), by_hash: HashMap::new(), by_path: HashMap::new() }
    }

    /// Load and, above the size threshold, compact against the hashes that
    /// still exist (`live`). Bad lines are skipped: a damaged ledger must not
    /// make the vault unopenable.
    pub fn load(vault_root: &Path, live: impl FnOnce() -> HashSet<String>) -> ProvenanceLedger {
        let mut ledger = Self::empty(vault_root);
        let Ok(text) = fs::read_to_string(&ledger.path) else { return ledger };
        let mut lines = 0usize;
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            lines += 1;
            if let Ok(entry) = serde_json::from_str::<ProvenanceEntry>(line) {
                ledger.apply(&entry);
            }
        }
        if lines > COMPACT_ABOVE_LINES {
            ledger.compact(&live());
        }
        ledger
    }

    fn apply(&mut self, entry: &ProvenanceEntry) {
        match entry {
            ProvenanceEntry::Agent { ts, session, path, blocks, hunks } => {
                for b in blocks {
                    self.by_hash.insert(
                        b.hash.clone(),
                        ProvRecord {
                            source: ProvSource::Agent { session: session.clone() },
                            ts: ts.clone(),
                            accepted: false,
                            hunks: hunks.clone(),
                        },
                    );
                    self.by_path.entry(path.clone()).or_default().insert(b.hash.clone());
                }
            }
            ProvenanceEntry::Insert { ts, origin, session, url, path, blocks } => {
                let source = match origin {
                    InsertOrigin::Agent => ProvSource::Agent { session: session.clone() },
                    InsertOrigin::Clip => ProvSource::Clip { url: url.clone() },
                    InsertOrigin::Capture => ProvSource::Capture,
                };
                for b in blocks {
                    self.by_hash
                        .insert(b.hash.clone(), ProvRecord { source: source.clone(), ts: ts.clone(), accepted: true, hunks: vec![] });
                    self.by_path.entry(path.clone()).or_default().insert(b.hash.clone());
                }
            }
            ProvenanceEntry::Accept { hashes, .. } => {
                for h in hashes {
                    if let Some(r) = self.by_hash.get_mut(h) {
                        r.accepted = true;
                    }
                }
            }
        }
    }

    /// Apply and persist one entry. A failed write is silent, like the audit
    /// log's: the record is best effort, the note itself is not at stake.
    pub fn append(&mut self, entry: ProvenanceEntry) {
        self.apply(&entry);
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(line) = serde_json::to_string(&entry) {
            if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&self.path) {
                let _ = writeln!(f, "{line}");
            }
        }
    }

    pub fn lookup(&self, hash: &str) -> Option<&ProvRecord> {
        self.by_hash.get(hash)
    }

    pub fn has_path(&self, path: &str) -> bool {
        self.by_path.contains_key(path)
    }

    /// Rewrite the file keeping one line per live hash (an `agent` or
    /// `insert` line carrying its state, plus one `accept` for the accepted
    /// ones). Written to a temporary file and renamed into place.
    fn compact(&mut self, live: &HashSet<String>) {
        self.by_hash.retain(|h, _| live.contains(h));
        for set in self.by_path.values_mut() {
            set.retain(|h| live.contains(h));
        }
        self.by_path.retain(|_, set| !set.is_empty());
        let mut lines: Vec<String> = Vec::new();
        let mut paths: Vec<(&String, &HashSet<String>)> = self.by_path.iter().collect();
        paths.sort_by(|a, b| a.0.cmp(b.0));
        for (path, hashes) in paths {
            let mut hs: Vec<&String> = hashes.iter().collect();
            hs.sort();
            for h in hs {
                let Some(rec) = self.by_hash.get(h) else { continue };
                let stamp = BlockStamp { hash: h.clone(), kind: String::new() };
                let entry = match &rec.source {
                    ProvSource::Agent { session } if !rec.accepted => ProvenanceEntry::Agent {
                        ts: rec.ts.clone(),
                        session: session.clone(),
                        path: path.clone(),
                        blocks: vec![stamp],
                        hunks: rec.hunks.clone(),
                    },
                    ProvSource::Agent { session } => ProvenanceEntry::Insert {
                        ts: rec.ts.clone(),
                        origin: InsertOrigin::Agent,
                        session: session.clone(),
                        url: None,
                        path: path.clone(),
                        blocks: vec![stamp],
                    },
                    ProvSource::Clip { url } => ProvenanceEntry::Insert {
                        ts: rec.ts.clone(),
                        origin: InsertOrigin::Clip,
                        session: None,
                        url: url.clone(),
                        path: path.clone(),
                        blocks: vec![stamp],
                    },
                    ProvSource::Capture => ProvenanceEntry::Insert {
                        ts: rec.ts.clone(),
                        origin: InsertOrigin::Capture,
                        session: None,
                        url: None,
                        path: path.clone(),
                        blocks: vec![stamp],
                    },
                };
                if let Ok(l) = serde_json::to_string(&entry) {
                    lines.push(l);
                }
            }
        }
        let tmp = self.path.with_extension("jsonl.tmp");
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let body = if lines.is_empty() { String::new() } else { lines.join("\n") + "\n" };
        if fs::write(&tmp, body).is_ok() {
            let _ = fs::rename(&tmp, &self.path);
        }
    }
}

/// Stamps of every block of `text`, parsed standalone (a frontmatter is
/// skipped like a note's).
pub fn stamps_of(text: &str) -> Vec<BlockStamp> {
    let parsed = parse_note(text);
    parsed
        .blocks
        .iter()
        .filter_map(|b| block_hash(&text[b.span.clone()], &b.kind).map(|hash| BlockStamp { hash, kind: kind_label(&b.kind).into() }))
        .collect()
}

/// The blocks of `new` that are not in `old`, as a multiset difference of
/// hashes — a block the writer merely moved is not "added", a block it
/// changed is (its old hash disappears, its new one appears).
pub fn added_blocks(old: &str, new: &str) -> Vec<BlockStamp> {
    let mut have: HashMap<String, usize> = HashMap::new();
    for s in stamps_of(old) {
        *have.entry(s.hash).or_default() += 1;
    }
    let mut added = Vec::new();
    for s in stamps_of(new) {
        match have.get_mut(&s.hash) {
            Some(n) if *n > 0 => *n -= 1,
            _ => added.push(s),
        }
    }
    added
}

/// Who authored a block's current text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum Origin {
    You,
    Agent {
        #[serde(skip_serializing_if = "Option::is_none")]
        session: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ts: Option<String>,
        pending: bool,
    },
    Clip {
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ts: Option<String>,
    },
    Capture {
        #[serde(skip_serializing_if = "Option::is_none")]
        ts: Option<String>,
    },
}

#[derive(Clone, Debug, Serialize)]
pub struct BlockProvenance {
    pub local_id: String,
    /// Byte range in the note's raw text (`Block.span`).
    pub span: std::ops::Range<usize>,
    /// For a pending block whose write the ledger holds as hunks: the part of
    /// the block that write added (a block the model extended keeps its
    /// reviewed lines out of the mark). None when the ledger cannot say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changed: Option<std::ops::Range<usize>>,
    /// Empty for a block whose normalized text is empty.
    pub hash: String,
    pub origin: Origin,
}

/// Which frontmatter facts the derived origins need.
pub struct NoteFacts<'a> {
    pub origin: Option<&'a str>,
    pub agent_session: Option<&'a str>,
    pub source: Option<&'a str>,
    pub clipped: Option<&'a str>,
}

/// The origin of one block: the ledger first, then the marks the text
/// itself carries (a capture stamp, a clip credit line), then the note's
/// frontmatter (`vemry.origin`), else the user. `path_in_ledger` limits the
/// frontmatter `agent` fallback to notes that predate the ledger: in a note
/// the ledger knows, a block without a record is the user's.
pub fn derive_origin(
    raw_slice: &str,
    record: Option<&ProvRecord>,
    facts: &NoteFacts<'_>,
    path_in_ledger: bool,
) -> Origin {
    if let Some(r) = record {
        return match &r.source {
            ProvSource::Agent { session } => {
                Origin::Agent { session: session.clone(), ts: Some(r.ts.clone()), pending: !r.accepted }
            }
            ProvSource::Clip { url } => Origin::Clip { url: url.clone(), ts: Some(r.ts.clone()) },
            ProvSource::Capture => Origin::Capture { ts: Some(r.ts.clone()) },
        };
    }
    if let Some(ts) = capture_stamp(raw_slice) {
        return Origin::Capture { ts: Some(ts) };
    }
    if let Some((url, date)) = clip_credit(raw_slice) {
        return Origin::Clip { url: Some(url), ts: Some(date) };
    }
    match facts.origin {
        Some("clip") => Origin::Clip { url: facts.source.map(String::from), ts: facts.clipped.map(String::from) },
        Some("agent") if !path_in_ledger => {
            Origin::Agent { session: facts.agent_session.map(String::from), ts: None, pending: false }
        }
        Some("capture") => Origin::Capture { ts: None },
        _ => Origin::You,
    }
}

/// Every block's stamp in a parsed note — the "live" set for compaction.
pub fn live_hashes<'a>(raw: &'a str, blocks: &'a [Block]) -> impl Iterator<Item = String> + 'a {
    blocks.iter().filter_map(move |b| block_hash(&raw[b.span.clone()], &b.kind))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(src: &str) -> Option<String> {
        let parsed = parse_note(src);
        let b = parsed.blocks.first()?;
        block_hash(&src[b.span.clone()], &b.kind)
    }

    #[test]
    fn fnv_known_vectors() {
        assert_eq!(format!("{:016x}", fnv1a64(b"")), "cbf29ce484222325");
        assert_eq!(format!("{:016x}", fnv1a64(b"a")), "af63dc4c8601ec8c");
    }

    #[test]
    fn identity_survives_markers_anchors_indent_and_stamps() {
        let base = h("hello world\n").unwrap();
        for variant in [
            "  hello   world  \n",
            "- hello world\n",
            "1. hello world\n",
            "- [ ] hello world\n",
            "- [x] hello world\n",
            "# hello world\n",
            "> hello world\n",
            "> [!note] hello world\n",
            "> [!private]- hello world\n",
            "hello world ^abc12\n",
            "- hello world *(2026-09-02 10:30)*\n",
            "\t- hello world\n",
        ] {
            assert_eq!(h(variant).unwrap(), base, "{variant:?}");
        }
        assert_ne!(h("hello there\n").unwrap(), base);
        assert_ne!(h("Hello world\n").unwrap(), base, "case is content");
        assert!(h("\n").is_none());
        assert!(h("- \n").is_none());
    }

    #[test]
    fn code_keeps_its_lines_and_drops_fences() {
        let a = h("```rust\n# not a heading\n- not a list\n```\n").unwrap();
        let b = h("```\n# not a heading\n- not a list\n```\n").unwrap();
        assert_eq!(a, b, "the fence language is not identity");
        assert_ne!(a, h("```\nnot a heading\nnot a list\n```\n").unwrap(), "markers inside code are code");
    }

    #[test]
    fn stamps_and_credits_are_recognised() {
        assert_eq!(capture_stamp("- x *(2026-09-02 10:30)*").as_deref(), Some("2026-09-02 10:30"));
        assert_eq!(capture_stamp("- x *(2026-09-02)*"), None, "a chat signature has no time");
        assert_eq!(
            clip_credit("> quoted\n> — [Title](https://x.y/z) · 2026-09-02"),
            Some(("https://x.y/z".into(), "2026-09-02".into()))
        );
        assert_eq!(clip_credit("> — [Title](https://x.y/z)"), None);
    }

    #[test]
    fn added_blocks_is_a_multiset_difference() {
        let old = "a\n\nb\n\nc\n";
        assert!(added_blocks(old, "c\n\na\n\nb\n").is_empty(), "a pure move adds nothing");
        let added = added_blocks(old, "a\n\nnew\n\nb\n\nc\n");
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].kind, "paragraph");
        assert_eq!(added_blocks(old, "a\n\nb changed\n\nc\n").len(), 1, "a changed block is a new one");
        assert_eq!(added_blocks("", "x\n\ny\n").len(), 2);
        assert_eq!(added_blocks("dup\n\ndup\n", "dup\n\ndup\n\ndup\n").len(), 1, "a third copy is new");
        assert!(added_blocks(old, "").is_empty());
    }

    #[test]
    fn ledger_round_trips_accepts_and_skips_bad_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut ledger = ProvenanceLedger::empty(root);
        ledger.append(ProvenanceEntry::Agent {
            ts: "t1".into(),
            session: Some("s1".into()),
            path: "n.md".into(),
            blocks: vec![BlockStamp { hash: "aaaa".into(), kind: "paragraph".into() }],
            hunks: vec![],
        });
        ledger.append(ProvenanceEntry::Insert {
            ts: "t2".into(),
            origin: InsertOrigin::Clip,
            session: None,
            url: Some("https://u".into()),
            path: "n.md".into(),
            blocks: vec![BlockStamp { hash: "bbbb".into(), kind: "paragraph".into() }],
        });
        fs::OpenOptions::new().append(true).open(root.join(LEDGER_FILE)).unwrap().write_all(b"not json\n").unwrap();
        let reloaded = ProvenanceLedger::load(root, HashSet::new);
        assert!(!reloaded.lookup("aaaa").unwrap().accepted);
        assert!(matches!(reloaded.lookup("aaaa").unwrap().source, ProvSource::Agent { .. }));
        assert!(reloaded.lookup("bbbb").unwrap().accepted);
        assert!(reloaded.has_path("n.md"));
        let mut reloaded = reloaded;
        reloaded.append(ProvenanceEntry::Accept { ts: "t3".into(), hashes: vec!["aaaa".into()] });
        let again = ProvenanceLedger::load(root, HashSet::new);
        assert!(again.lookup("aaaa").unwrap().accepted);
        // an agent write of the same hash makes it pending again
        let mut again = again;
        again.append(ProvenanceEntry::Agent { ts: "t4".into(), session: None, path: "n.md".into(), blocks: vec![BlockStamp { hash: "aaaa".into(), kind: "paragraph".into() }], hunks: vec![] });
        assert!(!again.lookup("aaaa").unwrap().accepted);
    }

    #[test]
    fn hunks_locate_by_context_then_by_their_lines() {
        let h = Hunk { before: "a\n".into(), old: String::new(), new: "NEW\n".into(), after: "b\n".into() };
        assert_eq!(locate_hunk("a\nNEW\nb\n", &h), Some(2..6));
        // the user edited the context: the added lines alone still locate it
        assert_eq!(locate_hunk("A\nNEW\nB\n", &h), Some(2..6));
        // copied: ambiguous; gone: nothing
        assert_eq!(locate_hunk("NEW\nx\nNEW\n", &h), None);
        assert_eq!(locate_hunk("a\nb\n", &h), None);
        // a deletion is the point where its context meets
        let del = Hunk { before: "a\n".into(), old: "gone\n".into(), new: String::new(), after: "b\n".into() };
        assert_eq!(locate_hunk("a\nb\n", &del), Some(2..2));
        assert_eq!(locate_hunk("a\nx\nb\n", &del), None);
    }

    #[test]
    fn the_hunk_of_an_extended_block_is_the_lines_it_added() {
        let rec = |h: Hunk| ProvRecord { source: ProvSource::Agent { session: None }, ts: "t".into(), accepted: false, hunks: vec![h] };
        // TEST6 appended to the reviewed line TEST4 without a blank line: one block
        let text = "TEST4\nTEST6\n";
        let ext = rec(Hunk { before: "TEST4\n".into(), old: String::new(), new: "TEST6\n".into(), after: String::new() });
        let (r, _) = hunk_of_block(text, &ext, &(0..11)).unwrap();
        assert_eq!(r, 6..12);
        let plan = plan_revert(text, |_| Some(&ext), None).unwrap();
        assert_eq!(plan.replacements, vec![Replacement { from: 6, to: 12, insert: String::new() }]);
        // a line deleted inside a block is restored at the point its context meets
        let del = rec(Hunk { before: "a\n".into(), old: "gone\n".into(), new: String::new(), after: "b\n".into() });
        let plan = plan_revert("a\nb\n", |_| Some(&del), None).unwrap();
        assert_eq!(plan.replacements, vec![Replacement { from: 2, to: 2, insert: "gone\n".into() }]);
        // a hunk elsewhere in the note does not belong to this block
        assert!(hunk_of_block("x\n\nTEST6\n", &ext, &(0..1)).is_none());
    }

    #[test]
    fn a_plan_puts_the_hunks_back_and_the_ledger_keeps_them() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let text = "a\n\nNEW one\n\nb\n\nNEW two\n";
        let stamps = stamps_of(text);
        let mut ledger = ProvenanceLedger::empty(root);
        ledger.append(ProvenanceEntry::Agent {
            ts: "t".into(),
            session: Some("s".into()),
            path: "n.md".into(),
            blocks: vec![stamps[1].clone(), stamps[3].clone()],
            hunks: vec![
                Hunk { before: "a\n\n".into(), old: "old one\n\n".into(), new: "NEW one\n\n".into(), after: "b\n".into() },
                Hunk { before: "b\n\n".into(), old: String::new(), new: "NEW two\n".into(), after: String::new() },
            ],
        });
        // the hunks survive the file
        let ledger = ProvenanceLedger::load(root, HashSet::new);
        assert_eq!(ledger.lookup(&stamps[1].hash).unwrap().hunks.len(), 2);
        let plan = plan_revert(text, |h| ledger.lookup(h), Some("b1")).unwrap();
        assert_eq!(plan.replacements, vec![Replacement { from: 3, to: 12, insert: "old one\n\n".into() }]);
        assert_eq!(plan.missed, 0);
        let all = plan_revert(text, |h| ledger.lookup(h), None).unwrap();
        assert_eq!(all.replacements.len(), 2);
        let mut out = text.to_string();
        for r in all.replacements.iter().rev() {
            out.replace_range(r.from..r.to, &r.insert);
        }
        assert_eq!(out, "a\n\nold one\n\nb\n\n");
        // a user block plans nothing; an unknown local id is an error
        assert!(plan_revert(text, |h| ledger.lookup(h), Some("b0")).unwrap().replacements.is_empty());
        assert!(plan_revert(text, |h| ledger.lookup(h), Some("b9")).is_err());
        // accepted blocks are out of the plan
        let mut ledger = ledger;
        ledger.append(ProvenanceEntry::Accept { ts: "t".into(), hashes: vec![stamps[3].hash.clone()] });
        assert_eq!(plan_revert(text, |h| ledger.lookup(h), None).unwrap().replacements.len(), 1);
        // a line older than the hunks: pending, but not revertible
        ledger.append(ProvenanceEntry::Agent {
            ts: "t".into(),
            session: None,
            path: "n.md".into(),
            blocks: vec![stamps[1].clone()],
            hunks: vec![],
        });
        let p = plan_revert(text, |h| ledger.lookup(h), Some("b1")).unwrap();
        assert!(p.replacements.is_empty() && p.missed == 1);
    }

    #[test]
    fn compaction_keeps_live_hashes_only() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let mut ledger = ProvenanceLedger::empty(root);
        for i in 0..(COMPACT_ABOVE_LINES + 5) {
            ledger.append(ProvenanceEntry::Agent {
                ts: "t".into(),
                session: Some("s".into()),
                path: "n.md".into(),
                blocks: vec![BlockStamp { hash: format!("h{i}"), kind: "paragraph".into() }],
                hunks: vec![],
            });
        }
        ledger.append(ProvenanceEntry::Accept { ts: "t".into(), hashes: vec!["h1".into()] });
        let live: HashSet<String> = ["h1".to_string(), "h2".to_string()].into_iter().collect();
        let compacted = ProvenanceLedger::load(root, || live.clone());
        assert!(compacted.lookup("h0").is_none());
        assert!(compacted.lookup("h1").unwrap().accepted);
        assert!(!compacted.lookup("h2").unwrap().accepted);
        let lines = fs::read_to_string(root.join(LEDGER_FILE)).unwrap().lines().count();
        assert_eq!(lines, 2);
        // the compacted file reloads to the same state
        let again = ProvenanceLedger::load(root, HashSet::new);
        assert!(again.lookup("h1").unwrap().accepted && !again.lookup("h2").unwrap().accepted);
    }

    #[test]
    fn derived_origins_follow_the_priority_table() {
        let facts = NoteFacts { origin: None, agent_session: None, source: None, clipped: None };
        assert_eq!(derive_origin("plain", None, &facts, false), Origin::You);
        assert!(matches!(derive_origin("- x *(2026-09-02 10:30)*", None, &facts, false), Origin::Capture { ts: Some(_) }));
        assert!(matches!(derive_origin("> q\n> — [t](https://u) · 2026-09-02", None, &facts, false), Origin::Clip { url: Some(_), .. }));
        let clip = NoteFacts { origin: Some("clip"), agent_session: None, source: Some("https://s"), clipped: Some("2026-09-01T00:00:00Z") };
        assert_eq!(derive_origin("body", None, &clip, false), Origin::Clip { url: Some("https://s".into()), ts: Some("2026-09-01T00:00:00Z".into()) });
        let agent = NoteFacts { origin: Some("agent"), agent_session: Some("s9"), source: None, clipped: None };
        assert_eq!(derive_origin("body", None, &agent, false), Origin::Agent { session: Some("s9".into()), ts: None, pending: false });
        assert_eq!(derive_origin("body", None, &agent, true), Origin::You, "a ledger-known note: unrecorded blocks are the user's");
        let rec = ProvRecord { source: ProvSource::Agent { session: Some("s1".into()) }, ts: "t".into(), accepted: false, hunks: vec![] };
        assert_eq!(derive_origin("body", Some(&rec), &facts, false), Origin::Agent { session: Some("s1".into()), ts: Some("t".into()), pending: true });
    }
}
