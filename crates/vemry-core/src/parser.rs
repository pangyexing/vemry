//! Block-level note parser.
//!
//! A deliberate line-based scanner instead of a full Markdown AST: everything
//! vemry extracts (frontmatter, headings, checkbox tasks, callouts, code
//! fences, `[[wikilinks]]`, `#tags`, `^block-ids`) is a line-level or inline
//! construct, and none of it is CommonMark — an AST parser would still need
//! all of this custom code on top.

use chrono::NaiveDate;

use crate::block::{Block, BlockKind, TaskState, WikiLink};
use crate::note::Frontmatter;

pub struct ParsedNote {
    pub frontmatter: Frontmatter,
    /// Byte offset in the raw text where the body (after frontmatter) starts.
    pub body_offset: usize,
    pub blocks: Vec<Block>,
}

impl ParsedNote {
    /// All tags in the note: frontmatter tags + inline block tags, deduped.
    pub fn all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self.frontmatter.tags.clone();
        for b in &self.blocks {
            for t in &b.tags {
                if !tags.contains(t) {
                    tags.push(t.clone());
                }
            }
        }
        tags
    }
}

pub fn parse_note(raw: &str) -> ParsedNote {
    let (frontmatter, body_offset) = extract_frontmatter(raw);
    let blocks = parse_blocks(raw, body_offset);
    ParsedNote { frontmatter, body_offset, blocks }
}

/// Returns the parsed frontmatter and the byte offset where the body begins.
pub fn extract_frontmatter(raw: &str) -> (Frontmatter, usize) {
    let mut lines = LineIter::new(raw);
    let Some((_, first)) = lines.next() else {
        return (Frontmatter::default(), 0);
    };
    if first.trim_end() != "---" {
        return (Frontmatter::default(), 0);
    }
    let yaml_start = lines.pos;
    let mut yaml_end = None;
    let mut body_start = raw.len();
    while let Some((start, line)) = lines.next() {
        let t = line.trim_end();
        if t == "---" || t == "..." {
            yaml_end = Some(start);
            body_start = lines.pos;
            break;
        }
    }
    let Some(yaml_end) = yaml_end else {
        return (Frontmatter::default(), 0);
    };
    let fm = serde_yaml::from_str::<Frontmatter>(&raw[yaml_start..yaml_end]).unwrap_or_default();
    (fm, body_start)
}

struct LineIter<'a> {
    raw: &'a str,
    pos: usize,
}

impl<'a> LineIter<'a> {
    fn new(raw: &'a str) -> Self {
        LineIter { raw, pos: 0 }
    }
}

impl<'a> Iterator for LineIter<'a> {
    /// Yields (byte offset of line start, line text without trailing newline).
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.raw.len() {
            return None;
        }
        let start = self.pos;
        let rest = &self.raw[start..];
        let (line, advance) = match rest.find('\n') {
            Some(i) => (&rest[..i], i + 1),
            None => (rest, rest.len()),
        };
        self.pos = start + advance;
        Some((start, line.strip_suffix('\r').unwrap_or(line)))
    }
}

fn parse_blocks(raw: &str, body_offset: usize) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let lines: Vec<(usize, &str)> = LineIter { raw, pos: body_offset }.collect();

    let mut i = 0;
    let mut para_start: Option<usize> = None; // index into `lines`
    let n = lines.len();

    macro_rules! flush_para {
        ($end_line:expr) => {
            if let Some(ps) = para_start.take() {
                let end = $end_line; // exclusive line index
                if end > ps {
                    let span_start = lines[ps].0;
                    let last = lines[end - 1];
                    let span_end = last.0 + last.1.len();
                    push_block(&mut blocks, raw, BlockKind::Paragraph, span_start..span_end);
                }
            }
        };
    }

    while i < n {
        let (start, line) = lines[i];
        let trimmed = line.trim_start();

        // Fenced code block
        if let Some((fence_char, fence_len, lang)) = parse_fence_open(trimmed) {
            flush_para!(i);
            let open_line = i;
            let mut close_line = None;
            let mut j = i + 1;
            while j < n {
                let t = lines[j].1.trim_start();
                if is_fence_close(t, fence_char, fence_len) {
                    close_line = Some(j);
                    break;
                }
                j += 1;
            }
            let end_line = close_line.unwrap_or(n - 1);
            let span_start = lines[open_line].0;
            let span_end = lines[end_line].0 + lines[end_line].1.len();
            let content_lines: Vec<&str> = lines[open_line + 1..if close_line.is_some() { end_line } else { n }]
                .iter()
                .map(|(_, l)| *l)
                .collect();
            let mut b = Block {
                local_id: String::new(),
                kind: BlockKind::CodeBlock { lang },
                content: content_lines.join("\n"),
                span: span_start..span_end,
                tags: vec![],
                links: vec![],
                explicit_id: false,
            };
            assign_id(&mut b, blocks.len());
            blocks.push(b);
            i = end_line + 1;
            continue;
        }

        // Heading
        if let Some((level, text)) = parse_heading(trimmed) {
            flush_para!(i);
            let span = start..start + line.len();
            let mut b = make_block(raw, BlockKind::Heading { level }, span);
            b.content = text.to_string();
            assign_id(&mut b, blocks.len());
            blocks.push(b);
            i += 1;
            continue;
        }

        // Task line
        if let Some((state, rest)) = parse_task_line(line) {
            flush_para!(i);
            let span = start..start + line.len();
            let due = parse_date_attr(rest, &["📅", "due::"]);
            let scheduled = parse_date_attr(rest, &["⏳", "scheduled::"]);
            let mut b = make_block(raw, BlockKind::Task { state, due, scheduled }, span);
            b.content = rest.trim().to_string();
            assign_id(&mut b, blocks.len());
            blocks.push(b);
            i += 1;
            continue;
        }

        // Callout (> [!type] ...), consuming subsequent quoted lines
        if let Some((ctype, title_rest)) = parse_callout_open(trimmed) {
            flush_para!(i);
            let open_line = i;
            let mut j = i + 1;
            while j < n && lines[j].1.trim_start().starts_with('>') {
                j += 1;
            }
            let end_line = j - 1;
            let span_start = lines[open_line].0;
            let span_end = lines[end_line].0 + lines[end_line].1.len();
            let mut content = String::new();
            if !title_rest.is_empty() {
                content.push_str(title_rest);
            }
            for (_, l) in &lines[open_line + 1..=end_line] {
                let stripped = l.trim_start().trim_start_matches('>').strip_prefix(' ').unwrap_or_else(|| l.trim_start().trim_start_matches('>'));
                if !content.is_empty() {
                    content.push('\n');
                }
                content.push_str(stripped);
            }
            let span = span_start..span_end;
            let (links, tags) = scan_inline(&raw[span.clone()], span.start);
            let kind = match ctype.as_str() {
                "claim" => BlockKind::Claim,
                "evidence" => {
                    let supports = match raw[span.clone()].find("supports:") {
                        Some(p) => {
                            let abs = span.start + p;
                            links
                                .iter()
                                .filter(|l| l.span.start >= abs)
                                .map(link_full_target)
                                .collect()
                        }
                        None => links.iter().map(link_full_target).collect(),
                    };
                    BlockKind::Evidence { supports }
                }
                _ => BlockKind::Paragraph,
            };
            let mut b = Block {
                local_id: String::new(),
                kind,
                content,
                span,
                tags,
                links,
                explicit_id: false,
            };
            assign_id(&mut b, blocks.len());
            blocks.push(b);
            i = end_line + 1;
            continue;
        }

        // Blank line: paragraph boundary
        if trimmed.is_empty() {
            flush_para!(i);
            i += 1;
            continue;
        }

        // Plain line: accumulate into paragraph
        if para_start.is_none() {
            para_start = Some(i);
        }
        i += 1;
    }
    flush_para!(n);
    blocks
}

fn make_block(raw: &str, kind: BlockKind, span: std::ops::Range<usize>) -> Block {
    let (links, tags) = scan_inline(&raw[span.clone()], span.start);
    Block {
        local_id: String::new(),
        kind,
        content: raw[span.clone()].to_string(),
        span,
        tags,
        links,
        explicit_id: false,
    }
}

fn push_block(blocks: &mut Vec<Block>, raw: &str, kind: BlockKind, span: std::ops::Range<usize>) {
    let mut b = make_block(raw, kind, span);
    assign_id(&mut b, blocks.len());
    blocks.push(b);
}

/// Extract a trailing ` ^id` marker from the block's raw text, else assign a
/// positional id.
fn assign_id(b: &mut Block, index: usize) {
    let last_line = b.content.lines().last().unwrap_or("");
    if let Some(pos) = last_line.rfind(" ^") {
        let candidate = &last_line[pos + 2..];
        if !candidate.is_empty()
            && candidate.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            b.local_id = candidate.to_string();
            b.explicit_id = true;
            return;
        }
    }
    b.local_id = format!("b{index}");
}

fn link_full_target(l: &WikiLink) -> String {
    if let Some(id) = &l.block_ref {
        format!("{}#^{}", l.target, id)
    } else if let Some(h) = &l.heading {
        format!("{}#{}", l.target, h)
    } else {
        l.target.clone()
    }
}

pub(crate) fn parse_fence_open(trimmed: &str) -> Option<(char, usize, String)> {
    for fence_char in ['`', '~'] {
        let count = trimmed.chars().take_while(|&c| c == fence_char).count();
        if count >= 3 {
            let lang = trimmed[count..].split_whitespace().next().unwrap_or("").to_string();
            return Some((fence_char, count, lang));
        }
    }
    None
}

pub(crate) fn is_fence_close(trimmed: &str, fence_char: char, fence_len: usize) -> bool {
    let count = trimmed.chars().take_while(|&c| c == fence_char).count();
    count >= fence_len && trimmed[count..].trim().is_empty()
}

pub(crate) fn parse_heading(trimmed: &str) -> Option<(u8, &str)> {
    let level = trimmed.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&level) {
        let rest = &trimmed[level..];
        if let Some(text) = rest.strip_prefix(' ') {
            return Some((level as u8, text.trim()));
        }
    }
    None
}

/// `- [ ] text` / `* [x] text` / `+ [/] text`, any indentation.
fn parse_task_line(line: &str) -> Option<(TaskState, &str)> {
    let t = line.trim_start();
    let rest = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ "))?;
    let rest = rest.trim_start();
    let mut chars = rest.chars();
    if chars.next() != Some('[') {
        return None;
    }
    let state_char = chars.next()?;
    if chars.next() != Some(']') {
        return None;
    }
    let after = chars.as_str();
    if !after.is_empty() && !after.starts_with(' ') {
        return None;
    }
    let state = match state_char {
        ' ' => TaskState::Todo,
        '/' => TaskState::Doing,
        'x' | 'X' => TaskState::Done,
        _ => TaskState::Todo,
    };
    Some((state, after.trim_start()))
}

/// `> [!claim] optional title` — returns (lowercased type, rest of line).
fn parse_callout_open(trimmed: &str) -> Option<(String, &str)> {
    let after_quote = trimmed.strip_prefix('>')?.trim_start();
    let after_bang = after_quote.strip_prefix("[!")?;
    let close = after_bang.find(']')?;
    let ctype = after_bang[..close].trim().to_lowercase();
    if ctype.is_empty() || ctype.contains(char::is_whitespace) {
        return None;
    }
    let mut rest = &after_bang[close + 1..];
    // optional fold markers
    rest = rest.strip_prefix('+').or_else(|| rest.strip_prefix('-')).unwrap_or(rest);
    Some((ctype, rest.trim()))
}

fn parse_date_attr(text: &str, markers: &[&str]) -> Option<NaiveDate> {
    for marker in markers {
        if let Some(p) = text.find(marker) {
            let after = text[p + marker.len()..].trim_start();
            let token: String = after.chars().take_while(|c| !c.is_whitespace()).collect();
            if let Ok(d) = NaiveDate::parse_from_str(&token, "%Y-%m-%d") {
                return Some(d);
            }
        }
    }
    None
}

/// Scan a raw slice for `[[wikilinks]]` and `#tags`, skipping inline code.
/// Spans are absolute (offset by `base`).
pub fn scan_inline(slice: &str, base: usize) -> (Vec<WikiLink>, Vec<String>) {
    let mut links = Vec::new();
    let mut tags = Vec::new();
    let bytes = slice.as_bytes();
    let mut i = 0;
    let mut in_code = false;
    let mut prev_char: Option<char> = None;

    while i < bytes.len() {
        let ch = match slice[i..].chars().next() {
            Some(c) => c,
            None => break,
        };
        if ch == '`' {
            in_code = !in_code;
            prev_char = Some(ch);
            i += 1;
            continue;
        }
        if !in_code && slice[i..].starts_with("[[") {
            if let Some(end) = slice[i + 2..].find("]]") {
                let inner = &slice[i + 2..i + 2 + end];
                if !inner.contains('\n') && !inner.is_empty() {
                    links.push(WikiLink::parse(inner, base + i..base + i + end + 4));
                    prev_char = Some(']');
                    i += end + 4;
                    continue;
                }
            }
        }
        if !in_code && ch == '#' {
            let boundary_ok = match prev_char {
                None => true,
                Some(p) => !(p.is_alphanumeric() || p == '#' || p == '&'),
            };
            if boundary_ok {
                let tag: String = slice[i + 1..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'))
                    .collect();
                if !tag.is_empty() && !tag.chars().all(|c| c.is_ascii_digit()) {
                    let tag_len = tag.len();
                    if !tags.contains(&tag) {
                        tags.push(tag);
                    }
                    prev_char = Some('#');
                    i += 1 + tag_len;
                    continue;
                }
            }
        }
        prev_char = Some(ch);
        i += ch.len_utf8();
    }
    (links, tags)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_roundtrip_preserves_unknown_fields() {
        let raw = "---\ntitle: Hello\ntags: [a, b]\ncustom_field: 42\nnested:\n  x: 1\n---\nbody text\n";
        let parsed = parse_note(raw);
        assert_eq!(parsed.frontmatter.title.as_deref(), Some("Hello"));
        assert_eq!(parsed.frontmatter.tags, vec!["a", "b"]);
        let yaml = parsed.frontmatter.to_yaml().unwrap();
        assert!(yaml.contains("custom_field: 42"), "unknown field lost: {yaml}");
        assert!(yaml.contains("x: 1"));
        assert_eq!(&raw[parsed.body_offset..], "body text\n");
    }

    #[test]
    fn frontmatter_tags_as_string() {
        let raw = "---\ntags: solo\n---\nx\n";
        let parsed = parse_note(raw);
        assert_eq!(parsed.frontmatter.tags, vec!["solo"]);
    }

    #[test]
    fn no_frontmatter() {
        let parsed = parse_note("just text");
        assert_eq!(parsed.body_offset, 0);
        assert_eq!(parsed.blocks.len(), 1);
    }

    #[test]
    fn wikilink_variants() {
        let (links, _) = scan_inline("see [[Target]] and [[folder/Note|别名]] and [[Ref#Head]] and [[N#^abc]]", 0);
        assert_eq!(links.len(), 4);
        assert_eq!(links[0].target, "Target");
        assert_eq!(links[1].target, "folder/Note");
        assert_eq!(links[1].alias.as_deref(), Some("别名"));
        assert_eq!(links[2].heading.as_deref(), Some("Head"));
        assert_eq!(links[3].block_ref.as_deref(), Some("abc"));
    }

    #[test]
    fn wikilinks_in_code_ignored() {
        let raw = "para with `[[NotALink]]` inline\n\n```\n[[AlsoNot]]\n```\n\n[[Real]]\n";
        let parsed = parse_note(raw);
        let all_links: Vec<_> = parsed.blocks.iter().flat_map(|b| &b.links).collect();
        assert_eq!(all_links.len(), 1);
        assert_eq!(all_links[0].target, "Real");
    }

    #[test]
    fn tags_scanned_with_boundaries() {
        let (_, tags) = scan_inline("text #alpha and #中文标签 but not x#glued nor #123 nor # empty", 0);
        assert_eq!(tags, vec!["alpha", "中文标签"]);
    }

    #[test]
    fn task_parsing_with_dates() {
        let raw = "- [ ] write spec 📅 2026-08-10 #work\n- [/] doing it scheduled::2026-08-07\n- [x] done thing\n";
        let parsed = parse_note(raw);
        assert_eq!(parsed.blocks.len(), 3);
        match &parsed.blocks[0].kind {
            BlockKind::Task { state, due, .. } => {
                assert_eq!(*state, TaskState::Todo);
                assert_eq!(due.unwrap().to_string(), "2026-08-10");
            }
            k => panic!("expected task, got {k:?}"),
        }
        assert_eq!(parsed.blocks[0].tags, vec!["work"]);
        match &parsed.blocks[1].kind {
            BlockKind::Task { state, scheduled, .. } => {
                assert_eq!(*state, TaskState::Doing);
                assert_eq!(scheduled.unwrap().to_string(), "2026-08-07");
            }
            k => panic!("expected task, got {k:?}"),
        }
        match &parsed.blocks[2].kind {
            BlockKind::Task { state, .. } => assert_eq!(*state, TaskState::Done),
            k => panic!("expected task, got {k:?}"),
        }
    }

    #[test]
    fn claim_and_evidence_callouts() {
        let raw = "\
> [!claim] 睡眠影响记忆巩固 ^c1

> [!evidence] Walker 2017 实验显示深睡阶段海马体回放
> supports:[[本笔记#^c1]], [[另一笔记#^c9]]
";
        let parsed = parse_note(raw);
        assert_eq!(parsed.blocks.len(), 2);
        assert_eq!(parsed.blocks[0].kind, BlockKind::Claim);
        assert_eq!(parsed.blocks[0].local_id, "c1");
        assert!(parsed.blocks[0].explicit_id);
        match &parsed.blocks[1].kind {
            BlockKind::Evidence { supports } => {
                assert_eq!(supports, &vec!["本笔记#^c1".to_string(), "另一笔记#^c9".to_string()]);
            }
            k => panic!("expected evidence, got {k:?}"),
        }
    }

    #[test]
    fn generic_callout_is_paragraph() {
        let parsed = parse_note("> [!note] just a note callout\n> second line\n");
        assert_eq!(parsed.blocks.len(), 1);
        assert_eq!(parsed.blocks[0].kind, BlockKind::Paragraph);
        assert!(parsed.blocks[0].content.contains("second line"));
    }

    #[test]
    fn code_block_lang_and_content() {
        let parsed = parse_note("```js\nconsole.log(1)\n```\n");
        assert_eq!(parsed.blocks.len(), 1);
        match &parsed.blocks[0].kind {
            BlockKind::CodeBlock { lang } => assert_eq!(lang, "js"),
            k => panic!("expected code block, got {k:?}"),
        }
        assert_eq!(parsed.blocks[0].content, "console.log(1)");
    }

    #[test]
    fn headings_and_paragraphs() {
        let raw = "# Title\n\npara one line a\nline b\n\n## Sub #tagged\n\npara two [[Link]]\n";
        let parsed = parse_note(raw);
        let kinds: Vec<&BlockKind> = parsed.blocks.iter().map(|b| &b.kind).collect();
        assert!(matches!(kinds[0], BlockKind::Heading { level: 1 }));
        assert!(matches!(kinds[1], BlockKind::Paragraph));
        assert!(matches!(kinds[2], BlockKind::Heading { level: 2 }));
        assert!(matches!(kinds[3], BlockKind::Paragraph));
        assert_eq!(parsed.blocks[2].tags, vec!["tagged"]);
        assert_eq!(parsed.blocks[3].links[0].target, "Link");
        // paragraph spans must slice the raw text exactly (editor mapping relies on this)
        for b in &parsed.blocks {
            if matches!(b.kind, BlockKind::Paragraph) {
                assert_eq!(&raw[b.span.clone()], b.content, "span mismatch for {:?}", b.kind);
            }
        }
    }

    #[test]
    fn paragraph_block_id() {
        let parsed = parse_note("some paragraph text ^myid\n");
        assert_eq!(parsed.blocks[0].local_id, "myid");
        assert!(parsed.blocks[0].explicit_id);
    }

    #[test]
    fn unclosed_fence_swallows_rest() {
        let parsed = parse_note("```\nunclosed\nmore\n");
        assert_eq!(parsed.blocks.len(), 1);
        assert!(matches!(parsed.blocks[0].kind, BlockKind::CodeBlock { .. }));
    }
}
