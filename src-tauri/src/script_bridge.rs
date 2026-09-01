//! Bridges the script host to the frontend: pushes ScriptUiEvents over the
//! `script://event` channel and blocks grant/prompt requests on the user's
//! answer via pending maps — the same oneshot pattern as agent_bridge, but
//! with std channels because the script thread is synchronous.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::json;
use tauri::{AppHandle, Emitter};
use vemry_script::{ScriptUiEvent, UiBridge};

/// The user gets this long to answer a write-grant dialog before the script
/// call fails as denied (scripts must never hang forever on a lost dialog).
const GRANT_TIMEOUT: Duration = Duration::from_secs(300);
/// `vemry.prompt` answer window.
const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Default)]
pub struct ScriptBridgeState {
    pending_grants: Mutex<HashMap<String, Sender<bool>>>,
    pending_prompts: Mutex<HashMap<String, Sender<Option<String>>>>,
    counter: AtomicU64,
}

impl ScriptBridgeState {
    fn next_id(&self, prefix: &str) -> String {
        format!("{prefix}-{}", self.counter.fetch_add(1, Ordering::Relaxed))
    }

    pub fn resolve_grant(&self, request_id: &str, allow: bool) -> Result<(), String> {
        let tx = self
            .pending_grants
            .lock()
            .unwrap()
            .remove(request_id)
            .ok_or_else(|| format!("no pending grant request: {request_id}"))?;
        tx.send(allow).map_err(|_| "script no longer waiting".to_string())
    }

    pub fn resolve_prompt(&self, request_id: &str, value: Option<String>) -> Result<(), String> {
        let tx = self
            .pending_prompts
            .lock()
            .unwrap()
            .remove(request_id)
            .ok_or_else(|| format!("no pending prompt request: {request_id}"))?;
        tx.send(value).map_err(|_| "script no longer waiting".to_string())
    }

    /// Vault switch / shutdown: wake every waiting script with a denial so
    /// nothing stays blocked on a dialog that no longer exists.
    pub fn deny_all(&self) {
        for (_, tx) in self.pending_grants.lock().unwrap().drain() {
            let _ = tx.send(false);
        }
        for (_, tx) in self.pending_prompts.lock().unwrap().drain() {
            let _ = tx.send(None);
        }
    }
}

/// The `UiBridge` the script host talks to. Lives on the script thread's side
/// of the channel; everything here must stay lock-light and re-entrant free.
pub struct TauriBridge {
    pub app: AppHandle,
    pub state: Arc<ScriptBridgeState>,
}

impl UiBridge for TauriBridge {
    fn emit(&self, event: ScriptUiEvent) {
        let _ = self.app.emit("script://event", &event);
    }

    fn vault_event(&self, event: vemry_core::VaultEvent) {
        // straight to the UI so views refresh; deliberately NOT through the
        // workflow dispatcher (script writes cannot trigger workflows)
        let _ = self.app.emit("vault://event", &event);
    }

    fn request_grant(&self, script: &str, path: &str) -> bool {
        let id = self.state.next_id("grant");
        let (tx, rx) = std::sync::mpsc::channel();
        self.state.pending_grants.lock().unwrap().insert(id.clone(), tx);
        let _ = self.app.emit(
            "script://event",
            json!({ "type": "grant_request", "id": id, "script": script, "path": path }),
        );
        let answer = rx.recv_timeout(GRANT_TIMEOUT);
        // drop the receiver BEFORE clearing the entry: a respond racing the
        // timeout then fails ("script no longer waiting") and the frontend
        // can tell the user, instead of Ok-ing an answer nobody consumed
        drop(rx);
        self.state.pending_grants.lock().unwrap().remove(&id);
        answer.unwrap_or(false)
    }

    fn request_prompt(&self, script: &str, message: &str, placeholder: Option<&str>) -> Option<String> {
        let id = self.state.next_id("prompt");
        let (tx, rx) = std::sync::mpsc::channel();
        self.state.pending_prompts.lock().unwrap().insert(id.clone(), tx);
        let _ = self.app.emit(
            "script://event",
            json!({ "type": "prompt_request", "id": id, "script": script, "message": message, "placeholder": placeholder }),
        );
        let answer = rx.recv_timeout(PROMPT_TIMEOUT);
        drop(rx); // same ordering rationale as request_grant
        self.state.pending_prompts.lock().unwrap().remove(&id);
        answer.unwrap_or(None)
    }
}
