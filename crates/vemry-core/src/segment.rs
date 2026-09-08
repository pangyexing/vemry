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
    /// the "jump to the source" anchor. Empty for a PDF's segments.
    pub span: Range<usize>,
    /// The 1-based page a PDF segment came from; None for Markdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
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
            page: None,
        });
    }
    Segmented { frontmatter, segments }
}

/// Cut a PDF's extracted pages into segments. Extraction gives lines, not
/// paragraphs, so paragraphs are rebuilt from two signals the text carries:
/// a justified line is padded with double spaces and the last line of a paragraph
/// is not (on a page where most lines are padded, an unpadded line that ends a
/// sentence ends the paragraph); elsewhere a line clearly shorter than the column
/// that ends a sentence does. A hyphen broken across lines is mended, a blank line
/// separates paragraphs unless the page is double-spaced throughout, a short line
/// standing alone without a full stop is a heading, and a bare number (the page
/// number) is dropped. Every segment carries its page, which is what the rows
/// align on.
pub fn segment_pages(pages: &[String]) -> Vec<Segment> {
    let mut out: Vec<Segment> = vec![];
    for (i, page) in pages.iter().enumerate() {
        let page_no = i as u32 + 1;
        let raw_lines: Vec<&str> = page.lines().map(str::trim).collect();
        let nonblank = raw_lines.iter().filter(|l| !l.is_empty()).count().max(1);
        let blank = raw_lines.len() - raw_lines.iter().filter(|l| !l.is_empty()).count();
        // Double-spaced output: a blank line after nearly every line means nothing
        let blanks_mean_nothing = blank * 10 >= nonblank * 4;
        let padded = raw_lines.iter().filter(|l| l.contains("  ")).count();
        // Most lines padded: the padding is justification, and its absence marks a paragraph's last line
        let justified_page = padded * 10 >= nonblank * 3;
        let lines: Vec<(String, bool)> = raw_lines
            .iter()
            .map(|l| (l.split_whitespace().collect::<Vec<_>>().join(" "), l.contains("  ")))
            .collect();
        let column = lines.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(0).max(40);
        let mut paragraphs: Vec<Vec<&str>> = vec![];
        let mut cur: Vec<&str> = vec![];
        for (li, (line, padded)) in lines.iter().enumerate() {
            if line.is_empty() {
                if !cur.is_empty() && !blanks_mean_nothing {
                    paragraphs.push(std::mem::take(&mut cur));
                }
                continue;
            }
            let short = line.chars().count() + 6 <= column;
            let last_line = if justified_page { !padded } else { short };
            // A line opening a paragraph that is no sentence, with a sentence starting
            // right under it: a heading set without a blank line around it
            if cur.is_empty() && last_line && looks_like_heading(line) && next_opens_sentence(&lines, li) {
                paragraphs.push(vec![line]);
                continue;
            }
            cur.push(line);
            if ends_sentence(line) && last_line {
                paragraphs.push(std::mem::take(&mut cur));
            }
        }
        if !cur.is_empty() {
            paragraphs.push(cur);
        }
        for para in paragraphs {
            let text = join_lines(&para);
            if text.is_empty() || is_page_number(&text) {
                continue;
            }
            let heading = para.len() == 1 && looks_like_heading(&text);
            let (kind, prefix) = if heading { (SegKind::Heading { level: 2 }, "## ".to_string()) } else { (SegKind::Para, String::new()) };
            out.push(Segment {
                id: out.len() as u32,
                kind,
                prefix,
                text,
                translate: true,
                private: false,
                span: 0..0,
                page: Some(page_no),
            });
        }
    }
    out
}

/// The next non-blank line (if any) starts like a sentence: a capital, a digit, a quote.
fn next_opens_sentence(lines: &[(String, bool)], at: usize) -> bool {
    match lines[at + 1..].iter().find(|(l, _)| !l.is_empty()) {
        None => true,
        Some((next, _)) => next.chars().next().is_some_and(|c| c.is_uppercase() || c.is_ascii_digit() || matches!(c, '"' | '“' | '(')),
    }
}

fn ends_sentence(line: &str) -> bool {
    line.trim_end_matches(['"', '\'', ')', ']', '”', '’']).ends_with(['.', '!', '?', ':', '。', '！', '？'])
}

/// A line of its own, short, not a sentence, mostly not punctuation — a title or a
/// section heading. A numbered heading (`3.2 Attention`) ends in a word, not a stop.
fn looks_like_heading(text: &str) -> bool {
    let n = text.chars().count();
    if n == 0 || n > 90 || ends_sentence(text) || text.ends_with(',') {
        return false;
    }
    let words = text.split_whitespace().count();
    words <= 14 && text.chars().any(|c| c.is_alphabetic())
}

fn is_page_number(text: &str) -> bool {
    let t = text.trim();
    !t.is_empty() && t.len() <= 4 && t.chars().all(|c| c.is_ascii_digit())
}

/// Lines of one paragraph → one string: a space between lines, none after a
/// hyphen that ends a line. The hyphen itself stays: `key-` + `value` and
/// `trans-` + `former` cannot be told apart without a dictionary, and a kept
/// hyphen reads right in the first case and passably in the second, where a
/// dropped one would glue the compound.
fn join_lines(lines: &[&str]) -> String {
    let mut s = String::new();
    for (i, l) in lines.iter().enumerate() {
        if i > 0 {
            let broke_word = s.ends_with('-')
                && l.chars().next().is_some_and(|c| c.is_lowercase())
                && s.chars().rev().nth(1).is_some_and(|c| c.is_alphabetic());
            if !broke_word {
                s.push(' ');
            }
        }
        s.push_str(l);
    }
    s.trim().to_string()
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
    fn pdf_pages_rebuild_paragraphs_headings_and_drop_page_numbers() {
        let p1 = "Scaled Dot-Product Attention\n\nAn attention function can be described as mapping a query and a set of\nkey-value pairs to an output, where the query, keys, values, and out-\nput are all vectors. The output is a weighted sum.\nWe call it scaled attention. The input consists of queries and keys of\ndimension dk, and values of dimension dv.\n1".to_string();
        let p2 = "Why scale\nThe two most commonly used attention functions are additive attention,\nand dot-product attention.\n2".to_string();
        let v = segment_pages(&[p1, p2]);
        let kinds: Vec<(u32, &SegKind, &str)> = v.iter().map(|s| (s.page.unwrap(), &s.kind, s.text.as_str())).collect();
        assert_eq!(kinds[0], (1, &SegKind::Heading { level: 2 }, "Scaled Dot-Product Attention"));
        assert_eq!(v[0].prefix, "## ");
        assert_eq!(kinds[1].1, &SegKind::Para);
        assert!(kinds[1].2.starts_with("An attention function") && kinds[1].2.contains("and out-put are all vectors. The output is a weighted sum."), "{}", kinds[1].2);
        assert!(kinds[2].2.starts_with("We call it scaled attention"), "{}", kinds[2].2);
        assert_eq!(kinds[3], (2, &SegKind::Heading { level: 2 }, "Why scale"));
        assert_eq!(v.len(), 5, "{kinds:?}");
        assert!(v.iter().all(|s| s.translate && !s.private && s.span.is_empty()));
        assert_eq!(v.iter().map(|s| s.id).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn double_spaced_justified_extraction_rebuilds_paragraphs() {
        // What pdf-extract gives for a justified page: a blank line after every line,
        // double spaces inside the full lines, single spaces in a paragraph's last line
        let p = "Scaled Dot-Product Attention\n\nAn  attention  function  can  be  described  as  mapping  a  query  and  a  set  of  key-\n\nvalue  pairs  to  an  output,  where  the  query,  keys,  values,  and  output  are  all\n\nvectors. The output is a weighted sum.\n\nWe  call  our  particular  attention  \"Scaled  Dot-Product  Attention\".  The  input\n\nconsists of queries and keys of dimension dk.\n\nWhy scale the dot products\n\nThe  two  most  commonly  used  attention  functions  are  additive  attention,  and\n\ndot-product attention.\n".to_string();
        let v = segment_pages(&[p]);
        let got: Vec<(&SegKind, &str)> = v.iter().map(|s| (&s.kind, s.text.as_str())).collect();
        assert_eq!(got.len(), 5, "{got:?}");
        assert_eq!(got[0], (&SegKind::Heading { level: 2 }, "Scaled Dot-Product Attention"));
        assert_eq!(got[1], (&SegKind::Para, "An attention function can be described as mapping a query and a set of key-value pairs to an output, where the query, keys, values, and output are all vectors. The output is a weighted sum."));
        assert_eq!(got[2].0, &SegKind::Para);
        assert!(got[2].1.starts_with("We call our particular attention") && got[2].1.ends_with("dimension dk."), "{}", got[2].1);
        assert_eq!(got[3], (&SegKind::Heading { level: 2 }, "Why scale the dot products"));
        assert_eq!(got[4], (&SegKind::Para, "The two most commonly used attention functions are additive attention, and dot-product attention."));
    }

    #[test]
    fn an_image_with_a_caption_line_is_prose() {
        let v = segs("![f](a.png)\nFigure 1: caption\n");
        assert_eq!(v[0].kind, SegKind::Para);
        assert!(v[0].translate);
    }
}
