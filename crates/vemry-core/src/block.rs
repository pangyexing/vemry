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
    /// Byte range in the note's raw text (the `[[…]]` itself; a leading `!` is
    /// not included).
    pub span: Range<usize>,
    /// True when the link was written as `![[…]]` — an embed. The link graph
    /// treats it like any other link; only rendering differs.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub embed: bool,
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
            embed: false,
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
    /// One list item (`- `, `* `, `+ `, `1. `, `1) `) at any indentation; a
    /// nested item is its own block, so an id on an item covers that item
    /// only, not its children. Checkbox items are `Task`, not `ListItem`.
    ListItem {
        /// Leading whitespace characters before the marker (a tab counts as one).
        indent: usize,
        ordered: bool,
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
    /// A `> [!private]` callout: the block-level privacy zone. Fully visible
    /// in the UI, nonexistent to the agent — and its tags, links and fields
    /// are never indexed, for anyone.
    Private,
}

/// An inline field `key:: value` found in a block — written as a whole line,
/// as `[key:: value]` / `(key:: value)` anywhere in a line, or as a bare
/// `key:: token` inside a line. The value is kept as written; consumers
/// interpret it (date, number, link, list).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub key: String,
    pub value: String,
    /// Byte range of the whole occurrence (brackets included) in the raw text.
    pub span: Range<usize>,
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
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub fields: Vec<Field>,
    /// True when the id came from an explicit `^id` marker.
    #[serde(default)]
    pub explicit_id: bool,
    /// When this block's current text appeared: its note's mtime (ms since the
    /// epoch) at the first indexing that saw it (see blocktimes.rs). 0 straight
    /// out of the parser; the vault stamps it before indexing.
    #[serde(default)]
    pub modified_ms: u64,
}

impl Block {
    /// How this block is cited: an explicit `^id` keeps its caret, a positional
    /// `bN` does not — so a reader can tell a stable reference from one that is
    /// only valid until the note changes, without a second field. The agent's
    /// answers and its query results both read this one rule.
    pub fn handle(&self) -> String {
        if self.explicit_id { format!("^{}", self.local_id) } else { self.local_id.clone() }
    }
}
