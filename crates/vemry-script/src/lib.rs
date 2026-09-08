//! vemry-script — the sandboxed JS runtime for user scripts (v0.3 programmability).
//!
//! Layering contract: depends on vemry-core only — no UI, no AI, no Tauri.
//! Scripts are the third consumer of the unified `query()` API (after UI
//! views and agent tools). The sandbox exposes exactly the `vemry.*` API
//! whitelist: no filesystem, no network, no process access — every capability
//! goes through the Vault, so path validation, self-write suppression and
//! incremental indexing all apply for free.
//!
//! Threading model: one dedicated OS thread owns the QuickJS `Runtime` (JS
//! values never cross threads); `ScriptHost` is the cloneable handle the rest
//! of the app talks to over a channel.

pub mod audit;
pub mod bindings;
pub mod engine;
pub mod grants;
pub mod lab;
pub mod scheduler;

use serde::Serialize;
use vemry_core::VaultEvent;

pub use engine::{HostConfig, ScriptHost};

/// Whose eyes a script execution sees the vault through. User-triggered
/// scripts (commands, workflows) run with the user's perspective — the
/// privacy zone is visible, same as the UI. The `hide_private` position is
/// reserved for a future agent-triggered path (v0.3 never enables it), but
/// the enforcement is implemented and tested now.
#[derive(Clone, Copy, Debug, Default)]
pub struct Perspective {
    pub hide_private: bool,
}

/// A command a script registered into the palette.
#[derive(Clone, Debug, Serialize)]
pub struct CommandInfo {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub hotkey: Option<String>,
}

/// Everything the UI needs to render one script row (settings page) and to
/// register its commands into the palette.
#[derive(Clone, Debug, Serialize)]
pub struct ScriptInfo {
    pub name: String,
    pub enabled: bool,
    pub write_grant: bool,
    pub commands: Vec<CommandInfo>,
    pub event_triggers: Vec<String>,
    pub schedules: Vec<String>,
    pub last_error: Option<String>,
}

/// Outcome of a one-shot `js` code-block evaluation (M4). `output` is the
/// captured console lines plus a final `⇒ value` line; nothing is persisted.
/// `lab` is the experiment the block declared through `vemry.lab.*` (controls
/// and views, see [`lab`]), present only when it called that API and ran without
/// error; the editor renders it and re-runs the block with the controls' values
/// as `params` while the user moves them.
#[derive(Clone, Debug, Serialize)]
pub struct BlockEvalResult {
    pub output: String,
    pub error: Option<String>,
    pub lab: Option<lab::LabSpec>,
}

/// Events the script host pushes to the UI (`script://event` on the Tauri
/// side). Grant/prompt requests are the blocking half of the write-permission
/// flow and arrive via [`UiBridge`] callbacks instead.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ScriptUiEvent {
    /// The set of scripts (and their commands) changed — resync the palette.
    CommandsChanged { scripts: Vec<ScriptInfo> },
    Toast { message: String, level: String },
    Error { script: String, message: String },
}

/// The host's window to the UI, implemented by the Tauri glue layer (and by
/// test doubles). Blocking methods are called from the script thread with no
/// locks held and with the execution deadline paused.
pub trait UiBridge: Send + Sync + 'static {
    fn emit(&self, event: ScriptUiEvent);

    /// A vault mutation performed by a script — forward to the UI so views
    /// refresh. Deliberately NOT routed into the workflow dispatcher: script
    /// writes are structurally unable to trigger workflows (loop prevention).
    fn vault_event(&self, event: VaultEvent) {
        let _ = event;
    }

    /// Block until the user answers "allow this script to write?". Returning
    /// `true` grants (persisted by the caller). Default: deny.
    fn request_grant(&self, script: &str, path: &str) -> bool {
        let _ = (script, path);
        false
    }

    /// Block until the user answers a `vemry.prompt()` input dialog.
    /// `None` = cancelled. Default: cancelled.
    fn request_prompt(&self, script: &str, message: &str, placeholder: Option<&str>) -> Option<String> {
        let _ = (script, message, placeholder);
        None
    }
}
