//! Block-level rearrangement of a note: the units a note is made of (a heading, a
//! paragraph, a list item with its subtree, a code block…), how they are moved
//! under an outline or into another note, and how two versions of a note compare
//! block by block. Three invariants the agent's restructure_note / move_blocks
//! tools rest on:
//!
//! 1. **A private block is an anchor.** It travels with the public unit it follows
//!    (a private tail), is never moved on its own, never moved to another note,
//!    and survives every removal — a restructure cannot lose or expose one.
//! 2. **Ids travel with their blocks.** A unit's text is moved byte for byte, so an
//!    explicit `^id` stays on its block; a cross-note move rewrites the vault's
//!    `[[from#^id]]` links to `[[to#^id]]` (relink.rs's job, planned by the caller).
//! 3. **Nothing disappears silently.** A unit the outline does not name lands in an
//!    "unassigned" tail unless the caller listed it as removed.
//!
//! Seams between units follow the formatter's rule (format.rs): two items of one
//! list type touch, everything else is separated by one blank line — so a
//! restructured note is already in the shape the format-note command would give it.

use std::collections::HashMap;
use std::ops::Range;

use crate::parser::parse_note;
use crate::provenance::block_hash;
use crate::BlockKind;

/// One movable unit: a public block and the private blocks attached to it.
#[derive(Clone, Debug)]
pub struct Unit {
    pub handle: String,
    pub kind: BlockKind,
    /// The unit's own text (the block, or a list item with its subtree).
    pub span: Range<usize>,
    /// The private blocks that follow it (each its own span); they stay with it in
    /// a restructure and stay behind in a cross-note move.
    pub private_tail: Vec<Range<usize>>,
}

/// One section of the target outline: a heading (None = the preamble before the
/// first heading) and the handles of the units under it, in order. A heading that
/// exists in the note is reused (its own block, id included); a new one is written.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct OutlineSection {
    #[serde(default)]
    pub heading: Option<String>,
    #[serde(default = "default_level")]
    pub level: u8,
    #[serde(default)]
    pub blocks: Vec<String>,
}

fn default_level() -> u8 {
    2
}

/// The units of a note in document order. A heading is a unit of its own (its
/// section's blocks are units of their own too); a list item or task spans its
/// subtree, and the nested items inside that subtree are not separate units.
pub fn units_of(raw: &str) -> Vec<Unit> {
    let parsed = parse_note(raw);
    let blocks = &parsed.blocks;
    let mut units: Vec<Unit> = Vec::new();
    let mut i = 0;
    while i < blocks.len() {
        let b = &blocks[i];
        if matches!(b.kind, BlockKind::Private) {
            // a private block before any public unit: an immovable unit of its own
            match units.last_mut() {
                Some(u) => u.private_tail.push(b.span.clone()),
                None => units.push(Unit { handle: String::new(), kind: BlockKind::Private, span: b.span.clone(), private_tail: vec![] }),
            }
            i += 1;
            continue;
        }
        let span = crate::locate::item_subtree_span(blocks, raw, i);
        // skip the blocks the subtree swallowed (nested items and their private blocks stay inside the unit's text)
        let mut j = i + 1;
        while j < blocks.len() && blocks[j].span.end <= span.end {
            j += 1;
        }
        units.push(Unit { handle: b.handle(), kind: b.kind.clone(), span, private_tail: vec![] });
        i = j;
    }
    units
}

/// The seam type the editor's rule reads.
fn seam_kind(kind: &BlockKind) -> &'static str {
    match kind {
        BlockKind::ListItem { ordered: true, .. } => "olist",
        BlockKind::ListItem { .. } | BlockKind::Task { .. } => "ulist",
        _ => "other",
    }
}

/// "" when `next` may follow `prev` directly (two items of one list), "\n" for a
/// blank line between (everything else, the formatter's rule).
fn seam(prev: Option<&BlockKind>, next: &BlockKind) -> &'static str {
    let Some(p) = prev else { return "" };
    let (a, b) = (seam_kind(p), seam_kind(next));
    if (a == "ulist" || a == "olist") && a == b {
        ""
    } else {
        "\n"
    }
}

/// A unit's text with its private tail, the trailing whitespace trimmed.
fn unit_text(raw: &str, u: &Unit) -> String {
    let mut s = raw[u.span.clone()].trim_end().to_string();
    for p in &u.private_tail {
        s.push_str("\n\n");
        s.push_str(raw[p.clone()].trim_end());
    }
    s
}

/// Join units with the seam rule; each `(kind, text)` is one unit's text.
fn join_units(parts: &[(BlockKind, String)]) -> String {
    let mut out = String::new();
    let mut prev: Option<&BlockKind> = None;
    for (kind, text) in parts {
        if !out.is_empty() {
            out.push('\n');
            out.push_str(seam(prev, kind));
        }
        out.push_str(text);
        prev = Some(kind);
    }
    out
}

/// The result of a restructure: the note's new text and the handles that were not
/// named anywhere and landed in the unassigned tail.
#[derive(Clone, Debug, PartialEq)]
pub struct Restructured {
    pub text: String,
    pub unassigned: Vec<String>,
}

/// Rearrange a note's public units under `outline`; `removed` names the units to
/// drop (their private tails are kept). Handles are `^id` or `bN` as read_note
/// and query_blocks return them. An unknown handle is an error naming it.
pub fn restructure(raw: &str, outline: &[OutlineSection], removed: &[String]) -> Result<Restructured, String> {
    let parsed = parse_note(raw);
    let front = &raw[..parsed.body_offset];
    let units = units_of(raw);
    let mut by_handle: HashMap<String, usize> = HashMap::new();
    for (i, u) in units.iter().enumerate() {
        if !u.handle.is_empty() {
            by_handle.insert(u.handle.clone(), i);
            by_handle.insert(u.handle.trim_start_matches('^').to_string(), i);
        }
    }
    let lookup = |h: &str| -> Result<usize, String> {
        let clean = h.trim();
        by_handle
            .get(clean)
            .or_else(|| by_handle.get(clean.trim_start_matches('^')))
            .copied()
            .ok_or_else(|| format!("block not found: {clean}"))
    };
    let mut used = vec![false; units.len()];
    let mut parts: Vec<(BlockKind, String)> = Vec::new();
    // private blocks with no public unit before them stay at the very top
    for (i, u) in units.iter().enumerate() {
        if matches!(u.kind, BlockKind::Private) {
            parts.push((BlockKind::Private, raw[u.span.clone()].trim_end().to_string()));
            used[i] = true;
        }
    }
    let mut removed_idx: Vec<usize> = Vec::new();
    for h in removed {
        let i = lookup(h)?;
        if matches!(units[i].kind, BlockKind::Heading { .. }) {
            // a removed heading takes nothing with it: its blocks are placed by the outline or land unassigned
        }
        removed_idx.push(i);
    }
    for section in outline {
        if let Some(text) = section.heading.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            // reuse the note's own heading block when one has this text (its id stays), else a fresh line
            let existing = units.iter().position(|u| {
                matches!(u.kind, BlockKind::Heading { .. }) && !used[units.iter().position(|x| std::ptr::eq(x, u)).unwrap()] && heading_text(raw, u) == text
            });
            match existing {
                Some(i) if !removed_idx.contains(&i) => {
                    used[i] = true;
                    parts.push((units[i].kind.clone(), unit_text(raw, &units[i])));
                }
                _ => {
                    let level = section.level.clamp(1, 6) as usize;
                    parts.push((BlockKind::Heading { level: level as u8 }, format!("{} {text}", "#".repeat(level))));
                }
            }
        }
        for h in &section.blocks {
            let i = lookup(h)?;
            if used[i] {
                return Err(format!("block {h} appears twice in the outline"));
            }
            if removed_idx.contains(&i) {
                return Err(format!("block {h} is both placed and removed"));
            }
            used[i] = true;
            parts.push((units[i].kind.clone(), unit_text(raw, &units[i])));
        }
    }
    // removed units: dropped, their private tails kept in place of them
    for &i in &removed_idx {
        if used[i] {
            continue;
        }
        used[i] = true;
        for p in &units[i].private_tail {
            parts.push((BlockKind::Private, raw[p.clone()].trim_end().to_string()));
        }
    }
    // everything else lands in the unassigned tail, in its original order
    let mut unassigned = Vec::new();
    let leftovers: Vec<usize> = (0..units.len()).filter(|&i| !used[i]).collect();
    if !leftovers.is_empty() {
        parts.push((BlockKind::Heading { level: 2 }, "## Unassigned".to_string()));
        for i in leftovers {
            unassigned.push(units[i].handle.clone());
            parts.push((units[i].kind.clone(), unit_text(raw, &units[i])));
        }
    }
    let mut text = String::with_capacity(raw.len());
    text.push_str(front);
    text.push_str(&join_units(&parts));
    if !text.ends_with('\n') {
        text.push('\n');
    }
    Ok(Restructured { text, unassigned })
}

fn heading_text(raw: &str, u: &Unit) -> String {
    let line = raw[u.span.clone()].lines().next().unwrap_or("");
    line.trim_start_matches('#').trim().to_string()
}

/// Take `handles` out of `from_raw` (their private tails stay behind, in place) and
/// return (the source without them, the moved text ready to insert, the explicit
/// ids that travelled). The moved text keeps the units' own seams.
pub fn extract_units(from_raw: &str, handles: &[String]) -> Result<(String, String, Vec<String>), String> {
    let parsed = parse_note(from_raw);
    let front = &from_raw[..parsed.body_offset];
    let units = units_of(from_raw);
    let mut by_handle: HashMap<String, usize> = HashMap::new();
    for (i, u) in units.iter().enumerate() {
        if !u.handle.is_empty() {
            by_handle.insert(u.handle.clone(), i);
            by_handle.insert(u.handle.trim_start_matches('^').to_string(), i);
        }
    }
    let mut moving = vec![false; units.len()];
    let mut order: Vec<usize> = Vec::new();
    for h in handles {
        let clean = h.trim();
        let i = by_handle.get(clean).or_else(|| by_handle.get(clean.trim_start_matches('^'))).copied().ok_or_else(|| format!("block not found: {clean}"))?;
        if matches!(units[i].kind, BlockKind::Private) {
            return Err(format!("block not found: {clean}"));
        }
        if !moving[i] {
            moving[i] = true;
            order.push(i);
        }
    }
    let mut stay: Vec<(BlockKind, String)> = Vec::new();
    for (i, u) in units.iter().enumerate() {
        if moving[i] {
            for p in &u.private_tail {
                stay.push((BlockKind::Private, from_raw[p.clone()].trim_end().to_string()));
            }
        } else {
            stay.push((u.kind.clone(), unit_text(from_raw, u)));
        }
    }
    let moved: Vec<(BlockKind, String)> = order.iter().map(|&i| (units[i].kind.clone(), from_raw[units[i].span.clone()].trim_end().to_string())).collect();
    let ids: Vec<String> = order
        .iter()
        .filter_map(|&i| units[i].handle.strip_prefix('^').map(str::to_string))
        .collect();
    let mut source = String::with_capacity(from_raw.len());
    source.push_str(front);
    source.push_str(&join_units(&stay));
    if !source.ends_with('\n') {
        source.push('\n');
    }
    Ok((source, join_units(&moved), ids))
}

/// How two versions of a note compare block by block.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
pub struct BlockDiff {
    pub moved: usize,
    pub changed: usize,
    pub added: usize,
    pub removed: usize,
    pub items: Vec<BlockDiffItem>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct BlockDiffItem {
    /// moved | changed | added | removed
    pub kind: &'static str,
    /// The block's first line (cut to 100 characters); for `changed`, the new one.
    pub text: String,
    /// For `changed`, the old first line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old: Option<String>,
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().chars().take(100).collect()
}

fn words(s: &str) -> std::collections::HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 1).map(|w| w.to_lowercase()).collect()
}

/// Blocks by hash: a hash in both versions is kept or moved (moved = out of the
/// common subsequence of shared hashes), one only in the new text is added, one only
/// in the old removed; a removed and an added block that share most of their words
/// are one changed block.
pub fn block_diff(old: &str, new: &str) -> BlockDiff {
    let stamps = |text: &str| -> Vec<(String, String)> {
        parse_note(text)
            .blocks
            .iter()
            .filter(|b| !matches!(b.kind, BlockKind::Private))
            .filter_map(|b| block_hash(&text[b.span.clone()], &b.kind).map(|h| (h, text[b.span.clone()].to_string())))
            .collect()
    };
    let (a, b) = (stamps(old), stamps(new));
    let mut count_a: HashMap<&str, usize> = HashMap::new();
    for (h, _) in &a {
        *count_a.entry(h).or_default() += 1;
    }
    let mut count_b: HashMap<&str, usize> = HashMap::new();
    for (h, _) in &b {
        *count_b.entry(h).or_default() += 1;
    }
    let shared = |h: &str| count_a.contains_key(h) && count_b.contains_key(h);
    let sa: Vec<&str> = a.iter().map(|(h, _)| h.as_str()).filter(|h| shared(h)).collect();
    let sb: Vec<&str> = b.iter().map(|(h, _)| h.as_str()).filter(|h| shared(h)).collect();
    // LCS over the shared sequence: what is not in it moved
    let n = sa.len();
    let m = sb.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if sa[i] == sb[j] { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
        }
    }
    let mut in_lcs = vec![false; m];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if sa[i] == sb[j] {
            in_lcs[j] = true;
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    let mut diff = BlockDiff::default();
    let mut k = 0;
    let mut added: Vec<String> = Vec::new();
    for (h, text) in &b {
        if shared(h) {
            if !in_lcs[k] {
                diff.moved += 1;
                diff.items.push(BlockDiffItem { kind: "moved", text: first_line(text), old: None });
            }
            k += 1;
        } else {
            added.push(text.clone());
        }
    }
    let mut removed: Vec<String> = a.iter().filter(|(h, _)| !shared(h)).map(|(_, t)| t.clone()).collect();
    // pair removed with added by word overlap → changed
    let mut used_added = vec![false; added.len()];
    removed.retain(|r| {
        let rw = words(r);
        let best = added
            .iter()
            .enumerate()
            .filter(|(i, _)| !used_added[*i])
            .map(|(i, t)| {
                let tw = words(t);
                let inter = rw.intersection(&tw).count();
                let union = rw.union(&tw).count().max(1);
                (i, inter as f64 / union as f64)
            })
            .max_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
        match best {
            Some((i, score)) if score >= 0.5 => {
                used_added[i] = true;
                diff.changed += 1;
                diff.items.push(BlockDiffItem { kind: "changed", text: first_line(&added[i]), old: Some(first_line(r)) });
                false
            }
            _ => true,
        }
    });
    for (i, t) in added.iter().enumerate() {
        if !used_added[i] {
            diff.added += 1;
            diff.items.push(BlockDiffItem { kind: "added", text: first_line(t), old: None });
        }
    }
    for r in &removed {
        diff.removed += 1;
        diff.items.push(BlockDiffItem { kind: "removed", text: first_line(r), old: None });
    }
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = "---\ntitle: T\n---\n# Top\n\nintro para ^p1\n\n> [!private]\n> secret one\n\n## A\n\n- item a ^a1\n  - nested\n- item b\n\ntext under a ^t1\n\n## B\n\n> [!private]\n> secret two\n\nlast para\n";

    #[test]
    fn units_attach_private_tails_and_span_subtrees() {
        let units = units_of(SRC);
        let handles: Vec<&str> = units.iter().map(|u| u.handle.as_str()).collect();
        assert_eq!(handles, vec!["b0", "^p1", "b3", "^a1", "b6", "^t1", "b8", "b10"], "{handles:?}");
        assert_eq!(units[1].private_tail.len(), 1, "secret one travels with the intro paragraph");
        assert_eq!(units[6].private_tail.len(), 1, "secret two travels with heading B");
        assert!(SRC[units[3].span.clone()].contains("- nested"), "a list item spans its subtree");
    }

    #[test]
    fn restructure_keeps_ids_and_private_blocks_and_collects_leftovers() {
        let outline = vec![
            OutlineSection { heading: Some("B".into()), level: 2, blocks: vec!["^t1".into(), "b10".into()] },
            OutlineSection { heading: Some("New".into()), level: 2, blocks: vec!["^a1".into()] },
        ];
        let r = restructure(SRC, &outline, &["b6".to_string()]).unwrap();
        let want = "---\ntitle: T\n---\n## B\n\n> [!private]\n> secret two\n\ntext under a ^t1\n\nlast para\n\n## New\n\n- item a ^a1\n  - nested\n\n## Unassigned\n\n# Top\n\nintro para ^p1\n\n> [!private]\n> secret one\n\n## A\n";
        assert_eq!(r.text, want);
        assert_eq!(r.unassigned, vec!["b0", "^p1", "b3"]);
        assert!(crate::privacy::private_blocks_preserved(SRC, &r.text));
        assert!(restructure(SRC, &outline, &["^t1".to_string()]).is_err(), "placed and removed at once");
        assert!(restructure(SRC, &[OutlineSection { heading: None, level: 2, blocks: vec!["^zz".into()] }], &[]).is_err());
    }

    #[test]
    fn extract_units_leaves_private_tails_behind() {
        let (source, moved, ids) = extract_units(SRC, &["^p1".into(), "^a1".into()]).unwrap();
        assert_eq!(moved, "intro para ^p1\n\n- item a ^a1\n  - nested");
        assert_eq!(ids, vec!["p1", "a1"]);
        assert!(source.contains("> [!private]\n> secret one") && !source.contains("intro para"), "{source}");
        assert!(crate::privacy::private_blocks_preserved(SRC, &source));
        assert!(extract_units(SRC, &["b2".into()]).is_err(), "a private block cannot be moved");
    }

    #[test]
    fn block_diff_tells_moved_changed_added_removed_apart() {
        let old = "# T\n\npara one about cats\n\n- a\n- b\n\nlast line here\n\ngone for good\n";
        let new = "# T\n\nlast line here\n\n- a\n- b\n\npara one about cats and dogs\n\nbrand new block\n";
        let d = block_diff(old, new);
        assert_eq!((d.moved, d.changed, d.added, d.removed), (1, 1, 1, 1), "{d:?}");
        assert!(d.items.iter().any(|i| i.kind == "moved" && i.text == "last line here"));
        assert!(d.items.iter().any(|i| i.kind == "changed" && i.old.as_deref() == Some("para one about cats")));
        assert!(d.items.iter().any(|i| i.kind == "removed" && i.text == "gone for good"));
    }
}
