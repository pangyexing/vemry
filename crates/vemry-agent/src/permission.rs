//! The permission gate. Write tools are prepared into a `WriteRequest`
//! (path + rendered diff + the exact content that would land on disk) and must
//! pass this gate before touching the vault. The gate lives in the Rust layer
//! that owns file-write capability, so no frontend can bypass it.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// What kind of write a `WriteRequest` proposes. Carried as a value rather than
/// as a rendered sentence: this layer cannot know the user's language, and the
/// wording belongs with the rest of it in `locales/*.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteKind {
    Create,
    Overwrite,
    Edit,
    Move,
    /// append_note onto an existing note (a new one is a Create).
    Append,
}

#[derive(Clone, Debug, Serialize)]
pub struct WriteRequest {
    pub id: String,
    pub tool: String,
    pub path: String,
    /// move_note's destination (None for every other tool). skip_serializing_if
    /// keeps the JSON shape of the old events byte for byte — a WriteRequest is
    /// raised, used and dropped through the event channel; it is never stored or
    /// sent back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_path: Option<String>,
    /// Unified diff of the proposed change (empty for brand-new files and moves).
    pub diff: String,
    /// What kind of write this is. The wording lives in the UI catalog — this
    /// layer carries no copy, because it cannot know the user's language.
    /// `WriteRequest` is thrown through an event and consumed immediately (never
    /// stored, never replayed), so this shape has no backwards-compatibility debt.
    pub kind: WriteKind,
    /// Full file content that would be written if allowed (empty for moves).
    pub new_content: String,
    /// A move's link rewrites, one summary per note that changes (the diffs are in
    /// `diff`); empty for every other write. The texts to land travel separately
    /// (`rewrite_texts`) and never over the event channel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rewrites: Vec<RewriteSummary>,
    #[serde(skip)]
    pub rewrite_texts: Vec<RewriteText>,
    /// For a block-level rewrite (a restructure, a block move, a whole-note
    /// overwrite): how the note compares block by block, so the dialog can show
    /// "moved 12 · changed 3 · added 2 · removed 0" instead of a wall of red and green.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_diff: Option<vemry_core::BlockDiff>,
    /// Lines for the model's tool result beyond "wrote …" (what landed unassigned, say).
    #[serde(skip)]
    pub notes: Vec<String>,
}

/// One note a move rewrites links in, as the dialog shows it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RewriteSummary {
    pub path: String,
    pub links: usize,
}

/// The text of one rewrite: what the note holds now (re-checked at execute time)
/// and what it becomes.
#[derive(Clone, Debug, PartialEq)]
pub struct RewriteText {
    pub path: String,
    pub old: String,
    pub new: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    /// Allow and stop asking for the rest of this session.
    AllowSession,
    /// Allow and persist the policy for this vault.
    AllowAlways,
    Deny { reason: String },
    /// User edited the proposed content before applying (takeover).
    AllowEdited { content: String },
}

#[async_trait]
pub trait PermissionGate: Send + Sync {
    async fn request(&self, req: &WriteRequest) -> PermissionDecision;
}

/// Test/automation gates.
pub struct AutoAllow;

#[async_trait]
impl PermissionGate for AutoAllow {
    async fn request(&self, _req: &WriteRequest) -> PermissionDecision {
        PermissionDecision::Allow
    }
}

pub struct AutoDeny(pub String);

#[async_trait]
impl PermissionGate for AutoDeny {
    async fn request(&self, _req: &WriteRequest) -> PermissionDecision {
        PermissionDecision::Deny { reason: self.0.clone() }
    }
}

/// The plan confirmation gate (propose_plan): a blocking gate of the same family
/// as the write permission — the runner suspends at an await, the user's decision
/// goes back to the model as a tool result, and once confirmed execution
/// continues within the same turn.
#[derive(Clone, Debug, Serialize)]
pub struct PlanRequest {
    /// The tool call id: the decision is paired back to its oneshot by it.
    pub id: String,
    pub items: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanDecision {
    Approve,
    /// The user edited the list before confirming — the edited list goes into the
    /// tool result and the model follows it.
    ApproveEdited { items: Vec<String> },
    Reject { reason: String },
}

#[async_trait]
pub trait PlanGate: Send + Sync {
    async fn request(&self, req: &PlanRequest) -> PlanDecision;
}

/// Test/automation gate.
pub struct AutoApprovePlan;

#[async_trait]
impl PlanGate for AutoApprovePlan {
    async fn request(&self, _req: &PlanRequest) -> PlanDecision {
        PlanDecision::Approve
    }
}

/// One action item the model extracted (a meeting's commitments, a tidy-up's moves):
/// the text, and the fields the card lets the user correct before anything is
/// written. `target` is the note it should land in; `source` the sentence it came
/// from (a citation handle or a quote). The wire shape is shared with the frontend.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionItem {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Extra fields the card lets the user edit (a scoring's reach / impact /
    /// confidence / effort, say), key → value; written by the model afterwards with
    /// set_block_field. Kept in the order given.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub fields: std::collections::BTreeMap<String, String>,
}

/// The action-list gate (propose_actions): the same family as the plan gate — the
/// runner suspends, the user reviews the list (unticking, editing dates, owners and
/// targets) and the confirmed list goes back to the model as a tool result; only
/// then does the model write each item into its target through the ordinary write
/// tools. Nothing here touches the vault.
#[derive(Clone, Debug, Serialize)]
pub struct ActionsRequest {
    pub id: String,
    pub items: Vec<ActionItem>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionsDecision {
    /// The items the user kept, as edited (the unticked ones are simply absent).
    Approve { items: Vec<ActionItem> },
    Reject { reason: String },
}

#[async_trait]
pub trait ActionsGate: Send + Sync {
    async fn request(&self, req: &ActionsRequest) -> ActionsDecision;
}

/// Test/automation gate: every item approved as proposed.
pub struct AutoApproveActions;

#[async_trait]
impl ActionsGate for AutoApproveActions {
    async fn request(&self, req: &ActionsRequest) -> ActionsDecision {
        ActionsDecision::Approve { items: req.items.clone() }
    }
}
