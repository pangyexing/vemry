//! The translation units of a Markdown document.
//!
//! Side-by-side translation shows a document as rows — one source block on the
//! left, its translation on the right — so the document has to be cut into the
//! blocks the parser already knows, then each block sorted into "what the model
//! translates" and "what is copied through": prose, headings, list items, quotes
//! and tables travel to the model; code, display maths, images, rules and private
//! blocks never leave the app. A block's structural marker (`## `, `- [ ] `, the
//! indentation of a nested item) is split off into `prefix`, so the translation
//! keeps the heading level and the list shape by construction rather than by
//! trusting the model to reproduce them.
//!
//! Pure: no vault, no privacy scope. Whether the whole note is private is the
//! caller's check; a `> [!private]` block inside a visible note is marked here,
//! since it is the parser that recognises it.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::block::BlockKind;
use crate::parser::{extract_frontmatter, parse_blocks, parse_list_marker};
use crate::Frontmatter;

/// What kind of block a segment is. The frontend picks the row's rendering from
/// it; the engine only reads `Segment::translate`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SegKind {
    Heading { level: u8 },
    Para,
    ListItem { ordered: bool, task: bool },
    Quote,
    Table,
    Code,
    Math,
    Image,
    Hr,
    Private,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    /// The position in the document, 0-based; the alignment key of every event.
    pub id: u32,
    pub kind: SegKind,
    /// The structural marker split off the first line: `"## "`, `"- [ ] "`,
    /// `"  1. "`. Empty for prose. Source and translation are both rendered as
    /// `prefix + text`.
    pub prefix: String,
    /// The block's Markdown without its prefix — what the model sees, and what the
    /// left column renders.
    pub text: String,
    /// False for the kinds that are copied through untouched.
    pub translate: bool,
    /// A `> [!private]` block: shown on the left, never sent, never translated.
    pub private: bool,
    /// Byte range of the block in the raw text (frontmatter offset included) —
    /// the "jump to the source" anchor.
    pub span: Range<usize>,
}

/// A document cut into segments, with the frontmatter it carried (the title is
/// the caller's to resolve: frontmatter first, then whatever it knows).
#[derive(Clone, Debug)]
pub struct Segmented {
    pub frontmatter: Frontmatter,
    pub segments: Vec<Segment>,
}

/// Cut `raw` into segments. Frontmatter is consumed and not a segment.
pub fn segment_markdown(raw: &str) -> Segmented {
    let (frontmatter, body_offset) = extract_frontmatter(raw);
    let blocks = parse_blocks(raw, body_offset);
    let mut segments = Vec::with_capacity(blocks.len());
    for b in blocks {
        let span = b.span.start.min(raw.len())..b.span.end.min(raw.len());
        let source = &raw[span.clone()];
        let (kind, prefix, text) = match &b.kind {
            BlockKind::Private => (SegKind::Private, String::new(), source.to_string()),
            BlockKind::CodeBlock { .. } => (SegKind::Code, String::new(), source.to_string()),
            BlockKind::Heading { level } => {
                (SegKind::Heading { level: *level }, format!("{} ", "#".repeat(*level as usize)), b.content.clone())
            }
            BlockKind::ListItem { .. } | BlockKind::Task { .. } => {
                let task = matches!(b.kind, BlockKind::Task { .. });
                let (prefix, text) = split_list_prefix(source, task);
                let ordered = matches!(b.kind, BlockKind::ListItem { ordered: true, .. });
                (SegKind::ListItem { ordered, task }, prefix, text)
            }
            BlockKind::Paragraph | BlockKind::Claim | BlockKind::Evidence { .. } => {
                (classify_paragraph(source), String::new(), source.to_string())
            }
        };
        if text.trim().is_empty() && prefix.is_empty() {
            continue;
        }
        let translate = matches!(
            kind,
            SegKind::Heading { .. } | SegKind::Para | SegKind::ListItem { .. } | SegKind::Quote | SegKind::Table
        );
        segments.push(Segment {
            id: segments.len() as u32,
            private: kind == SegKind::Private,
            kind,
            prefix,
            text,
            translate,
            span,
        });
    }
    Segmented { frontmatter, segments }
}

/// A paragraph-kind block sorted by its first line: display maths, a lone image or
/// embed, a thematic break, a pipe table, a quote (callout or plain), else prose.
fn classify_paragraph(source: &str) -> SegKind {
    let first = source.lines().next().unwrap_or("").trim();
    if first.starts_with("$$") {
        return SegKind::Math;
    }
    if is_thematic_break(first) && source.lines().count() == 1 {
        return SegKind::Hr;
    }
    if is_lone_image(source) {
        return SegKind::Image;
    }
    if first.starts_with('>') {
        return SegKind::Quote;
    }
    if first.starts_with('|') && is_table_separator(source.lines().nth(1).unwrap_or("")) {
        return SegKind::Table;
    }
    SegKind::Para
}

/// `---` / `***` / `___`, three or more, spaces allowed between.
fn is_thematic_break(line: &str) -> bool {
    let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
    compact.len() >= 3 && ["-", "*", "_"].iter().any(|m| compact.chars().all(|c| c.to_string() == *m))
}

/// One line that is only an image or an embed (`![alt](url)`, `![[file]]`), with
/// an optional trailing `^id`.
fn is_lone_image(source: &str) -> bool {
    let mut lines = source.lines();
    let (Some(line), None) = (lines.next(), lines.next()) else { return false };
    let (body, _) = crate::blockid::split_anchor(line.trim());
    let body = body.trim();
    if let Some(inner) = body.strip_prefix("![[") {
        return inner.ends_with("]]") && !inner[..inner.len() - 2].contains("]]");
    }
    if let Some(rest) = body.strip_prefix("![") {
        if let Some(close) = rest.find("](") {
            return rest.ends_with(')') && !rest[close + 2..rest.len() - 1].contains(')');
        }
    }
    false
}

/// The second line of a pipe table: `|---|:--:|` and friends.
fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    if !t.contains('-') {
        return false;
    }
    let inner = t.trim_start_matches('|').trim_end_matches('|');
    !inner.is_empty()
        && inner.split('|').all(|cell| {
            let c = cell.trim();
            !c.is_empty() && c.trim_start_matches(':').trim_end_matches(':').chars().all(|ch| ch == '-')
        })
}

/// Split a list item's first line into its marker (indentation, bullet or number,
/// the checkbox of a task) and the item text; continuation lines follow the text
/// with their indentation dropped, as the parser keeps them.
fn split_list_prefix(source: &str, task: bool) -> (String, String) {
    let mut lines = source.lines();
    let first = lines.next().unwrap_or("");
    let Some((_, _, text_off)) = parse_list_marker(first) else {
        return (String::new(), source.to_string());
    };
    let mut prefix_end = text_off;
    if task {
        // `[ ] ` / `[x] `: the state token plus the space after it
        let rest = &first[text_off..];
        let after_ws = rest.len() - rest.trim_start().len();
        let token = &rest[after_ws..];
        if token.starts_with('[') {
            if let Some(close) = token.find(']') {
                let mut end = after_ws + close + 1;
                if token[close + 1..].starts_with(' ') {
                    end += 1;
                }
                prefix_end = text_off + end;
            }
        }
    }
    let mut text = first[prefix_end..].to_string();
    for l in lines {
        text.push('\n');
        text.push_str(l.trim());
    }
    (first[..prefix_end].to_string(), text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segs(raw: &str) -> Vec<Segment> {
        segment_markdown(raw).segments
    }

    #[test]
    fn frontmatter_is_consumed_and_prose_headings_lists_are_split() {
        let raw = "---\ntitle: Paper\n---\n\n# Intro\n\nA paragraph\nover two lines.\n\n- first\n- [x] done task\n  1. nested\n";
        let s = segment_markdown(raw);
        assert_eq!(s.frontmatter.title.as_deref(), Some("Paper"));
        let v = s.segments;
        assert_eq!(v.len(), 5);
        assert_eq!(v[0].kind, SegKind::Heading { level: 1 });
        assert_eq!((v[0].prefix.as_str(), v[0].text.as_str()), ("# ", "Intro"));
        assert_eq!(v[1].kind, SegKind::Para);
        assert_eq!(v[1].text, "A paragraph\nover two lines.");
        assert_eq!(v[2].kind, SegKind::ListItem { ordered: false, task: false });
        assert_eq!((v[2].prefix.as_str(), v[2].text.as_str()), ("- ", "first"));
        assert_eq!(v[3].kind, SegKind::ListItem { ordered: false, task: true });
        assert_eq!((v[3].prefix.as_str(), v[3].text.as_str()), ("- [x] ", "done task"));
        assert_eq!(v[4].kind, SegKind::ListItem { ordered: true, task: false });
        assert_eq!((v[4].prefix.as_str(), v[4].text.as_str()), ("  1. ", "nested"));
        assert!(v.iter().all(|s| s.translate && !s.private));
        assert_eq!(v.iter().map(|s| s.id).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        // the span points back at the raw text
        assert_eq!(&raw[v[1].span.clone()], "A paragraph\nover two lines.");
    }

    #[test]
    fn copied_through_kinds_never_translate() {
        let raw = "```rust\nfn x() {}\n```\n\n$$\nE = mc^2\n$$\n\n$$a^2$$ (1) ^eq1\n\n![Figure 1](img.png) ^fig1\n\n![[embed.md]]\n\n---\n\n> [!private]\n> secret\n";
        let v = segs(raw);
        let kinds: Vec<&SegKind> = v.iter().map(|s| &s.kind).collect();
        assert_eq!(
            kinds,
            vec![&SegKind::Code, &SegKind::Math, &SegKind::Math, &SegKind::Image, &SegKind::Image, &SegKind::Hr, &SegKind::Private]
        );
        assert!(v.iter().all(|s| !s.translate));
        assert!(v[6].private && !v[5].private);
        assert_eq!(v[0].text, "```rust\nfn x() {}\n```");
    }

    #[test]
    fn tables_and_quotes_translate_as_one_block() {
        let raw = "| a | b |\n|---|:-:|\n| 1 | 2 |\n\n> [!note] Title\n> body\n\n> plain quote\n\n| not a table\n";
        let v = segs(raw);
        assert_eq!(v[0].kind, SegKind::Table);
        assert_eq!(v[0].text, "| a | b |\n|---|:-:|\n| 1 | 2 |");
        assert_eq!(v[1].kind, SegKind::Quote);
        assert_eq!(v[2].kind, SegKind::Quote);
        assert_eq!(v[3].kind, SegKind::Para);
        assert!(v.iter().all(|s| s.translate));
    }

    #[test]
    fn an_image_with_a_caption_line_is_prose() {
        let v = segs("![f](a.png)\nFigure 1: caption\n");
        assert_eq!(v[0].kind, SegKind::Para);
        assert!(v[0].translate);
    }
}
