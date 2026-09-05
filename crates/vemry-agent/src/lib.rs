//! vemry-agent — the AI layer.
//!
//! Speaks the OpenAI-compatible Chat Completions protocol (Zhipu GLM by
//! default). Depends on vemry-core for tool execution; has no UI/Tauri
//! dependency. The permission gate lives here, at the boundary that owns the
//! file-write capability — a frontend can render decisions but never bypass
//! them.

pub mod client;
pub mod compact;
pub mod context;
pub mod permission;
pub mod prompt;
pub mod provider;
pub mod responses;
pub mod runner;
pub mod session;
pub mod skills;
pub mod sources;
#[doc(hidden)]
pub mod testing;
pub mod tools;
pub mod types;

pub use client::{ChatTransport, HttpTransport};
pub use context::ContextRef;
pub use responses::ResponsesTransport;
pub use permission::{
    ActionItem, ActionsDecision, ActionsGate, ActionsRequest, PermissionDecision, PermissionGate, PlanDecision, PlanGate,
    PlanRequest, WriteRequest,
};
pub use prompt::{PageContext, TurnContext, PAGE_CONTEXT_MAX_CHARS};
pub use provider::ProviderConfig;
pub use runner::{AgentEvent, AgentRunner, RunnerConfig, WritePolicy};
pub use session::Follow;
pub use sources::Source;
pub use tools::{is_local_write, PlanItem, PlanStatus};

#[derive(thiserror::Error, Debug)]
pub enum AgentError {
    #[error("api error: {0}")]
    Api(String),
    /// A non-2xx HTTP response (client.rs post_streaming is the only producer).
    /// The status code is kept structured: retry classification (429/5xx) reads
    /// the code instead of prefix-matching the message text.
    #[error("api error: {message}")]
    Status { code: u16, message: String },
    #[error("http error: {0}")]
    Http(String),
    #[error("no api key configured")]
    NoApiKey,
    #[error("core error: {0}")]
    Core(#[from] vemry_core::VemryError),
    #[error("cancelled")]
    Cancelled,
    #[error("max iterations ({0}) reached")]
    MaxIterations(usize),
}

pub type Result<T> = std::result::Result<T, AgentError>;

/// Character-level truncation plus a footnote (shared by the four size limits:
/// tool results, read_note, read_skill and VEMRY.md). Returns None when nothing
/// was over the limit, so the caller keeps the original and copies nothing.
pub(crate) fn truncate_chars(s: &str, max: usize, note: &str) -> Option<String> {
    let cut = s.char_indices().nth(max)?.0;
    let mut out = String::with_capacity(cut + note.len() + 2);
    out.push_str(&s[..cut]);
    out.push_str("\n\n");
    out.push_str(note);
    Some(out)
}
