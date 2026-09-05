//! The session's source table: every web citation the model has produced in this
//! session, with whether it was verified. Kept in Rust so that both windows show
//! the same list, a compaction cannot lose it, and the model can read it back
//! (`list_sources`). The audit jsonl carries one `sources` line per turn that added
//! entries; replaying those lines rebuilds the table (`read_sources`).
//!
//! Verification rule (written once, here): a URL is verified when a web search
//! ran in the turn that produced it, or when it appeared in something the user
//! handed over (the attached web page). Otherwise it came out of the model's
//! prose alone and may be from memory — the frontend shows it as "unverified".

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub url: String,
    /// The link's title as the model wrote it (empty for a bare URL).
    #[serde(default)]
    pub title: String,
    /// Whether the URL can be trusted to exist (see the module comment).
    #[serde(default)]
    pub verified: bool,
    /// The turn id that first produced it (a regenerate or an edit-and-resend of
    /// that turn drops its sources again).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<String>,
}

/// A citation found in the model's text: a Markdown link's title and target, or a
/// bare URL with an empty title.
#[derive(Clone, Debug, PartialEq)]
pub struct Citation {
    pub title: String,
    pub url: String,
}

/// URL characters (the frontend's BARE_URL grammar): ASCII only — the URLs a model
/// outputs are percent-encoded ASCII, and opening this up would swallow the CJK
/// sentence pressed against a bare link.
fn is_url_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-._~:/?#@!$&*+,;=%()".contains(c)
}

/// Trailing punctuation a bare URL is often ended by (CJK or otherwise). A ')'
/// stays when the prefix still holds an unmatched '(' — a Wikipedia-style path.
fn strip_trailing_punct(url: &str) -> &str {
    let mut end = url.len();
    while let Some(last) = url[..end].chars().next_back() {
        if !")）]】』」》>\"'.,，。;；:：!！?？".contains(last) {
            break;
        }
        if last == ')' {
            let prefix = &url[..end - 1];
            if prefix.matches('(').count() > prefix.matches(')').count() {
                break;
            }
        }
        end -= last.len_utf8();
    }
    &url[..end]
}

/// Markdown emphasis and code markers stripped from a link title (the frontend's plainLabel).
fn plain_label(s: &str) -> String {
    s.replace(['*', '`'], "").trim().to_string()
}

/// Every web citation in `text`, in document order, deduplicated by URL (the first
/// non-empty title wins) — the same rule the frontend numbers its superscripts by,
/// so the card and the markers agree. An image (`![...](url)`) is not a source.
pub fn extract_citations(text: &str) -> Vec<Citation> {
    let bytes = text.as_bytes();
    let mut hits: Vec<(usize, Citation)> = Vec::new();
    let mut link_spans: Vec<(usize, usize)> = Vec::new();
    // Markdown links: [title](http...)
    let mut i = 0;
    while let Some(open) = text[i..].find('[') {
        let start = i + open;
        let Some(close_rel) = text[start..].find("](") else { break };
        let close = start + close_rel;
        let target_start = close + 2;
        let Some(end_rel) = text[target_start..].find(')') else { break };
        // A ')' inside the target that belongs to a '(' in it: keep scanning
        let mut end = target_start + end_rel;
        while text[target_start..end].matches('(').count() > text[target_start..end].matches(')').count() {
            match text[end + 1..].find(')') {
                Some(n) => end = end + 1 + n,
                None => break,
            }
        }
        let target = &text[target_start..end];
        let is_image = start > 0 && bytes[start - 1] == b'!';
        if (target.starts_with("http://") || target.starts_with("https://")) && !target.contains(char::is_whitespace) {
            if is_image {
                link_spans.push((start - 1, end + 1));
            } else {
                let title = plain_label(&text[start + 1..close]);
                hits.push((start, Citation { title, url: strip_trailing_punct(target).to_string() }));
                link_spans.push((start, end + 1));
            }
            i = end + 1;
        } else {
            i = start + 1;
        }
    }
    // Bare URLs outside any link
    let mut pos = 0usize;
    while let Some(at) = text[pos..].find("http") {
        let start = pos + at;
        let rest = &text[start..];
        if !(rest.starts_with("http://") || rest.starts_with("https://")) {
            pos = start + 4;
            continue;
        }
        if link_spans.iter().any(|&(s, e)| start >= s && start < e) {
            pos = start + 4;
            continue;
        }
        // Not the target of a Markdown link (`](` precedes it) and not right after '(' or '['
        if start > 0 && matches!(bytes[start - 1], b'(' | b'[') {
            pos = start + 4;
            continue;
        }
        let end = rest.char_indices().find(|&(_, c)| !is_url_char(c)).map(|(i, _)| i).unwrap_or(rest.len());
        let url = strip_trailing_punct(&rest[..end]);
        if url.len() > "https://".len() {
            hits.push((start, Citation { title: String::new(), url: url.to_string() }));
        }
        pos = start + end.max(4);
    }
    hits.sort_by_key(|(at, _)| *at);
    let mut out: Vec<Citation> = Vec::new();
    for (_, c) in hits {
        match out.iter_mut().find(|o| o.url == c.url) {
            Some(prev) => {
                if prev.title.is_empty() && !c.title.is_empty() {
                    prev.title = c.title;
                }
            }
            None => out.push(c),
        }
    }
    out
}

/// Merge one turn's citations into the table; returns the entries that are new to
/// the session (what the `sources` audit line and the event carry).
/// `searched`: a web search ran this turn (every citation counts as verified);
/// `exempt`: URLs the user handed over this turn (verified on their own).
pub fn merge_turn(
    table: &mut Vec<Source>,
    turn: Option<&str>,
    cites: &[Citation],
    searched: bool,
    exempt: &HashSet<String>,
) -> Vec<Source> {
    let mut added = Vec::new();
    for c in cites {
        if let Some(existing) = table.iter_mut().find(|s| s.url == c.url) {
            // An earlier unverified entry becomes verified once a search confirms it
            if searched && !existing.verified {
                existing.verified = true;
            }
            if existing.title.is_empty() && !c.title.is_empty() {
                existing.title = c.title.clone();
            }
            continue;
        }
        let src = Source {
            url: c.url.clone(),
            title: c.title.clone(),
            verified: searched || exempt.contains(&c.url),
            turn: turn.map(String::from),
        };
        table.push(src.clone());
        added.push(src);
    }
    added
}

/// Drop the sources a turn produced (a regenerate or an edit-and-resend of it).
pub fn drop_turn(table: &mut Vec<Source>, turn: &str) {
    table.retain(|s| s.turn.as_deref() != Some(turn));
}

/// Rebuild the table from an audit jsonl: `sources` lines add, and a `retry` or
/// `edit_resend` marker drops the sources of the turn it abandons (the last user
/// line before it) — the same rule the live path applies.
pub fn read_sources(jsonl: &str) -> Vec<Source> {
    let mut table: Vec<Source> = Vec::new();
    let mut last_turn: Option<String> = None;
    for line in jsonl.lines().filter(|l| !l.trim().is_empty()) {
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        match v["kind"].as_str() {
            Some("user") => last_turn = v["turn"].as_str().map(String::from),
            Some("sources") => {
                let turn = v["turn"].as_str().map(String::from);
                if let Some(items) = v["items"].as_array() {
                    for it in items {
                        let Ok(mut s) = serde_json::from_value::<Source>(it.clone()) else { continue };
                        if s.turn.is_none() {
                            s.turn = turn.clone();
                        }
                        if !table.iter().any(|t| t.url == s.url) {
                            table.push(s);
                        }
                    }
                }
            }
            Some("retry") | Some("edit_resend") => {
                if let Some(t) = &last_turn {
                    drop_turn(&mut table, t);
                }
            }
            _ => {}
        }
    }
    table
}

/// The `list_sources` tool result: one line per source, numbered in table order
/// (the same numbers the frontend's source cards show for that turn are not
/// guaranteed across turns, so the line says the URL in full).
pub fn format_sources(table: &[Source]) -> String {
    if table.is_empty() {
        return "(no web sources in this session yet)".into();
    }
    table
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let title = if s.title.is_empty() { String::new() } else { format!("{} — ", s.title) };
            let flag = if s.verified { "" } else { "  [unverified: appeared only in your own prose, no search confirmed it]" };
            format!("{}. {title}{}{flag}", i + 1, s.url)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_links_and_bare_urls_in_order_without_duplicates() {
        let text = "见 [论文](https://a.org/p?x=1) 与 https://b.org/q。再看 ![图](https://img.io/x.png) 与 https://a.org/p?x=1";
        let c = extract_citations(text);
        assert_eq!(c.len(), 2, "{c:?}");
        assert_eq!(c[0], Citation { title: "论文".into(), url: "https://a.org/p?x=1".into() });
        assert_eq!(c[1], Citation { title: "".into(), url: "https://b.org/q".into() });
    }

    #[test]
    fn bare_url_stops_at_cjk_and_keeps_wikipedia_parens() {
        let c = extract_citations("参考 https://zh.wikipedia.org/wiki/Rust_(编程语言)，以及 https://en.wikipedia.org/wiki/Rust_(programming_language).");
        assert_eq!(c[0].url, "https://zh.wikipedia.org/wiki/Rust_(");
        assert_eq!(c[1].url, "https://en.wikipedia.org/wiki/Rust_(programming_language)");
    }

    #[test]
    fn first_nonempty_title_wins() {
        let c = extract_citations("https://x.io/a 然后 [标题](https://x.io/a)");
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].title, "标题");
    }

    #[test]
    fn merge_marks_verification_and_drop_turn_removes() {
        let mut table = vec![];
        let cites = extract_citations("[a](https://a.io) [b](https://b.io)");
        let exempt: HashSet<String> = ["https://b.io".to_string()].into_iter().collect();
        let added = merge_turn(&mut table, Some("t1"), &cites, false, &exempt);
        assert_eq!(added.len(), 2);
        assert!(!table[0].verified && table[1].verified);
        // a later searched turn confirms a.io and adds nothing new
        let added = merge_turn(&mut table, Some("t2"), &cites[..1], true, &HashSet::new());
        assert!(added.is_empty() && table[0].verified);
        drop_turn(&mut table, "t1");
        assert!(table.is_empty(), "both entries belonged to t1");
    }

    #[test]
    fn read_sources_replays_and_honours_retry_marker() {
        let jsonl = r#"{"kind":"user","turn":"t1","content":"q"}
{"kind":"sources","turn":"t1","items":[{"url":"https://a.io","title":"A","verified":true}]}
{"kind":"user","turn":"t2","content":"q2"}
{"kind":"sources","turn":"t2","items":[{"url":"https://b.io","title":"","verified":false}]}
{"kind":"retry"}
{"kind":"sources","turn":"t2","items":[{"url":"https://c.io","title":"","verified":false}]}
"#;
        let table = read_sources(jsonl);
        let urls: Vec<&str> = table.iter().map(|s| s.url.as_str()).collect();
        assert_eq!(urls, vec!["https://a.io", "https://c.io"]);
        assert!(format_sources(&table).contains("[unverified"));
        assert_eq!(format_sources(&[]), "(no web sources in this session yet)");
    }
}
