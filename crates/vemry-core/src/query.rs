//! The unified block query API — the single entry point shared by UI views,
//! user scripts and agent tools. A view is nothing more than a `BlockQuery`
//! plus rendering.

use chrono::NaiveDate;
use serde::{Deserialize, Deserializer, Serialize};

use crate::block::{Block, BlockKind, TaskState};
use crate::index::BlockIndex;
use crate::note::NoteId;
use crate::parser::is_field_key;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKindFilter {
    Paragraph,
    Heading,
    ListItem,
    Task,
    Claim,
    Evidence,
    CodeBlock,
    /// UI, query blocks and scripts only: the agent's `query_blocks` drops
    /// private blocks unconditionally.
    Private,
}

impl BlockKindFilter {
    fn matches(&self, kind: &BlockKind) -> bool {
        matches!(
            (self, kind),
            (BlockKindFilter::Paragraph, BlockKind::Paragraph)
                | (BlockKindFilter::Heading, BlockKind::Heading { .. })
                | (BlockKindFilter::ListItem, BlockKind::ListItem { .. })
                | (BlockKindFilter::Task, BlockKind::Task { .. })
                | (BlockKindFilter::Claim, BlockKind::Claim)
                | (BlockKindFilter::Evidence, BlockKind::Evidence { .. })
                | (BlockKindFilter::CodeBlock, BlockKind::CodeBlock { .. })
                | (BlockKindFilter::Private, BlockKind::Private)
        )
    }
}

/// How a field filter compares. `Lt`/`Gt` compare as dates when both sides
/// are `YYYY-M-D`, else as numbers when both parse, else never match.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldOp {
    Eq,
    Ne,
    Has,
    Lt,
    Gt,
    Contains,
}

/// One inline-field condition. Keys compare case-insensitively; a block's
/// own fields are consulted first, then the note's frontmatter scalars.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldFilter {
    pub key: String,
    pub op: FieldOp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldFilterError {
    Empty,
    BadKey(String),
    MissingValue(FieldOp),
}

impl std::fmt::Display for FieldFilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldFilterError::Empty => f.write_str("empty field filter"),
            FieldFilterError::BadKey(k) => write!(f, "bad field key: {k}"),
            FieldFilterError::MissingValue(op) => write!(f, "field operator {op:?} needs a value"),
        }
    }
}

impl std::error::Error for FieldFilterError {}

/// The flat grammar shared by query blocks, the agent tool and scripts:
/// `key` (has), `key = v`, `key != v`, `key < v`, `key > v`, `key ~ v`
/// (contains). Values may be quoted.
pub fn parse_field_filter(s: &str) -> Result<FieldFilter, FieldFilterError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(FieldFilterError::Empty);
    }
    let key_len = s
        .char_indices()
        .find(|(_, c)| !(c.is_alphanumeric() || matches!(c, '_' | '-')))
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let key = &s[..key_len];
    if !is_field_key(key) {
        return Err(FieldFilterError::BadKey(s.to_string()));
    }
    let rest = s[key_len..].trim_start();
    if rest.is_empty() {
        return Ok(FieldFilter { key: key.to_string(), op: FieldOp::Has, value: None });
    }
    let (op, after) = if let Some(r) = rest.strip_prefix("!=") {
        (FieldOp::Ne, r)
    } else if let Some(r) = rest.strip_prefix('=') {
        (FieldOp::Eq, r)
    } else if let Some(r) = rest.strip_prefix('<') {
        (FieldOp::Lt, r)
    } else if let Some(r) = rest.strip_prefix('>') {
        (FieldOp::Gt, r)
    } else if let Some(r) = rest.strip_prefix('~') {
        (FieldOp::Contains, r)
    } else {
        return Err(FieldFilterError::BadKey(s.to_string()));
    };
    let mut value = after.trim();
    if value.len() >= 2 {
        let first = value.as_bytes()[0];
        if (first == b'"' || first == b'\'') && value.ends_with(first as char) {
            value = &value[1..value.len() - 1];
        }
    }
    if value.is_empty() {
        return Err(FieldFilterError::MissingValue(op));
    }
    Ok(FieldFilter { key: key.to_string(), op, value: Some(value.to_string()) })
}

/// Accepts a single flat string, a single object, or a list of either —
/// so query blocks, the agent tool (a string) and scripts (an object) all
/// deserialize straight into `BlockQuery`.
fn field_filters_flexible<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<FieldFilter>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum One {
        Flat(String),
        Typed(FieldFilter),
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(One),
        Many(Vec<One>),
    }
    let parse = |o: One| match o {
        One::Flat(s) => parse_field_filter(&s).map_err(serde::de::Error::custom),
        One::Typed(f) => Ok(f),
    };
    match Option::<OneOrMany>::deserialize(d)? {
        None => Ok(vec![]),
        Some(OneOrMany::One(o)) => Ok(vec![parse(o)?]),
        Some(OneOrMany::Many(v)) => v.into_iter().map(parse).collect(),
    }
}

fn parse_ymd(s: &str) -> Option<NaiveDate> {
    let mut it = s.trim().split('-');
    let y = it.next()?.parse().ok()?;
    let m = it.next()?.parse().ok()?;
    let d = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    NaiveDate::from_ymd_opt(y, m, d)
}

/// Does one field value satisfy the filter?
pub(crate) fn field_matches(value: &str, op: FieldOp, wanted: Option<&str>) -> bool {
    let v = value.trim();
    match (op, wanted) {
        (FieldOp::Has, _) => true,
        (FieldOp::Eq, Some(w)) => v == w.trim(),
        (FieldOp::Ne, Some(w)) => v != w.trim(),
        (FieldOp::Contains, Some(w)) => v.to_lowercase().contains(&w.trim().to_lowercase()),
        (FieldOp::Lt | FieldOp::Gt, Some(w)) => {
            let less = op == FieldOp::Lt;
            if let (Some(a), Some(b)) = (parse_ymd(v), parse_ymd(w)) {
                return if less { a < b } else { a > b };
            }
            if let (Ok(a), Ok(b)) = (v.parse::<f64>(), w.trim().parse::<f64>()) {
                return if less { a < b } else { a > b };
            }
            false
        }
        _ => false,
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BlockQuery {
    #[serde(default)]
    pub kind: Option<BlockKindFilter>,
    /// Restrict to one note (vault-relative path).
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    /// Case-insensitive substring match on block content.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub task_state: Option<TaskState>,
    #[serde(default)]
    pub due_before: Option<NaiveDate>,
    /// Inline-field conditions, all of which must hold (see `parse_field_filter`).
    #[serde(default, deserialize_with = "field_filters_flexible", skip_serializing_if = "Vec::is_empty")]
    pub field: Vec<FieldFilter>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlockHit {
    pub note: NoteId,
    pub note_title: String,
    pub block: Block,
}

impl BlockIndex {
    pub fn query(&self, q: &BlockQuery) -> Vec<BlockHit> {
        let limit = q.limit.unwrap_or(200);
        let text_lower = q.text.as_ref().map(|t| t.to_lowercase());
        let note_filter = q.note.as_ref().map(|n| NoteId::new(n));
        let mut hits = Vec::new();

        // Note order is deterministic so `limit` cuts a stable prefix;
        // blocks are already in document order within a note.
        'outer: for (id, entry) in self.entries_sorted() {
            if let Some(nf) = &note_filter {
                if id != nf {
                    continue;
                }
            }
            let summary = &entry.summary;
            for block in &entry.blocks {
                if let Some(kf) = &q.kind {
                    if !kf.matches(&block.kind) {
                        continue;
                    }
                }
                if let Some(tag) = &q.tag {
                    if !block.tags.iter().any(|t| t == tag) && !summary.tags.contains(tag) {
                        continue;
                    }
                }
                if let Some(tl) = &text_lower {
                    if !block.content.to_lowercase().contains(tl.as_str()) {
                        continue;
                    }
                }
                if let Some(ts) = &q.task_state {
                    match &block.kind {
                        BlockKind::Task { state, .. } if state == ts => {}
                        _ => continue,
                    }
                }
                if let Some(before) = &q.due_before {
                    match &block.kind {
                        BlockKind::Task { due: Some(d), .. } if d <= before => {}
                        _ => continue,
                    }
                }
                if !q.field.iter().all(|f| {
                    let key = f.key.to_lowercase();
                    let mut own = block.fields.iter().filter(|x| x.key.to_lowercase() == key).map(|x| x.value.as_str()).peekable();
                    if own.peek().is_some() {
                        own.any(|v| field_matches(v, f.op, f.value.as_deref()))
                    } else {
                        entry
                            .fm_fields
                            .iter()
                            .filter(|(k, _)| k.to_lowercase() == key)
                            .any(|(_, v)| field_matches(v, f.op, f.value.as_deref()))
                    }
                }) {
                    continue;
                }
                hits.push(BlockHit {
                    note: id.clone(),
                    note_title: summary.title.clone(),
                    block: block.clone(),
                });
                if hits.len() >= limit {
                    break 'outer;
                }
            }
        }
        hits.sort_by(|a, b| a.note.cmp(&b.note).then(a.block.span.start.cmp(&b.block.span.start)));
        hits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_grammar_parses_every_operator() {
        let f = |s: &str| parse_field_filter(s).unwrap();
        assert_eq!(f("author"), FieldFilter { key: "author".into(), op: FieldOp::Has, value: None });
        assert_eq!(f("author = X").value.as_deref(), Some("X"));
        assert_eq!(f("author=X").op, FieldOp::Eq);
        assert_eq!(f("status != done").op, FieldOp::Ne);
        assert_eq!(f("due < 2026-09-10").op, FieldOp::Lt);
        assert_eq!(f("rating > 3").op, FieldOp::Gt);
        assert_eq!(f("author ~ guin").op, FieldOp::Contains);
        assert_eq!(f("topic = \"#work\"").value.as_deref(), Some("#work"));
        assert_eq!(f("作者 = 勒古恩").key, "作者");
        assert_eq!(parse_field_filter(""), Err(FieldFilterError::Empty));
        assert_eq!(parse_field_filter("12 = x"), Err(FieldFilterError::BadKey("12 = x".into())));
        assert_eq!(parse_field_filter("author X"), Err(FieldFilterError::BadKey("author X".into())));
        assert_eq!(parse_field_filter("author ="), Err(FieldFilterError::MissingValue(FieldOp::Eq)));
    }

    #[test]
    fn flexible_deserialization_accepts_string_object_and_list() {
        let q: BlockQuery = serde_json::from_str(r#"{"field": "status = reading"}"#).unwrap();
        assert_eq!(q.field[0].value.as_deref(), Some("reading"));
        let q: BlockQuery = serde_json::from_str(r#"{"field": {"key": "author", "op": "has"}}"#).unwrap();
        assert_eq!(q.field[0].op, FieldOp::Has);
        let q: BlockQuery =
            serde_json::from_str(r#"{"field": ["rating > 3", {"key": "status", "op": "ne", "value": "done"}]}"#).unwrap();
        assert_eq!(q.field.len(), 2);
        let q: BlockQuery = serde_json::from_str(r#"{"kind": "task"}"#).unwrap();
        assert!(q.field.is_empty());
        assert!(serde_json::from_str::<BlockQuery>(r#"{"field": "author X"}"#).is_err());
    }

    #[test]
    fn value_comparison_by_date_number_and_text() {
        assert!(field_matches("2026-09-01", FieldOp::Lt, Some("2026-09-10")));
        assert!(!field_matches("2026-09-10", FieldOp::Lt, Some("2026-9-10")));
        assert!(field_matches("4", FieldOp::Gt, Some("3.5")));
        assert!(!field_matches("high", FieldOp::Gt, Some("3")));
        assert!(field_matches("Ursula K. Le Guin", FieldOp::Contains, Some("guin")));
        assert!(field_matches("x", FieldOp::Ne, Some("y")));
        assert!(field_matches("anything", FieldOp::Has, None));
        assert!(!field_matches("x", FieldOp::Eq, None));
    }
}
