use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// The checkbox of a task line: `[ ]` to do, `[/]` in progress, `[x]` done, `[-]`
/// cancelled. The checkbox is the one truth of the state; the ✅ / ❌ dates only say when.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Todo,
    Doing,
    Done,
    Cancelled,
}

impl TaskState {
    /// Done or cancelled: the task is closed and leaves every list of open work.
    pub fn is_closed(self) -> bool {
        matches!(self, TaskState::Done | TaskState::Cancelled)
    }
}

/// A task's priority: four steps around the unwritten default. Ordered, so a list sorts
/// by it (`Highest` first when descending).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Lowest,
    Low,
    #[default]
    Normal,
    High,
    Highest,
}

impl Priority {
    pub fn is_normal(&self) -> bool {
        *self == Priority::Normal
    }
    /// The `priority::` spelling of the value.
    pub fn as_str(self) -> &'static str {
        match self {
            Priority::Lowest => "lowest",
            Priority::Low => "low",
            Priority::Normal => "normal",
            Priority::High => "high",
            Priority::Highest => "highest",
        }
    }
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

    /// The target with its anchor and without the alias, as `resolve_ref` reads
    /// it: `note`, `note#heading` or `note#^id`.
    pub fn ref_target(&self) -> String {
        match (&self.heading, &self.block_ref) {
            (Some(h), _) => format!("{}#{h}", self.target),
            (None, Some(id)) => format!("{}#^{id}", self.target),
            (None, None) => self.target.clone(),
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
    /// A checkbox line. The attributes after the text are read by `taskattrs.rs` (each
    /// with an emoji and a `key::` spelling); every one is optional and left out of the
    /// JSON when absent.
    Task {
        state: TaskState,
        /// 📅 / `due::` — the deadline.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        due: Option<NaiveDate>,
        /// ⏳ / `scheduled::` — the day the task is planned for.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scheduled: Option<NaiveDate>,
        /// ⏩ / `start::` — the earliest day it can be started.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<NaiveDate>,
        /// ➕ / `created::`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        created: Option<NaiveDate>,
        /// ✅ / `completion::` — stamped when the checkbox is ticked.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        done: Option<NaiveDate>,
        /// ❌ / `cancelled::` — stamped when the checkbox becomes `[-]`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cancelled: Option<NaiveDate>,
        #[serde(default, skip_serializing_if = "Priority::is_normal")]
        priority: Priority,
        /// 🔁 / `repeat::` — the rule's text as written (`every 2 weeks when done`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        repeat: Option<String>,
        /// 🚮 / `onCompletion:: delete` — the line is removed once done.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        delete_on_done: bool,
        /// 🆔 / `id::` — the handle other tasks depend on (not the block id).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task_id: Option<String>,
        /// ⛔ / `dependsOn::` — task ids that must be closed first.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        depends_on: Vec<String>,
        /// 🕐 / `estimate::` — in minutes (`30m`, `2h`, `1d` = 8h).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        estimate: Option<u32>,
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
