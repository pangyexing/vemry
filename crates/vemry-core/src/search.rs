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

    fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let terms: Vec<String> = query.split_whitespace().map(|t| t.to_lowercase()).filter(|t| !t.is_empty()).collect();
        if terms.is_empty() {
            return vec![];
        }
        let mut hits = Vec::new();
        for (id, doc) in &self.docs {
            let mut score = 0f32;
            let mut first_pos: Option<usize> = None;
            let mut all_match = true;
            for term in &terms {
                let in_title = doc.title_lower.contains(term.as_str());
                let pos = doc.lower.find(term.as_str());
                if !in_title && pos.is_none() {
                    all_match = false;
                    break;
                }
                if in_title {
                    score += 5.0;
                }
                if let Some(p) = pos {
                    score += doc.lower.matches(term.as_str()).count() as f32;
                    first_pos = Some(first_pos.map_or(p, |f| f.min(p)));
                }
            }
            if !all_match {
                continue;
            }
            let (snippet, line) = make_snippet(&doc.content, first_pos.unwrap_or(0));
            hits.push(SearchHit { note: id.clone(), title: doc.title.clone(), snippet, line, score });
        }
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then(a.note.cmp(&b.note)));
        hits.truncate(limit);
        hits
    }
}

/// The line containing `pos`, trimmed to ~160 chars around the hit.
fn make_snippet(content: &str, pos: usize) -> (String, usize) {
    let pos = pos.min(content.len());
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
