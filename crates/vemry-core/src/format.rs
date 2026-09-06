//! The Markdown formatter: a deterministic, idempotent pass that normalises how a
//! note is written without changing what it says. Format is code's business, not
//! the model's — the agent's own writes, a clip and a rich-text paste go through
//! here before they land, and the user runs it on a note by command. It never runs
//! on autosave: a user's own text changes only when they ask.
//!
//! What it does by default: one blank line between blocks and none between the
//! items of a list (the editor's seam rule), trailing whitespace off (a hard break's
//! two spaces kept), one newline at the end; `-` for every bullet and ordered lists
//! renumbered; `# ` with one space; `[X]` → `[x]`; `[!Note]` → `[!note]`; one space
//! before a trailing ` ^id`; table columns padded to width; a bare URL carrying a
//! `#` wrapped in `<…>` (a fragment reads as a tag otherwise); frontmatter keys in
//! the order title / tags / aliases / created, the rest as written.
//!
//! What it leaves alone: fenced code, `$$` math, `> [!private]` and `> [!quote]`
//! blocks (verbatim, body included), frontmatter values, and every character of
//! prose unless an opt-in says otherwise (`cjk_spacing`, `punctuation`: they change
//! bytes, so the block hashes behind provenance and change times move — the user
//! turns them on knowingly).

use crate::parser::{is_fence_close, parse_fence_open, parse_heading, parse_list_marker};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FormatOptions {
    /// A space between CJK and Latin / digit runs.
    pub cjk_spacing: bool,
    /// ASCII `,.;:!?` after a CJK character become their full-width forms.
    pub punctuation: bool,
}

/// One line's kind, for the blank-line rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Blank,
    Heading,
    List,
    /// An indented line under a list item (its continuation).
    ListCont,
    Quote,
    Table,
    Rule,
    Text,
    /// A verbatim region (fence, math): kept as is, separated like a block.
    Verbatim,
}

pub fn format_markdown(text: &str, opts: &FormatOptions) -> String {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let src = text.replace("\r\n", "\n");
    let (front, body) = split_frontmatter(&src);
    let mut out = String::with_capacity(src.len() + 16);
    if let Some(f) = front {
        out.push_str(&format_frontmatter(f));
    }
    let lines: Vec<&str> = body.lines().collect();
    let formatted = format_body(&lines, opts);
    out.push_str(&formatted);
    // exactly one newline at the end (an empty body stays empty)
    let mut result = out.trim_end_matches('\n').to_string();
    if !result.is_empty() {
        result.push('\n');
    }
    if eol == "\r\n" {
        result = result.replace('\n', "\r\n");
    }
    result
}

/// Format a fragment that lands inside an existing note (an appended section, a
/// pasted passage): the body rules only. A fragment that happens to open with
/// `---` is a thematic break or a paragraph, never frontmatter — only a whole
/// note has one, and `format_markdown` handles that case.
pub fn format_fragment(text: &str, opts: &FormatOptions) -> String {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let src = text.replace("\r\n", "\n");
    let lines: Vec<&str> = src.lines().collect();
    let mut result = format_body(&lines, opts).trim_end_matches('\n').to_string();
    if !result.is_empty() {
        result.push('\n');
    }
    if eol == "\r\n" {
        result = result.replace('\n', "\r\n");
    }
    result
}

/// (the frontmatter block including both fences and its trailing newline, the body)
fn split_frontmatter(src: &str) -> (Option<&str>, &str) {
    let (_, offset) = crate::parser::extract_frontmatter(src);
    if offset == 0 {
        return (None, src);
    }
    (Some(&src[..offset]), &src[offset..])
}

/// Frontmatter: known keys first (title, tags, aliases, created), the rest in their
/// order, a key's continuation lines and the comment lines above it travelling with
/// it; values untouched.
fn format_frontmatter(block: &str) -> String {
    let lines: Vec<&str> = block.lines().collect();
    if lines.len() < 2 {
        return block.to_string();
    }
    let (open, close) = (lines[0], lines[lines.len() - 1]);
    let inner = &lines[1..lines.len() - 1];
    // group into key blocks: a top-level `key:` line starts a block; indented / `- ` /
    // comment lines attach to the current block (comments above a key attach to the next)
    let mut groups: Vec<(Option<String>, Vec<&str>)> = Vec::new();
    let mut pending_comments: Vec<&str> = Vec::new();
    for l in inner {
        let is_top_key = !l.starts_with([' ', '\t', '-', '#']) && l.contains(':');
        if is_top_key {
            let key = l.split(':').next().unwrap_or("").trim().to_string();
            let mut g = std::mem::take(&mut pending_comments);
            g.push(l);
            groups.push((Some(key), g));
        } else if l.trim_start().starts_with('#') {
            // a comment belongs to the key below it
            pending_comments.push(l);
        } else if let Some(g) = groups.last_mut() {
            g.1.append(&mut pending_comments);
            g.1.push(l);
        } else {
            pending_comments.push(l);
        }
    }
    let rank = |k: &Option<String>| match k.as_deref() {
        Some("title") => 0,
        Some("tags") => 1,
        Some("aliases") => 2,
        Some("created") => 3,
        _ => 10,
    };
    groups.sort_by_key(|(k, _)| rank(k)); // stable: the rest keep their order
    let mut out = String::new();
    out.push_str(open);
    out.push('\n');
    for (_, g) in &groups {
        for l in g {
            out.push_str(l);
            out.push('\n');
        }
    }
    for l in pending_comments {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str(close);
    out.push('\n');
    out
}

fn is_table_row(t: &str) -> bool {
    t.starts_with('|') && t.trim_end().ends_with('|') && t.len() > 1
}

/// Width for table padding: CJK and full-width forms count two columns.
fn display_width(s: &str) -> usize {
    s.chars()
        .map(|c| {
            let u = c as u32;
            if (0x1100..=0x115F).contains(&u)
                || (0x2E80..=0xA4CF).contains(&u)
                || (0xAC00..=0xD7A3).contains(&u)
                || (0xF900..=0xFAFF).contains(&u)
                || (0xFE30..=0xFE4F).contains(&u)
                || (0xFF00..=0xFF60).contains(&u)
                || (0xFFE0..=0xFFE6).contains(&u)
                || (0x20000..=0x3FFFD).contains(&u)
            {
                2
            } else {
                1
            }
        })
        .sum()
}

fn format_body(lines: &[&str], opts: &FormatOptions) -> String {
    // Pass 1: classify lines and normalise each one, copying verbatim regions
    let mut items: Vec<(Kind, String)> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_start();
        // fenced code: verbatim through the closer
        if let Some((ch, len, _)) = parse_fence_open(trimmed) {
            let mut block = vec![line.to_string()];
            i += 1;
            while i < lines.len() {
                let l = lines[i];
                block.push(l.to_string());
                i += 1;
                if is_fence_close(l.trim_start(), ch, len) {
                    break;
                }
            }
            items.push((Kind::Verbatim, block.join("\n")));
            continue;
        }
        // display math: verbatim through the closing $$
        if let Some(after_open) = trimmed.strip_prefix("$$") {
            let mut block = vec![line.to_string()];
            let closed_same_line = !after_open.is_empty() && after_open.trim_end().ends_with("$$");
            i += 1;
            if !closed_same_line {
                while i < lines.len() {
                    let l = lines[i];
                    block.push(l.to_string());
                    i += 1;
                    if l.trim().ends_with("$$") {
                        break;
                    }
                }
            }
            items.push((Kind::Verbatim, block.join("\n")));
            continue;
        }
        // private and quote callouts: the whole quote run verbatim (the body is the user's or a citation)
        if let Some(quote_body) = trimmed.strip_prefix('>') {
            let is_protected = {
                let after = quote_body.trim_start();
                let lower = after.to_ascii_lowercase();
                lower.starts_with("[!private]") || lower.starts_with("[!quote]") || lower.starts_with("[!cite]")
            };
            let mut block = Vec::new();
            // a fenced code block quoted inside the run is copied verbatim through its
            // closer (trailing whitespace and bare URLs included), like a fence outside
            let mut fence: Option<(char, usize)> = None;
            while i < lines.len() && lines[i].trim_start().starts_with('>') {
                let l = lines[i];
                i += 1;
                if is_protected {
                    block.push(l.to_string());
                    continue;
                }
                let inner = quote_inner(l);
                match fence {
                    Some((ch, len)) => {
                        block.push(l.to_string());
                        if is_fence_close(inner, ch, len) {
                            fence = None;
                        }
                    }
                    None => match parse_fence_open(inner) {
                        Some((ch, len, _)) => {
                            fence = Some((ch, len));
                            block.push(l.to_string());
                        }
                        None => block.push(normalise_quote_line(l, opts)),
                    },
                }
            }
            items.push((if is_protected { Kind::Verbatim } else { Kind::Quote }, block.join("\n")));
            continue;
        }
        if trimmed.is_empty() {
            items.push((Kind::Blank, String::new()));
            i += 1;
            continue;
        }
        if let Some((level, text)) = parse_heading(trimmed) {
            let text = strip_trailing_ws(&inline(text, opts));
            items.push((Kind::Heading, format!("{} {}", "#".repeat(level as usize), text)));
            i += 1;
            continue;
        }
        if crate::parser::is_thematic_break(trimmed) {
            // `---` right under a text line is that paragraph's setext underline (a
            // heading), not a rule: it stays glued to the line above
            let underline = trimmed.trim_end().chars().all(|c| c == '-') && items.last().is_some_and(|(k, _)| *k == Kind::Text);
            items.push((if underline { Kind::Text } else { Kind::Rule }, trimmed.trim_end().to_string()));
            i += 1;
            continue;
        }
        if is_table_row(trimmed) {
            let mut rows = Vec::new();
            while i < lines.len() && is_table_row(lines[i].trim_start()) {
                rows.push(lines[i].trim().to_string());
                i += 1;
            }
            items.push((Kind::Table, format_table(&rows, opts)));
            continue;
        }
        if parse_list_marker(line).is_some() {
            items.push((Kind::List, normalise_list_line(line, opts)));
            i += 1;
            continue;
        }
        let indented = line.starts_with("  ") || line.starts_with('\t');
        let prev_is_list = items.iter().rev().find(|(k, _)| *k != Kind::Blank).is_some_and(|(k, _)| matches!(k, Kind::List | Kind::ListCont));
        if indented && prev_is_list {
            items.push((Kind::ListCont, strip_trailing_ws(&inline(line, opts))));
            i += 1;
            continue;
        }
        items.push((Kind::Text, strip_trailing_ws(&inline(line, opts))));
        i += 1;
    }
    // Pass 1b: renumber ordered lists (per indent, consecutive items)
    renumber_ordered(&mut items);
    // Pass 2: emit with the blank-line rules. Items of one list pack tight whatever
    // the source had between them; two text lines are one paragraph unless the
    // source separated them; a list item's continuation keeps a blank the author put
    // there (it is a paragraph inside the item); an unindented text line right under
    // a list item or a quote line is its lazy continuation and stays glued to it;
    // everything else gets exactly one.
    let mut out = String::new();
    let mut prev: Option<Kind> = None;
    let mut blank_between = false;
    for (kind, text) in &items {
        if *kind == Kind::Blank {
            blank_between = true;
            continue;
        }
        if let Some(p) = prev {
            let tight = match (p, *kind) {
                (Kind::List, Kind::List) | (Kind::ListCont, Kind::List) => true,
                (Kind::List | Kind::ListCont | Kind::Quote | Kind::Text, Kind::Text) | (Kind::List | Kind::ListCont, Kind::ListCont) => !blank_between,
                _ => false,
            };
            // a hard break's two spaces mean something only when the paragraph goes on
            // in the next line; at a block boundary they are trailing whitespace
            let continues = tight && matches!(*kind, Kind::Text | Kind::ListCont);
            if !continues && p != Kind::Verbatim {
                out.truncate(out.trim_end_matches(' ').len());
            }
            out.push('\n');
            if !tight {
                out.push('\n');
            }
        }
        out.push_str(text);
        prev = Some(*kind);
        blank_between = false;
    }
    if !out.is_empty() {
        if prev != Some(Kind::Verbatim) {
            out.truncate(out.trim_end_matches(' ').len());
        }
        out.push('\n');
    }
    out
}

/// Ordered items at one indent, uninterrupted by anything but their own continuation
/// lines and nested items, count 1, 2, 3…
fn renumber_ordered(items: &mut [(Kind, String)]) {
    let mut counters: Vec<(usize, usize)> = Vec::new(); // (indent, next number)
    for (kind, text) in items.iter_mut() {
        match kind {
            Kind::List => {
                let Some((indent, ordered, _)) = parse_list_marker(text) else { continue };
                counters.retain(|(ind, _)| *ind <= indent);
                if !ordered {
                    counters.retain(|(ind, _)| *ind < indent);
                    continue;
                }
                let n = match counters.iter_mut().find(|(ind, _)| *ind == indent) {
                    Some(c) => {
                        c.1 += 1;
                        c.1
                    }
                    None => {
                        counters.push((indent, 1));
                        1
                    }
                };
                let rest = &text[indent..];
                let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
                let delim = &rest[digits..digits + 1];
                *text = format!("{}{n}{delim}{}", &text[..indent], &rest[digits + 1..]);
            }
            Kind::ListCont | Kind::Blank => {}
            _ => counters.clear(),
        }
    }
}

fn normalise_list_line(line: &str, opts: &FormatOptions) -> String {
    let Some((indent, ordered, at)) = parse_list_marker(line) else { return line.to_string() };
    let indent_s = &line[..indent];
    let rest = &line[at..];
    let marker = if ordered { line[indent..at].trim_end().to_string() } else { "-".to_string() };
    // the checkbox: `[X]` → `[x]`
    let mut rest = rest.to_string();
    if rest.starts_with("[X]") {
        rest.replace_range(0..3, "[x]");
    }
    let rest = strip_trailing_ws(&inline(&rest, opts));
    format!("{indent_s}{marker} {rest}")
}

/// The text after a quote line's `>` marker and the whitespace that follows it.
fn quote_inner(line: &str) -> &str {
    line.trim_start()[1..].trim_start()
}

fn normalise_quote_line(line: &str, opts: &FormatOptions) -> String {
    let indent = line.len() - line.trim_start().len();
    let after = quote_inner(line);
    // a callout head: lowercase the type
    let after = if let Some(rest) = after.strip_prefix("[!") {
        match rest.find(']') {
            Some(end) => format!("[!{}]{}", rest[..end].to_ascii_lowercase(), &rest[end + 1..]),
            None => after.to_string(),
        }
    } else {
        after.to_string()
    };
    let body = strip_trailing_ws(&inline(&after, opts));
    if body.is_empty() {
        format!("{}>", &line[..indent])
    } else {
        format!("{}> {body}", &line[..indent])
    }
}

/// Trailing whitespace off; a hard break (two or more trailing spaces after some
/// text) is kept as exactly two.
fn strip_trailing_ws(s: &str) -> String {
    let trimmed = s.trim_end_matches([' ', '\t']);
    let spaces = s.len() - s.trim_end_matches(' ').len();
    if spaces >= 2 && !trimmed.is_empty() {
        return format!("{trimmed}  ");
    }
    trimmed.to_string()
}

/// Inline normalisation of one line's text (code spans, wikilinks and Markdown link
/// targets untouched): the ` ^id` anchor gets one space; a bare URL with a `#`
/// gets `<…>`; the opt-in CJK spacing and punctuation.
fn inline(s: &str, opts: &FormatOptions) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut segments = split_protected(s);
    for seg in segments.iter_mut() {
        if seg.protected {
            out.push_str(&seg.text);
            continue;
        }
        let mut t = wrap_fragment_urls(&seg.text);
        if opts.cjk_spacing {
            t = cjk_space(&t);
        }
        if opts.punctuation {
            t = cjk_punct(&t);
        }
        out.push_str(&t);
    }
    // the anchor: `text  ^id` / `text\t^id` → `text ^id`
    if let Some(at) = crate::blockid::anchor_start(&out) {
        let (before, anchor) = out.split_at(at);
        let before = before.trim_end_matches([' ', '\t']);
        return format!("{before} {}", anchor.trim_start_matches([' ', '\t']));
    }
    out
}

struct Segment {
    text: String,
    protected: bool,
}

/// Split a line into prose and protected spans: code spans, `[[wikilinks]]`, the
/// target of a Markdown link `](…)`, autolinks `<…>` and inline math `$…$`.
fn split_protected(s: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut i = 0;
    let flush = |cur: &mut String, out: &mut Vec<Segment>| {
        if !cur.is_empty() {
            out.push(Segment { text: std::mem::take(cur), protected: false });
        }
    };
    while i < s.len() {
        let rest = &s[i..];
        let take = if rest.starts_with('`') {
            let run = rest.chars().take_while(|c| *c == '`').count();
            rest[run..].find(&"`".repeat(run)).map(|e| run + e + run)
        } else if rest.starts_with("[[") {
            rest.find("]]").map(|e| e + 2)
        } else if rest.starts_with("](") {
            rest.find(')').map(|e| e + 1)
        } else if rest.starts_with('<') && rest[1..].starts_with("http") {
            rest.find('>').map(|e| e + 1)
        } else if rest.starts_with('$') && !rest.starts_with("$$") {
            rest[1..].find('$').filter(|e| *e > 0).map(|e| e + 2)
        } else {
            None
        };
        match take {
            Some(len) => {
                flush(&mut cur, &mut out);
                out.push(Segment { text: rest[..len].to_string(), protected: true });
                i += len;
            }
            None => {
                let ch = rest.chars().next().unwrap();
                cur.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// A bare `http(s)://…` carrying a `#` becomes `<url>` (the fragment would read as a
/// tag): only when it stands on its own (not inside `(`, `[`, `<`).
fn wrap_fragment_urls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pos = 0;
    while let Some(at) = s[pos..].find("http") {
        let start = pos + at;
        let rest = &s[start..];
        if !(rest.starts_with("http://") || rest.starts_with("https://")) {
            out.push_str(&s[pos..start + 4]);
            pos = start + 4;
            continue;
        }
        let prev = s[..start].chars().next_back();
        let end = rest.find(|c: char| c.is_whitespace() || !(c.is_ascii_graphic())).unwrap_or(rest.len());
        let url = trim_url_end(&rest[..end]);
        let bare = !matches!(prev, Some('(' | '[' | '<' | '"' | '\''));
        out.push_str(&s[pos..start]);
        if bare && url.contains('#') {
            out.push('<');
            out.push_str(url);
            out.push('>');
        } else {
            out.push_str(url);
        }
        pos = start + url.len();
    }
    out.push_str(&s[pos..]);
    out
}

/// Trailing punctuation off the end of a bare URL. A `)` stays while the URL still
/// holds an unmatched `(` — a disambiguation path like `Rust#Foo_(bar)` — and only
/// the `)` that closes prose around the URL is dropped.
fn trim_url_end(url: &str) -> &str {
    let mut end = url.len();
    while let Some(last) = url[..end].chars().next_back() {
        if !matches!(last, ')' | '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\'') {
            break;
        }
        if last == ')' {
            let prefix = &url[..end - 1];
            if prefix.matches('(').count() > prefix.matches(')').count() {
                break;
            }
        }
        end -= 1;
    }
    &url[..end]
}

fn is_cjk(c: char) -> bool {
    let u = c as u32;
    (0x4E00..=0x9FFF).contains(&u) || (0x3400..=0x4DBF).contains(&u) || (0x3040..=0x30FF).contains(&u) || (0xAC00..=0xD7A3).contains(&u) || (0x20000..=0x2FFFF).contains(&u)
}

/// A space between a CJK character and a Latin letter or digit either way.
fn cjk_space(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    let mut prev: Option<char> = None;
    for c in s.chars() {
        if let Some(p) = prev {
            let latin = |x: char| x.is_ascii_alphanumeric();
            if (is_cjk(p) && latin(c)) || (latin(p) && is_cjk(c)) {
                out.push(' ');
            }
        }
        out.push(c);
        prev = Some(c);
    }
    out
}

/// ASCII punctuation right after a CJK character → the full-width form (and the
/// space that often follows it dropped).
fn cjk_punct(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let prev_cjk = i > 0 && is_cjk(chars[i - 1]);
        let mapped = match c {
            ',' => Some('，'),
            '.' => Some('。'),
            ';' => Some('；'),
            ':' => Some('：'),
            '!' => Some('！'),
            '?' => Some('？'),
            _ => None,
        };
        match mapped {
            Some(m) if prev_cjk => {
                out.push(m);
                if i + 1 < chars.len() && chars[i + 1] == ' ' {
                    i += 1;
                }
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// Cells padded to the column's width; the alignment row rebuilt from its colons.
/// Rows with a different cell count are left as they are.
fn format_table(rows: &[String], opts: &FormatOptions) -> String {
    let split = |r: &str| -> Vec<String> {
        let inner = r.trim().trim_start_matches('|').trim_end_matches('|');
        let mut cells = Vec::new();
        let mut cur = String::new();
        let mut esc = false;
        for c in inner.chars() {
            if esc {
                cur.push(c);
                esc = false;
            } else if c == '\\' {
                cur.push(c);
                esc = true;
            } else if c == '|' {
                cells.push(cur.trim().to_string());
                cur = String::new();
            } else {
                cur.push(c);
            }
        }
        cells.push(cur.trim().to_string());
        cells
    };
    let table: Vec<Vec<String>> = rows.iter().map(|r| split(r).into_iter().map(|c| inline(&c, opts)).collect()).collect();
    let cols = table[0].len();
    if table.iter().any(|r| r.len() != cols) || cols == 0 {
        return rows.join("\n");
    }
    let is_align = |c: &str| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':');
    let align_row = table.get(1).is_some_and(|r| r.iter().all(|c| is_align(c)));
    let mut widths = vec![3usize; cols];
    for (ri, r) in table.iter().enumerate() {
        if ri == 1 && align_row {
            continue;
        }
        for (ci, c) in r.iter().enumerate() {
            widths[ci] = widths[ci].max(display_width(c));
        }
    }
    let mut out = Vec::new();
    for (ri, r) in table.iter().enumerate() {
        if ri == 1 && align_row {
            let cells: Vec<String> = r
                .iter()
                .enumerate()
                .map(|(ci, c)| {
                    let left = c.starts_with(':');
                    let right = c.ends_with(':');
                    let dashes = "-".repeat(widths[ci].saturating_sub(left as usize + right as usize).max(1));
                    format!("{}{dashes}{}", if left { ":" } else { "" }, if right { ":" } else { "" })
                })
                .collect();
            out.push(format!("| {} |", cells.join(" | ")));
            continue;
        }
        let cells: Vec<String> = r
            .iter()
            .enumerate()
            .map(|(ci, c)| format!("{c}{}", " ".repeat(widths[ci].saturating_sub(display_width(c)))))
            .collect();
        out.push(format!("| {} |", cells.join(" | ")));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(s: &str) -> String {
        format_markdown(s, &FormatOptions::default())
    }

    #[test]
    fn blank_lines_between_blocks_and_tight_lists() {
        let src = "# Title\ntext line   \n\n\n\n* a\n\n* b\n  cont\n+ c\n1) one\n3) two\n### Sub\n---\nend";
        let want = "# Title\n\ntext line\n\n- a\n- b\n  cont\n- c\n1) one\n2) two\n\n### Sub\n\n---\n\nend\n";
        assert_eq!(fmt(src), want);
        assert_eq!(fmt(&fmt(src)), fmt(src), "idempotent");
    }

    #[test]
    fn markers_checkbox_callout_anchor_and_fragment_url() {
        assert_eq!(fmt("#   Heading\n- [X] done  ^k1\n> [!Note] hi\n> body\n\nsee https://a.io/p#frag now"), "# Heading\n\n- [x] done ^k1\n\n> [!note] hi\n> body\n\nsee <https://a.io/p#frag> now\n");
        // `#word` at the start of a line is a tag, not a heading: left alone
        assert_eq!(fmt("#tag line"), "#tag line\n");
        // already wrapped or inside a link: untouched
        assert_eq!(fmt("[t](https://a.io/p#x) <https://b.io/#y>"), "[t](https://a.io/p#x) <https://b.io/#y>\n");
        // a hard break's two spaces stay
        assert_eq!(fmt("line one  \nline two"), "line one  \nline two\n");
    }

    #[test]
    fn verbatim_regions_are_untouched() {
        let src = "```md\n* keep\n#raw\n```\n$$\nx  \n$$\n> [!private]\n> secret  * x\n> [!quote] a\n> b  \n\n- item";
        let out = fmt(src);
        assert!(out.contains("```md\n* keep\n#raw\n```"));
        assert!(out.contains("$$\nx  \n$$"));
        assert!(out.contains("> [!private]\n> secret  * x"));
        assert!(out.contains("> [!quote] a\n> b  "), "{out}");
        assert!(out.ends_with("- item\n"));
        assert_eq!(fmt(&out), out);
    }

    #[test]
    fn frontmatter_keys_reorder_and_values_stay() {
        let src = "---\nstatus: draft\ntags:\n  - a\n  - b\n# note\ntitle: \"T: x\"\ncreated: 2026-09-06\n---\nbody\n";
        let out = fmt(src);
        // the comment above title travels with it
        assert_eq!(out, "---\n# note\ntitle: \"T: x\"\ntags:\n  - a\n  - b\ncreated: 2026-09-06\nstatus: draft\n---\nbody\n");
        assert_eq!(fmt(&out), out);
    }

    #[test]
    fn tables_pad_to_width_with_cjk() {
        let out = fmt("|a|b|\n|-|:-:|\n|中文|x|");
        assert_eq!(out, "| a    | b   |\n| ---- | :-: |\n| 中文 | x   |\n");
        assert_eq!(fmt(&out), out);
    }

    #[test]
    fn opt_ins_change_bytes_only_when_asked() {
        let src = "中文abc和123,好.";
        assert_eq!(fmt(src), "中文abc和123,好.\n");
        let opts = FormatOptions { cjk_spacing: true, punctuation: true };
        // punctuation turns full-width after a CJK character only (`123,` stays ASCII)
        assert_eq!(format_markdown(src, &opts), "中文 abc 和 123,好。\n");
        // code spans and links are never touched by the opt-ins
        assert_eq!(format_markdown("中文`a中`和[[链接A]]", &opts), "中文`a中`和[[链接A]]\n");
    }

    #[test]
    fn a_fence_inside_a_quote_is_verbatim() {
        let src = "> [!tip]\n> ```sh\n> curl https://api.example.com/v1#anchor  \n> ```\n";
        assert_eq!(fmt(src), src);
        // the run after the closer is normalised again
        assert_eq!(fmt("> ```\n> x  \n> ```\n> tail   "), "> ```\n> x  \n> ```\n> tail\n");
    }

    #[test]
    fn three_trailing_spaces_are_still_a_hard_break() {
        assert_eq!(fmt("line one   \nline two"), "line one  \nline two\n");
        assert_eq!(fmt("- item   \n  cont"), "- item  \n  cont\n");
        // at the end of a block the spaces are trailing whitespace, not a break
        assert_eq!(fmt("- item    \n- next"), "- item\n- next\n");
        assert_eq!(fmt("last line  "), "last line\n");
        assert_eq!(fmt("# Title  \n\ntext"), "# Title\n\ntext\n");
        // whitespace alone is never a break
        assert_eq!(fmt("   \nx"), "x\n");
    }

    #[test]
    fn lazy_continuations_and_setext_underlines_stay_glued() {
        assert_eq!(fmt("- item\nstill the item\n"), "- item\nstill the item\n");
        assert_eq!(fmt("> quoted\nlazy\n"), "> quoted\nlazy\n");
        assert_eq!(fmt("Title\n---\n"), "Title\n---\n");
        assert_eq!(fmt("Title\n===\n"), "Title\n===\n");
        // a rule after a blank, or one that is not dashes only, is a thematic break
        assert_eq!(fmt("para\n\n---\n\npara"), "para\n\n---\n\npara\n");
        assert_eq!(fmt("para\n***\n"), "para\n\n***\n");
        assert_eq!(fmt("- a\n- b"), "- a\n- b\n");
        // a blank still separates a list from the paragraph after it
        assert_eq!(fmt("- a\n\nafter"), "- a\n\nafter\n");
    }

    #[test]
    fn a_fragment_url_keeps_its_balanced_parenthesis() {
        assert_eq!(fmt("see https://en.wikipedia.org/wiki/Rust#Foo_(bar) now"), "see <https://en.wikipedia.org/wiki/Rust#Foo_(bar)> now\n");
        assert_eq!(fmt("(see https://a.io/p#x)"), "(see <https://a.io/p#x>)\n");
    }

    #[test]
    fn crlf_is_preserved_and_paragraphs_stay_apart() {
        assert_eq!(fmt("a\r\n\r\n\r\nb\r\n"), "a\r\n\r\nb\r\n");
        assert_eq!(fmt("- item\n\n  inner paragraph\n- next\n\n- after gap"), "- item\n\n  inner paragraph\n- next\n- after gap\n");
    }
}
