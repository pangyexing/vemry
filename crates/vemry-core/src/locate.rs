//! One definition of "which block" and "which section" for every caller.
//! The insert landing rule, reference resolution (`[[Note#Heading]]`,
//! `[[Note#^id]]`) and block-id minting all locate blocks through these
//! functions, so they can never disagree about where a section ends or which
//! block an id names.

use std::ops::Range;

use crate::block::{Block, BlockKind};
use crate::parser::leading_ws;

/// A heading's text as written, minus a trailing ` ^id` — an id is an anchor,
/// not part of the title, so `[[Note#Title]]` still finds `# Title ^h1`.
pub fn heading_text(b: &Block) -> &str {
    let t = b.content.trim();
    if b.explicit_id {
        if let Some(stripped) = t.strip_suffix(&format!(" ^{}", b.local_id)) {
            return stripped.trim_end();
        }
    }
    t
}

/// Index of the heading block named `text`: an exact (trimmed) match first,
/// then a case-insensitive one; within each pass the first in document
/// order wins.
pub fn find_heading(blocks: &[Block], text: &str) -> Option<usize> {
    let want = text.trim();
    let is_heading = |b: &Block| matches!(b.kind, BlockKind::Heading { .. });
    if let Some(i) = blocks.iter().position(|b| is_heading(b) && heading_text(b) == want) {
        return Some(i);
    }
    let lower = want.to_lowercase();
    blocks
        .iter()
        .position(|b| is_heading(b) && heading_text(b).to_lowercase() == lower)
}

/// The section of the heading at `heading_idx`: from its line start to the
/// last non-blank byte before the next heading of the same or a higher
/// level (else end of text).
pub fn section_span_at(blocks: &[Block], src: &str, heading_idx: usize) -> Range<usize> {
    let head = &blocks[heading_idx];
    let level = match head.kind {
        BlockKind::Heading { level } => level,
        _ => u8::MAX,
    };
    let next = blocks[heading_idx + 1..]
        .iter()
        .find_map(|b| match &b.kind {
            BlockKind::Heading { level: l } if *l <= level => Some(b.span.start),
            _ => None,
        })
        .unwrap_or(src.len());
    head.span.start..src[..next].trim_end().len()
}

/// `section_span_at` for the heading named `heading` (see `find_heading`).
pub fn section_span(blocks: &[Block], src: &str, heading: &str) -> Option<Range<usize>> {
    find_heading(blocks, heading).map(|i| section_span_at(blocks, src, i))
}

/// The block whose local id is `local_id`. An explicit `^id` wins over a
/// positional `bN` that happens to be spelled the same.
pub fn find_block<'a>(blocks: &'a [Block], local_id: &str) -> Option<&'a Block> {
    blocks
        .iter()
        .find(|b| b.explicit_id && b.local_id == local_id)
        .or_else(|| blocks.iter().find(|b| !b.explicit_id && b.local_id == local_id))
}

/// Index of the block whose span contains `offset` (a byte offset into the
/// raw text; both ends inclusive). `None` on a blank line between blocks.
pub fn find_block_at(blocks: &[Block], offset: usize) -> Option<usize> {
    blocks.iter().position(|b| b.span.start <= offset && offset <= b.span.end)
}

/// A list item or task together with its children: every following block
/// indented deeper than the item, up to the first that is not. Any other
/// kind returns its own span. This is what a reference or embed of a list
/// item denotes, even though the item's own block covers only its lines.
pub fn item_subtree_span(blocks: &[Block], src: &str, idx: usize) -> Range<usize> {
    let b = &blocks[idx];
    let indent = match &b.kind {
        BlockKind::ListItem { indent, .. } => *indent,
        BlockKind::Task { .. } => leading_ws(&src[b.span.start..]),
        _ => return b.span.clone(),
    };
    let mut end = b.span.end;
    for nb in &blocks[idx + 1..] {
        let nested = match &nb.kind {
            BlockKind::ListItem { indent, .. } => *indent,
            _ => leading_ws(&src[nb.span.start..]),
        };
        if nested > indent {
            end = nb.span.end;
        } else {
            break;
        }
    }
    b.span.start..end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_note;

    fn blocks(src: &str) -> Vec<Block> {
        parse_note(src).blocks
    }

    #[test]
    fn heading_lookup_exact_then_case_insensitive_and_strips_anchor() {
        let src = "## inbox\n一\n\n## Inbox ^h1\n二\n";
        let b = blocks(src);
        assert_eq!(find_heading(&b, "Inbox"), Some(2));
        assert_eq!(find_heading(&b, "INBOX"), Some(0));
        assert_eq!(heading_text(&b[2]), "Inbox");
        assert_eq!(find_heading(&b, "nope"), None);
    }

    #[test]
    fn section_span_mirrors_the_insert_rule() {
        let src = "## 大\n### 小\n正文\n\n## 下一个\nx";
        let b = blocks(src);
        let r = section_span(&b, src, "大").unwrap();
        assert_eq!(&src[r], "## 大\n### 小\n正文");
        let r = section_span(&b, src, "下一个").unwrap();
        assert_eq!(&src[r], "## 下一个\nx");
        assert_eq!(section_span(&blocks("```\n## 假\n```\n"), "```\n## 假\n```\n", "假"), None);
    }

    #[test]
    fn find_block_prefers_explicit_ids_and_locates_by_offset() {
        let src = "first\n\nsecond ^b0\n";
        let b = blocks(src);
        assert_eq!(find_block(&b, "b0").unwrap().content, "second ^b0");
        assert!(find_block(&b, "b1").is_none());
        assert_eq!(find_block_at(&b, 0), Some(0));
        assert_eq!(find_block_at(&b, 6), None);
        assert_eq!(find_block_at(&b, 8), Some(1));
    }

    #[test]
    fn item_subtree_includes_children_only() {
        let src = "- a\n  - a1\n    - a1a\n  - a2\n- b\n\npara\n";
        let b = blocks(src);
        assert_eq!(&src[item_subtree_span(&b, src, 0)], "- a\n  - a1\n    - a1a\n  - a2");
        assert_eq!(&src[item_subtree_span(&b, src, 1)], "  - a1\n    - a1a");
        assert_eq!(&src[item_subtree_span(&b, src, 4)], "- b");
        assert_eq!(&src[item_subtree_span(&b, src, 5)], "para");
    }

    #[test]
    fn task_subtree_uses_its_indentation() {
        let src = "- [ ] t\n  - sub\n- [ ] u\n";
        let b = blocks(src);
        assert_eq!(&src[item_subtree_span(&b, src, 0)], "- [ ] t\n  - sub");
    }
}
