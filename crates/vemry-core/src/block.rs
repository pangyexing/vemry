use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Todo,
    Doing,
    Done,
}

/// A `[[wikilink]]` occurrence. `target` is the note part; heading/block refs
/// (`[[Note#Heading]]`, `[[Note#^id]]`) are split out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WikiLink {
    pub raw: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_ref: Option<String>,
    /// Byte range in the note's raw text.
    pub span: Range<usize>,
}

impl WikiLink {
    /// Parse the inside of `[[...]]` (without brackets).
    pub fn parse(inner: &str, span: Range<usize>) -> Self {
        let (target_part, alias) = match inner.split_once('|') {
            Some((t, a)) => (t.trim(), Some(a.trim().to_string())),
            None => (inner.trim(), None),
        };
        let (target, heading, block_ref) = match target_part.split_once('#') {
            Some((t, rest)) => {
                if let Some(id) = rest.strip_prefix('^') {
                    (t.trim(), None, Some(id.trim().to_string()))
                } else {
                    (t.trim(), Some(rest.trim().to_string()), None)
                }
            }
            None => (target_part, None, None),
        };
        WikiLink {
            raw: format!("[[{inner}]]"),
            target: target.to_string(),
            alias,
            heading,
            block_ref,
            span,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BlockKind {
    Paragraph,
    Heading {
        level: u8,
    },
    Task {
        state: TaskState,
        #[serde(skip_serializing_if = "Option::is_none")]
        due: Option<NaiveDate>,
        #[serde(skip_serializing_if = "Option::is_none")]
        scheduled: Option<NaiveDate>,
    },
    Claim,
    Evidence {
        /// Raw wikilink targets this evidence supports, e.g. "Note#^c1".
        supports: Vec<String>,
    },
    CodeBlock {
        lang: String,
    },
}

/// A typed block extracted from a note. The unit of every projection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Block {
    /// Local id within the note: an explicit `^id` if present, otherwise a
    /// positional id like "b3" (regenerated on each parse; only explicit `^id`
    /// is stable for cross-note references).
    pub local_id: String,
    pub kind: BlockKind,
    pub content: String,
    /// Byte range in the note's raw text (frontmatter offset included).
    pub span: Range<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub links: Vec<WikiLink>,
    /// True when the id came from an explicit `^id` marker.
    #[serde(default)]
    pub explicit_id: bool,
}
