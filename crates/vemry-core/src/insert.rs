//! Positioned insertion — the write primitive behind "clip/capture into a
//! heading's section" (and the normalized tail append both flows share).
//!
//! A heading's section runs from its line to the next heading of the same or
//! a higher level (else EOF). Content lands after the section's last
//! non-blank line, with separators rebuilt so the result is stable markdown:
//! block payloads get one blank line on each side, a single-line list item
//! packs tightly against a preceding list item (or its own heading line),
//! and exactly one blank line separates the payload from what follows.
//!
//! The frontend mirrors these rules in `src/lib/insert.ts` for the
//! open-note buffer path (a disk write there would be clobbered by the
//! stale buffer's autosave) — change the rules in both places together.

use crate::block::Block;
use crate::locate::section_span;
use crate::parser::{parse_heading, parse_list_marker};

/// Byte offset where content lands inside `heading`'s section: right after
/// the section's last non-blank character. `None` = heading not present
/// (the caller falls back to [`content_end_offset`] and reports it).
/// Which heading and where its section ends is `locate::section_span`'s
/// definition (exact match first, then case-insensitive; the first in
/// document order wins).
pub fn section_insert_offset(blocks: &[Block], src: &str, heading: &str) -> Option<usize> {
    section_span(blocks, src, heading).map(|r| r.end)
}

/// Byte offset after the last non-blank character of the whole note.
pub fn content_end_offset(src: &str) -> usize {
    src.trim_end().len()
}

/// Splice `text` in at `offset` (a content-end offset from the fns above).
/// The blank run that followed the offset is consumed and separators are
/// rebuilt per the module rules; the result always ends with exactly one
/// trailing newline before any remaining content, and one at EOF.
pub fn insert_at(src: &str, offset: usize, text: &str) -> String {
    let payload = text.trim_matches(['\n', '\r']);
    let before = &src[..offset];
    let rest = &src[offset..];
    let after = &rest[blank_run_len(rest)..];

    let mut out = String::with_capacity(src.len() + payload.len() + 4);
    out.push_str(before);
    if !before.is_empty() {
        out.push('\n');
        let last_line = before.rsplit('\n').next().unwrap_or(before);
        if !packs_tight(last_line, payload) {
            out.push('\n');
        }
    }
    out.push_str(payload);
    out.push('\n');
    if !after.is_empty() {
        out.push('\n');
        out.push_str(after);
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// No blank line between the previous line and the payload: right under the
/// section's own heading line, or a list item joining a list.
fn packs_tight(prev_line: &str, payload: &str) -> bool {
    parse_heading(prev_line.trim_start()).is_some()
        || (!payload.contains('\n') && is_list_line(payload) && is_list_line(prev_line))
}

fn is_list_line(line: &str) -> bool {
    parse_list_marker(line).is_some()
}

/// Length of the leading run of blank lines (`rest` starts right after a
/// non-blank character, so the run begins with that line's tail).
fn blank_run_len(rest: &str) -> usize {
    let mut len = 0;
    for line in rest.split_inclusive('\n') {
        if !line.trim().is_empty() {
            break;
        }
        len += line.len();
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_note;

    fn offset(src: &str, heading: &str) -> Option<usize> {
        section_insert_offset(&parse_note(src).blocks, src, heading)
    }

    fn insert(src: &str, heading: &str, text: &str) -> String {
        insert_at(src, offset(src, heading).unwrap(), text)
    }

    #[test]
    fn quote_block_lands_before_next_heading() {
        let src = "# 阅读\n\n## 收集箱\n- 旧速记\n\n## 已整理\n内容\n";
        assert_eq!(
            insert(src, "收集箱", "> 引用\n> — 来源"),
            "# 阅读\n\n## 收集箱\n- 旧速记\n\n> 引用\n> — 来源\n\n## 已整理\n内容\n"
        );
    }

    #[test]
    fn list_payload_packs_against_list() {
        let src = "## 收集箱\n- 一\n\n## 下\n";
        assert_eq!(
            insert(src, "收集箱", "- 二 *(2026-08-18 10:00)*"),
            "## 收集箱\n- 一\n- 二 *(2026-08-18 10:00)*\n\n## 下\n"
        );
    }

    #[test]
    fn empty_section_packs_after_heading_line() {
        let src = "## 空\n## 下\n";
        assert_eq!(insert(src, "空", "- x"), "## 空\n- x\n\n## 下\n");
    }

    #[test]
    fn nested_subheadings_stay_inside_section() {
        let src = "## 大\n### 小\n正文\n## 下一个\n";
        assert_eq!(
            insert(src, "大", "> q"),
            "## 大\n### 小\n正文\n\n> q\n\n## 下一个\n"
        );
    }

    #[test]
    fn section_at_eof() {
        let src = "## 尾\n内容";
        assert_eq!(insert(src, "尾", "> q"), "## 尾\n内容\n\n> q\n");
    }

    #[test]
    fn heading_inside_fence_is_not_a_section() {
        let src = "```\n## 假标题\n```\n";
        assert_eq!(offset(src, "假标题"), None);
    }

    #[test]
    fn missing_heading_is_none() {
        assert_eq!(offset("# a\n", "b"), None);
    }

    #[test]
    fn heading_matches_case_insensitively_and_without_its_anchor() {
        assert_eq!(insert("## Inbox ^h1\n- a\n", "inbox", "- b"), "## Inbox ^h1\n- a\n- b\n");
        // An exact match beats a case-insensitive one earlier in the note
        let src = "## inbox\n一\n\n## Inbox\n二\n";
        assert_eq!(insert(src, "Inbox", "- x"), "## inbox\n一\n\n## Inbox\n二\n\n- x\n");
    }

    #[test]
    fn duplicate_headings_resolve_to_first() {
        let src = "## 重\n一\n\n## 重\n二\n";
        assert_eq!(insert(src, "重", "- x"), "## 重\n一\n\n- x\n\n## 重\n二\n");
    }

    #[test]
    fn tail_insert_normalizes_separators() {
        // Exactly one blank line between a block payload and what precedes it
        assert_eq!(insert_at("段落\n", content_end_offset("段落\n"), "> q"), "段落\n\n> q\n");
        // A list packs against a list (fixes the asymmetric newline the clip/memo append had)
        assert_eq!(insert_at("- a\n", content_end_offset("- a\n"), "- b"), "- a\n- b\n");
        // Empty file / note that does not exist
        assert_eq!(insert_at("", 0, "- a"), "- a\n");
        // Trailing blank lines collapse; the file ends in exactly one newline
        assert_eq!(insert_at("x\n\n\n", content_end_offset("x\n\n\n"), "- a"), "x\n\n- a\n");
    }

    #[test]
    fn frontmatter_only_note_separates_with_blank() {
        let src = "---\ntitle: x\n---\n";
        assert_eq!(
            insert_at(src, content_end_offset(src), "- a"),
            "---\ntitle: x\n---\n\n- a\n"
        );
    }
}
