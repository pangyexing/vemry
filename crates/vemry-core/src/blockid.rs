//! Block ids: what counts as one, which blocks may carry one, and how a new
//! one is minted. vemry writes an id only on demand — when the user
//! references, embeds or copies a link to a block that has none — and only
//! for that block; positional ids never reach disk.

use std::collections::HashSet;
use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::block::{Block, BlockKind};
use crate::locate::{find_block, find_block_at, heading_text};
use crate::parser::parse_note;
use crate::{Result, VemryError};

/// Which block a caller means: by local id (an explicit `^id` or a
/// positional `bN` from a query hit, with the hit's content as a guard
/// against the note having changed since), or by a byte offset (the cursor).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum BlockSelector {
    Id {
        local_id: String,
        #[serde(default)]
        guard: Option<String>,
    },
    Offset {
        offset: usize,
    },
}

/// What a link to the selected block should point at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanTarget {
    /// A heading needs no id: `[[Note#text]]`.
    Heading { text: String },
    /// `[[Note#^id]]` — minted now when `insert` is present.
    Block { id: String },
}

/// The single edit that gives the block its id: ` ^id` appended at `offset`
/// (a byte offset into the source, the end of the block's last line).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BlockIdInsert {
    pub offset: usize,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BlockIdPlan {
    pub target: PlanTarget,
    /// `None` when the block already has an explicit id (or is a heading):
    /// nothing to write.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert: Option<BlockIdInsert>,
}

/// The one placement rule for a block id, for disk and editor buffer alike:
/// the caller hands over the text it holds and gets back at most one insert.
/// The editor never computes where an id goes.
pub fn plan_block_id(source: &str, selector: &BlockSelector) -> Result<BlockIdPlan> {
    let parsed = parse_note(source);
    let blocks = &parsed.blocks;
    let idx = match selector {
        BlockSelector::Id { local_id, guard } => {
            let by_id = find_block(blocks, local_id);
            match (by_id, guard) {
                (Some(i), Some(g)) if !blocks[i].explicit_id && blocks[i].content != *g => unique_by_content(blocks, g)
                    .ok_or_else(|| VemryError::BlockChanged { local_id: local_id.clone() })?,
                (Some(i), _) => i,
                (None, Some(g)) => unique_by_content(blocks, g)
                    .ok_or_else(|| VemryError::BlockNotFound { local_id: local_id.clone() })?,
                (None, None) => return Err(VemryError::BlockNotFound { local_id: local_id.clone() }),
            }
        }
        BlockSelector::Offset { offset } => {
            find_block_at(blocks, *offset).ok_or_else(|| VemryError::BlockNotFound { local_id: format!("@{offset}") })?
        }
    };
    let block = &blocks[idx];
    if matches!(block.kind, BlockKind::Heading { .. }) {
        return Ok(BlockIdPlan { target: PlanTarget::Heading { text: heading_text(block).to_string() }, insert: None });
    }
    if block.explicit_id {
        return Ok(BlockIdPlan { target: PlanTarget::Block { id: block.local_id.clone() }, insert: None });
    }
    if !is_anchorable(block, source) {
        return Err(VemryError::BlockNotIdable);
    }
    let taken: HashSet<&str> = blocks.iter().filter(|b| b.explicit_id).map(|b| b.local_id.as_str()).collect();
    let id = mint_block_id(&taken);
    Ok(BlockIdPlan {
        insert: Some(BlockIdInsert { offset: block.span.end, text: format!(" ^{id}") }),
        target: PlanTarget::Block { id },
    })
}

/// The one block whose content equals `guard`, if exactly one does — the
/// fallback when a positional id has shifted because the note was edited.
fn unique_by_content(blocks: &[Block], guard: &str) -> Option<usize> {
    let mut hits = blocks.iter().enumerate().filter(|(_, b)| b.content == guard).map(|(i, _)| i);
    let first = hits.next()?;
    if hits.next().is_some() {
        return None;
    }
    Some(first)
}

/// Length of a minted id (`a-z0-9`, so 36^6 ≈ 2·10⁹ per note).
pub const MINTED_LEN: usize = 6;

/// The parser's acceptance rule for `^id`: ASCII letters, digits and `-`.
pub fn is_valid_block_id(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// A fresh random id not in `taken`. Random rather than content-derived so
/// two identical paragraphs never collide and an edit never changes an id.
pub fn mint_block_id(taken: &HashSet<&str>) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut attempt: u64 = 0;
    loop {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.write_u64(attempt);
        let mut n = h.finish();
        let mut id = String::with_capacity(MINTED_LEN);
        for _ in 0..MINTED_LEN {
            id.push(ALPHABET[(n % 36) as usize] as char);
            n /= 36;
        }
        if !taken.contains(id.as_str()) {
            return id;
        }
        attempt += 1;
    }
}

/// Whether ` ^id` may be appended to this block's last line without breaking
/// the construct: not a fenced code block (text after the closing fence
/// would reopen it), not a table row, not a thematic break.
pub fn is_anchorable(b: &Block, src: &str) -> bool {
    match &b.kind {
        BlockKind::CodeBlock { .. } => false,
        BlockKind::Paragraph => {
            let raw = src.get(b.span.clone()).unwrap_or("").trim_start();
            !raw.starts_with('|') && !crate::parser::is_thematic_break(raw.trim())
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_note;

    #[test]
    fn validity_matches_the_parser_rule() {
        assert!(is_valid_block_id("abc-1"));
        assert!(!is_valid_block_id(""));
        assert!(!is_valid_block_id("my_id"));
        assert!(!is_valid_block_id("中文"));
    }

    #[test]
    fn minted_ids_are_valid_unique_and_avoid_taken() {
        let mut seen: HashSet<String> = HashSet::new();
        for _ in 0..200 {
            let taken: HashSet<&str> = seen.iter().map(|s| s.as_str()).collect();
            let id = mint_block_id(&taken);
            assert_eq!(id.len(), MINTED_LEN);
            assert!(id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()), "{id}");
            assert!(is_valid_block_id(&id));
            assert!(seen.insert(id));
        }
    }

    fn plan(src: &str, sel: BlockSelector) -> Result<BlockIdPlan> {
        plan_block_id(src, &sel)
    }

    fn by_id(id: &str, guard: Option<&str>) -> BlockSelector {
        BlockSelector::Id { local_id: id.into(), guard: guard.map(String::from) }
    }

    #[test]
    fn plan_mints_at_the_end_of_the_block_and_respects_existing_ids() {
        let src = "# Title ^h1\n\npara one\n\n- item\n  cont\n\n> [!claim] c ^c1\n\n- [ ] task\n";
        // heading: a text target, nothing to write
        assert_eq!(
            plan(src, by_id("h1", None)).unwrap(),
            BlockIdPlan { target: PlanTarget::Heading { text: "Title".into() }, insert: None }
        );
        // explicit id: returned as is
        assert_eq!(plan(src, by_id("c1", None)).unwrap().insert, None);
        // positional: minted, inserted after the block's last line
        let p = plan(src, by_id("b1", Some("para one"))).unwrap();
        let ins = p.insert.unwrap();
        assert_eq!(ins.offset, src.find("para one").unwrap() + "para one".len());
        assert!(ins.text.starts_with(" ^") && ins.text.len() == 2 + MINTED_LEN);
        let multi = plan(src, by_id("b2", None)).unwrap().insert.unwrap();
        assert_eq!(&src[..multi.offset], "# Title ^h1\n\npara one\n\n- item\n  cont");
        let task = plan(src, by_id("b4", None)).unwrap().insert.unwrap();
        assert_eq!(&src[..task.offset], src.trim_end());
        // by offset: the cursor inside the paragraph
        let off = plan(src, BlockSelector::Offset { offset: src.find("one").unwrap() }).unwrap();
        assert!(off.insert.is_some());
        assert!(plan(src, BlockSelector::Offset { offset: 12 }).is_err(), "blank line between blocks");
    }

    #[test]
    fn plan_guard_falls_back_to_unique_content_or_refuses() {
        let src = "moved here\n\npara one\n";
        // b1 is now "para one" but the guard says "moved here" → the unique block with that content
        let p = plan(src, by_id("b1", Some("moved here"))).unwrap().insert.unwrap();
        assert_eq!(p.offset, "moved here".len());
        assert!(matches!(plan(src, by_id("b1", Some("gone"))), Err(VemryError::BlockChanged { .. })));
        assert!(matches!(plan(src, by_id("b9", None)), Err(VemryError::BlockNotFound { .. })));
        assert!(matches!(plan(src, by_id("b9", Some("para one"))).unwrap().target, PlanTarget::Block { .. }));
        let dup = "same\n\nsame\n";
        assert!(matches!(plan(dup, by_id("b7", Some("same"))), Err(VemryError::BlockNotFound { .. })));
    }

    #[test]
    fn plan_refuses_code_tables_rules_and_uniqueness_holds() {
        let src = "```\nx\n```\n\n| a |\n|---|\n\n---\n\npara ^aaaaaa\n";
        assert!(matches!(plan(src, by_id("b0", None)), Err(VemryError::BlockNotIdable)));
        assert!(matches!(plan(src, by_id("b1", None)), Err(VemryError::BlockNotIdable)));
        assert!(matches!(plan(src, by_id("b2", None)), Err(VemryError::BlockNotIdable)));
    }

    #[test]
    fn anchorable_excludes_code_tables_and_rules() {
        let src = "para\n\n```\ncode\n```\n\n| a | b |\n|---|---|\n\n---\n\n- item\n\n# H\n";
        let blocks = parse_note(src).blocks;
        let kinds: Vec<bool> = blocks.iter().map(|b| is_anchorable(b, src)).collect();
        assert_eq!(kinds, vec![true, false, false, false, true, true]);
    }
}
