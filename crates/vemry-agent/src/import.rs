//! import_page: a web page's extracted Markdown → the source note a study's chapters
//! cite. The model used to copy the page into the note as tool-call arguments — tens
//! of thousands of characters in the slow direction (output tokens), six minutes of a
//! twelve-minute study — and it renumbered ids along the way. The app composes the
//! note itself: the page's sections as headings, one block per paragraph with a
//! stable id, the equations, tables and figures the page holds, and hands the model
//! an outline of ids to read the note by instead of the text.
//!
//! The shape it reads is what the reading pipeline emits for a paper's HTML (an
//! arXiv page: `###### Abstract`, numbered headings, an equation as a one-row pipe
//! table with the number in its last cell, a figure as an image line followed by
//! its `Figure N:` caption, citations as `\[[13](#bib.bib13)\]`, the reference list
//! as a bullet list under `## References`); a plain long article goes through the
//! same rules and simply has fewer of these.

/// What import_page produced: the note's text and the outline the model reads.
pub struct Imported {
    pub content: String,
    pub outline: String,
    pub paragraphs: usize,
    pub equations: usize,
    pub figures: usize,
    pub tables: usize,
}

/// The kinds of block the note is assembled from, before ids are assigned.
#[derive(Debug, Clone)]
enum Block {
    Heading { level: usize, key: String, text: String, numbered: bool },
    Para(Vec<String>),
    Equation(Vec<String>),
    Table(Vec<String>),
    /// `Table N: …` — the caption above (or below) a table; it carries `^tabN`
    TableCaption { n: String, text: String },
    /// A page image, kept apart until its `Figure N:` caption claims it
    Image { url: String },
    /// `Figure N: …` with the images it captions: one block, `^figN` on the caption
    Figure { n: String, images: Vec<(String, String)>, caption: String },
    Fence(Vec<String>),
}

/// The page's Markdown → the note. `front` is extra frontmatter (authors, year,
/// genre) the caller knows; `title` and `source` are always written.
pub fn compose_source_note(markdown: &str, url: &str, title: &str, front: &[(String, String)]) -> Imported {
    let lines = fold_bullets(markdown);
    let raw = group_blocks(&lines);
    let mut blocks: Vec<Block> = raw.into_iter().map(classify).collect();
    attach_figures(&mut blocks);
    cut_sections(&mut blocks);
    number_headings(&mut blocks);
    assemble(blocks, url, title, front)
}

/// The reading pipeline renders a bullet whose marker is a `•` glyph as a list item
/// holding the glyph alone, with the item's text as an indented paragraph below;
/// folded back into `- text`.
fn fold_bullets(markdown: &str) -> Vec<String> {
    let src: Vec<&str> = markdown.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        let line = src[i];
        let t = line.trim();
        if (t == "* •" || t == "- •" || t.starts_with("*   •") || t.starts_with("-   •")) && t.trim_start_matches(['*', '-', ' ']).trim() == "•" {
            let mut j = i + 1;
            while j < src.len() && src[j].trim().is_empty() {
                j += 1;
            }
            if j < src.len() && src[j].starts_with("    ") {
                out.push(format!("- {}", src[j].trim()));
                i = j + 1;
                continue;
            }
        }
        out.push(line.to_string());
        i += 1;
    }
    out
}

fn is_table_row(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.ends_with('|') && t.len() > 1
}

fn is_heading(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('#') && t.trim_start_matches('#').starts_with(' ')
}

/// Lines → raw blocks: a heading alone, a fence whole, table rows together, and
/// otherwise the lines between two blank lines.
fn group_blocks(lines: &[String]) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut in_fence = false;
    let mut in_table = false;
    let flush = |cur: &mut Vec<String>, out: &mut Vec<Vec<String>>| {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    for line in lines {
        if in_fence {
            cur.push(line.clone());
            if line.trim_start().starts_with("```") {
                in_fence = false;
                flush(&mut cur, &mut out);
            }
            continue;
        }
        if line.trim_start().starts_with("```") {
            flush(&mut cur, &mut out);
            in_table = false;
            in_fence = true;
            cur.push(line.clone());
            continue;
        }
        if line.trim().is_empty() {
            flush(&mut cur, &mut out);
            in_table = false;
            continue;
        }
        if is_heading(line) {
            flush(&mut cur, &mut out);
            in_table = false;
            out.push(vec![line.clone()]);
            continue;
        }
        let row = is_table_row(line);
        if row != in_table {
            flush(&mut cur, &mut out);
            in_table = row;
        }
        cur.push(line.clone());
    }
    flush(&mut cur, &mut out);
    out
}

/// The number a caption opens with: `Figure 2: …` → ("2", "(left) …").
fn caption_of<'a>(line: &'a str, word: &str) -> Option<(String, &'a str)> {
    let rest = line.trim().strip_prefix(word)?.strip_prefix(' ')?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let after = rest[digits.len()..].trim_start_matches([':', '.']).trim_start();
    Some((digits, after))
}

fn image_line(line: &str) -> Option<String> {
    let t = line.trim();
    let rest = t.strip_prefix("![")?;
    let close = rest.find("](")?;
    let target = rest[close + 2..].strip_suffix(')')?;
    // a title after the URL (`![a](u "t")`) is dropped with it
    let url = target.split_whitespace().next()?.to_string();
    Some(url)
}

fn classify(raw: Vec<String>) -> Block {
    if raw.len() == 1 && is_heading(&raw[0]) {
        let t = raw[0].trim_start();
        let level = t.chars().take_while(|&c| c == '#').count();
        let text = t.trim_start_matches('#').trim().trim_end_matches(':').trim().to_string();
        return Block::Heading { level, key: String::new(), text, numbered: false };
    }
    if raw[0].trim_start().starts_with("```") {
        return Block::Fence(raw);
    }
    if raw.iter().all(|l| is_table_row(l)) {
        return match equation_lines(&raw) {
            Some(lines) => Block::Equation(lines),
            None => Block::Table(raw),
        };
    }
    if raw.len() == 1 {
        if let Some(url) = image_line(&raw[0]) {
            return Block::Image { url };
        }
        if let Some((n, text)) = caption_of(&raw[0], "Table") {
            return Block::TableCaption { n, text: clean_refs(text) };
        }
    }
    Block::Para(raw.into_iter().map(|l| clean_refs(&l)).collect())
}

/// A pipe table whose cells hold nothing but formulas and equation numbers is an
/// equation the page laid out as a table: each row a `$$…$$` line, the number kept
/// after the closer (`$$…$$ (1)`), an aligned pair of cells joined.
fn equation_lines(rows: &[String]) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for row in rows {
        let inner = row.trim().trim_start_matches('|').trim_end_matches('|');
        let cells: Vec<&str> = inner.split('|').map(str::trim).collect();
        if cells.iter().all(|c| c.is_empty() || c.chars().all(|ch| ch == '-' || ch == ':')) {
            continue; // the separator row
        }
        let mut tex: Vec<String> = Vec::new();
        let mut number: Option<&str> = None;
        for c in cells {
            if c.is_empty() {
                continue;
            }
            if c.starts_with('(') && c.ends_with(')') && c[1..c.len() - 1].chars().all(|ch| ch.is_ascii_digit()) {
                number = Some(c);
            } else if c.starts_with('$') && c.ends_with('$') && c.len() > 2 {
                let body = c.trim_matches('$').replace("\\displaystyle", "");
                tex.push(body.trim().to_string());
            } else {
                return None;
            }
        }
        if tex.is_empty() {
            return None;
        }
        let mut line = format!("$${}$$", tex.join(" "));
        if let Some(n) = number {
            line.push(' ');
            line.push_str(n);
        }
        out.push(line);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// In-page citation links → their text: `\[[13](#bib.bib13)\]` → `[13]`,
/// `Table [1](#S4.T1)` → `Table 1`. A link to anywhere else is left as written.
fn clean_refs(line: &str) -> String {
    let mut s = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find("](#") {
        // the label starts at the `[` that opens this link
        let Some(open) = rest[..at].rfind('[') else {
            s.push_str(&rest[..at + 3]);
            rest = &rest[at + 3..];
            continue;
        };
        let Some(close) = rest[at..].find(')') else { break };
        s.push_str(&rest[..open]);
        s.push_str(&rest[open + 1..at]);
        rest = &rest[at + close + 1..];
    }
    s.push_str(rest);
    // the brackets the pipeline escaped around a citation group are plain brackets now
    s.replace("\\[", "[").replace("\\]", "]")
}

/// A `Figure N:` caption claims the images just above it (with the one-line label
/// the page set over each half, when there is one): one block, the images then the
/// caption. A caption with no image above it stays a paragraph.
fn attach_figures(blocks: &mut Vec<Block>) {
    let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
    for b in blocks.drain(..) {
        let caption = match &b {
            Block::Para(lines) if lines.len() == 1 => caption_of(&lines[0], "Figure").map(|(n, t)| (n, t.to_string())),
            _ => None,
        };
        let Some((n, text)) = caption else {
            out.push(b);
            continue;
        };
        // walk back over images and their one-line labels, six blocks at most
        let mut k = out.len();
        let mut taken = 0;
        while k > 0 && taken < 6 {
            match &out[k - 1] {
                Block::Image { .. } => {}
                Block::Para(l) if l.len() == 1 && l[0].chars().count() <= 80 && !l[0].ends_with('.') => {}
                _ => break,
            }
            k -= 1;
            taken += 1;
        }
        // the tail starts at an image, or at the label right over one; a short line
        // with no image after it is prose and stays where it was
        while k < out.len()
            && !matches!(out[k], Block::Image { .. })
            && !matches!(out.get(k + 1), Some(Block::Image { .. }))
        {
            k += 1;
        }
        let tail: Vec<Block> = out.drain(k..).collect();
        let mut images: Vec<(String, String)> = Vec::new();
        let mut label: Option<String> = None;
        for t in tail {
            match t {
                Block::Para(l) => label = Some(l[0].clone()),
                Block::Image { url } => images.push((label.take().unwrap_or_default(), url)),
                _ => {}
            }
        }
        if images.is_empty() {
            out.push(Block::Para(vec![format!("Figure {n}: {text}")]));
            continue;
        }
        let first_sentence = text.split(". ").next().unwrap_or(&text).trim_end_matches('.').to_string();
        let halves = ["left", "right"];
        let images = images
            .into_iter()
            .enumerate()
            .map(|(i, (label, url))| {
                let alt = if images_count_is_one(i, &label) {
                    format!("Figure {n}: {first_sentence}")
                } else if !label.is_empty() {
                    format!("Figure {n} ({}): {label}", halves.get(i).copied().unwrap_or("part"))
                } else {
                    format!("Figure {n} ({})", halves.get(i).copied().unwrap_or("part"))
                };
                (alt, url)
            })
            .collect::<Vec<_>>();
        out.push(Block::Figure { n, images, caption: text });
    }
    *blocks = out;
}

/// Only a lone image takes the caption's first sentence as its alt (the pairs are
/// told apart by their labels); decided per image so the closure above stays flat.
fn images_count_is_one(index: usize, label: &str) -> bool {
    index == 0 && label.is_empty()
}

fn heading_matches(text: &str, words: &[&str]) -> bool {
    let t = text.to_ascii_lowercase();
    words.iter().any(|w| t.starts_with(w))
}

/// What the note does not archive: the front matter above an Abstract (the license
/// line, the author block), an acknowledgements section, and everything from the
/// reference list on (the appendix behind it included).
fn cut_sections(blocks: &mut Vec<Block>) {
    let first_heading = blocks.iter().position(|b| matches!(b, Block::Heading { .. }));
    if let Some(i) = first_heading {
        if let Block::Heading { text, .. } = &blocks[i] {
            if heading_matches(text, &["abstract"]) {
                blocks.drain(..i);
            }
        }
    }
    if let Some(end) = blocks.iter().position(|b| matches!(b, Block::Heading { text, .. } if heading_matches(text, &["references", "bibliography"]))) {
        blocks.truncate(end);
    }
    let mut out: Vec<Block> = Vec::with_capacity(blocks.len());
    let mut skipping = false;
    for b in blocks.drain(..) {
        if let Block::Heading { text, .. } = &b {
            skipping = heading_matches(text, &["acknowledg"]);
        }
        if !skipping {
            out.push(b);
        }
    }
    *blocks = out;
}

/// Heading keys and levels: a numbered heading (`3.2.1 Scaled …`) is section
/// `321` at depth three; an unnumbered one under it (`Encoder:`) belongs to the
/// enclosing section, one level deeper; `Abstract` is section 0.
fn number_headings(blocks: &mut [Block]) {
    let mut key = "0".to_string();
    let mut level = 2usize;
    let mut seen_numbered = false;
    for b in blocks.iter_mut() {
        if let Block::Heading { level: l, key: k, text, numbered } = b {
            if let Some((digits, rest)) = section_number(text) {
                key = digits;
                level = (rest.0 + 1).min(6);
                *text = rest.1;
                *numbered = true;
                seen_numbered = true;
                *k = key.clone();
                *l = level;
            } else if heading_matches(text, &["abstract"]) {
                key = "0".into();
                level = 2;
                *k = key.clone();
                *l = level;
            } else {
                *k = key.clone();
                // under a numbered section: one level deeper; a plain article's own
                // headings keep their depth (never above ##)
                *l = if seen_numbered { (level + 1).min(6) } else { (*l).clamp(2, 6) };
            }
        }
    }
}

/// `3.2.1 Scaled Dot-Product Attention` → ("321", (3, "3.2.1 Scaled Dot-Product Attention")).
fn section_number(text: &str) -> Option<(String, (usize, String))> {
    let (num, _rest) = text.split_once(' ')?;
    let num = num.trim_end_matches('.');
    let parts: Vec<&str> = num.split('.').collect();
    if parts.is_empty() || parts.len() > 4 || !parts.iter().all(|p| !p.is_empty() && p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit())) {
        return None;
    }
    Some((parts.concat(), (parts.len(), text.to_string())))
}

fn assemble(blocks: Vec<Block>, url: &str, title: &str, front: &[(String, String)]) -> Imported {
    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(&format!("title: {}\n", yaml_scalar(title)));
    for (k, v) in front {
        if !v.trim().is_empty() && k != "title" && k != "source" {
            out.push_str(&format!("{k}: {}\n", yaml_scalar(v)));
        }
    }
    out.push_str(&format!("source: {url}\n---\n\n# {}\n", title.trim()));

    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut key = "0".to_string();
    let mut paragraphs = 0;
    let mut equations = 0;
    let mut figures = 0;
    let mut tables = 0;
    // the outline: one line per heading, the ids its section holds
    let mut outline: Vec<String> = Vec::new();
    let mut section_ids: Vec<String> = Vec::new();
    let mut section_line = String::new();
    let flush_section = |line: &mut String, ids: &mut Vec<String>, outline: &mut Vec<String>| {
        if line.is_empty() {
            return;
        }
        let para: Vec<&String> = ids.iter().filter(|i| i.starts_with("^s")).collect();
        let mut parts: Vec<String> = Vec::new();
        match (para.first(), para.last()) {
            (Some(a), Some(b)) if a != b => parts.push(format!("{a}…{b}")),
            (Some(a), _) => parts.push((*a).clone()),
            _ => {}
        }
        parts.extend(ids.iter().filter(|i| !i.starts_with("^s")).cloned());
        let tail = if parts.is_empty() { String::new() } else { format!(" — {}", parts.join(", ")) };
        outline.push(format!("{line}{tail}"));
        ids.clear();
    };
    let next_id = |key: &str, counts: &mut std::collections::HashMap<String, usize>| {
        let n = counts.entry(key.to_string()).or_insert(0);
        *n += 1;
        format!("^s{key}p{n}")
    };
    for b in blocks {
        out.push('\n');
        match b {
            Block::Heading { level, key: k, text, .. } => {
                flush_section(&mut section_line, &mut section_ids, &mut outline);
                key = k;
                section_line = format!("{} {text}", "#".repeat(level));
                out.push_str(&section_line);
                out.push('\n');
            }
            Block::Para(lines) => {
                let id = next_id(&key, &mut counts);
                paragraphs += 1;
                push_with_id(&mut out, &lines, &id);
                section_ids.push(id);
            }
            Block::Equation(lines) => {
                let id = next_id(&key, &mut counts);
                equations += 1;
                push_with_id(&mut out, &lines, &id);
                section_ids.push(id);
            }
            Block::Table(rows) => {
                tables += 1;
                for r in rows {
                    out.push_str(r.trim());
                    out.push('\n');
                }
            }
            Block::TableCaption { n, text } => {
                let id = format!("^tab{n}");
                out.push_str(&format!("Table {n}: {text} {id}\n"));
                section_ids.push(id);
            }
            Block::Image { url } => {
                // an image no caption claimed: kept, no id
                out.push_str(&format!("![image]({url})\n"));
            }
            Block::Figure { n, images, caption } => {
                figures += 1;
                let id = format!("^fig{n}");
                for (alt, url) in images {
                    out.push_str(&format!("![{alt}]({url})\n"));
                }
                out.push_str(&format!("Figure {n}: {caption} {id}\n"));
                section_ids.push(id);
            }
            Block::Fence(lines) => {
                for l in lines {
                    out.push_str(&l);
                    out.push('\n');
                }
            }
        }
    }
    flush_section(&mut section_line, &mut section_ids, &mut outline);
    let chars = out.chars().count();
    let parts = chars.div_ceil(crate::prompt::PAGE_CONTEXT_MAX_CHARS).max(1);
    let mut summary = format!(
        "{paragraphs} paragraphs, {equations} equations, {figures} figures, {tables} tables; {chars} characters — read_note reads it in {parts} part(s) of {} (part: 1, then 2, …).\n\n",
        crate::prompt::PAGE_CONTEXT_MAX_CHARS
    );
    summary.push_str(&outline.join("\n"));
    Imported { content: out, outline: summary, paragraphs, equations, figures, tables }
}

fn push_with_id(out: &mut String, lines: &[String], id: &str) {
    for (i, l) in lines.iter().enumerate() {
        out.push_str(l.trim_end());
        if i + 1 == lines.len() {
            out.push(' ');
            out.push_str(id);
        }
        out.push('\n');
    }
}

/// A frontmatter value: quoted when YAML would read it as something else.
fn yaml_scalar(v: &str) -> String {
    let v = v.trim();
    if v.is_empty() || v.contains(':') || v.contains('#') || v.starts_with(['[', '{', '"', '\'', '*', '&', '!', '%', '@', '`', '-', '?']) || v.parse::<f64>().is_ok() || matches!(v, "true" | "false" | "null" | "yes" | "no") {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "Provided proper attribution is provided, Google hereby grants permission.\n\n Ashish Vaswani ††thanks: Equal contribution.\n\n###### Abstract\n\nThe dominant sequence transduction models are based on complex recurrent networks.\n\n## 1 Introduction\n\nRecurrent neural networks \\[[13](#bib.bib13)\\] and gated recurrent \\[[7](#bib.bib7)\\] networks have been established.\n\nAs noted in Table [1](#S4.T1), a self-attention layer connects all positions.\n\n## 3 Model Architecture\n\n![Refer to caption](https://arxiv.org/html/1706.03762v7/Figures/ModalNet-21.png)\n\nFigure 1: The Transformer - model architecture.\n\nMost competitive models have an encoder-decoder structure \\[[5](#bib.bib5), [2](#bib.bib2)\\].\n\n### 3.1 Encoder and Decoder Stacks\n\n##### Encoder:\n\nThe encoder is composed of a stack of $N=6$ identical layers.\n\n##### Decoder:\n\nThe decoder is also composed of a stack of $N=6$ identical layers.\n\n#### 3.2.1 Scaled Dot-Product Attention\n\nWe compute the dot products of the query with all keys.\n\n|  | $$\\mathrm{Attention}(Q,K,V)=\\mathrm{softmax}(\\frac{QK^{T}}{\\sqrt{d_{k}}})V$$ |  | (1) |\n| --- | --- | --- | --- |\n\nFor large values of $d_k$ the dot products grow large.\n\n#### 3.2.2 Multi-Head Attention\n\nScaled Dot-Product Attention\n\n![Refer to caption](https://arxiv.org/html/1706.03762v7/Figures/ModalNet-19.png)\n\nMulti-Head Attention\n\n![Refer to caption](https://arxiv.org/html/1706.03762v7/Figures/ModalNet-20.png)\n\nFigure 2: (left) Scaled Dot-Product Attention. (right) Multi-Head Attention consists of several attention layers.\n\n|  | $\\displaystyle\\mathrm{MultiHead}(Q,K,V)$ | $\\displaystyle=\\mathrm{Concat}(\\mathrm{head_{1}},...,\\mathrm{head_{h}})W^{O}$ |  |\n| --- | --- | --- | --- |\n|  | $\\displaystyle\\text{where}~\\mathrm{head_{i}}$ | $\\displaystyle=\\mathrm{Attention}(QW^{Q}_{i},KW^{K}_{i},VW^{V}_{i})$ |  |\n\nThe Transformer uses attention in three ways:\n\n*   •\n\n    In encoder-decoder attention layers, the queries come from the decoder.\n\n*   •\n\n    The encoder contains self-attention layers.\n\n## 4 Why Self-Attention\n\nTable 1: Maximum path lengths, per-layer complexity and minimum number of sequential operations.\n\n| Layer Type | Complexity per Layer | Sequential | Maximum Path Length |\n| --- | --- | --- | --- |\n| Self-Attention | $O(n^{2}\\cdot d)$ | $O(1)$ | $O(1)$ |\n| Recurrent | $O(n\\cdot d^{2})$ | $O(n)$ | $O(n)$ |\n\n## 7 Conclusion\n\nIn this work, we presented the Transformer.\n\n##### Acknowledgements\n\nWe are grateful to Nal Kalchbrenner.\n\n## References\n\n*   [1] Jimmy Lei Ba. Layer normalization. 2016.\n\n## Attention Visualizations\n\nFigure 3: An example of the attention mechanism.\n";

    fn imported() -> Imported {
        compose_source_note(PAGE, "https://arxiv.org/html/1706.03762", "Attention Is All You Need", &[("authors".into(), "Vaswani et al.".into()), ("year".into(), "2017".into())])
    }

    #[test]
    fn frontmatter_title_and_the_cut_front_matter() {
        let c = imported().content;
        assert!(c.starts_with("---\ntitle: Attention Is All You Need\nauthors: Vaswani et al.\nyear: \"2017\"\nsource: https://arxiv.org/html/1706.03762\n---\n\n# Attention Is All You Need\n"), "{c}");
        assert!(!c.contains("Google hereby grants"), "the license line above the abstract is not archived");
        assert!(!c.contains("††thanks"), "the author block is not archived");
        assert!(c.contains("\n## Abstract\n\nThe dominant sequence transduction models are based on complex recurrent networks. ^s0p1\n"), "{c}");
    }

    #[test]
    fn section_ids_follow_the_paper_numbering() {
        let c = imported().content;
        assert!(c.contains("networks have been established. ^s1p1\n"), "{c}");
        assert!(c.contains("connects all positions. ^s1p2\n"));
        // an unnumbered heading under 3.1 keeps the section's numbering, one level deeper
        assert!(c.contains("\n### 3.1 Encoder and Decoder Stacks\n\n#### Encoder\n\nThe encoder is composed of a stack of $N=6$ identical layers. ^s31p1\n\n#### Decoder\n\nThe decoder is also composed of a stack of $N=6$ identical layers. ^s31p2\n"), "{c}");
        assert!(c.contains("\n#### 3.2.1 Scaled Dot-Product Attention\n\nWe compute the dot products of the query with all keys. ^s321p1\n"));
    }

    #[test]
    fn equations_leave_their_tables() {
        let c = imported().content;
        assert!(c.contains("\n$$\\mathrm{Attention}(Q,K,V)=\\mathrm{softmax}(\\frac{QK^{T}}{\\sqrt{d_{k}}})V$$ (1) ^s321p2\n"), "{c}");
        assert!(c.contains("the dot products grow large. ^s321p3\n"));
        // an aligned pair: the two cells joined, one block, the id on its last line
        assert!(c.contains("\n$$\\mathrm{MultiHead}(Q,K,V) =\\mathrm{Concat}(\\mathrm{head_{1}},...,\\mathrm{head_{h}})W^{O}$$\n$$\\text{where}~\\mathrm{head_{i}} =\\mathrm{Attention}(QW^{Q}_{i},KW^{K}_{i},VW^{V}_{i})$$ ^s322p1\n"), "{c}");
        assert_eq!(imported().equations, 2);
    }

    #[test]
    fn figures_take_their_captions_and_a_pair_its_labels() {
        let c = imported().content;
        assert!(c.contains("\n![Figure 1: The Transformer - model architecture](https://arxiv.org/html/1706.03762v7/Figures/ModalNet-21.png)\nFigure 1: The Transformer - model architecture. ^fig1\n"), "{c}");
        assert!(c.contains("\n![Figure 2 (left): Scaled Dot-Product Attention](https://arxiv.org/html/1706.03762v7/Figures/ModalNet-19.png)\n![Figure 2 (right): Multi-Head Attention](https://arxiv.org/html/1706.03762v7/Figures/ModalNet-20.png)\nFigure 2: (left) Scaled Dot-Product Attention. (right) Multi-Head Attention consists of several attention layers. ^fig2\n"), "{c}");
        assert!(!c.contains("\nScaled Dot-Product Attention\n"), "a half's label went into its alt, not the note");
        assert_eq!(imported().figures, 2);
    }

    #[test]
    fn tables_keep_their_rows_and_the_caption_its_id() {
        let c = imported().content;
        assert!(c.contains("\nTable 1: Maximum path lengths, per-layer complexity and minimum number of sequential operations. ^tab1\n\n| Layer Type | Complexity per Layer | Sequential | Maximum Path Length |\n| --- | --- | --- | --- |\n| Self-Attention | $O(n^{2}\\cdot d)$ | $O(1)$ | $O(1)$ |\n"), "{c}");
        assert_eq!(imported().tables, 1);
    }

    #[test]
    fn citations_bullets_and_the_tail_of_the_paper() {
        let c = imported().content;
        assert!(c.contains("Recurrent neural networks [13] and gated recurrent [7] networks"), "{c}");
        assert!(c.contains("structure [5, 2]."));
        assert!(c.contains("As noted in Table 1, a self-attention"));
        assert!(c.contains("\n- In encoder-decoder attention layers, the queries come from the decoder. ^s322p3\n\n- The encoder contains self-attention layers. ^s322p4\n"), "{c}");
        assert!(c.contains("we presented the Transformer. ^s7p1\n"));
        assert!(!c.contains("Acknowledgements") && !c.contains("Nal Kalchbrenner"), "acknowledgements are not archived");
        assert!(!c.contains("References") && !c.contains("Layer normalization") && !c.contains("Figure 3"), "the reference list and the appendix behind it are not archived: {c}");
        assert!(c.ends_with("^s7p1\n"), "{c}");
    }

    #[test]
    fn the_outline_names_each_section_by_its_ids() {
        let o = imported().outline;
        assert!(o.starts_with("12 paragraphs, 2 equations, 2 figures, 1 tables;"), "{o}");
        assert!(o.contains("\n## Abstract — ^s0p1\n## 1 Introduction — ^s1p1…^s1p2\n## 3 Model Architecture — ^s3p1, ^fig1\n"), "{o}");
        assert!(o.contains("\n#### 3.2.2 Multi-Head Attention — ^s322p1…^s322p4, ^fig2\n"), "{o}");
        assert!(o.contains("\n## 4 Why Self-Attention — ^tab1\n"), "{o}");
    }

    #[test]
    fn a_plain_article_keeps_its_opening_and_gets_section_zero() {
        let c = compose_source_note("An opening paragraph.\n\n## Why\n\nBecause.\n", "https://a.io/x", "Post: a title", &[]).content;
        assert!(c.starts_with("---\ntitle: \"Post: a title\"\nsource: https://a.io/x\n---\n\n# Post: a title\n\nAn opening paragraph. ^s0p1\n\n## Why\n\nBecause. ^s0p2\n"), "{c}");
    }
}
