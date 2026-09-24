//! The web page → vault note clipping pipeline (v0.4).
//!
//! A pure-logic crate: HTML or selection text in, the full Markdown note and
//! its destination path out. The dependency direction is core ← clip ←
//! src-tauri — nothing here touches windows, the network or IPC, which is why
//! every behaviour is covered by `cargo test -p vemry-clip`.
//!
//! Output constraint (aligned with vemry-core's parser): every URL in the body
//! appears as `[title](url)` or `<url>` — a bare URL's `#fragment` is mistaken
//! for a tag by the inline scanner (it triggers whenever `/` precedes the `#`),
//! and there are regression assertions for it in the tests.

use chrono::{DateTime, Local, SecondsFormat};
use vemry_core::{Frontmatter, VemryMeta};

/// Where one clip came from. `clipped_at` is passed in by the caller (for
/// testability: the library never reads the clock itself).
#[derive(Clone, Debug)]
pub struct ClipMeta {
    pub url: String,
    pub title: String,
    pub clipped_at: DateTime<Local>,
}

impl ClipMeta {
    /// Title for display: an empty title falls back to the domain, then to the
    /// URL as written.
    pub fn display_title(&self) -> String {
        let t = self.title.trim();
        if !t.is_empty() {
            return t.to_string();
        }
        domain_of(&self.url).unwrap_or_else(|| self.url.clone())
    }
}

#[derive(Clone, Debug)]
pub struct ExtractedPage {
    pub markdown: String,
    pub title: String,
    pub byline: Option<String>,
    /// Readability failed to extract the body, so this took the whole-page
    /// html→md fallback.
    pub fallback: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ClipError {
    #[error("the page has no content")]
    EmptyPage,
    #[error("content extraction failed: {0}")]
    Extract(String),
}

/// The math sentinel element: the grabbing JS (GRAB_PAGE in src-tauri) turns
/// KaTeX/MathJax/MathML back into LaTeX source wrapped in
/// `<x-vemry-math display="block">…</x-vemry-math>`, and this converts it to
/// `$…$` / `$$…$$`. The content must pass through verbatim — htmd's default text
/// escaping mangles LaTeX's `\ _ { }`, which is also why the grabbing side does
/// not inline `$…$` text directly.
pub const MATH_TAG: &str = "x-vemry-math";

/// The code sentinel element: the grabbing JS turns code viewers whose line
/// breaks exist only in the CSS rendering box (lake cards, CodeMirror and other
/// per-line span/div renderings) back into plain-text code wrapped in
/// `<x-vemry-code lang="…">`, and this converts it to a fenced code block.
/// Without that normalisation, serialising outerHTML would splice the code lines
/// seamlessly into one line — and a shell comment `# xx` landing at the start of
/// a line would even become a phantom heading. The content passes through
/// verbatim, the same contract as MATH_TAG.
pub const CODE_TAG: &str = "x-vemry-code";

/// Collect a node subtree's plain text (a sentinel element holds text only, but
/// recurse defensively).
fn node_text(node: &std::rc::Rc<markup5ever_rcdom::Node>, out: &mut String) {
    match &node.data {
        markup5ever_rcdom::NodeData::Text { contents } => out.push_str(&contents.borrow()),
        _ => {
            for child in node.children.borrow().iter() {
                node_text(child, out);
            }
        }
    }
}

fn math_handler(el: htmd::Element) -> Option<String> {
    let mut tex = String::new();
    node_text(el.node, &mut tex);
    let tex = tex.split_whitespace().collect::<Vec<_>>().join(" ");
    if tex.is_empty() {
        return Some(String::new());
    }
    let display = el
        .attrs
        .iter()
        .any(|a| a.name.local.as_ref() == "display" && a.value.as_ref() == "block");
    // Inline sticks to its neighbours; block level stands as its own paragraph
    // (the blank lines around it make the renderer treat it as one)
    Some(if display { format!("\n\n$${tex}$$\n\n") } else { format!("${tex}$") })
}

fn code_handler(el: htmd::Element) -> Option<String> {
    let mut code = String::new();
    node_text(el.node, &mut code);
    // Strip leading and trailing blank lines, keep the indentation inside
    // (for code it is semantics)
    let code = code.trim_matches('\n').trim_end();
    if code.trim().is_empty() {
        return Some(String::new());
    }
    // lang collapses to a single token: whitespace or backticks in a fence info
    // string break the fence syntax
    let lang: String = el
        .attrs
        .iter()
        .find(|a| a.name.local.as_ref() == "lang")
        .map(|a| {
            a.value
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-' | '#' | '.'))
                .collect()
        })
        .unwrap_or_default();
    // The fence must be longer than the longest backtick run in the content, or
    // a content line closes the fence early (splitting on non-backtick
    // characters leaves the backtick runs as the pieces)
    let longest = code.split(|c: char| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat((longest + 1).max(3));
    Some(format!("\n\n{fence}{lang}\n{code}\n{fence}\n\n"))
}

/// Links: `[text](url)`, with the `title` attribute dropped and a citation
/// marker link dropped altogether.
/// - htmd keeps the title (`[text](url "title")`); on an encyclopedia page every
///   link carries one, so a clipped paragraph doubles in size and reads as URL
///   soup wherever the source shows — a quotation needs the destination only.
/// - A citation superscript (`<sup><a href="…#cite_note-1">[1]</a></sup>`) is
///   noise in a quotation, and worse than noise: its text is itself bracketed, so
///   the link would come out as `[[1]](url)` — a wikilink followed by a bare URL,
///   which the editor's autolink then extends through the next link on the line.
///   Dropped when the text is a short bracketed marker and the destination is a
///   fragment link (a citation points into its own page); `[citation needed]`
///   pointing at a help page stays a link.
///
/// The destination is percent-encoded the way `md_link` does it (htmd would write
/// `\(` `\)` and wrap a URL holding spaces in `<>`); leading whitespace inside
/// the anchor is dropped and trailing whitespace moved after the link, as htmd
/// does.
fn anchor_handler(h: &dyn htmd::element_handler::Handlers, el: &htmd::Element) -> Option<String> {
    let href = el
        .attrs
        .iter()
        .find(|a| a.name.local.as_ref() == "href")
        .map(|a| a.value.chars().filter(|c| !matches!(c, '\n' | '\r' | '\t')).collect::<String>())
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    let content = h.walk_children(el.node).content;
    let Some(href) = href else {
        return Some(content);
    };
    let text = content.trim_start();
    let body = text.trim_end();
    let trailing = &text[body.len()..];
    // An empty anchor (a bare target, an icon that did not convert) has nothing to link
    if body.is_empty() || (href.contains('#') && is_citation_marker(body)) {
        return Some(trailing.to_string());
    }
    Some(format!("[{body}]({}){trailing}", encode_link_url(&href)))
}

/// `\[1\]` (htmd escapes the brackets of text), `[1]`, `[note 3]`: a bracketed
/// marker up to CITE_MAX_BYTES of content with nothing nested.
fn is_citation_marker(text: &str) -> bool {
    let inner = text
        .strip_prefix("\\[")
        .or_else(|| text.strip_prefix('['))
        .and_then(|s| s.strip_suffix("\\]").or_else(|| s.strip_suffix(']')));
    match inner {
        Some(s) => !s.is_empty() && s.len() <= CITE_MAX_BYTES && !s.contains(['[', ']', '\\', '\n']),
        None => false,
    }
}

/// The tag name when the node is an element.
fn element_tag(node: &markup5ever_rcdom::Node) -> Option<&str> {
    match &node.data {
        markup5ever_rcdom::NodeData::Element { name, .. } => Some(name.local.as_ref()),
        _ => None,
    }
}

/// An element attribute's value (None when it is not an element or has no such
/// attribute).
fn element_attr(node: &markup5ever_rcdom::Node, attr: &str) -> Option<String> {
    match &node.data {
        markup5ever_rcdom::NodeData::Element { attrs, .. } => attrs
            .borrow()
            .iter()
            .find(|a| a.name.local.as_ref() == attr)
            .map(|a| a.value.to_string()),
        _ => None,
    }
}

/// Whether a td/th sits on the ancestor chain — i.e. this is nested inside
/// another table cell. parent is a Cell<Option<Weak>>, so it can only be taken
/// and put back.
fn inside_table_cell(node: &std::rc::Rc<markup5ever_rcdom::Node>) -> bool {
    let mut cur = node.clone();
    loop {
        let Some(weak) = cur.parent.take() else { return false };
        let parent = weak.upgrade();
        cur.parent.set(Some(weak));
        let Some(parent) = parent else { return false };
        if matches!(element_tag(&parent), Some("td" | "th")) {
            return true;
        }
        cur = parent;
    }
}

/// Squash a cell's Markdown onto one line (several paragraphs fold into
/// space-separated text) and escape `|` — both the editor's table widget and
/// renderBlocks split cells on unescaped `|`.
fn table_cell_text(md: &str) -> String {
    md.split_whitespace().collect::<Vec<_>>().join(" ").replace('|', "\\|")
}

/// Every cell of one `<tr>` (th and td collected together in document order, as
/// a header row is often styled td). colspan is padded with empty cells so the
/// later columns stay aligned; rowspan cannot be expressed in pipe syntax and is
/// ignored.
fn table_row_cells(handlers: &dyn htmd::element_handler::Handlers, tr: &std::rc::Rc<markup5ever_rcdom::Node>) -> Vec<String> {
    let mut cells = Vec::new();
    for cell in tr.children.borrow().iter() {
        if !matches!(element_tag(cell), Some("td" | "th")) {
            continue;
        }
        let content = handlers.handle(cell).map(|r| r.content).unwrap_or_default();
        cells.push(table_cell_text(&content));
        let span = element_attr(cell, "colspan").and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(1);
        for _ in 1..span.min(8) {
            cells.push(String::new());
        }
    }
    cells
}

/// `<table>` → a GFM pipe table. htmd's built-in table conversion requires an
/// explicit `<th>`/`<thead>` in its default (Pure) mode, while a CMS page's
/// header row is often a styled `<td>`, which flattens the whole table into one
/// paragraph per cell. This takes the conversion over entirely: with no `<th>`,
/// the first row is promoted to the header (the same heuristic turndown-gfm
/// uses); a table nested inside a cell (inexpressible in pipe syntax) and a
/// single-column table (usually there for layout) have their content flattened;
/// an empty table returns None and is skipped altogether.
fn table_handler(handlers: &dyn htmd::element_handler::Handlers, el: &htmd::Element) -> Option<String> {
    let flatten = |handlers: &dyn htmd::element_handler::Handlers| {
        let content = handlers.walk_children(el.node).content;
        let content = content.trim_matches('\n');
        if content.is_empty() { None } else { Some(format!("\n\n{content}\n\n")) }
    };
    if inside_table_cell(el.node) {
        return flatten(handlers);
    }

    let mut captions: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    for child in el.node.children.borrow().iter() {
        match element_tag(child) {
            Some("caption") => {
                if let Some(res) = handlers.handle(child) {
                    let text = res.content.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !text.is_empty() {
                        captions.push(text);
                    }
                }
            }
            Some("thead" | "tbody" | "tfoot") => {
                for tr in child.children.borrow().iter() {
                    if element_tag(tr) == Some("tr") {
                        let cells = table_row_cells(handlers, tr);
                        if !cells.is_empty() {
                            rows.push(cells);
                        }
                    }
                }
            }
            Some("tr") => {
                let cells = table_row_cells(handlers, child);
                if !cells.is_empty() {
                    rows.push(cells);
                }
            }
            _ => {}
        }
    }

    let cols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if cols < 2 {
        return flatten(handlers);
    }

    let mut out = String::from("\n\n");
    for caption in &captions {
        out.push_str(caption);
        out.push_str("\n\n");
    }
    let row_line = |cells: &[String]| {
        let mut line = String::from("|");
        for i in 0..cols {
            line.push(' ');
            line.push_str(cells.get(i).map(String::as_str).unwrap_or(""));
            line.push_str(" |");
        }
        line.push('\n');
        line
    };
    out.push_str(&row_line(&rows[0]));
    out.push('|');
    for _ in 0..cols {
        out.push_str(" --- |");
    }
    out.push('\n');
    for row in &rows[1..] {
        out.push_str(&row_line(row));
    }
    out.push('\n');
    Some(out)
}

/// The sentinel handler registration shared by all three conversion scenarios
/// (the Readability main path, the whole-page fallback, a selection fragment);
/// skip_tags is the only thing that differs between them. Kept in one place —
/// the symptom of a missing registration is that path quietly leaking raw
/// `<x-vemry-*>`, visible only on the pages that take it.
fn sentinel_converter(skip_tags: &[&'static str]) -> htmd::HtmlToMarkdown {
    let mut builder = htmd::HtmlToMarkdown::builder();
    if !skip_tags.is_empty() {
        builder = builder.skip_tags(skip_tags.to_vec());
    }
    builder
        .add_handler(vec![MATH_TAG], |_: &dyn htmd::element_handler::Handlers, el: htmd::Element| {
            math_handler(el).map(Into::into)
        })
        .add_handler(vec![CODE_TAG], |_: &dyn htmd::element_handler::Handlers, el: htmd::Element| {
            code_handler(el).map(Into::into)
        })
        .add_handler(vec!["table"], |h: &dyn htmd::element_handler::Handlers, el: htmd::Element| {
            table_handler(h, &el).map(Into::into)
        })
        .add_handler(vec!["a"], |h: &dyn htmd::element_handler::Handlers, el: htmd::Element| {
            anchor_handler(h, &el).map(Into::into)
        })
        .build()
}

/// The longest content a citation marker like `[1]`/`[note 3]` can have, in bytes.
const CITE_MAX_BYTES: usize = 32;

/// htmd's conservative escaping is noise in a clipping context: the numbered
/// list guard `1\.`, the line-leading list markers `\-`/`\+`/`\>`, citation
/// markers `\[1\]` — the source text already looked like a list or a citation,
/// and the backslashes only make the note unreadable (measured: htmd escapes
/// these only at the start of a text block; an inline `3.5`/`A-B` is not escaped
/// anyway). So this un-escapes in a **controlled** way:
/// - `\.` everywhere (htmd only produces it as a list guard after a digit);
/// - `\-`/`\+`/`\>` only at the start of a line (where the restored form is the
///   natural list or quote);
/// - `\[x\]` only when the content is short, unnested, does not start with `^`
///   (footnote syntax), is not followed by `(` or `](` and is not preceded by `[`
///   (each would splice a link — or, as `[[1]]`, a wikilink) — what that matches
///   is a citation marker like `[1]`/`[note 3]`;
/// - `\\` (a backslash the source itself carried) is kept as is; `\*`/`\_`/`\#`
///   are left alone — removing them would conjure emphasis or headings out of
///   nothing, which is worse than the noise.
///
/// Code fences, inline code and `$…$` math spans are skipped wholesale: what the
/// sentinel handlers emit passes through verbatim, so every backslash in there is
/// real content (LaTeX's `\\`, a regex's `\.`).
/// CJK punctuation right inside an emphasis delimiter — `**…分析。**当然` — leaves the
/// `**` literal in every conforming renderer: CommonMark's flanking rule lets a
/// closer that follows punctuation close only when whitespace or punctuation
/// follows it, and `当` is neither. Moving the trailing punctuation out of the span
/// (`**…分析**。当然`) keeps the visible text and lets the emphasis parse; a leading
/// one (`当然**。x**`) is mirrored. Only `**` and `*` are handled, only outside
/// fences, only on one line.
fn cjk_emphasis(md: &str) -> String {
    let mut out = String::with_capacity(md.len());
    let mut fence = false;
    for (i, line) in md.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = !fence;
        }
        if fence || !line.contains('*') {
            out.push_str(line);
            continue;
        }
        out.push_str(&cjk_emphasis_line(line));
    }
    out
}

/// The punctuation that makes a delimiter unable to flank in CommonMark's reading.
const CJK_PUNCT: &str = "。，、；：！？）」』】》〉…—～（「『【《〈";
fn is_cjk_punct(c: char) -> bool {
    CJK_PUNCT.contains(c)
}

/// Copy the delimited span opening at `at` (a run of backticks or dollars) into `out`
/// verbatim, and answer where it ended; an unclosed opener is just its own characters.
fn copy_span(chars: &[char], at: usize, out: &mut Vec<char>) -> usize {
    let d = chars[at];
    let run = chars[at..].iter().take_while(|&&c| c == d).count();
    let mut j = at + run;
    while j < chars.len() {
        if chars[j] == d {
            let r = chars[j..].iter().take_while(|&&c| c == d).count();
            if r >= run {
                out.extend(&chars[at..j + run]);
                return j + run;
            }
            j += r;
            continue;
        }
        j += 1;
    }
    out.extend(&chars[at..at + run]);
    at + run
}

fn cjk_emphasis_line(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        // An inline code span or a `$…$` math span is content: copied through whole, the
        // same spans soften_escapes skips. Without this a `。` inside one moves out of it
        if chars[i] == '`' || chars[i] == '$' {
            i = copy_span(&chars, i, &mut out);
            continue;
        }
        if chars[i] != '*' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        // the delimiter run: `**` or `*` (three or more are left alone)
        let run = chars[i..].iter().take_while(|&&c| c == '*').count();
        if run > 2 {
            out.extend(&chars[i..i + run]);
            i += run;
            continue;
        }
        // the matching closer on this line, with no `*` inside
        let start = i + run;
        let mut j = start;
        let mut close = None;
        while j < chars.len() {
            if chars[j] == '*' {
                let r = chars[j..].iter().take_while(|&&c| c == '*').count();
                if r == run && j > start {
                    close = Some(j);
                }
                break;
            }
            j += 1;
        }
        let Some(close) = close else {
            out.extend(&chars[i..i + run]);
            i += run;
            continue;
        };
        let mut inner: Vec<char> = chars[start..close].to_vec();
        let after = chars.get(close + run).copied();
        let before = if i > 0 { Some(chars[i - 1]) } else { None };
        // trailing punctuation moves after the closer when a word follows it
        let mut tail: Vec<char> = Vec::new();
        if after.is_some_and(|c| !c.is_whitespace() && !is_cjk_punct(c) && !c.is_ascii_punctuation()) {
            while inner.last().is_some_and(|&c| is_cjk_punct(c)) {
                tail.insert(0, inner.pop().unwrap());
            }
        }
        // leading punctuation moves before the opener when a word precedes it
        let mut head: Vec<char> = Vec::new();
        if before.is_some_and(|c| !c.is_whitespace() && !is_cjk_punct(c) && !c.is_ascii_punctuation()) {
            while inner.first().is_some_and(|&c| is_cjk_punct(c)) {
                head.push(inner.remove(0));
            }
        }
        if inner.is_empty() || inner.iter().all(|c| c.is_whitespace()) {
            // nothing but punctuation inside: leave the span as it was
            out.extend(&chars[i..close + run]);
        } else {
            out.extend(head);
            out.extend(&chars[i..i + run]);
            out.extend(inner);
            out.extend(&chars[close..close + run]);
            out.extend(tail);
        }
        i = close + run;
    }
    out.into_iter().collect()
}

fn soften_escapes(md: &str) -> String {
    cjk_emphasis(&soften_escapes_inner(&strip_invisible(md)))
}

/// Zero-width spaces and byte-order marks a page carries in its text (break hints a site
/// scatters through long URLs and CJK runs, an editor's stray BOM): invisible on the page,
/// in the note they are characters that split a word for search and a link for the parser,
/// and an editor shows them as marks. Dropped on the way in. A soft hyphen stays — it is a
/// hyphenation hint the note's renderers honour at a line end — and so do the zero-width
/// joiner and non-joiner, which shape emoji sequences and Persian and Indic words.
fn strip_invisible(s: &str) -> std::borrow::Cow<'_, str> {
    const INVISIBLE: [char; 2] = ['\u{200B}', '\u{FEFF}'];
    if s.contains(INVISIBLE) {
        std::borrow::Cow::Owned(s.replace(INVISIBLE, ""))
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

fn soften_escapes_inner(md: &str) -> String {
    let b = md.as_bytes();
    // Every decision point is an ASCII byte; multi-byte UTF-8 is only ever moved
    // across verbatim, so accumulating byte by byte is safe
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    let mut line_start = true; // no character other than whitespace/quote markers yet on this line
    let mut fence: Option<(u8, usize)> = None; // fenced code block (character, length)
    let mut in_span: Option<u8> = None; // an inline `code` or $math$ span
    // Inside a bare URL (from `http(s)://` to the next whitespace or angle bracket): an
    // escaped `_` or `*` there is part of the address, and the backslash would only
    // break the link (`attention\_tf.py`)
    let mut in_url = false;
    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            out.push(b'\n');
            i += 1;
            line_start = true;
            in_span = None; // an inline span does not cross lines
            in_url = false;
            continue;
        }
        if in_url && (c == b' ' || c == b'\t' || c == b'<' || c == b'>' || c == b')' || c == b']') {
            in_url = false;
        }
        if !in_url && fence.is_none() && in_span.is_none() && (b[i..].starts_with(b"http://") || b[i..].starts_with(b"https://")) {
            in_url = true;
        }
        if in_url && c == b'\\' && i + 1 < b.len() && (b[i + 1] == b'_' || b[i + 1] == b'*') {
            out.push(b[i + 1]);
            i += 2;
            line_start = false;
            continue;
        }
        // Fence open/close (only at the start of a line): the parser's rule, ≥3 `/~
        if line_start && (c == b'`' || c == b'~') {
            let run = b[i..].iter().take_while(|&&x| x == c).count();
            if run >= 3 {
                match fence {
                    Some((ch, len)) if ch == c && run >= len => fence = None,
                    None => fence = Some((c, run)),
                    _ => {}
                }
                out.extend_from_slice(&b[i..i + run]);
                i += run;
                line_start = false;
                continue;
            }
        }
        if fence.is_some() {
            out.push(c);
            i += 1;
            continue;
        }
        // An inline code/math span: copied through verbatim
        if let Some(open) = in_span {
            if c == open {
                in_span = None;
            }
            out.push(c);
            i += 1;
            continue;
        }
        if c == b'`' || c == b'$' {
            // A `$` is usually currency or an ordinary character (htmd does not
            // escape it): only when the rest of the line holds a matching `$` is
            // it skipped as a math span, because a lone `$` would switch
            // un-escaping off for everything after it on that line — and the
            // citation noise in `share price $189, research shows\[1\]` is exactly
            // what we are here to remove.
            // A backtick needs no such test: a ` in text becomes \` under htmd, so
            // a bare ` only ever comes out of the sentinel/code handlers and is
            // necessarily paired.
            let paired = c == b'`' || b[i + 1..].iter().take_while(|&&x| x != b'\n').any(|&x| x == b'$');
            if paired {
                in_span = Some(c);
                out.push(c);
                i += 1;
                line_start = false;
                continue;
            }
            // A lone $ falls through as an ordinary character (through the
            // line-start check and the byte-by-byte copy)
        }
        if c == b'\\' && i + 1 < b.len() {
            let n = b[i + 1];
            if n == b'\\' {
                out.extend_from_slice(b"\\\\");
                i += 2;
                line_start = false;
                continue;
            }
            if n == b'.' {
                out.push(b'.');
                i += 2;
                line_start = false;
                continue;
            }
            if (n == b'-' || n == b'+' || n == b'>') && line_start {
                out.push(n);
                i += 2;
                // After a restored quote marker it still counts as the start of a
                // line (so the second level of `> \- x` can be restored too)
                line_start = n == b'>';
                continue;
            }
            // A `[` just before would make the restored marker a wikilink (`[[1]]`)
            if n == b'[' && out.last() != Some(&b'[') {
                if let Some(inner_len) = citation_len(&b[i + 2..], CITE_MAX_BYTES) {
                    out.push(b'[');
                    out.extend_from_slice(&b[i + 2..i + 2 + inner_len]);
                    out.push(b']');
                    i += 2 + inner_len + 2; // \[ content \]
                    line_start = false;
                    continue;
                }
            }
            // Every other escape is kept as written
            out.push(b'\\');
            out.push(n);
            i += 2;
            line_start = false;
            continue;
        }
        if line_start && !(c == b' ' || c == b'\t' || c == b'>') {
            line_start = false;
        }
        out.push(c);
        i += 1;
    }
    String::from_utf8(out).expect("only original bytes moved and ASCII inserted")
}

/// Whether what follows `\[` is a citation marker that is safe to restore;
/// returns the content's byte length (up to the matching `\]`).
/// Rejected: too long, containing brackets/backslashes/newlines, starting with
/// `^` (a footnote), or a `\]` followed immediately by `(` or by `](` — the
/// marker is a link's text, or a link's whole text, and restoring it would splice
/// a Markdown link or turn `[\[1\]](url)` into a wikilink followed by a bare URL.
fn citation_len(rest: &[u8], max: usize) -> Option<usize> {
    let mut j = 0;
    while j < rest.len() && j <= max {
        match rest[j] {
            b'\\' if rest.get(j + 1) == Some(&b']') => {
                let next = rest.get(j + 2);
                let links = next == Some(&b'(') || (next == Some(&b']') && rest.get(j + 3) == Some(&b'('));
                if j == 0 || rest[0] == b'^' || links {
                    return None;
                }
                return Some(j);
            }
            b'[' | b']' | b'\\' | b'\n' => return None,
            _ => j += 1,
        }
    }
    None
}

/// Readability extracts the body → htmd converts it to Markdown; a failed
/// extraction falls back to converting the whole page.
pub fn extract_page(html: &str, url: &str) -> Result<ExtractedPage, ClipError> {
    if html.trim().is_empty() {
        return Err(ClipError::EmptyPage);
    }

    // Main path: dom_smoothie (a Readability port) extracts the body
    let unclassed = unclass_tables(html);
    let html: &str = &unclassed;
    let article = dom_smoothie::Readability::new(html, Some(url), None)
        .ok()
        .and_then(|mut r| r.parse().ok());
    if let Some(article) = article {
        if let Ok(md) = sentinel_converter(&[]).convert(article.content.as_ref()) {
            let md = md.trim();
            if !md.is_empty() {
                return Ok(ExtractedPage {
                    markdown: soften_escapes(md),
                    title: strip_invisible(&article.title).trim().to_string(),
                    byline: article.byline.filter(|b| !b.trim().is_empty()),
                    fallback: false,
                });
            }
        }
    }

    // Fallback: the whole page through htmd, skipping the obvious non-body tags
    let md = sentinel_converter(&["script", "style", "noscript", "iframe", "svg", "head", "nav", "footer", "form"])
        .convert(html)
        .map_err(|e| ClipError::Extract(e.to_string()))?;
    let md = md.trim();
    if md.is_empty() {
        return Err(ClipError::EmptyPage);
    }
    Ok(ExtractedPage { markdown: soften_escapes(md), title: String::new(), byline: None, fallback: true })
}

/// The `class` attribute dropped from every `<table>` start tag. Readability removes any
/// element whose class or id says one of its "unlikely candidate" words (header, footer,
/// menu, comment…) — a rule for page chrome that a data table's class can trip by
/// accident: arXiv's HTML marks a table whose header row it inferred
/// `ltx_guessed_headers`, and Table 1 of a paper vanished with all its rows while its
/// caption stayed. A table's class carries nothing the extraction needs (classes are not
/// kept in the output; a layout table is still told apart by its shape), so the tag is
/// handed to Readability without it. The tag is scanned as text — a start tag holds no
/// markup — and everything else is copied through untouched.
fn unclass_tables(html: &str) -> std::borrow::Cow<'_, str> {
    if table_tag_at(html).is_none() {
        return std::borrow::Cow::Borrowed(html); // the common page has no table at all
    }
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = table_tag_at(rest) {
        let (before, tag_on) = rest.split_at(at);
        out.push_str(before);
        let end = tag_on.find('>').map_or(tag_on.len(), |e| e + 1);
        let (tag, tail) = tag_on.split_at(end);
        out.push_str(&strip_class_attr(tag));
        rest = tail;
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
}

/// The offset of the next `<table` start tag (any case; `<tablet>` is not one).
fn table_tag_at(s: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = s[from..].find('<') {
        let i = from + rel;
        if let Some(name) = s.get(i + 1..i + 6) {
            if name.eq_ignore_ascii_case("table") {
                let next = s.as_bytes().get(i + 6);
                if next.is_none_or(|b| b.is_ascii_whitespace() || matches!(b, b'>' | b'/')) {
                    return Some(i);
                }
            }
        }
        from = i + 1;
    }
    None
}

/// One start tag with its `class` attribute removed: quoted (either quote) or bare
/// values, the attribute name in any case. The tag is walked attribute by attribute, so
/// `class=` inside another attribute's value (`title="a class=b"`) is text — searching for
/// the spelling instead would cut the tag in half there — and an attribute whose name
/// merely ends in it (`data-class`) is not the one dropped.
fn strip_class_attr(tag: &str) -> String {
    let b = tag.as_bytes();
    let mut out = String::with_capacity(tag.len());
    // the tag name (`<table`), up to the first space
    let mut i = b.iter().position(|c| c.is_ascii_whitespace()).unwrap_or(b.len());
    out.push_str(&tag[..i]);
    while i < b.len() {
        let from = i; // the whitespace before the attribute belongs to it
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= b.len() || b[i] == b'>' || b[i] == b'/' {
            out.push_str(&tag[from..]);
            return out;
        }
        let name_start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let name = &tag[name_start..i];
        let mut j = i;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if b.get(j) == Some(&b'=') {
            j += 1;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            match b.get(j) {
                Some(&q @ (b'"' | b'\'')) => {
                    j += 1;
                    while j < b.len() && b[j] != q {
                        j += 1;
                    }
                    j = (j + 1).min(b.len());
                }
                _ => {
                    while j < b.len() && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                        j += 1;
                    }
                }
            }
            i = j;
        }
        if !name.eq_ignore_ascii_case("class") {
            out.push_str(&tag[from..i]);
        }
    }
    out
}

/// A selection's DOM fragment → Markdown: the same htmd configuration as the
/// whole-page fallback (math sentinel handler plus skipped non-content tags) but
/// without Readability — a selection is the body, as drawn by the user. Relative
/// URLs were already made absolute by the grabbing JS, so no base is needed here.
/// When nothing converts (an empty or purely decorative fragment) it returns an
/// empty string and the caller falls back to the plain-text selection.
pub fn fragment_to_markdown(html: &str) -> String {
    if html.trim().is_empty() {
        return String::new();
    }
    sentinel_converter(&["script", "style", "noscript", "iframe", "svg", "head"])
        .convert(html)
        .map(|md| soften_escapes(md.trim()))
        .unwrap_or_default()
}

/// The full text of a whole-page clip note: frontmatter (source/clipped are
/// ordinary top-level keys, preserved across a round trip; the origin marker
/// lives in the `vemry: origin: clip` namespace) + the source line + the body.
/// `source_label` is the interface-language word for "Source" (the glue layer
/// passes it from `locales/*.json` — this crate stays copy-free, as core does).
pub fn compose_clip_note(page: &ExtractedPage, meta: &ClipMeta, source_label: &str) -> String {
    let title = if page.title.trim().is_empty() { meta.display_title() } else { page.title.trim().to_string() };

    let mut fm = Frontmatter {
        title: Some(title),
        vemry: Some(VemryMeta { origin: Some("clip".into()), agent_session: None }),
        ..Frontmatter::default()
    };
    fm.extra.insert("source".into(), meta.url.clone().into());
    // Clips no longer land in a per-domain directory — the domain is kept as
    // metadata, and grouping by site is a query projection
    if let Some(domain) = domain_of(&meta.url) {
        fm.extra.insert("domain".into(), domain.into());
    }
    fm.extra
        .insert("clipped".into(), meta.clipped_at.to_rfc3339_opts(SecondsFormat::Secs, false).into());
    if page.fallback {
        fm.extra.insert("extraction".into(), "fallback".into());
    }

    let yaml = fm.to_yaml().unwrap_or_default();
    let mut out = format!("---\n{yaml}---\n\n> {source_label}: <{}>", meta.url);
    if let Some(byline) = &page.byline {
        // The byline is page content extracted by Readability: an embedded
        // newline would break out of the source line and become a phantom block
        // (or even a phantom #tag) in the note body, so squash it onto one line
        // before splicing
        let one_line = byline.split_whitespace().collect::<Vec<_>>().join(" ");
        if !one_line.is_empty() {
            out.push_str(&format!(" · {one_line}"));
        }
    }
    out.push_str("\n\n");
    out.push_str(&page.markdown);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

// The clips directory literal is written only inside this macro: concatenating
// &str in const position is something only concat! can do, and concat! takes
// literals only, so the literal sinks into a macro and both CLIPS_DIR and
// ASSETS_DIR derive from it — moving the directory is a one-line change, and the
// compiler holds the derivation together.
macro_rules! clips_dir {
    () => {
        "clips/"
    };
}

/// The directory prefix clip notes land in (relative to the vault root). The
/// destination, the asset directory and the host-side clips/ checks (reverse
/// lookup, the partial marker) all derive from here — moving the directory means
/// editing clips_dir! and nothing else.
pub const CLIPS_DIR: &str = clips_dir!();

/// The destination path: `clips/<sanitized title>.md`, flat rather than split
/// into per-domain directories (same-name deduplication is Vault::dedup_rel's
/// job). Retrieval works by topic, not by source site — the domain goes into the
/// frontmatter (`domain:`), and "browse by site" is left to a projection over
/// query().
pub fn clip_rel_path(meta: &ClipMeta) -> String {
    let mut title = vemry_core::sanitize_filename(&meta.display_title());
    // Keep file names from growing too long (path limits, a readable file tree)
    if title.chars().count() > 80 {
        title = title.chars().take(80).collect::<String>().trim_end().to_string();
    }
    format!("{CLIPS_DIR}{title}.md")
}

/// The body length past which a quote landing in a note opens folded: about four
/// lines of prose at the readable width, where a quotation stops being a passage
/// and becomes a wall the note is *not* about.
pub const FOLD_MIN_CHARS: usize = 300;
/// The line count past which it opens folded as well (a table, a list).
pub const FOLD_MIN_LINES: usize = 8;

/// A selection → a quote callout: `> [!quote] [title](url) · date` as the head
/// line, the selection as the body with a `> ` prefix per line. The source sits
/// where every callout renderer puts a title — icon, tint, and the one line that
/// stays visible while the body is folded — and a standard renderer shows an
/// ordinary blockquote whose first line reads `[!quote]`.
/// `foldable`: a long body opens folded (the `-` marker) when the quote lands in a
/// note that is about something else; a clip note that is nothing but the quote
/// passes false and opens expanded. The fold state itself is never written back.
pub fn quote_block(selection: &str, meta: &ClipMeta, foldable: bool) -> String {
    // the plain-text selection (the fallback when its HTML converted to nothing) has not
    // been through soften_escapes, which drops the invisible characters on the other path
    let cleaned = strip_invisible(selection);
    let trimmed = cleaned.trim_matches('\n');
    let long = trimmed.chars().count() > FOLD_MIN_CHARS || trimmed.lines().count() > FOLD_MIN_LINES;
    let marker = if foldable && long { "-" } else { "" };
    let mut out = format!(
        "> [!quote]{marker} {} · {}\n",
        md_link(&meta.display_title(), &meta.url),
        meta.clipped_at.format("%Y-%m-%d")
    );
    for line in trimmed.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            out.push_str(">\n");
        } else {
            out.push_str("> ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Clipping a link: a `[title](url)` reference entry.
pub fn link_entry(meta: &ClipMeta) -> String {
    md_link(&meta.display_title(), &meta.url)
}

/// One image reference in the body waiting to be localised. `literal` is the
/// text inside the `![..](..)` parentheses as written, `resolved` the absolute
/// http(s) address it makes once resolved against the page URL (the fallback
/// path often produces relative or protocol-relative addresses; the Readability
/// main path has already resolved them).
#[derive(Clone, Debug, PartialEq)]
pub struct ImageRef {
    pub literal: String,
    pub resolved: String,
}

/// The directory localised assets land in (relative to the vault root). The
/// editor resolves `![](path)` against the vault root, so a rewrite must use the
/// full path with this prefix; while scanning, a target already carrying this
/// prefix counts as localised and is skipped.
pub const ASSETS_DIR: &str = concat!(clips_dir!(), "assets/");

/// Find every `![alt](target)` / `![alt](target "title")` one by one and hand the
/// callback the (whole start, whole end, target start, target end) byte offsets.
/// Fenced code blocks and inline code are always skipped — a code sample in a
/// clipped page (a Markdown syntax tutorial, say) must never be rewritten.
fn scan_images(md: &str, mut on_image: impl FnMut(usize, usize, usize, usize)) {
    // (fence character, opening length): the closer must use the same character
    // and be at least as long as the opener (CommonMark semantics). A single
    // boolean would be flipped by the other fence character appearing inside the
    // block (a ~~~ tutorial sample inside a ``` block), and images inside a code
    // sample would then be rewritten for real.
    let mut fence: Option<(u8, usize)> = None;
    let mut pos = 0;
    for line in md.split_inclusive('\n') {
        let line_start = pos;
        pos += line.len();
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let marker = trimmed.as_bytes().first().copied();
        // Indented 4 or more, CommonMark calls it indented code or fence content,
        // not a fence line
        if indent < 4 && matches!(marker, Some(b'`') | Some(b'~')) {
            let ch = marker.unwrap();
            let run = trimmed.bytes().take_while(|&b| b == ch).count();
            if run >= 3 {
                match fence {
                    None => {
                        fence = Some((ch, run));
                        continue;
                    }
                    Some((open_ch, open_len)) => {
                        // Closing: same character, no shorter than the opener and
                        // nothing but trailing whitespace after it; a fence line
                        // that does not match is code content and stays inside
                        if ch == open_ch && run >= open_len && trimmed[run..].trim().is_empty() {
                            fence = None;
                        }
                        continue;
                    }
                }
            }
        }
        if fence.is_some() {
            continue;
        }
        let lb = line.as_bytes();
        let mut i = 0;
        while i < lb.len() {
            if lb[i] == b'`' {
                // Inline code: backtick runs pair by equal length per CommonMark,
                // and the whole span is skipped
                let run_start = i;
                while i < lb.len() && lb[i] == b'`' {
                    i += 1;
                }
                let run = i - run_start;
                let mut j = i;
                loop {
                    let mut k = j;
                    while k < lb.len() && lb[k] != b'`' {
                        k += 1;
                    }
                    if k >= lb.len() {
                        break; // unclosed: a literal backtick
                    }
                    let mut m = k;
                    while m < lb.len() && lb[m] == b'`' {
                        m += 1;
                    }
                    if m - k == run {
                        i = m;
                        break;
                    }
                    j = m;
                }
                continue;
            }
            if lb[i] == b'!' && i + 1 < lb.len() && lb[i + 1] == b'[' {
                if let Some((len, ts, te)) = parse_image(&line[i..]) {
                    on_image(line_start + i, line_start + i + len, line_start + i + ts, line_start + i + te);
                    i += len;
                    continue;
                }
            }
            i += 1;
        }
    }
}

/// Parse one image reference starting at `![`, returning (whole byte length,
/// target start, target end).
fn parse_image(s: &str) -> Option<(usize, usize, usize)> {
    let b = s.as_bytes();
    let mut i = 2;
    while i < b.len() {
        match b[i] {
            b'\\' => i += 2,
            b']' => break,
            _ => i += 1,
        }
    }
    if i >= b.len() || b[i] != b']' {
        return None;
    }
    i += 1;
    if i >= b.len() || b[i] != b'(' {
        return None;
    }
    i += 1;
    let target_start = i;
    while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b')' {
        i += 1;
    }
    let target_end = i;
    while i < b.len() && b[i] != b')' {
        i += 1;
    }
    if i >= b.len() {
        return None;
    }
    Some((i + 1, target_start, target_end))
}

/// Collect the localisable image references in the body, resolving relative
/// addresses against the page URL. Inline `data:` images, targets already
/// localised (carrying the ASSETS_DIR prefix) and targets that fail to parse or
/// are not http(s) are skipped.
pub fn image_refs(markdown: &str, base_url: &str) -> Vec<ImageRef> {
    let base = url::Url::parse(base_url).ok();
    let mut out = Vec::new();
    scan_images(markdown, |_s, _e, ts, te| {
        let target = &markdown[ts..te];
        if target.is_empty()
            || target.starts_with('#')
            || target.starts_with("data:")
            || target.starts_with(ASSETS_DIR)
        {
            return;
        }
        let resolved = match &base {
            Some(b) => b.join(target).ok(),
            None => url::Url::parse(target).ok(),
        };
        let Some(resolved) = resolved else { return };
        if matches!(resolved.scheme(), "http" | "https") {
            out.push(ImageRef { literal: target.to_string(), resolved: resolved.to_string() });
        }
    });
    out
}

/// Swap the target of `![alt](target)` for a local path per the mapping: alt is
/// kept as written, the title dropped (a local file needs none). References
/// outside the mapping, and samples inside code blocks, are left untouched.
pub fn rewrite_images(markdown: &str, replace: &std::collections::HashMap<String, String>) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut last = 0;
    scan_images(markdown, |_s, e, ts, te| {
        if let Some(local) = replace.get(&markdown[ts..te]) {
            out.push_str(&markdown[last..ts]);
            out.push_str(local);
            out.push(')');
            last = e;
        }
    });
    out.push_str(&markdown[last..]);
    out
}

/// Determine the image format from its magic bytes and return the extension to
/// save it under; unrecognised (or not an image) returns None. The extension is
/// never taken from the URL — a URL can lie, bytes cannot. SVG is a text format
/// with no magic bytes, so it is recognised by text features in the first 1KB; it
/// is only ever rendered through `<img>`, where scripts do not execute.
pub fn image_ext_of(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("png");
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return Some("jpg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("gif");
    }
    if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Some("webp");
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" && (&bytes[8..12] == b"avif" || &bytes[8..12] == b"avis") {
        return Some("avif");
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]);
    let t = head.trim_start_matches('\u{feff}').trim_start();
    if t.starts_with("<svg") || (t.starts_with("<?xml") && head.contains("<svg")) {
        return Some("svg");
    }
    None
}

/// Mark the frontmatter `images: partial` (some images could not be localised
/// and the body still holds remote URLs). Already marked, or no frontmatter at
/// all, returns the note unchanged — idempotent, so it can be called again after
/// each later top-up.
pub fn mark_images_partial(note: &str) -> String {
    let Some(rest) = note.strip_prefix("---\n") else { return note.to_string() };
    let Some(close) = rest.find("\n---\n").map(|i| i + 1) else { return note.to_string() };
    let fm = &rest[..close];
    if fm.lines().any(|l| l.trim_start().starts_with("images:")) {
        return note.to_string();
    }
    format!("---\n{fm}images: partial\n{}", &rest[close..])
}

/// A Markdown link, with square brackets in the text escaped and parentheses or
/// spaces in the URL encoded per CommonMark.
fn md_link(text: &str, url: &str) -> String {
    let text = text.replace('[', "\\[").replace(']', "\\]");
    format!("[{text}]({})", encode_link_url(url))
}

/// A link destination as it goes inside `(…)`: parentheses and spaces
/// percent-encoded, so no backslash escapes or `<>` wrapping are ever needed.
fn encode_link_url(url: &str) -> String {
    url.chars()
        .map(|c| match c {
            '(' => "%28".to_string(),
            ')' => "%29".to_string(),
            ' ' => "%20".to_string(),
            c => c.to_string(),
        })
        .collect()
}

fn domain_of(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    Some(host.strip_prefix("www.").unwrap_or(host).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_class_is_dropped_before_readability() {
        assert_eq!(
            unclass_tables(r#"<p class="a">x</p><TABLE id="t" class="ltx_tabular ltx_guessed_headers"><tr><td class="c">1</td></tr></TABLE><table class='x'/><tablet class="k">"#),
            r#"<p class="a">x</p><TABLE id="t"><tr><td class="c">1</td></tr></TABLE><table/><tablet class="k">"#
        );
        // an attribute that merely ends in the name is not it, and the spelling inside
        // another attribute's value is text — neither may cut the tag short
        assert_eq!(unclass_tables(r#"<table data-class="a" class="ltx_guessed_headers"><tr>"#), r#"<table data-class="a"><tr>"#);
        assert_eq!(unclass_tables(r#"<table title="a class=b" class="x" id=t><tr>"#), r#"<table title="a class=b" id=t><tr>"#);
        // a data table whose class says "header" survives the extraction with its rows
        let para = "<p>Recurrent neural networks, long short-term memory and gated recurrent neural networks in particular, have been firmly established as state of the art approaches in sequence modeling and transduction problems such as language modeling and machine translation.</p>".repeat(8);
        let html = format!(
            "<html><body><article><h1>T</h1>{para}<figure><table class=\"ltx_tabular ltx_guessed_headers\"><thead><tr><th>Layer Type</th><th>Complexity</th></tr></thead><tbody><tr><td>Self-Attention</td><td>n2d</td></tr><tr><td>Recurrent</td><td>nd2</td></tr></tbody></table><figcaption>Table 1: caption</figcaption></figure>{para}</article></body></html>"
        );
        let page = extract_page(&html, "https://example.com/paper").unwrap();
        assert!(!page.fallback);
        assert!(page.markdown.contains("| Self-Attention | n2d |"), "{}", page.markdown);
    }
    use chrono::TimeZone;
    use vemry_core::parser::parse_note;

    fn meta() -> ClipMeta {
        ClipMeta {
            url: "https://example.com/article?id=1#section-2".into(),
            title: "测试文章".into(),
            clipped_at: Local.with_ymd_and_hms(2026, 8, 11, 10, 30, 0).unwrap(),
        }
    }

    const ARTICLE_HTML: &str = r#"<!DOCTYPE html>
<html><head><title>页面标题</title></head><body>
<nav><a href="/">首页</a><a href="/about">关于</a></nav>
<article>
<h1>正文标题</h1>
<p>这是第一段正文,长度要够 Readability 认为它是正文内容。它介绍了一个重要的概念,并且给出了详细的解释和示例。</p>
<p>这是第二段,包含一个<a href="https://other.example.org/doc#anchor">外部链接</a>和一些<strong>加粗文字</strong>。</p>
<table><tr><th>列一</th><th>列二</th></tr><tr><td>甲</td><td>乙</td></tr></table>
</article>
<footer>页脚版权信息</footer>
</body></html>"#;

    #[test]
    fn extract_page_gets_article_body() {
        let page = extract_page(ARTICLE_HTML, "https://example.com/article").unwrap();
        assert!(!page.fallback);
        assert!(page.markdown.contains("第一段正文"), "markdown: {}", page.markdown);
        assert!(page.markdown.contains("加粗文字"));
        // Navigation and footer stay out of the body
        assert!(!page.markdown.contains("页脚版权"));
    }

    #[test]
    fn extract_page_falls_back_on_unreadable_html() {
        let page = extract_page("<p>短</p>", "https://example.com/x").unwrap();
        assert!(page.markdown.contains('短'));
    }

    #[test]
    fn extract_page_rejects_empty() {
        assert!(matches!(extract_page("  ", "https://example.com/"), Err(ClipError::EmptyPage)));
    }

    #[test]
    fn compose_roundtrips_through_core_parser() {
        let page = extract_page(ARTICLE_HTML, "https://example.com/article?id=1").unwrap();
        let note = compose_clip_note(&page, &meta(), "Source");
        let parsed = parse_note(&note);
        assert_eq!(
            parsed.frontmatter.extra.get("source").and_then(|v| v.as_str()),
            Some("https://example.com/article?id=1#section-2")
        );
        assert_eq!(
            parsed.frontmatter.extra.get("domain").and_then(|v| v.as_str()),
            Some("example.com")
        );
        assert!(parsed.frontmatter.extra.get("clipped").is_some());
        assert_eq!(parsed.frontmatter.vemry.as_ref().unwrap().origin.as_deref(), Some("clip"));
        assert!(parsed.frontmatter.title.is_some());
    }

    #[test]
    fn urls_with_fragments_produce_no_phantom_tags() {
        // A bare URL's #fragment is read as a tag by the inline scanner — a clip
        // must never produce one
        let page = extract_page(ARTICLE_HTML, "https://example.com/article").unwrap();
        let note = compose_clip_note(&page, &meta(), "Source");
        let parsed = parse_note(&note);
        let tags: Vec<String> = parsed.blocks.iter().flat_map(|b| b.tags.clone()).collect();
        assert!(tags.is_empty(), "the clip produced a phantom tag: {tags:?}\n---\n{note}");
    }

    // Math sentinel: the grabbing JS has already normalised KaTeX/MathJax/MathML
    // into <x-vemry-math>, and this asserts that both conversion paths (the
    // Readability main path and the whole-page fallback) emit `$` syntax, and
    // that LaTeX backslashes and underscores are not escaped by htmd.
    const MATH_HTML: &str = r#"<!DOCTYPE html>
<html><head><title>公式页</title></head><body>
<article>
<h1>二次方程</h1>
<p>这是第一段正文,长度要够 Readability 认为它是正文内容。它介绍了求根公式的推导过程,并且给出了详细的解释和示例。</p>
<p>行内公式 <x-vemry-math>E_0 = m c^2</x-vemry-math> 出现在句子里。</p>
<p><x-vemry-math display="block">x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}</x-vemry-math></p>
<p>这是收尾的一段正文,同样保持足够的长度,让抽取器不至于把整篇文章判为不可读的碎片内容。</p>
</article>
</body></html>"#;

    #[test]
    fn math_sentinel_survives_readability_path() {
        let page = extract_page(MATH_HTML, "https://example.com/math").unwrap();
        assert!(!page.fallback, "must take the Readability main path: {}", page.markdown);
        assert!(page.markdown.contains("$E_0 = m c^2$"), "inline math: {}", page.markdown);
        assert!(
            page.markdown.contains(r"$$x = \frac{-b \pm \sqrt{b^2 - 4ac}}{2a}$$"),
            "block math (backslashes must not be escaped): {}",
            page.markdown
        );
    }

    #[test]
    fn math_sentinel_survives_fallback_path() {
        // A short page takes the whole-page fallback
        let page = extract_page(
            r#"<p>短 <x-vemry-math>a_1 + b_2</x-vemry-math></p>"#,
            "https://example.com/x",
        )
        .unwrap();
        assert!(page.markdown.contains("$a_1 + b_2$"), "{}", page.markdown);
    }

    #[test]
    fn math_sentinel_collapses_whitespace_and_skips_empty() {
        let page = extract_page(
            "<p>短 <x-vemry-math>a\n  +\n  b</x-vemry-math> 与空 <x-vemry-math>  </x-vemry-math> 哨兵</p>",
            "https://example.com/x",
        )
        .unwrap();
        assert!(page.markdown.contains("$a + b$"), "multi-line LaTeX squashed onto one line: {}", page.markdown);
        assert!(!page.markdown.contains("$$"), "an empty sentinel emits no delimiters: {}", page.markdown);
    }

    // Code sentinel: the grabbing JS has already turned lake cards and per-line
    // rendered code viewers back into <x-vemry-code>, and this asserts that both
    // conversion paths emit a fenced block and that the line structure — and a
    // line-leading `#` (a shell comment, which becomes a phantom heading without
    // the normalisation) — survives verbatim.
    const CODE_HTML: &str = "<!DOCTYPE html>\n<html><head><title>代码页</title></head><body>\n<article>\n<h1>训练示例</h1>\n<p>这是第一段正文,长度要够 Readability 认为它是正文内容。它介绍了模型微调的整体流程,并且给出了详细的解释和示例。</p>\n<x-vemry-code lang=\"bash\"># 4 * 30GiB\nPYTORCH_CUDA_ALLOC_CONF='expandable_segments:True' \\\ngit clone https://github.com/modelscope/ms-swift.git\ncd ms-swift</x-vemry-code>\n<p>这是收尾的一段正文,同样保持足够的长度,让抽取器不至于把整篇文章判为不可读的碎片内容。</p>\n</article>\n</body></html>";

    #[test]
    fn code_sentinel_survives_readability_path() {
        let page = extract_page(CODE_HTML, "https://example.com/code").unwrap();
        assert!(!page.fallback, "must take the Readability main path: {}", page.markdown);
        assert!(
            page.markdown.contains(
                "```bash\n# 4 * 30GiB\nPYTORCH_CUDA_ALLOC_CONF='expandable_segments:True' \\\ngit clone https://github.com/modelscope/ms-swift.git\ncd ms-swift\n```"
            ),
            "line structure or fence lost: {}",
            page.markdown
        );
        // Both the # comment and the bare URL in the code stay shut inside the
        // fence: no phantom heading, no phantom tag
        let note = compose_clip_note(&page, &meta(), "Source");
        let parsed = parse_note(&note);
        let tags: Vec<String> = parsed.blocks.iter().flat_map(|b| b.tags.clone()).collect();
        assert!(tags.is_empty(), "code content leaked out of the fence and made a phantom tag: {tags:?}\n---\n{note}");
    }

    #[test]
    fn code_sentinel_survives_fallback_path() {
        let page = extract_page(
            "<p>短</p><x-vemry-code>line1\nline2</x-vemry-code>",
            "https://example.com/x",
        )
        .unwrap();
        assert!(page.markdown.contains("```\nline1\nline2\n```"), "{}", page.markdown);
    }

    #[test]
    fn code_sentinel_escalates_fence_and_sanitizes_lang() {
        // The content holds a ``` line: the fence must be longer than the longest
        // backtick run inside, or it closes early
        let page = extract_page(
            "<p>短</p><x-vemry-code lang=\"mark down`\">```\ninner\n```</x-vemry-code>",
            "https://example.com/x",
        )
        .unwrap();
        assert!(
            page.markdown.contains("````markdown\n```\ninner\n```\n````"),
            "{}",
            page.markdown
        );
        // An empty sentinel emits no fence
        let empty = extract_page(
            "<p>短 <x-vemry-code>  \n </x-vemry-code></p>",
            "https://example.com/x",
        )
        .unwrap();
        assert!(!empty.markdown.contains("```"), "{}", empty.markdown);
    }

    // htmd escape de-noising: list guards and citation markers restored, code and
    // math spans untouched. Measured on a real page: an Alibaba Cloud article's
    // 「1\. 统一视觉语言基础」.
    #[test]
    fn soften_escapes_restores_list_guards_and_citations() {
        // The digit guard is restored bare at line start, inside bold and inside a heading
        assert_eq!(
            fragment_to_markdown("<p>1. 统一视觉语言基础 (Unified Vision-Language Foundation)</p>"),
            "1. 统一视觉语言基础 (Unified Vision-Language Foundation)"
        );
        assert_eq!(fragment_to_markdown("<p><strong>2. 高效混合架构</strong></p>"), "**2. 高效混合架构**");
        assert_eq!(fragment_to_markdown("<h3>3. 百万 Agent</h3>"), "### 3. 百万 Agent");
        // Line-leading list markers restored; an inline hyphen was never escaped by htmd
        assert_eq!(fragment_to_markdown("<p>- 减号开头 Qwen3-VL</p>"), "- 减号开头 Qwen3-VL");
        // Citation markers restored; a link shape ([x] followed by parentheses) and a footnote (leading ^) are left alone
        assert_eq!(fragment_to_markdown("<p>研究表明[1],性能提升[注 3]。</p>"), "研究表明[1],性能提升[注 3]。");
        assert_eq!(fragment_to_markdown("<p>[x](见下)</p>"), "\\[x\\](见下)");
        assert_eq!(fragment_to_markdown("<p>[^1]</p>"), "\\[^1\\]");
        // A marker that is a link's whole text, or sits right after `[`, stays escaped:
        // restored, `[[1]](url)` would be a wikilink followed by a bare URL
        assert_eq!(soften_escapes("研究[\\[1\\]](https://x/y#c)表明"), "研究[\\[1\\]](https://x/y#c)表明");
        assert_eq!(soften_escapes("[\\[1\\]]"), "[\\[1\\]]");
        // Emphasis and heading escapes are kept: dropping them would change the structure out of nothing
        assert_eq!(fragment_to_markdown("<p>2*3=6 且 *强调*不是</p>"), "2\\*3=6 且 \\*强调\\*不是");
        assert_eq!(fragment_to_markdown("<p># 不是标题</p>"), "\\# 不是标题");
        // A backslash the source itself carried (htmd turns it into \\) is kept verbatim
        assert_eq!(fragment_to_markdown("<p>正则 \\. 匹配</p>"), "正则 \\\\. 匹配");
        // An underscore inside a bare URL is part of the address: no backslash there,
        // while the same escape in prose stays
        assert_eq!(
            fragment_to_markdown("<p>见 https://github.com/bojone/attention/blob/master/attention_tf.py 与 snake_case 变量</p>"),
            "见 https://github.com/bojone/attention/blob/master/attention_tf.py 与 snake\\_case 变量"
        );
    }

    #[test]
    fn cjk_punctuation_leaves_the_emphasis_span() {
        assert_eq!(cjk_emphasis("**在本篇文章中，做一点简单的分析。**当然，这两篇"), "**在本篇文章中，做一点简单的分析**。当然，这两篇");
        assert_eq!(cjk_emphasis("他说*可以！*然后走了"), "他说*可以*！然后走了");
        assert_eq!(cjk_emphasis("看这个**（注）说明**"), "看这个（**注）说明**"); // a leading opener moves before it
        // a closer followed by a space or the line's end needs nothing; a fence is left alone
        assert_eq!(cjk_emphasis("**分析。** 当然"), "**分析。** 当然");
        assert_eq!(cjk_emphasis("**分析。**"), "**分析。**");
        assert_eq!(cjk_emphasis("```\n**a。**b\n```"), "```\n**a。**b\n```");
        assert_eq!(cjk_emphasis("**。**当"), "**。**当");
        // an inline code or math span is content: the punctuation inside it stays put
        assert_eq!(cjk_emphasis("代码 `*p。*q` 结束"), "代码 `*p。*q` 结束");
        assert_eq!(cjk_emphasis("公式 $a*b。*c$ 之后**分析。**当"), "公式 $a*b。*c$ 之后**分析**。当");
        assert_eq!(fragment_to_markdown("<p><strong>在本篇文章中，分析。</strong>当然</p>"), "**在本篇文章中，分析**。当然");
    }

    #[test]
    fn invisible_characters_are_dropped_and_soft_hyphens_kept() {
        let md = fragment_to_markdown("<p>\u{FEFF}long\u{200B}word re\u{00AD}search 👩\u{200D}💻 نمی\u{200C}خواهم</p>");
        assert_eq!(md, "longword re\u{00AD}search 👩\u{200D}💻 نمی\u{200C}خواهم");
        let q = quote_block("a\u{200B}b", &meta(), false);
        assert!(q.ends_with("> ab\n"), "{q}");
    }

    #[test]
    fn soften_escapes_skips_code_and_math() {
        // The \. and \[ inside a fenced block are real content
        let md = fragment_to_markdown("<x-vemry-code lang=\"re\">a\\.b \\[c\\]</x-vemry-code>");
        assert!(md.contains("a\\.b \\[c\\]"), "{md}");
        // LaTeX backslashes in a math span are untouched
        let md = fragment_to_markdown("<p>见 <x-vemry-math>\\alpha \\. x</x-vemry-math> 与 1. 编号</p>");
        assert!(md.contains("$\\alpha \\. x$"), "{md}");
        assert!(md.contains("1. 编号"), "{md}");
        // A lone $ (currency and the like) does not open a math span: the citation noise later on the same line is still restored
        assert_eq!(fragment_to_markdown("<p>股价升至 $189,研究表明[1]。</p>"), "股价升至 $189,研究表明[1]。");
    }

    #[test]
    fn fragment_to_markdown_keeps_code_sentinel() {
        let md = fragment_to_markdown(
            "<p>选区里的代码:</p><x-vemry-code lang=\"py\">def f():\n    return 1</x-vemry-code>",
        );
        assert!(md.contains("```py\ndef f():\n    return 1\n```"), "{md}");
    }

    // Tables: htmd's built-in conversion requires an explicit <th> in Pure mode,
    // so a CMS table (whose header row is a styled <td>) is flattened into one
    // paragraph per cell — the actual shape of an Alibaba Cloud article. The
    // custom handler takes over entirely; this asserts that with no header the
    // first row is promoted to one.
    #[test]
    fn headerless_table_becomes_gfm_with_first_row_promoted() {
        let md = fragment_to_markdown(
            "<table><tbody>\
             <tr><td>模型</td><td>定位</td></tr>\
             <tr><td>Qwen3.5-Flash</td><td>生产级托管版</td></tr>\
             <tr><td>Qwen3.5-27B</td><td>均衡型</td></tr>\
             </tbody></table>",
        );
        assert!(md.contains("| 模型 | 定位 |"), "{md}");
        assert!(md.contains("| --- | --- |"), "{md}");
        assert!(md.contains("| Qwen3.5-Flash | 生产级托管版 |"), "{md}");
        assert!(md.contains("| Qwen3.5-27B | 均衡型 |"), "{md}");
    }

    #[test]
    fn th_table_survives_readability_path() {
        // The <th> table in ARTICLE_HTML must become a table on the Readability main path too
        let page = extract_page(ARTICLE_HTML, "https://example.com/article").unwrap();
        assert!(page.markdown.contains("| 列一 | 列二 |"), "{}", page.markdown);
        assert!(page.markdown.contains("| --- | --- |"), "{}", page.markdown);
        assert!(page.markdown.contains("| 甲 | 乙 |"), "{}", page.markdown);
    }

    #[test]
    fn table_cells_collapse_lines_and_escape_pipes() {
        let md = fragment_to_markdown(
            "<table>\
             <tr><td><p>第一段</p><p>第二段</p></td><td>a|b</td></tr>\
             <tr><td>甲</td><td>乙</td></tr>\
             </table>",
        );
        assert!(md.contains("| 第一段 第二段 | a\\|b |"), "paragraphs squashed onto one line, pipe escaped: {md}");
    }

    #[test]
    fn table_colspan_pads_columns() {
        let md = fragment_to_markdown(
            "<table>\
             <tr><td colspan=\"2\">跨两列</td><td>丙</td></tr>\
             <tr><td>甲</td><td>乙</td><td>丁</td></tr>\
             </table>",
        );
        assert!(md.contains("| 跨两列 |  | 丙 |"), "{md}");
        assert!(md.contains("| 甲 | 乙 | 丁 |"), "{md}");
    }

    #[test]
    fn layout_and_nested_tables_flatten() {
        // A single-column table is usually there for layout and is not worth a table
        let single = fragment_to_markdown("<table><tr><td>只有一格的内容</td></tr></table>");
        assert!(!single.contains('|'), "{single}");
        assert!(single.contains("只有一格的内容"), "{single}");
        // A table nested inside a cell cannot be expressed in pipe syntax; once
        // flattened it is squashed onto one line with the outer cell
        let nested = fragment_to_markdown(
            "<table>\
             <tr><td>外甲</td><td><table><tr><td>内1</td><td>内2</td></tr></table></td></tr>\
             <tr><td>外乙</td><td>外丙</td></tr>\
             </table>",
        );
        assert!(nested.contains("| 外甲 | 内1 内2 |"), "{nested}");
        assert!(nested.contains("| 外乙 | 外丙 |"), "{nested}");
    }

    #[test]
    fn table_keeps_inline_markdown_and_caption() {
        let md = fragment_to_markdown(
            "<table><caption>模型一览</caption>\
             <tr><th>名称</th><th>说明</th></tr>\
             <tr><td><strong>旗舰</strong></td><td><a href=\"https://e.com/d\">文档</a></td></tr>\
             </table>",
        );
        assert!(md.contains("模型一览"), "{md}");
        assert!(md.contains("| **旗舰** | [文档](https://e.com/d) |"), "{md}");
    }

    #[test]
    fn quote_block_wraps_table_lines() {
        // The whole selection-clip chain: fragment to Markdown, then wrapped in a
        // quote block, where the table rows pick up the `> ` prefix
        let md = fragment_to_markdown(
            "<p>对比:</p><table><tr><td>模型</td><td>定位</td></tr><tr><td>甲</td><td>乙</td></tr></table>",
        );
        let q = quote_block(&md, &meta(), true);
        assert!(q.contains("> | 模型 | 定位 |\n> | --- | --- |\n> | 甲 | 乙 |\n"), "{q}");
        let tags: Vec<String> = parse_note(&q).blocks.iter().flat_map(|b| b.tags.clone()).collect();
        assert!(tags.is_empty(), "{tags:?}");
    }

    #[test]
    fn quote_block_handles_multiline_selection() {
        let q = quote_block("第一行\n\n第三行  ", &meta(), true);
        assert_eq!(
            q,
            "> [!quote] [测试文章](https://example.com/article?id=1#section-2) · 2026-08-11\n> 第一行\n>\n> 第三行\n"
        );
        // One callout block, and no phantom tag from the `#section-2` in the head line
        let parsed = parse_note(&q);
        assert_eq!(parsed.blocks.len(), 1, "{:?}", parsed.blocks);
        let tags: Vec<String> = parsed.blocks.iter().flat_map(|b| b.tags.clone()).collect();
        assert!(tags.is_empty(), "{tags:?}");
    }

    #[test]
    fn quote_block_folds_a_long_body_landing_in_a_note() {
        // Past FOLD_MIN_CHARS of prose the `-` marker opens the callout folded…
        let long = "字".repeat(FOLD_MIN_CHARS + 1);
        assert!(quote_block(&long, &meta(), true).starts_with("> [!quote]- [测试文章]"));
        // …and so do many short lines (a list, a table)
        let lines = ["- 项"; FOLD_MIN_LINES + 1].join("\n");
        assert!(quote_block(&lines, &meta(), true).starts_with("> [!quote]- "));
        // A short quote opens expanded, and so does a long one that is its own clip note
        assert!(quote_block("一句话", &meta(), true).starts_with("> [!quote] [测试文章]"));
        assert!(quote_block(&long, &meta(), false).starts_with("> [!quote] [测试文章]"));
    }

    #[test]
    fn anchors_drop_titles_and_citation_links() {
        let md = fragment_to_markdown(
            r#"<p>在<a href="https://z/wiki/A" title="甲地">甲</a>成立<sup class="reference"><a href="https://z/wiki/X#cite_note-1">[1]</a></sup>，见<a href="https://z/w/B_(c)">乙</a>。<a href="/wiki/Citation_needed">[citation needed]</a> <a href="https://z/e"></a>尾</p>"#,
        );
        assert_eq!(md, "在[甲](https://z/wiki/A)成立，见[乙](https://z/w/B_%28c%29)。[\\[citation needed\\]](/wiki/Citation_needed) 尾");
        // A link without a destination is its text
        assert_eq!(fragment_to_markdown("<p>见<a name=\"x\">此处</a>。</p>"), "见此处。");
    }

    #[test]
    fn clip_rel_path_is_flat_with_sanitized_title() {
        let mut m = meta();
        m.title = "标题/带:非法*字符?".into();
        assert_eq!(clip_rel_path(&m), "clips/标题 带 非法 字符.md");
        // An empty title falls back to the domain (www stripped); no per-domain directory
        m.url = "https://www.zhihu.com/question/1".into();
        m.title = String::new();
        assert_eq!(clip_rel_path(&m), "clips/zhihu.com.md");
    }

    #[test]
    fn fragment_to_markdown_keeps_images_math_links() {
        let html = r#"<p>要点是 <strong>注意力</strong>,见 <a href="https://e.com/doc">文档</a>。</p>
<p><img src="https://e.com/chart.png" alt="架构图"></p>
<p><x-vemry-math>a_1 + b_2</x-vemry-math></p>"#;
        let md = fragment_to_markdown(html);
        assert!(md.contains("**注意力**"), "{md}");
        assert!(md.contains("[文档](https://e.com/doc)"), "{md}");
        assert!(md.contains("![架构图](https://e.com/chart.png)"), "{md}");
        assert!(md.contains("$a_1 + b_2$"), "{md}");
        // An empty or purely decorative fragment returns an empty string and the caller falls back to plain text
        assert_eq!(fragment_to_markdown(""), "");
        assert_eq!(fragment_to_markdown("<style>.x{}</style>"), "");
    }

    #[test]
    fn quote_block_wraps_fragment_markdown_with_images() {
        let q = quote_block("看这张图\n\n![](https://e.com/a.png)", &meta(), true);
        assert!(q.contains("> 看这张图\n"), "{q}");
        assert!(q.contains("> ![](https://e.com/a.png)\n"), "{q}");
        // An image reference inside a quote block is still found by the localisation scan
        let refs = image_refs(&q, "https://e.com/post");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].resolved, "https://e.com/a.png");
    }

    #[test]
    fn image_refs_resolves_relative_and_skips_nonremote() {
        let md = "\
![图一](https://example.com/a.png)\n\
![](/rel/b.jpg)\n\
![协议相对](//cdn.example.org/c.webp)\n\
![内联](data:image/png;base64,xxx)\n\
![已本地化](clips/assets/abc.png)\n\
![锚点](#top)\n";
        let refs = image_refs(md, "https://example.com/post/1");
        let resolved: Vec<&str> = refs.iter().map(|r| r.resolved.as_str()).collect();
        assert_eq!(
            resolved,
            vec![
                "https://example.com/a.png",
                "https://example.com/rel/b.jpg",
                "https://cdn.example.org/c.webp",
            ]
        );
        assert_eq!(refs[1].literal, "/rel/b.jpg");
    }

    #[test]
    fn image_refs_skips_code_blocks_and_inline_code() {
        let md = "\
正文 ![真图](https://example.com/a.png) 之后\n\
```\n![代码示例](https://example.com/fake.png)\n```\n\
行内 `![也是示例](https://example.com/fake2.png)` 结束\n\
双反引号 ``![还是示例](https://example.com/fake3.png)`` 收尾\n";
        let refs = image_refs(md, "https://example.com/");
        assert_eq!(refs.len(), 1, "{refs:?}");
        assert_eq!(refs[0].resolved, "https://example.com/a.png");
    }

    #[test]
    fn rewrite_images_replaces_target_keeps_alt_drops_title() {
        let md = "前 ![图](https://e.com/a.png \"标题\") 中 ![](https://e.com/b.png) 后 ![原样](https://e.com/c.png)";
        let map: std::collections::HashMap<String, String> = [
            ("https://e.com/a.png".to_string(), "clips/assets/1111.png".to_string()),
            ("https://e.com/b.png".to_string(), "clips/assets/2222.png".to_string()),
        ]
        .into();
        assert_eq!(
            rewrite_images(md, &map),
            "前 ![图](clips/assets/1111.png) 中 ![](clips/assets/2222.png) 后 ![原样](https://e.com/c.png)"
        );
    }

    #[test]
    fn rewrite_images_leaves_code_examples_alone() {
        let md = "```\n![x](https://e.com/a.png)\n```\n![x](https://e.com/a.png)\n";
        let map: std::collections::HashMap<String, String> =
            [("https://e.com/a.png".to_string(), "clips/assets/h.png".to_string())].into();
        assert_eq!(rewrite_images(md, &map), "```\n![x](https://e.com/a.png)\n```\n![x](clips/assets/h.png)\n");
    }

    #[test]
    fn rewrite_images_fence_marker_inside_block_does_not_close_it() {
        // Neither a ~~~ tutorial line nor a shorter ``` line inside a ``` block
        // closes it; the closer must use the same character and be no shorter than
        // the opener, and a real image after the block is rewritten as usual
        let md = "````\n~~~\n![x](https://e.com/a.png)\n```\n![x](https://e.com/a.png)\n````\n![x](https://e.com/a.png)\n";
        let map: std::collections::HashMap<String, String> =
            [("https://e.com/a.png".to_string(), "clips/assets/h.png".to_string())].into();
        assert_eq!(
            rewrite_images(md, &map),
            "````\n~~~\n![x](https://e.com/a.png)\n```\n![x](https://e.com/a.png)\n````\n![x](clips/assets/h.png)\n"
        );
    }

    #[test]
    fn image_ext_of_sniffs_magic_bytes_not_names() {
        assert_eq!(image_ext_of(b"\x89PNG\r\n\x1a\n...."), Some("png"));
        assert_eq!(image_ext_of(b"\xff\xd8\xff\xe0..JFIF"), Some("jpg"));
        assert_eq!(image_ext_of(b"GIF89a......"), Some("gif"));
        assert_eq!(image_ext_of(b"RIFF\x00\x00\x00\x00WEBPVP8 "), Some("webp"));
        assert_eq!(image_ext_of(b"\x00\x00\x00\x1cftypavif....."), Some("avif"));
        assert_eq!(image_ext_of(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"), Some("svg"));
        assert_eq!(image_ext_of(b"<?xml version=\"1.0\"?>\n<svg/>"), Some("svg"));
        assert_eq!(image_ext_of(b"<!doctype html><html>"), None, "HTML is not an image");
        assert_eq!(image_ext_of(b"MZ\x90\x00"), None);
    }

    #[test]
    fn mark_images_partial_is_idempotent() {
        let page = extract_page(ARTICLE_HTML, "https://example.com/article").unwrap();
        let note = compose_clip_note(&page, &meta(), "Source");
        let marked = mark_images_partial(&note);
        assert!(marked.contains("\nimages: partial\n---\n"), "{marked}");
        assert_eq!(mark_images_partial(&marked), marked, "marking twice does not stack");
        // It still parses with the core parser after marking
        let parsed = parse_note(&marked);
        assert_eq!(parsed.frontmatter.extra.get("images").and_then(|v| v.as_str()), Some("partial"));
        // Text without frontmatter comes back unchanged
        assert_eq!(mark_images_partial("普通文本"), "普通文本");
    }

    #[test]
    fn link_entry_escapes_brackets_and_parens() {
        let mut m = meta();
        m.title = "标题[注]".into();
        m.url = "https://en.wikipedia.org/wiki/Rust_(programming_language)".into();
        assert_eq!(
            link_entry(&m),
            "[标题\\[注\\]](https://en.wikipedia.org/wiki/Rust_%28programming_language%29)"
        );
    }
}
