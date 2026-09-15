//! The permission gate. Write tools are prepared into a `WriteRequest`
//! (path + rendered diff + the exact content that would land on disk) and must
//! pass this gate before touching the vault. The gate lives in the Rust layer
//! that owns file-write capability, so no frontend can bypass it.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// The reason a gate's decision carries when nobody decided: the turn was stopped,
/// the window closed or the vault switched while the request waited. The app's
/// fallbacks send it, the runner words the tool result from it (a cancellation,
/// never a request to revise or retry that would outlive the turn), and the
/// frontend keys its "cancelled" badge on the same literal.
pub const CANCELLED: &str = "cancelled";

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
    /// The note's raw text when the request was prepared (`None`: no note there
    /// yet). execute_write lands only on this exact text — see there.
    #[serde(skip)]
    pub baseline: Option<String>,
}

impl WriteRequest {
    /// A write with no move destination, no link rewrites, no block diff and no
    /// extra result lines — the shape most write tools raise. The four that carry
    /// more (the two moves, the two block-level rewrites) set those fields after.
    pub fn new(id: String, tool: &str, path: String, kind: WriteKind, diff: String, new_content: String) -> Self {
        Self {
            id,
            tool: tool.to_string(),
            path,
            to_path: None,
            diff,
            kind,
            new_content,
            rewrites: vec![],
            rewrite_texts: vec![],
            block_diff: None,
            notes: vec![],
            baseline: None,
        }
    }

    /// Every note the write would touch: the main path, a move's destination, and
    /// the notes the side writes land in (`rewrite_texts` — a block move's
    /// destination, the referrers that follow a move). A standing grant covers the
    /// request only when it covers all of them.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.path.as_str())
            .chain(self.to_path.as_deref())
            .chain(self.rewrite_texts.iter().map(|r| r.path.as_str()))
    }
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

impl RewriteText {
    /// Land this rewrite, or leave the note alone. The note takes the new text only
    /// while it still holds the old one — a note edited since the plan was made is
    /// never overwritten. `revert` swaps the two (undo's direction, same rule).
    /// Returns whether it landed.
    pub fn apply(&self, vault: &mut vemry_core::Vault, revert: bool) -> bool {
        let (expect, land) = if revert { (&self.new, &self.old) } else { (&self.old, &self.new) };
        let Ok(id) = vault.note_id(&self.path) else { return false };
        matches!(vault.read_note(&id), Ok(cur) if cur == *expect) && vault.write_note_as(&id, land, vemry_core::Writer::Agent).is_ok()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    /// Allow and stop asking for the rest of this session.
    AllowSession,
    /// Allow, and stop asking for the rest of this session for writes under
    /// `prefix` alone — a folder (`projects/x/`, trailing slash) or one note's path.
    AllowScope { prefix: String },
    /// Allow and persist the policy for this vault.
    AllowAlways,
    Deny { reason: String },
    /// User edited the proposed content before applying (takeover).
    AllowEdited { content: String },
}

/// The standing write grants of a session — what answers a write request without a
/// dialog. `all` is "allow for this session"; `prefixes` are the scoped grants (a
/// folder with its trailing slash, or one note's exact path). A write is covered only
/// when every note it touches is (`WriteRequest::paths`: a move's two ends, a block
/// move's destination, the referrers relinked). The glue layer holds one per session and clears it
/// with the session; the runner reads it at the start of a turn and adds to it as
/// the user grants.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteGrants {
    pub all: bool,
    pub prefixes: Vec<String>,
}

impl WriteGrants {
    pub fn covers_path(&self, path: &str) -> bool {
        self.all || self.prefixes.iter().any(|p| path == p || (p.ends_with('/') && path.starts_with(p.as_str())))
    }

    pub fn covers(&self, req: &WriteRequest) -> bool {
        req.paths().all(|p| self.covers_path(p))
    }

    /// Add a scoped grant (a duplicate is not added twice).
    pub fn add(&mut self, prefix: &str) {
        if !self.prefixes.iter().any(|p| p == prefix) {
            self.prefixes.push(prefix.to_string());
        }
    }

    pub fn clear(&mut self) {
        self.all = false;
        self.prefixes.clear();
    }
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
    /// set_block_field. Sorted by key (the order the model gave them is not kept).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub fields: std::collections::BTreeMap<String, String>,
    /// The item stays where it already is (a capture with no home in a tidy-up): the
    /// card shows it as staying, unticked, and it comes back in the kept list only if
    /// the user gives it a target there. Proposed without a target.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stay: bool,
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

/// One web page the read gate fetched for the model (through the reader window's
/// extraction pipeline, the same one a clip goes through).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FetchedPage {
    /// The URL that was asked for. A redirect makes it differ from `url`, and the model
    /// reads on by what it wrote, so this is what the turn's cache is keyed by.
    pub requested: String,
    pub url: String,
    pub title: String,
    pub markdown: String,
}

/// The read gate (open_for_reading): the agent's autonomous network capability
/// stays at zero — it can only *propose* URLs, and each fetch is the user's click,
/// until the user gives the session's standing consent (the card's checkbox), after
/// which the gate fetches what is proposed and the card only reports it. The gate's
/// implementation fetches the pages allowed and hands them back; the runner wraps
/// them in the untrusted web-content envelope.
#[derive(Clone, Debug, Serialize)]
pub struct ReadRequest {
    pub id: String,
    pub urls: Vec<String>,
    /// The standing consent answers this request: the card lists the URLs without
    /// its buttons, and the gate fetches them without waiting.
    #[serde(default)]
    pub auto: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadDecision {
    /// The pages of the URLs the user allowed (an allowed URL that failed to load is
    /// listed in `failed`, worded for the model).
    Pages {
        pages: Vec<FetchedPage>,
        #[serde(default)]
        failed: Vec<String>,
    },
    Deny { reason: String },
}

#[async_trait]
pub trait ReadGate: Send + Sync {
    /// Whether the standing consent answers this request (asked before the card is
    /// raised, so the card can say so). The default has no standing consent.
    fn standing(&self, _req: &ReadRequest) -> bool {
        false
    }
    async fn request(&self, req: &ReadRequest) -> ReadDecision;
}

/// Test gate: every URL denied (the default for a runner with no reader).
pub struct DenyReads;

#[async_trait]
impl ReadGate for DenyReads {
    async fn request(&self, _req: &ReadRequest) -> ReadDecision {
        ReadDecision::Deny { reason: "no reader available".into() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scoped grant covers a write only when every note it touches is inside the
    /// scope: the side writes (a block move's destination, a relinked referrer)
    /// count, not just the main path and a move's destination.
    #[test]
    fn scope_grant_covers_every_path_the_write_touches() {
        let mut grants = WriteGrants::default();
        grants.add("inbox/");
        let mut req = WriteRequest::new("r".into(), "move_blocks", "inbox/a.md".into(), WriteKind::Edit, String::new(), String::new());
        assert!(grants.covers(&req));
        req.rewrite_texts.push(RewriteText { path: "projects/plan.md".into(), old: "x".into(), new: "y".into() });
        assert!(!grants.covers(&req), "the destination outside the scope needs a dialog");
        req.rewrite_texts[0].path = "inbox/b.md".into();
        assert!(grants.covers(&req));
        req.to_path = Some("archive/a.md".into());
        assert!(!grants.covers(&req));
        assert!(WriteGrants { all: true, prefixes: vec![] }.covers(&req));
    }
}
