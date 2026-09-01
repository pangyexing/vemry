//! The unified block query API — the single entry point shared by UI views,
//! user scripts and agent tools. A view is nothing more than a `BlockQuery`
//! plus rendering.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::block::{Block, BlockKind, TaskState};
use crate::index::BlockIndex;
use crate::note::NoteId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKindFilter {
    Paragraph,
    Heading,
    Task,
    Claim,
    Evidence,
    CodeBlock,
}

impl BlockKindFilter {
    fn matches(&self, kind: &BlockKind) -> bool {
        matches!(
            (self, kind),
            (BlockKindFilter::Paragraph, BlockKind::Paragraph)
                | (BlockKindFilter::Heading, BlockKind::Heading { .. })
                | (BlockKindFilter::Task, BlockKind::Task { .. })
                | (BlockKindFilter::Claim, BlockKind::Claim)
                | (BlockKindFilter::Evidence, BlockKind::Evidence { .. })
                | (BlockKindFilter::CodeBlock, BlockKind::CodeBlock { .. })
        )
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

        'outer: for (id, summary, blocks) in self.entries() {
            if let Some(nf) = &note_filter {
                if id != nf {
                    continue;
                }
            }
            for block in blocks {
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
