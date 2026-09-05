//! Block-level note parser.
//!
//! A deliberate line-based scanner instead of a full Markdown AST: everything
//! vemry extracts (frontmatter, headings, list items, checkbox tasks, callouts,
//! code fences, `[[wikilinks]]`, `#tags`, `^block-ids`) is a line-level or
//! inline construct, and none of it is CommonMark — an AST parser would still
//! need all of this custom code on top.

use chrono::NaiveDate;

use crate::block::{Block, BlockKind, Field, TaskState, WikiLink};
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

/// Split the body (from `body_offset`) into blocks. Public so that callers
/// holding a standalone slice of Markdown (a selection, a pasted block) can
/// segment it with the same rules as a whole note.
pub fn parse_blocks(raw: &str, body_offset: usize) -> Vec<Block> {
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
                fields: vec![],
                explicit_id: false,
                modified_ms: 0,
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
            // A private block contributes nothing to the tag table, the link
            // graph or the field index: what is invisible to the agent must not
            // leak through aggregates either
            let private = ctype == "private";
            let (links, tags) = if private { (vec![], vec![]) } else { scan_inline(&raw[span.clone()], span.start) };
            let fields = if private { vec![] } else { scan_fields(&raw[span.clone()], span.start) };
            let kind = match ctype.as_str() {
                "private" => BlockKind::Private,
                // A decision is a claim about what to do: evidence supports it the same
                // way, the Claims view lists it, and audit_links checks its supports
                "claim" | "decision" => BlockKind::Claim,
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
                fields,
                explicit_id: false,
                modified_ms: 0,
            };
            assign_id(&mut b, blocks.len());
            blocks.push(b);
            i = end_line + 1;
            continue;
        }

        // List item: the marker line plus continuation lines indented deeper
        // than the marker. A new marker at any depth starts the next block;
        // a lazy continuation (not indented) is left to the paragraph rule.
        if let Some((indent, ordered, text_off)) = parse_list_marker(line) {
            flush_para!(i);
            let mut content = line[text_off..].trim().to_string();
            let mut end_line = i;
            let mut j = i + 1;
            while j < n {
                let l = lines[j].1;
                if l.trim().is_empty() || leading_ws(l) <= indent || starts_new_block(l) {
                    break;
                }
                content.push('\n');
                content.push_str(l.trim());
                end_line = j;
                j += 1;
            }
            let span = start..lines[end_line].0 + lines[end_line].1.len();
            let mut b = make_block(raw, BlockKind::ListItem { indent, ordered }, span);
            b.content = content;
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
    let fields = scan_fields(&raw[span.clone()], span.start);
    Block {
        local_id: String::new(),
        kind,
        content: raw[span.clone()].to_string(),
        span,
        tags,
        links,
        fields,
        explicit_id: false,
        modified_ms: 0,
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
    if let (_, Some(id)) = crate::blockid::split_anchor(last_line) {
        b.local_id = id.to_string();
        b.explicit_id = true;
        return;
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

/// `- item` / `* item` / `+ item` / `12. item` / `12) item` at any indentation.
/// Returns (indent, ordered, byte offset of the item text within `line`).
/// A thematic break spelled with spaced markers (`- - -`, `* * *`) is not an
/// item. Checkbox items match here too; the task rule runs first and wins.
pub(crate) fn parse_list_marker(line: &str) -> Option<(usize, bool, usize)> {
    let indent = leading_ws(line);
    let rest = &line[indent..];
    if rest.starts_with("- ") || rest.starts_with("* ") || rest.starts_with("+ ") {
        if is_thematic_break(rest) {
            return None;
        }
        return Some((indent, false, indent + 2));
    }
    let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
    if (1..=9).contains(&digits) {
        let after = &rest[digits..];
        if after.starts_with(". ") || after.starts_with(") ") {
            return Some((indent, true, indent + digits + 2));
        }
    }
    None
}

/// Number of leading space/tab characters (a tab counts as one).
pub(crate) fn leading_ws(line: &str) -> usize {
    line.chars().take_while(|c| matches!(c, ' ' | '\t')).count()
}

/// `---`, `***`, `- - -`: three or more of one marker with only spaces between.
pub(crate) fn is_thematic_break(trimmed: &str) -> bool {
    let mut marker = None;
    let mut n = 0;
    for c in trimmed.chars() {
        if matches!(c, ' ' | '\t') {
            continue;
        }
        match marker {
            None if matches!(c, '-' | '*' | '_') => {
                marker = Some(c);
                n += 1;
            }
            Some(m) if c == m => n += 1,
            _ => return false,
        }
    }
    n >= 3
}

/// A line that opens a block of its own and therefore ends a list item's
/// continuation: fence, heading, task, callout or another list marker.
fn starts_new_block(line: &str) -> bool {
    let t = line.trim_start();
    parse_fence_open(t).is_some()
        || parse_heading(t).is_some()
        || parse_task_line(line).is_some()
        || parse_callout_open(t).is_some()
        || parse_list_marker(line).is_some()
}

/// `> [!claim] optional title` — returns (lowercased type, rest of line after the
/// optional fold marker, trimmed). Shared with provenance's clip-credit reader.
pub(crate) fn parse_callout_open(trimmed: &str) -> Option<(String, &str)> {
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
                    let mut link = WikiLink::parse(inner, base + i..base + i + end + 4);
                    link.embed = i > 0 && bytes[i - 1] == b'!';
                    links.push(link);
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

/// A field key: letters, digits, `_`, `-`; not all digits (so `12::30` is a
/// time, not a field).
pub fn is_field_key(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(is_key_char)
        && !s.chars().all(|c| c.is_ascii_digit())
}

fn is_key_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-')
}

/// Scan a raw slice for inline fields. Three forms, one boundary rule (the
/// `::` must be followed by whitespace, a digit or the end of the line):
/// a whole line `key:: value` (after the block prefix — indent, `>`, list
/// marker, checkbox); `[key:: value]` / `(key:: value)` anywhere in a
/// line; a bare `key:: token` inside a line, whose value is one token.
/// Not scanned inside backtick code spans or `[[wikilinks]]`. Spans are
/// absolute (offset by `base`).
pub fn scan_fields(slice: &str, base: usize) -> Vec<Field> {
    let mut fields = Vec::new();
    if !slice.contains("::") {
        return fields;
    }
    let mut in_code = false;
    for (line_start, line) in LineIter::new(slice) {
        let bytes = line.as_bytes();
        let content_start = block_content_start(line);
        let mut i = 0;
        while i < bytes.len() {
            let ch = match line[i..].chars().next() {
                Some(c) => c,
                None => break,
            };
            if ch == '`' {
                in_code = !in_code;
                i += 1;
                continue;
            }
            if in_code {
                i += ch.len_utf8();
                continue;
            }
            if line[i..].starts_with("[[") {
                match line[i + 2..].find("]]") {
                    Some(end) => {
                        i += end + 4;
                        continue;
                    }
                    None => break,
                }
            }
            if !line[i..].starts_with("::") {
                i += ch.len_utf8();
                continue;
            }
            let after = &line[i + 2..];
            let after_ok =
                after.is_empty() || after.starts_with(|c: char| c.is_whitespace() || c.is_ascii_digit());
            let key_start = line[..i]
                .char_indices()
                .rev()
                .take_while(|(_, c)| is_key_char(*c))
                .last()
                .map(|(p, _)| p)
                .unwrap_or(i);
            let key = &line[key_start..i];
            if !after_ok || !is_field_key(key) {
                i += 2;
                continue;
            }
            let before = line[..key_start].chars().next_back();
            let found = match before {
                Some(open @ ('[' | '(')) => {
                    let close = if open == '[' { ']' } else { ')' };
                    let rest = after.trim_start();
                    match rest.find(close) {
                        Some(c) if !rest[..c].contains(open) => {
                            let close_at = i + 2 + (after.len() - rest.len()) + c;
                            Some((key_start - 1, close_at + 1, rest[..c].trim().to_string()))
                        }
                        _ => None,
                    }
                }
                _ if key_start == content_start => {
                    let value = crate::blockid::strip_anchor(after.trim());
                    Some((key_start, line.trim_end().len(), value.to_string()))
                }
                Some(c) if c.is_whitespace() => {
                    let rest = after.trim_start();
                    let token_len = rest.find(char::is_whitespace).unwrap_or(rest.len());
                    let value_end = i + 2 + (after.len() - rest.len()) + token_len;
                    Some((key_start, value_end, rest[..token_len].to_string()))
                }
                _ => None,
            };
            match found {
                Some((from, to, value)) if !value.is_empty() => {
                    fields.push(Field {
                        key: key.to_string(),
                        value,
                        span: base + line_start + from..base + line_start + to,
                    });
                    i = to.max(i + 2);
                }
                _ => i += 2,
            }
        }
    }
    fields
}

/// Byte offset where a line's own text starts: past indentation, quote
/// markers, a list marker and a checkbox.
fn block_content_start(line: &str) -> usize {
    let mut i = leading_ws(line);
    loop {
        let rest = &line[i..];
        if let Some(r) = rest.strip_prefix('>') {
            i += 1 + leading_ws(r);
            continue;
        }
        if let Some((_, _, off)) = parse_list_marker(rest) {
            i += off;
            let after = &line[i..];
            let mut chars = after.chars();
            if chars.next() == Some('[') {
                chars.next();
                if chars.next() == Some(']') {
                    let box_len = after.len() - chars.as_str().len();
                    i += box_len + leading_ws(&after[box_len..]);
                }
            }
            continue;
        }
        return i;
    }
}

/// Set or clear the `disabled` key of a frontmatter file (a block type, a skill),
/// every other line kept as it is (the file is the user's, comments and order
/// included) — the one write point for enabling and disabling; how `disabled` is
/// read lives in each file kind's parser. Disabling rewrites
/// an existing top-level `disabled:` line or adds one before the closing `---`
/// (a file with no frontmatter gets one); enabling drops the line, false being
/// the default. A frontmatter too damaged to parse takes the line without
/// effect — the caller checks `parse_block_type` and reports it.
pub fn set_frontmatter_disabled(raw: &str, disabled: bool) -> String {
    let eol = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let line = format!("disabled: true{eol}");
    let mut lines: Vec<&str> = raw.split_inclusive('\n').collect();
    // The boundary is `extract_frontmatter`'s, which owns the rule: the opener is
    // `---`, the closer `---` or YAML's `...`. Reading a `...` close as ordinary text would
    // prepend a second frontmatter block ahead of the file's own.
    let opens = |l: &str| l.trim_end() == "---";
    let closes = |l: &str| matches!(l.trim_end(), "---" | "...");
    let close = match lines.first() {
        Some(first) if opens(first) => lines.iter().skip(1).position(|l| closes(l)).map(|i| i + 1),
        _ => None,
    };
    let Some(close) = close else {
        return if disabled { format!("---{eol}{line}---{eol}{raw}") } else { raw.to_string() };
    };
    let existing = lines[1..close].iter().position(|l| l.starts_with("disabled:")).map(|i| i + 1);
    match (existing, disabled) {
        (Some(i), true) => lines[i] = &line,
        (Some(i), false) => {
            lines.remove(i);
        }
        (None, true) => lines.insert(close, &line),
        (None, false) => {}
    }
    lines.concat()
}

/// A frontmatter value the agent may set: one scalar, or a list of scalars.
#[derive(Clone, Debug, PartialEq)]
pub enum FrontmatterValue {
    Scalar(String),
    List(Vec<String>),
}

/// A top-level frontmatter key the agent may write: letters, digits, `_` and `-`
/// (YAML's plain keys); `private` and `vemry` are the app's own and refused —
/// a write must not change a note's privacy or its provenance record.
pub fn is_writable_frontmatter_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && !matches!(key, "private" | "vemry")
}

/// A scalar as a YAML value: bare when it reads back as itself (a URL with its
/// colon included — `https://x` is a plain scalar, since the colon is not
/// followed by a space), double-quoted (with `"` and `\\` escaped) when it holds
/// something YAML would read as structure, a comment or a type — the round trip
/// through `extract_frontmatter` must give the string back unchanged.
fn yaml_scalar(v: &str) -> String {
    let lower = v.to_ascii_lowercase();
    let plain = !v.is_empty()
        && !v.starts_with(['-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%', '@', '`', ' '])
        && !v.ends_with([':', ' '])
        && !v.contains(": ")
        && !v.contains(" #")
        && !v.contains(['\n', '\r', '\t', '\\'])
        && !matches!(lower.as_str(), "true" | "false" | "null" | "~" | "yes" | "no" | "on" | "off")
        && v.parse::<f64>().is_err();
    if plain {
        v.to_string()
    } else {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// Set, replace or remove (`None`) one top-level frontmatter key, every other line
/// kept as it is (the same boundary rule as `set_frontmatter_disabled`: opener
/// `---`, closer `---` or `...`). A note with no frontmatter gets one when a value
/// is set; the continuation lines of a list or block value (indented, or `- ` items)
/// are replaced together with their key. Returns `None` when the key is not one the
/// agent may write.
pub fn set_frontmatter_key(raw: &str, key: &str, value: Option<&FrontmatterValue>) -> Option<String> {
    if !is_writable_frontmatter_key(key) {
        return None;
    }
    let eol = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let rendered: Option<String> = value.map(|v| match v {
        FrontmatterValue::Scalar(s) => format!("{key}: {}{eol}", yaml_scalar(s)),
        FrontmatterValue::List(items) if items.is_empty() => format!("{key}: []{eol}"),
        FrontmatterValue::List(items) => {
            let mut out = format!("{key}:{eol}");
            for it in items {
                out.push_str(&format!("  - {}{eol}", yaml_scalar(it)));
            }
            out
        }
    });
    let lines: Vec<&str> = raw.split_inclusive('\n').collect();
    let opens = |l: &str| l.trim_end() == "---";
    let closes = |l: &str| matches!(l.trim_end(), "---" | "...");
    let close = match lines.first() {
        Some(first) if opens(first) => lines.iter().skip(1).position(|l| closes(l)).map(|i| i + 1),
        _ => None,
    };
    let Some(close) = close else {
        return Some(match rendered {
            Some(r) => format!("---{eol}{r}---{eol}{raw}"),
            None => raw.to_string(),
        });
    };
    let prefix = format!("{key}:");
    let start = lines[1..close]
        .iter()
        .position(|l| l.starts_with(&prefix) && l[prefix.len()..].starts_with([' ', '\n', '\r']) || l.trim_end() == prefix.trim_end())
        .map(|i| i + 1);
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 4);
    match start {
        Some(i) => {
            // the key's continuation lines: indented, or list items, up to the next top-level key or the closer
            let mut end = i + 1;
            while end < close && (lines[end].starts_with([' ', '\t']) || lines[end].starts_with("- ")) {
                end += 1;
            }
            out.extend(lines[..i].iter().map(|l| l.to_string()));
            if let Some(r) = rendered {
                out.push(r);
            }
            out.extend(lines[end..].iter().map(|l| l.to_string()));
        }
        None => {
            out.extend(lines[..close].iter().map(|l| l.to_string()));
            if let Some(r) = rendered {
                out.push(r);
            }
            out.extend(lines[close..].iter().map(|l| l.to_string()));
        }
    }
    Some(out.concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_frontmatter_key_sets_replaces_removes_and_refuses() {
        let raw = "---\ntitle: T\nsources:\n  - https://a\n  - https://b\nstatus: draft\n---\nbody\n";
        let set = set_frontmatter_key(raw, "status", Some(&FrontmatterValue::Scalar("reviewing".into()))).unwrap();
        assert!(set.contains("status: reviewing\n") && !set.contains("draft"));
        let (fm, _) = extract_frontmatter(&set);
        assert_eq!(fm.extra.get("status").and_then(|v| v.as_str()), Some("reviewing"));
        let list = set_frontmatter_key(raw, "sources", Some(&FrontmatterValue::List(vec!["https://c".into()]))).unwrap();
        assert!(list.contains("sources:\n  - https://c\nstatus: draft\n") && !list.contains("https://a"), "{list}");
        let removed = set_frontmatter_key(raw, "sources", None).unwrap();
        assert_eq!(removed, "---\ntitle: T\nstatus: draft\n---\nbody\n");
        let added = set_frontmatter_key("no frontmatter\n", "question", Some(&FrontmatterValue::Scalar("why: because #1".into()))).unwrap();
        assert!(added.starts_with("---\nquestion: \"why: because #1\"\n---\nno frontmatter\n"), "{added}");
        let (fm, _) = extract_frontmatter(&added);
        assert_eq!(fm.extra.get("question").and_then(|v| v.as_str()), Some("why: because #1"));
        assert!(set_frontmatter_key(raw, "private", Some(&FrontmatterValue::Scalar("true".into()))).is_none());
        assert!(set_frontmatter_key(raw, "vemry", None).is_none());
        assert!(set_frontmatter_key(raw, "bad key", None).is_none());
        // a value YAML would read as a number or a bool is quoted so it reads back as text
        let n = set_frontmatter_key(raw, "code", Some(&FrontmatterValue::Scalar("007".into()))).unwrap();
        assert!(n.contains("code: \"007\"\n"));
    }

    #[test]
    fn list_items_are_blocks() {
        let src = "- a\n* b\n+ c\n1. d\n2) e\n";
        let blocks = parse_note(src).blocks;
        assert_eq!(blocks.len(), 5);
        let ordered: Vec<bool> = blocks
            .iter()
            .map(|b| match b.kind {
                BlockKind::ListItem { ordered, .. } => ordered,
                _ => panic!("not a list item: {:?}", b.kind),
            })
            .collect();
        assert_eq!(ordered, vec![false, false, false, true, true]);
        assert_eq!(blocks[3].content, "d");
        assert_eq!(&src[blocks[4].span.clone()], "2) e");
    }

    #[test]
    fn nested_items_keep_indent_with_spaces_and_tabs() {
        let src = "- a\n  - a1\n\t- a2\n- b\n";
        let blocks = parse_note(src).blocks;
        let indents: Vec<usize> = blocks
            .iter()
            .map(|b| match b.kind {
                BlockKind::ListItem { indent, .. } => indent,
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(indents, vec![0, 2, 1, 0]);
    }

    #[test]
    fn item_continuation_lines_join_and_lazy_lines_do_not() {
        let src = "- a\n  more\n  still\nlazy\n";
        let blocks = parse_note(src).blocks;
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].content, "a\nmore\nstill");
        assert_eq!(&src[blocks[0].span.clone()], "- a\n  more\n  still");
        assert!(matches!(blocks[1].kind, BlockKind::Paragraph));
        assert_eq!(blocks[1].content, "lazy");
    }

    #[test]
    fn item_ends_at_marker_task_heading_and_fence() {
        let src = "- a\n  - [ ] t\n- b\n  # not a heading? it is\n- c\n  ```\n  code\n  ```\n";
        let blocks = parse_note(src).blocks;
        let kinds: Vec<&str> = blocks
            .iter()
            .map(|b| match b.kind {
                BlockKind::ListItem { .. } => "item",
                BlockKind::Task { .. } => "task",
                BlockKind::Heading { .. } => "heading",
                BlockKind::CodeBlock { .. } => "code",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["item", "task", "item", "heading", "item", "code"]);
    }

    #[test]
    fn ordered_checkbox_is_a_list_item_and_tasks_stay_tasks() {
        let blocks = parse_note("1. [ ] x\n- [ ] y\n").blocks;
        assert!(matches!(blocks[0].kind, BlockKind::ListItem { ordered: true, .. }));
        assert_eq!(blocks[0].content, "[ ] x");
        assert!(matches!(blocks[1].kind, BlockKind::Task { .. }));
    }

    #[test]
    fn list_item_block_id_on_its_last_line() {
        let blocks = parse_note("- a ^one\n- b\n  cont ^two\n- c ^three\n  tail\n").blocks;
        assert_eq!((blocks[0].local_id.as_str(), blocks[0].explicit_id), ("one", true));
        assert_eq!((blocks[1].local_id.as_str(), blocks[1].explicit_id), ("two", true));
        // an id that is not on the item's last line stays positional
        assert_eq!((blocks[2].local_id.as_str(), blocks[2].explicit_id), ("b2", false));
    }

    #[test]
    fn spaced_thematic_break_is_not_an_item() {
        let blocks = parse_note("- - -\n\n* * *\n\n---\n").blocks;
        assert_eq!(blocks.len(), 3);
        assert!(blocks.iter().all(|b| matches!(b.kind, BlockKind::Paragraph)));
    }

    #[test]
    fn embed_flag_set_by_bang_and_ignored_in_code() {
        let (links, _) = scan_inline("see ![[img.png]] and [[Note]] `![[x]]`", 0);
        assert_eq!(links.len(), 2);
        assert!(links[0].embed);
        assert_eq!(links[0].target, "img.png");
        assert_eq!(links[0].span, 5..16);
        assert!(!links[1].embed);
    }

    fn fields_of(src: &str) -> Vec<(String, String, String)> {
        parse_note(src)
            .blocks
            .iter()
            .flat_map(|b| b.fields.iter().map(|f| (f.key.clone(), f.value.clone(), src[f.span.clone()].to_string())))
            .collect()
    }

    #[test]
    fn fields_three_forms_with_exact_spans() {
        let src = "author:: Ursula K. Le Guin\n- 再读 [rating:: 5] (status:: done)\n- [ ] 写规格 due::2026-08-10 #work\n> 引用\n> status:: reading\n";
        let got = fields_of(src);
        assert_eq!(
            got,
            vec![
                ("author".into(), "Ursula K. Le Guin".into(), "author:: Ursula K. Le Guin".into()),
                ("rating".into(), "5".into(), "[rating:: 5]".into()),
                ("status".into(), "done".into(), "(status:: done)".into()),
                ("due".into(), "2026-08-10".into(), "due::2026-08-10".into()),
                ("status".into(), "reading".into(), "status:: reading".into()),
            ]
        );
    }

    #[test]
    fn task_due_is_both_a_date_and_a_field() {
        let blocks = parse_note("- [ ] x due::2026-08-20\n").blocks;
        assert!(matches!(blocks[0].kind, BlockKind::Task { due: Some(_), .. }));
        assert_eq!(blocks[0].fields[0].key, "due");
    }

    #[test]
    fn field_boundary_rule_and_exclusions() {
        for (src, expect) in [
            ("std::vector\n", 0),
            ("C++::foo\n", 0),
            ("12::30\n", 0),
            ("see http://x\n", 0),
            ("a::1\n", 1),
            ("作者:: 勒古恩\n", 1),
            ("std:: vector\n", 1),
            ("author::\n", 0),
            ("`k:: v`\n", 0),
            ("[[k:: v]]\n", 0),
            ("```\nk:: v\n```\n", 0),
            ("---\nk:: v\n---\nbody\n", 0),
            ("**bold**:: x\n", 0),
            ("[k:: [nested]]\n", 0),
            ("title ^abc\n\nk:: v ^id1\n", 1),
        ] {
            assert_eq!(fields_of(src).len(), expect, "{src:?}");
        }
        let got = fields_of("k:: v ^id1\n");
        assert_eq!(got[0].1, "v");
        let got = fields_of("- [x] done [k:: v] [k:: w]\n");
        assert_eq!(got.iter().map(|f| f.1.as_str()).collect::<Vec<_>>(), vec!["v", "w"]);
    }

    #[test]
    fn private_callout_kind() {
        let src = "> [!Private]- secret #tag [[Link]] k:: v\n> more\nlazy\n\n- item\n  > [!private] nested\n- > [!private] not one\n";
        let blocks = parse_note(src).blocks;
        assert!(matches!(blocks[0].kind, BlockKind::Private));
        assert_eq!(blocks[0].content, "secret #tag [[Link]] k:: v\nmore");
        assert!(blocks[0].tags.is_empty() && blocks[0].links.is_empty() && blocks[0].fields.is_empty());
        // a lazy line without `>` is outside the block
        assert!(matches!(blocks[1].kind, BlockKind::Paragraph));
        assert_eq!(blocks[1].content, "lazy");
        // inside a list item: the item ends and the callout is its own block
        assert!(matches!(blocks[2].kind, BlockKind::ListItem { .. }));
        assert!(matches!(blocks[3].kind, BlockKind::Private));
        // on a marker line it is a list item, not a callout
        assert!(matches!(blocks[4].kind, BlockKind::ListItem { .. }));
    }

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
    #[test]
    fn set_disabled_touches_the_one_frontmatter_line() {
        use crate::types::{parse_block_type, BOOK_TYPE_TEMPLATE};
        let off = set_frontmatter_disabled(BOOK_TYPE_TEMPLATE, true);
        assert!(parse_block_type("book.md", &off).disabled);
        assert!(
            off.ends_with("disabled: true\n---\n{{cursor}} [author:: {{author}}] [status:: {{status}}]\n"),
            "{off}"
        );
        // enabling drops the line and gives the original back; enabling twice is a no-op
        assert_eq!(set_frontmatter_disabled(&off, false), BOOK_TYPE_TEMPLATE);
        assert_eq!(set_frontmatter_disabled(BOOK_TYPE_TEMPLATE, false), BOOK_TYPE_TEMPLATE);
        // an existing line is rewritten where it is, whatever it said
        assert_eq!(
            set_frontmatter_disabled("---\ndisabled: false\nname: x\n---\nbody\n", true),
            "---\ndisabled: true\nname: x\n---\nbody\n"
        );
        // no frontmatter: disabling adds one, enabling changes nothing
        assert_eq!(set_frontmatter_disabled("body\n", true), "---\ndisabled: true\n---\nbody\n");
        assert_eq!(set_frontmatter_disabled("body\n", false), "body\n");
        // CRLF files keep their line endings
        assert_eq!(
            set_frontmatter_disabled("---\r\nname: x\r\n---\r\n", true),
            "---\r\nname: x\r\ndisabled: true\r\n---\r\n"
        );
        // a damaged frontmatter takes the line but the parser cannot read it (the command reports that)
        let bad = set_frontmatter_disabled("---\nname: [unclosed\n---\nbody\n", true);
        assert!(bad.contains("[unclosed\ndisabled: true\n---"));
        assert!(!parse_block_type("odd.md", &bad).disabled);
    }

}
