//! Privacy zone: paths the AI agent must never see. Rules live in
//! `.vemry/privacy` (gitignore syntax, one glob per line) and travel with the
//! vault — privacy is a property of the vault, not of a session or model
//! endpoint. The per-note half (`private: true` frontmatter) lives in the
//! index; this module compiles the glob half.
//!
//! Read semantics are "does not exist", never "no permission": callers filter
//! private paths out of results instead of reporting them as forbidden,
//! because an error would leak that the path exists.
//!
//! The block-level half: a `> [!private]` callout is a privacy zone the size
//! of one block. Every agent-visible read goes through the redaction
//! functions below (`Vault::*_for_agent` name them so the choke points can
//! be grepped); the block is removed outright, with no placeholder — the
//! same "does not exist" rule as a path.

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::fs;
use std::ops::Range;
use std::path::Path;
use unicode_normalization::UnicodeNormalization;

use crate::block::{Block, BlockKind};

/// Vault-relative path of the rules file. The file itself is inside `.vemry`
/// and therefore always private (hidden segments are built-in rules).
pub const PRIVACY_FILE: &str = ".vemry/privacy";

pub struct PrivacyScope {
    matcher: Gitignore,
    rules: Vec<String>,
}

impl PrivacyScope {
    /// Load `<root>/.vemry/privacy`. Missing file means no user rules; bad
    /// glob lines are skipped — privacy must never make the vault unopenable.
    pub fn load(root: &Path) -> PrivacyScope {
        match fs::read_to_string(root.join(PRIVACY_FILE)) {
            Ok(content) => Self::from_lines(content.lines()),
            Err(_) => Self::from_lines(std::iter::empty::<&str>()),
        }
    }

    pub fn from_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> PrivacyScope {
        let mut builder = GitignoreBuilder::new("");
        let _ = builder.case_insensitive(true); // APFS opens `SECRET/` for a `secret/` rule
        let mut rules = Vec::new();
        for line in lines {
            let line: String = line.nfc().collect(); // NoteId paths are NFC
            let _ = builder.add_line(None, &line); // bad globs skipped
            if !line.trim().is_empty() {
                rules.push(line);
            }
        }
        let matcher = builder.build().unwrap_or_else(|_| Gitignore::empty());
        PrivacyScope { matcher, rules }
    }

    /// Raw rule lines as stored (comments included), for UI display.
    pub fn rules(&self) -> &[String] {
        &self.rules
    }

    /// Whether a vault-relative path is in the privacy zone. `rel` must be
    /// `/`-separated and NFC-normalized (`NoteId` already is). Any path with
    /// a dot-leading segment is private by construction — that covers
    /// `.vemry/privacy` itself, session logs and `.obsidian`.
    pub fn matches_rel(&self, rel: &str, is_dir: bool) -> bool {
        if rel.split('/').any(|seg| seg.starts_with('.')) {
            return true;
        }
        // `_or_any_parents`: a `secret/` rule must catch `secret/deep/a.md`.
        self.matcher.matched_path_or_any_parents(rel, is_dir).is_ignore()
    }
}

/// A note's text with its private blocks removed, plus the byte ranges of
/// the raw text that were cut (ascending, disjoint, whole lines).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Redacted {
    pub text: String,
    pub removed: Vec<Range<usize>>,
}

impl Redacted {
    pub fn count(&self) -> usize {
        self.removed.len()
    }
}

pub fn has_private_blocks(blocks: &[Block]) -> bool {
    blocks.iter().any(|b| matches!(b.kind, BlockKind::Private))
}

/// The byte ranges to cut for the private blocks of `raw`: each block's
/// span widened to whole lines including the line break, plus one adjacent
/// blank line — the one after it, or the one before when it ends the text —
/// so the neighbours keep exactly the separation they had.
pub fn private_spans(raw: &str, blocks: &[Block]) -> Vec<Range<usize>> {
    let mut spans: Vec<Range<usize>> = Vec::new();
    for b in blocks.iter().filter(|b| matches!(b.kind, BlockKind::Private)) {
        let mut start = raw[..b.span.start].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let mut end = b.span.end.min(raw.len());
        end += line_break_len(&raw[end..]);
        let blank_after = blank_line_len(&raw[end..]);
        if blank_after > 0 {
            end += blank_after;
        } else if end >= raw.len() {
            let before = &raw[..start];
            if before.ends_with("\n\n") {
                start -= 1;
            } else if before.ends_with("\r\n\r\n") {
                start -= 2;
            }
        }
        spans.push(start..end);
    }
    spans
}

fn line_break_len(rest: &str) -> usize {
    if rest.starts_with("\r\n") {
        2
    } else if rest.starts_with('\n') {
        1
    } else {
        0
    }
}

/// Length of a leading blank line (whitespace and its line break), 0 if the
/// next line has content or the text ends.
fn blank_line_len(rest: &str) -> usize {
    let line_end = rest.find('\n').map(|i| i + 1).unwrap_or(rest.len());
    if line_end == 0 {
        return 0;
    }
    if rest[..line_end].trim().is_empty() {
        line_end
    } else {
        0
    }
}

/// `raw` without its private blocks — no placeholder, nothing marks the
/// place. What the agent reads.
pub fn redact_private_blocks(raw: &str, blocks: &[Block]) -> Redacted {
    let removed = private_spans(raw, blocks);
    let mut text = String::with_capacity(raw.len());
    let mut at = 0;
    for r in &removed {
        text.push_str(&raw[at..r.start]);
        at = r.end;
    }
    text.push_str(&raw[at..]);
    Redacted { text, removed }
}

/// `raw` with every byte of its private blocks turned into a space except
/// the line breaks, so offsets and line numbers stay those of the raw text.
/// What a line-accurate re-match (search) runs over.
pub fn blank_private_blocks(raw: &str, blocks: &[Block]) -> String {
    let mut bytes = raw.as_bytes().to_vec();
    for r in private_spans(raw, blocks) {
        for byte in &mut bytes[r] {
            if *byte != b'\n' && *byte != b'\r' {
                *byte = b' ';
            }
        }
    }
    // only ASCII bytes were substituted, so the text is still valid UTF-8
    String::from_utf8(bytes).unwrap_or_else(|_| raw.to_string())
}

/// An edit the model made in its view of a note — `redacted.text`, the note
/// as `read_note` returned it — applied to the raw text. `range` and
/// `replacement` are in terms of that view. A private block whose cut point
/// lies strictly inside the replaced span sat among the replaced lines: it is
/// kept, re-inserted right after the replacement on lines of its own (a blank
/// line on each side, blocks in their order). A block at either edge of the
/// span stays where it is — the replaced text starts after a block at its
/// start and ends before a block at its end — and the line break in front of
/// it is restored when the edit took it, so it still begins a line. Whether
/// the surroundings still let every block parse as private is
/// `private_blocks_preserved`'s job.
pub fn apply_redacted_edit(raw: &str, redacted: &Redacted, range: Range<usize>, replacement: &str) -> String {
    let nl = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    // Each removed range's cut point in view coordinates: where the text on
    // both sides of the block meets once the block is gone
    let mut cuts = Vec::with_capacity(redacted.removed.len());
    let mut shift = 0;
    for r in &redacted.removed {
        cuts.push(r.start - shift);
        shift += r.end - r.start;
    }
    let width = |keep: &dyn Fn(usize) -> bool| -> usize {
        redacted.removed.iter().zip(&cuts).filter(|&(_, &c)| keep(c)).map(|(r, _)| r.end - r.start).sum()
    };
    let raw_start = range.start + width(&|c| c <= range.start);
    let raw_end = if range.is_empty() { raw_start } else { range.end + width(&|c| c < range.end) };
    let mut out = String::with_capacity(raw.len() + replacement.len());
    out.push_str(&raw[..raw_start]);
    out.push_str(replacement);
    let mut reinserted = false;
    for (r, &c) in redacted.removed.iter().zip(&cuts) {
        if range.start < c && c < range.end {
            ensure_blank_line(&mut out, nl);
            out.push_str(raw[r.clone()].trim_matches(['\n', '\r']));
            out.push_str(nl);
            reinserted = true;
        }
    }
    let tail = &raw[raw_end..];
    let block_follows = redacted.removed.iter().any(|r| r.start == raw_end);
    if (reinserted && !tail.is_empty() && !tail.starts_with(['\n', '\r']))
        || (block_follows && !out.is_empty() && !out.ends_with('\n'))
    {
        out.push_str(nl);
    }
    out.push_str(tail);
    out
}

/// Makes `out` end in a blank line (unless it is empty), so what follows
/// starts a block of its own.
fn ensure_blank_line(out: &mut String, nl: &str) {
    if out.is_empty() {
        return;
    }
    let blank = format!("{nl}{nl}");
    if out.ends_with(&blank) {
        return;
    }
    if !out.ends_with(nl) {
        out.push_str(nl);
    }
    out.push_str(nl);
}

/// Whether `new` still holds every private block of `old`, byte for byte and
/// still parsed as a private block (as multisets, so a moved block passes).
/// An edit confined to the public text can otherwise swallow a block it was
/// made around: a fence opened above it and never closed, its first line
/// glued onto a quote line before it.
pub fn private_blocks_preserved(old: &str, new: &str) -> bool {
    fn slices(raw: &str) -> Vec<String> {
        let blocks = crate::parser::parse_note(raw).blocks;
        let mut v: Vec<String> = blocks
            .iter()
            .filter(|b| matches!(b.kind, BlockKind::Private))
            .map(|b| raw[b.span.start.min(raw.len())..b.span.end.min(raw.len())].to_string())
            .collect();
        v.sort_unstable();
        v
    }
    slices(old) == slices(new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_note;

    fn redact(src: &str) -> Redacted {
        redact_private_blocks(src, &parse_note(src).blocks)
    }

    #[test]
    fn redaction_removes_blocks_and_one_separator() {
        assert_eq!(redact("a\n\n> [!private] s\n> t\n\nb\n").text, "a\n\nb\n");
        assert_eq!(redact("> [!private] s\n\nb\n").text, "b\n");
        assert_eq!(redact("a\n\n> [!private] s").text, "a\n");
        assert_eq!(redact("a\n\n> [!private] s\n").text, "a\n");
        assert_eq!(redact("- x\n  > [!private] s\n- y\n").text, "- x\n- y\n");
        assert_eq!(redact("---\ntitle: t\n---\n> [!private] only\n").text, "---\ntitle: t\n---\n");
        assert_eq!(redact("a\r\n\r\n> [!private] s\r\n\r\nb\r\n").text, "a\r\n\r\nb\r\n");
        let r = redact("x\n\n> [!private] one\n\n> [!private] two\n\ny\n");
        assert_eq!(r.text, "x\n\ny\n");
        assert_eq!(r.count(), 2);
        assert_eq!(redact("no private here\n").count(), 0);
    }

    #[test]
    fn blanking_keeps_length_and_lines() {
        let src = "a\n> [!private] secret word\nb word\n";
        let blocks = parse_note(src).blocks;
        let blanked = blank_private_blocks(src, &blocks);
        assert_eq!(blanked.len(), src.len());
        assert_eq!(blanked.lines().count(), src.lines().count());
        assert!(!blanked.contains("secret"));
        assert_eq!(blanked.lines().nth(2), Some("b word"));
    }

    fn edit(src: &str, old: &str, new: &str) -> String {
        let r = redact(src);
        let at = r.text.find(old).expect("old_str is in the view");
        apply_redacted_edit(src, &r, at..at + old.len(), new)
    }

    #[test]
    fn edits_made_in_the_view_land_around_the_private_blocks() {
        // no private block: a plain splice
        assert_eq!(edit("a\n\nb\n", "b", "c"), "a\n\nc\n");
        let src = "a\n\n> [!private] s\n\nb\n";
        assert_eq!(redact(src).text, "a\n\nb\n");
        // on either side of the block, the block is untouched
        assert_eq!(edit(src, "a", "A"), "A\n\n> [!private] s\n\nb\n");
        assert_eq!(edit(src, "b", "B"), "a\n\n> [!private] s\n\nB\n");
        // the edit takes the line break before the block: the block still begins a line
        assert_eq!(edit(src, "a\n\n", "a"), "a\n> [!private] s\n\nb\n");
        // a span across the block: the block is kept right after the replacement
        assert_eq!(edit(src, "a\n\nb", "TEST"), "TEST\n\n> [!private] s\n\n");
        assert_eq!(edit(src, "a\n\nb\n", "TEST\n"), "TEST\n\n> [!private] s\n");
        assert_eq!(edit(src, "a\n\nb\n", ""), "> [!private] s\n");
        // content right after the span gets a blank line between it and the block
        assert_eq!(edit("a\n\n> [!private] s\n\nb c\n", "a\n\nb", "X"), "X\n\n> [!private] s\n\n c\n");
        // two blocks keep their order and their separation
        let two = "x\n\n> [!private] one\n\n> [!private] two\n\ny\n";
        assert_eq!(edit(two, "x\n\ny", "Z"), "Z\n\n> [!private] one\n\n> [!private] two\n\n");
        // a block that opens the note: the edit lands after it
        assert_eq!(edit("> [!private] s\n\na\n", "a", "b"), "> [!private] s\n\nb\n");
        // an indented block inside a list keeps its indentation
        assert_eq!(edit("- x\n  > [!private] s\n- y\n", "- x\n- y", "- z"), "- z\n\n  > [!private] s\n\n");
        // CRLF text gets CRLF separators
        assert_eq!(edit("a\r\n\r\n> [!private] s\r\n\r\nb\r\n", "a\r\n\r\nb", "T"), "T\r\n\r\n> [!private] s\r\n\r\n");
        // every result above still holds its blocks
        for (s, o, n) in [(src, "a\n\nb", "TEST"), (src, "a\n\n", "a"), (two, "x\n\ny", "Z")] {
            assert!(private_blocks_preserved(s, &edit(s, o, n)), "{o:?}");
        }
    }

    #[test]
    fn preserved_means_every_private_block_still_parses_as_one() {
        let src = "a\n\n> [!private] s\n\nb\n";
        assert!(private_blocks_preserved(src, src));
        assert!(private_blocks_preserved(src, "b\n\n> [!private] s\n\na\n"), "moved is fine");
        assert!(private_blocks_preserved("no\n", "still no\n"));
        assert!(!private_blocks_preserved(src, "a\n\nb\n"), "gone");
        assert!(!private_blocks_preserved(src, "a> [!private] s\n\nb\n"), "glued onto the line before");
        // (a `> [!private]` line under a plain quote line still opens a private
        // block for the parser, which detects callouts line by line — the
        // index's view is what the agent gets, so that one stays hidden)
        assert!(!private_blocks_preserved(src, "```\na\n\n> [!private] s\n\nb\n"), "inside an unclosed fence");
        assert!(!private_blocks_preserved(src, &edit(src, "a", "```\na")), "the edit helper does not prevent it");
    }

    #[test]
    fn dir_rule_covers_descendants() {
        let s = PrivacyScope::from_lines(["secret/"]);
        assert!(s.matches_rel("secret", true));
        assert!(s.matches_rel("secret/a.md", false));
        assert!(s.matches_rel("secret/deep/b.md", false));
        assert!(!s.matches_rel("public/a.md", false));
        assert!(!s.matches_rel("secretive.md", false));
        // bare name (no trailing slash) works the same, gitignore-style
        let s = PrivacyScope::from_lines(["secret"]);
        assert!(s.matches_rel("secret/a.md", false));
        assert!(s.matches_rel("secret", true));
    }

    #[test]
    fn star_and_negation() {
        let s = PrivacyScope::from_lines(["diary/**", "!diary/Public.md"]);
        assert!(s.matches_rel("diary/2026.md", false));
        assert!(s.matches_rel("diary/deep/x.md", false));
        assert!(!s.matches_rel("diary/Public.md", false));
    }

    #[test]
    fn case_insensitive_matching() {
        let s = PrivacyScope::from_lines(["secret/"]);
        assert!(s.matches_rel("Secret/a.md", false));
        assert!(s.matches_rel("SECRET/A.MD", false));
    }

    #[test]
    fn rules_are_nfc_normalized() {
        // rule written in NFD ("e" + combining acute), path arrives NFC
        let rule = "cafe\u{0301}/";
        let s = PrivacyScope::from_lines([rule]);
        assert!(s.matches_rel("caf\u{e9}/x.md", false));
    }

    #[test]
    fn hidden_segments_always_private() {
        let s = PrivacyScope::from_lines(std::iter::empty::<&str>());
        assert!(s.matches_rel(".vemry/privacy", false));
        assert!(s.matches_rel(".vemry/sessions/x.jsonl", false));
        assert!(s.matches_rel(".obsidian/a.md", false));
        assert!(s.matches_rel("sub/.hidden/a.md", false));
        assert!(!s.matches_rel("normal.md", false));
    }

    #[test]
    fn bad_and_comment_lines_skipped() {
        let s = PrivacyScope::from_lines(["# comment", "", "a**b**c[", "secret/"]);
        assert!(s.matches_rel("secret/a.md", false));
        assert!(!s.matches_rel("comment", false));
    }
}
