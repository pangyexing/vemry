//! Block ids: what counts as one, which blocks may carry one, and how a new
//! one is minted. vemry writes an id only on demand — when the user
//! references, embeds or copies a link to a block that has none — and only
//! for that block; positional ids never reach disk.

use std::collections::HashSet;
use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::block::{Block, BlockKind};

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
            !raw.starts_with('|') && !is_rule(raw)
        }
        _ => true,
    }
}

fn is_rule(t: &str) -> bool {
    let t = t.trim();
    let first = match t.chars().next() {
        Some(c) if matches!(c, '-' | '*' | '_') => c,
        _ => return false,
    };
    t.chars().filter(|c| !matches!(c, ' ' | '\t')).all(|c| c == first)
        && t.chars().filter(|&c| c == first).count() >= 3
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

    #[test]
    fn anchorable_excludes_code_tables_and_rules() {
        let src = "para\n\n```\ncode\n```\n\n| a | b |\n|---|---|\n\n---\n\n- item\n\n# H\n";
        let blocks = parse_note(src).blocks;
        let kinds: Vec<bool> = blocks.iter().map(|b| is_anchorable(b, src)).collect();
        assert_eq!(kinds, vec![true, false, false, false, true, true]);
    }
}
