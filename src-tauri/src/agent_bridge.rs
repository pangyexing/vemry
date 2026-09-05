//! Bridges the agent runner to the frontend: streams AgentEvents over a Tauri
//! Channel and routes permission decisions back into the pending gate via
//! oneshot channels.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tauri::Manager;
use tokio::sync::oneshot;
use vemry_agent::types::ChatMessage;
use vemry_agent::{
    ActionsDecision, ActionsGate, ActionsRequest, FetchedPage, PermissionDecision, PermissionGate, PlanDecision, PlanGate,
    PlanRequest, ReadDecision, ReadGate, ReadRequest, WriteRequest,
};

/// One pending table: the gate inserts a oneshot at `park` and awaits it, and the
/// decision resolves it back through the matching `agent_*_respond` command.
pub type Pending<D> = Arc<Mutex<HashMap<String, oneshot::Sender<D>>>>;

/// What every user-decision type has to provide: the fallback decision for a
/// broken channel (cancelled, the window closed, the vault switched).
/// With it, the three pieces of plumbing — gate, resolve, drain — are written once
/// for every gate.
pub trait Cancellable: Send + 'static {
    fn cancelled() -> Self;
}
impl Cancellable for PermissionDecision {
    fn cancelled() -> Self {
        PermissionDecision::Deny { reason: "cancelled".into() }
    }
}
impl Cancellable for PlanDecision {
    fn cancelled() -> Self {
        PlanDecision::Reject { reason: "cancelled".into() }
    }
}
impl Cancellable for ActionsDecision {
    fn cancelled() -> Self {
        ActionsDecision::Reject { reason: "cancelled".into() }
    }
}

/// The user's answer to a read request, before the fetch: which URLs to open, and
/// whether URLs already in the session's source table may be opened without asking
/// from now on. The fetch itself happens in the gate after this comes back.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReadAnswer {
    Allow { urls: Vec<String>, #[serde(default)] auto_sources: bool },
    Deny { reason: String },
}
impl Cancellable for ReadAnswer {
    fn cancelled() -> Self {
        ReadAnswer::Deny { reason: "cancelled".into() }
    }
}

/// The generic suspension of a blocking gate: insert into the table → await → fall
/// back when the channel breaks.
async fn park<D: Cancellable>(pending: &Pending<D>, id: &str) -> D {
    let (tx, rx) = oneshot::channel();
    pending.lock().unwrap().insert(id.to_string(), tx);
    rx.await.unwrap_or_else(|_| D::cancelled())
}

/// Handing a decision back: take the pending entry out and send it to the await.
fn resolve<D>(pending: &Pending<D>, what: &str, request_id: &str, decision: D) -> Result<(), String> {
    let tx = pending
        .lock()
        .unwrap()
        .remove(request_id)
        .ok_or_else(|| format!("no pending {what} request: {request_id}"))?;
    tx.send(decision).map_err(|_| "agent no longer waiting".to_string())
}

fn drain<D: Cancellable>(pending: &Pending<D>) {
    for (_, tx) in pending.lock().unwrap().drain() {
        let _ = tx.send(D::cancelled());
    }
}

/// Every blocking gate of the agent, and its pending table. One struct rather than
/// parallel fields: a cancellation, a window close or a vault switch must drain
/// every table, and missing one leaves the loop hanging forever at that gate's
/// await — `deny_all` is the one exit, adding a gate changes only this file, and
/// the four call sites stay as they are.
#[derive(Default)]
pub struct PendingGates {
    pub writes: Pending<PermissionDecision>,
    pub plans: Pending<PlanDecision>,
    pub actions: Pending<ActionsDecision>,
    pub reads: Pending<ReadAnswer>,
}

impl PendingGates {
    pub fn deny_all(&self) {
        drain(&self.writes);
        drain(&self.plans);
        drain(&self.actions);
        drain(&self.reads);
    }
}

/// The session-level snapshot: the system prompt prefix is frozen and the
/// web_search/plan_confirm switches are fixed for the session. Rebuilding the
/// prompt between turns would invalidate the provider's prefix cache every time (on
/// DeepSeek a hit is roughly 30× cheaper); switching web_search or plan_confirm
/// mid-session changes the tool table, which blows the cache just as thoroughly. It
/// lives as long as the session, is cleared when a session is created or switched
/// and when the vault changes, and is rebuilt on the next turn's first send.
pub struct SessionSetup {
    pub system: String,
    pub web_search: bool,
    pub plan_confirm: bool,
}

pub struct AgentState {
    /// Conversation history for the current vault session.
    pub history: tokio::sync::Mutex<Vec<ChatMessage>>,
    /// Which vault `history` belongs to — a turn in a different vault starts
    /// fresh, so no conversation content crosses vault boundaries.
    pub history_vault: Mutex<Option<PathBuf>>,
    /// The current session id (= the file stem under `.vemry/sessions/`). None means
    /// "a new session awaiting its first send" — the id is allocated only on the
    /// first turn, so an empty session never reaches disk.
    /// It moves with `history`: wherever history is cleared, this must be cleared too.
    pub current_session: Mutex<Option<String>>,
    pub cancel: Arc<AtomicBool>,
    pub running: Arc<AtomicBool>,
    /// The pending tables of the blocking gates (write permission / plan
    /// confirmation), see `PendingGates`.
    pub pending: Arc<PendingGates>,
    /// The incremental parse cache behind the session list (the key is the full
    /// path, so switching vaults invalidates it naturally).
    pub session_meta: Mutex<vemry_agent::session::SessionMetaCache>,
    /// The session-level prompt/web_search/plan_confirm snapshot; it moves with current_session.
    pub session_setup: Mutex<Option<SessionSetup>>,
    /// The cross-turn carrier of "allow writes for this session" (the runner sets
    /// it, and it is cleared here with the session).
    /// It used to be a local of run(), where the button said "session" but the grant
    /// really only lived for one turn.
    pub session_grant: Arc<AtomicBool>,
    /// The session's web source table (vemry_agent::sources): the runner settles each
    /// turn's citations into it, `list_sources` reads it, and it is rebuilt from the
    /// session's jsonl wherever the session identity changes — it moves with
    /// `current_session` exactly as `history` does.
    pub sources: Arc<Mutex<Vec<vemry_agent::Source>>>,
    /// The note the session follows (vemry_agent::Follow): local writes on it pass
    /// the gate without a dialog. Rebuilt from the jsonl's `follow` lines with the
    /// session, exactly like `sources`.
    pub follow: Mutex<Option<vemry_agent::Follow>>,
    /// The write journal (vemry_agent::undo): what each landed write of this session
    /// replaced, so "undo this turn's writes" can put them back. In memory only —
    /// cleared with the session, gone after a restart (the provenance bar remains).
    pub journal: Arc<Mutex<Vec<vemry_agent::WriteRecord>>>,
    /// The session's standing consent for the read gate: URLs already in the
    /// session's source table (search results, cited pages) may be fetched without
    /// a card. Set from the card's checkbox; cleared with the session.
    pub read_auto_sources_arc: Arc<AtomicBool>,
}

impl AgentState {
    /// The single cleanup for a change of session identity (created, switched, vault
    /// changed, the current one deleted): neither the snapshot nor the write grant
    /// may outlive a session.
    pub fn reset_session_scope(&self) {
        self.invalidate_prompt_snapshot();
        self.session_grant.store(false, std::sync::atomic::Ordering::Relaxed);
        self.read_auto_sources_arc.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// Point the source table at a session: rebuilt from its jsonl (None = a new
    /// session, empty). Every place that sets `current_session` calls it.
    pub fn load_sources(&self, meta_dir: &std::path::Path, id: Option<&str>) {
        *self.sources.lock().unwrap() =
            id.map(|id| vemry_agent::session::read_session_sources(meta_dir, id)).unwrap_or_default();
        *self.follow.lock().unwrap() = id.and_then(|id| vemry_agent::session::read_session_follow(meta_dir, id));
        self.journal.lock().unwrap().clear();
    }

    /// A change to the privacy zones invalidates the prompt snapshot alone: the
    /// system prompt is rebuilt next turn, and newly marked private content (folder
    /// names and tags in the vault overview, the body of VEMRY.md) disappears from
    /// the prompt at once — the promise that "PrivacyChanged takes effect
    /// immediately" must not be broken by a session-level freeze. A write grant has
    /// nothing to do with privacy, so session_grant is kept. The rebuilt snapshot
    /// picks up whatever the web_search/plan_confirm settings are at that moment —
    /// privacy outranks the cache, consistent with web_search's established
    /// behaviour.
    pub fn invalidate_prompt_snapshot(&self) {
        *self.session_setup.lock().unwrap() = None;
    }
}

impl Default for AgentState {
    fn default() -> Self {
        AgentState {
            history: tokio::sync::Mutex::new(vec![]),
            history_vault: Mutex::new(None),
            current_session: Mutex::new(None),
            cancel: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(false)),
            pending: Arc::new(PendingGates::default()),
            session_meta: Mutex::new(Default::default()),
            session_setup: Mutex::new(None),
            session_grant: Arc::new(AtomicBool::new(false)),
            sources: Arc::new(Mutex::new(vec![])),
            follow: Mutex::new(None),
            journal: Arc::new(Mutex::new(vec![])),
            read_auto_sources_arc: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Permission gate that suspends the agent loop until the user answers.
/// The runner has already emitted `PermissionRequest` to the UI channel; we
/// just wait for `agent_permission_respond` to resolve our oneshot.
pub struct ChannelGate {
    pub pending: Pending<PermissionDecision>,
}

#[async_trait]
impl PermissionGate for ChannelGate {
    async fn request(&self, req: &WriteRequest) -> PermissionDecision {
        park(&self.pending, &req.id).await
    }
}

/// The channel implementation of the plan gate: the decision comes back through
/// `agent_plan_respond`.
pub struct ChannelPlanGate {
    pub pending: Pending<PlanDecision>,
}

#[async_trait]
impl PlanGate for ChannelPlanGate {
    async fn request(&self, req: &PlanRequest) -> PlanDecision {
        park(&self.pending, &req.id).await
    }
}

pub fn resolve_permission(
    pending: &Pending<PermissionDecision>,
    request_id: &str,
    decision: PermissionDecision,
) -> Result<(), String> {
    resolve(pending, "permission", request_id, decision)
}

pub fn resolve_plan(pending: &Pending<PlanDecision>, request_id: &str, decision: PlanDecision) -> Result<(), String> {
    resolve(pending, "plan", request_id, decision)
}

/// The channel implementation of the actions gate: the decision comes back through
/// `agent_actions_respond`.
pub struct ChannelActionsGate {
    pub pending: Pending<ActionsDecision>,
}

#[async_trait]
impl ActionsGate for ChannelActionsGate {
    async fn request(&self, req: &ActionsRequest) -> ActionsDecision {
        park(&self.pending, &req.id).await
    }
}

pub fn resolve_actions(pending: &Pending<ActionsDecision>, request_id: &str, decision: ActionsDecision) -> Result<(), String> {
    resolve(pending, "actions", request_id, decision)
}

pub fn resolve_read(pending: &Pending<ReadAnswer>, request_id: &str, answer: ReadAnswer) -> Result<(), String> {
    resolve(pending, "read", request_id, answer)
}

/// The channel implementation of the read gate: the user's answer comes back through
/// `agent_read_respond`; then the gate fetches the allowed pages through the reader
/// window's extraction (`page_read`, the same pipeline a clip uses) and hands them to
/// the runner. With the session's standing consent, URLs already in the source table
/// skip the card. Page bodies travel only to the model; the ledger records URLs.
pub struct ChannelReadGate {
    pub app: tauri::AppHandle,
    pub pending: Pending<ReadAnswer>,
    pub sources: Arc<Mutex<Vec<vemry_agent::Source>>>,
    pub auto_sources: Arc<AtomicBool>,
}

#[async_trait]
impl ReadGate for ChannelReadGate {
    async fn request(&self, req: &ReadRequest) -> ReadDecision {
        let auto = self.auto_sources.load(std::sync::atomic::Ordering::Relaxed);
        let known = |u: &str| self.sources.lock().unwrap().iter().any(|s| s.url == u);
        let allowed: Vec<String> = if auto && req.urls.iter().all(|u| known(u)) {
            req.urls.clone()
        } else {
            match park(&self.pending, &req.id).await {
                ReadAnswer::Allow { urls, auto_sources } => {
                    if auto_sources {
                        self.auto_sources.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    urls
                }
                ReadAnswer::Deny { reason } => return ReadDecision::Deny { reason },
            }
        };
        let mut pages = Vec::new();
        let mut failed = Vec::new();
        for url in allowed {
            let state = self.app.state::<crate::AppState>();
            match crate::browser::reader::page_read(self.app.clone(), state, url.clone()).await {
                Ok(Some(page)) if !page.markdown.trim().is_empty() => {
                    let markdown: String = page.markdown.chars().take(vemry_agent::PAGE_CONTEXT_MAX_CHARS).collect();
                    pages.push(FetchedPage { url: page.url, title: page.title, markdown });
                }
                _ => failed.push(url),
            }
        }
        ReadDecision::Pages { pages, failed }
    }
}
