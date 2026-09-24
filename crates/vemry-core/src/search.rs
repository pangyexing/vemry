//! Full-text search. The MVP provider is an in-memory multi-term AND
//! substring matcher — no tokenizer, which makes it work for CJK text out of
//! the box. Swap in tantivy/embeddings later by implementing `SearchProvider`.

use serde::Serialize;
use std::collections::HashMap;
use unicode_normalization::{is_nfkc, UnicodeNormalization};

use crate::note::NoteId;

#[derive(Clone, Debug, Serialize)]
pub struct SearchHit {
    pub note: NoteId,
    pub title: String,
    pub snippet: String,
    pub line: usize,
    /// Where on `line` the hit starts, in UTF-16 units (the editor's offsets), so the
    /// palette opens the note on the word rather than re-finding it itself.
    pub col: usize,
    pub score: f32,
}

/// One string as every lookup in the vault reads it: NFKC, then lower case.
///
/// NFKC folds the forms a keyboard or a filesystem can hand over without the user
/// meaning anything by them — half-width kana (`ｶ` → `カ`), full-width digits (`３` →
/// `3`), the compatibility ligatures — and it composes the decomposed accents a macOS
/// filename carries (NFD `café` → NFC `café`), so a Hangul or Latin name typed in one
/// form finds a note stored in the other. Both the stored key and the probe go through
/// this one function; nothing compares a folded string against an unfolded one.
pub fn fold(s: &str) -> String {
    normalize(s).to_lowercase()
}

/// The key a note name or path is looked up by: canonical equivalence (NFC, so a
/// name typed on a Mac's decomposed filesystem meets its composed spelling) and
/// case-insensitive, but no compatibility folding — `①.md` and `1.md` are two
/// files and must stay two keys.
pub fn fold_name(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    s.nfc().collect::<String>().to_lowercase()
}

/// NFKC alone, and only where it changes something: for the overwhelming majority of
/// documents the normalised text is the text, and the caller keeps the original.
pub fn normalize(s: &str) -> std::borrow::Cow<'_, str> {
    if is_nfkc(s) {
        std::borrow::Cow::Borrowed(s)
    } else {
        std::borrow::Cow::Owned(s.nfkc().collect())
    }
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
    /// The text as it is on disk — what `note_text` hands back, and what the vault
    /// diffs its own writes against. Never used for matching.
    content: String,
    /// The NFKC form, kept only when it differs from `content` — the text the snippet
    /// and the line number are read out of. `lower` is its lower case, so the two share
    /// every byte offset except where case folding changes a character's length
    /// (`İ` → `i̇`), which `make_snippet` snaps back from. Reading the snippet out of
    /// `content` instead would not work: NFKC moves offsets much further than case does.
    norm: Option<String>,
    lower: String,
    title_lower: String,
}

impl Doc {
    /// The text the offsets in `lower` point into.
    fn snippet_src(&self) -> &str {
        self.norm.as_deref().unwrap_or(&self.content)
    }
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
                norm: match normalize(content) {
                    std::borrow::Cow::Borrowed(_) => None,
                    std::borrow::Cow::Owned(n) => Some(n),
                },
                lower: fold(content),
                title_lower: fold(title),
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
        // Scoring is all a match costs; the snippet, the line and the column are worked out
        // only for the hits that survive the cut
        let mut scored: Vec<(f32, &NoteId, &Doc)> = self
            .docs
            .iter()
            .filter_map(|(id, doc)| score_doc(&doc.title_lower, &doc.lower, &terms).map(|score| (score, id, doc)))
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal).then(a.1.cmp(b.1)));
        scored.truncate(limit);
        scored
            .into_iter()
            .map(|(score, id, doc)| {
                let (snippet, line, col) = locate(&doc.lower, doc.snippet_src(), &terms);
                SearchHit { note: id.clone(), title: doc.title.clone(), snippet, line, col, score }
            })
            .collect()
    }
}

/// The query's terms, folded (empty = nothing to search for).
pub fn query_terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(fold).filter(|t| !t.is_empty()).collect()
}

/// Score one document against the terms the way `InMemorySearch` does —
/// (score, snippet, line, col) when every term matches, else `None`. Exposed so
/// a caller can re-match a document over an altered text (the agent's
/// search re-checks notes with private blocks against a blanked copy).
pub fn match_doc(title: &str, content: &str, terms: &[String]) -> Option<(f32, String, usize, usize)> {
    let norm = normalize(content);
    let lower = norm.to_lowercase();
    let score = score_doc(&fold(title), &lower, terms)?;
    let (snippet, line, col) = locate(&lower, &norm, terms);
    Some((score, snippet, line, col))
}

/// Score order, ties by note id — the one ordering every search result uses.
pub fn sort_hits(hits: &mut [SearchHit]) {
    hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then(a.note.cmp(&b.note)));
}

/// The score when every term is in the title or the body, else `None`.
fn score_doc(title_lower: &str, lower: &str, terms: &[String]) -> Option<f32> {
    let mut score = 0f32;
    for term in terms {
        let in_title = title_lower.contains(term.as_str());
        let n = lower.matches(term.as_str()).count();
        if !in_title && n == 0 {
            return None;
        }
        if in_title {
            score += 5.0;
        }
        score += n as f32;
    }
    Some(score)
}

/// The snippet, the 1-based line and the column (UTF-16 units) a hit is shown and opened at.
fn locate(lower: &str, content: &str, terms: &[String]) -> (String, usize, usize) {
    make_snippet(content, best_line_pos(lower, terms))
}

/// Where the snippet (and the line a hit opens at) is taken from: the line holding the
/// most of the query's terms, the earliest such line on a tie, at its first term. Taking
/// the first occurrence of any term instead lands a query of several words on whichever
/// line a short word happens to appear in first ("it" inside a frontmatter "title:").
fn best_line_pos(lower: &str, terms: &[String]) -> usize {
    if let [term] = terms {
        return lower.find(term.as_str()).unwrap_or(0);
    }
    let mut best: Option<(usize, usize)> = None; // (terms on the line, position of the first)
    let mut start = 0;
    for line in lower.split('\n') {
        let mut count = 0;
        let mut first = usize::MAX;
        for term in terms {
            if let Some(p) = line.find(term.as_str()) {
                count += 1;
                first = first.min(p);
            }
        }
        if count > 0 && best.is_none_or(|(n, _)| count > n) {
            best = Some((count, start + first));
            // Nothing later can hold more, and a tie keeps the earliest
            if count == terms.len() {
                break;
            }
        }
        start += line.len() + 1;
    }
    best.map_or(0, |(_, pos)| pos)
}

/// The line containing `pos`, trimmed to ~160 chars around the hit. `content` is the
/// NFKC text and `pos` was found in its lower case, whose byte offsets drift from it
/// where a character changes length when folded (`İ` → `i̇`), so it is snapped back to
/// a character boundary before slicing. Newlines survive both foldings, so the line
/// number is the one the file has.
fn make_snippet(content: &str, pos: usize) -> (String, usize, usize) {
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
    (snippet.trim().to_string(), line_no + 1, content[line_start..pos].encode_utf16().count())
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

    /// Several words land on the line that holds them together, not on the first line
    /// where any one of them occurs.
    #[test]
    fn snippet_takes_the_line_with_the_most_terms() {
        let mut s = InMemorySearch::new();
        s.index_note(&NoteId::new("n1"), "t", "---\ntitle: Start\n---\nnothing here\nThe claim has nothing behind it yet.");
        let hits = s.search("nothing behind it", 10);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 5);
        assert_eq!(hits[0].col, "The claim has ".len());
        assert!(hits[0].snippet.starts_with("The claim"));
        // the column counts UTF-16 units, as the editor does
        s.index_note(&NoteId::new("n2"), "t", "中文 𠀀 target");
        assert_eq!(s.search("target", 10)[0].col, 6);
    }

    /// Half-width kana and full-width digits are what an IME hands over; NFKC folds
    /// them to the forms the user typed in the search box.
    #[test]
    fn halfwidth_kana_and_fullwidth_digits_fold() {
        let mut s = InMemorySearch::new();
        s.index_note(&NoteId::new("n1"), "ｶﾀｶﾅ３", "body");
        assert_eq!(s.search("カタカナ3", 10).len(), 1);
        s.index_note(&NoteId::new("n2"), "t", "ｾｸｼｮﾝ１ here");
        assert_eq!(s.search("セクション1", 10).len(), 1);
    }

    /// A macOS filename and a note typed on it arrive decomposed; a query typed
    /// anywhere else is composed. Both forms have to meet.
    #[test]
    fn decomposed_and_composed_meet() {
        let mut s = InMemorySearch::new();
        s.index_note(&NoteId::new("n1"), "cafe\u{301}", "cafe\u{301} au lait");
        assert_eq!(s.search("caf\u{e9}", 10).len(), 1);
        // and the other way round
        let mut s2 = InMemorySearch::new();
        s2.index_note(&NoteId::new("n2"), "caf\u{e9}", "x");
        assert_eq!(s2.search("cafe\u{301}", 10).len(), 1);
        // Hangul: NFD jamo against NFC syllables
        let mut s3 = InMemorySearch::new();
        s3.index_note(&NoteId::new("n3"), "t", "\u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af} note");
        assert_eq!(s3.search("한글", 10).len(), 1);
    }

    /// The snippet is shown to the user: normalising must not lower-case it.
    #[test]
    fn snippet_keeps_its_case() {
        let mut s = InMemorySearch::new();
        s.index_note(&NoteId::new("n1"), "t", "The Quick Brown Fox");
        assert_eq!(s.search("quick", 10)[0].snippet, "The Quick Brown Fox");
    }
}
