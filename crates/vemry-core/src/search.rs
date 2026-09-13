//! Full-text search. The MVP provider is an in-memory multi-term AND
//! substring matcher — no tokenizer, which makes it work for CJK text out of
//! the box. Swap in tantivy/embeddings later by implementing `SearchProvider`.

use serde::Serialize;
use std::collections::HashMap;

use crate::note::NoteId;

#[derive(Clone, Debug, Serialize)]
pub struct SearchHit {
    pub note: NoteId,
    pub title: String,
    pub snippet: String,
    pub line: usize,
    pub score: f32,
}

pub trait SearchProvider: Send + Sync {
    fn index_note(&mut self, id: &NoteId, title: &str, content: &str);
    fn remove_note(&mut self, id: &NoteId);
    fn search(&self, query: &str, limit: usize) -> Vec<SearchHit>;
    /// The text as indexed, for a provider that keeps it. The vault reads it to
    /// tell the echo of its own write from a change made outside the app, and to
    /// keep the text such a change replaced (history.rs); a provider that answers
    /// `None` loses only those two — every note change still gets its event.
    fn note_text(&self, _id: &NoteId) -> Option<&str> {
        None
    }
}

struct Doc {
    title: String,
    content: String,
    lower: String,
    title_lower: String,
}

#[derive(Default)]
pub struct InMemorySearch {
    docs: HashMap<NoteId, Doc>,
}

impl InMemorySearch {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SearchProvider for InMemorySearch {
    fn index_note(&mut self, id: &NoteId, title: &str, content: &str) {
        self.docs.insert(
            id.clone(),
            Doc {
                title: title.to_string(),
                content: content.to_string(),
                lower: content.to_lowercase(),
                title_lower: title.to_lowercase(),
            },
        );
    }

    fn remove_note(&mut self, id: &NoteId) {
        self.docs.remove(id);
    }

    fn note_text(&self, id: &NoteId) -> Option<&str> {
        self.docs.get(id).map(|d| d.content.as_str())
    }

    fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let terms = query_terms(query);
        if terms.is_empty() {
            return vec![];
        }
        let mut hits = Vec::new();
        for (id, doc) in &self.docs {
            if let Some((score, snippet, line)) = score_doc(&doc.title_lower, &doc.lower, &doc.content, &terms) {
                hits.push(SearchHit { note: id.clone(), title: doc.title.clone(), snippet, line, score });
            }
        }
        sort_hits(&mut hits);
        hits.truncate(limit);
        hits
    }
}

/// The query's terms, lowercased (empty = nothing to search for).
pub fn query_terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(|t| t.to_lowercase()).filter(|t| !t.is_empty()).collect()
}

/// Score one document against the terms the way `InMemorySearch` does —
/// (score, snippet, line) when every term matches, else `None`. Exposed so
/// a caller can re-match a document over an altered text (the agent's
/// search re-checks notes with private blocks against a blanked copy).
pub fn match_doc(title: &str, content: &str, terms: &[String]) -> Option<(f32, String, usize)> {
    score_doc(&title.to_lowercase(), &content.to_lowercase(), content, terms)
}

/// Score order, ties by note id — the one ordering every search result uses.
pub fn sort_hits(hits: &mut [SearchHit]) {
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then(a.note.cmp(&b.note)));
}

fn score_doc(title_lower: &str, lower: &str, content: &str, terms: &[String]) -> Option<(f32, String, usize)> {
    let mut score = 0f32;
    let mut first_pos: Option<usize> = None;
    for term in terms {
        let in_title = title_lower.contains(term.as_str());
        let pos = lower.find(term.as_str());
        if !in_title && pos.is_none() {
            return None;
        }
        if in_title {
            score += 5.0;
        }
        if let Some(p) = pos {
            score += lower.matches(term.as_str()).count() as f32;
            first_pos = Some(first_pos.map_or(p, |f| f.min(p)));
        }
    }
    let (snippet, line) = make_snippet(content, first_pos.unwrap_or(0));
    Some((score, snippet, line))
}

/// The line containing `pos`, trimmed to ~160 chars around the hit. `pos`
/// was found in the lowercased text, whose byte offsets drift from the
/// original's where a character changes length when folded (`İ` → `i̇`),
/// so it is snapped back to a character boundary before slicing.
fn make_snippet(content: &str, pos: usize) -> (String, usize) {
    let mut pos = pos.min(content.len());
    while !content.is_char_boundary(pos) {
        pos -= 1;
    }
    let line_no = content[..pos].matches('\n').count();
    let line_start = content[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = content[pos..].find('\n').map(|i| pos + i).unwrap_or(content.len());
    let line = &content[line_start..line_end];
    let snippet = if line.chars().count() > 160 {
        let s: String = line.chars().take(160).collect();
        format!("{s}…")
    } else {
        line.to_string()
    };
    (snippet.trim().to_string(), line_no + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hit is found in the lowercased text; where a character changes length
    /// when folded, the offset drifts from the original's and must not be sliced
    /// into the middle of a character.
    #[test]
    fn snippet_survives_a_case_fold_that_changes_byte_length() {
        let mut s = InMemorySearch::new();
        s.index_note(&NoteId::new("n1"), "t", "İa中文");
        let hits = s.search("文", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 1);
        assert!(hits[0].snippet.contains("中文"));
        assert!(match_doc("t", "Ka中文\nsecond", &["second".into()]).is_some());
    }
}
