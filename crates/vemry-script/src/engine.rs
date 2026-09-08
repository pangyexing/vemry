//! The script engine: one OS thread owns the QuickJS `Runtime` (64 MB memory
//! cap, wall-clock interrupt fuse) plus one `Context` per enabled script.
//! `ScriptHost` is the cloneable handle; jobs arrive over an mpsc channel and
//! run strictly serially — single-threaded execution is what makes concurrent
//! writes and re-entrancy structurally impossible.

use chrono::Utc;
use rquickjs::{CatchResultExt, Context, Function, Runtime};
use serde_json::json;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use vemry_core::vault::SCRIPTS_DIR;
use vemry_core::{Vault, VaultEvent};

use crate::bindings::{self, BindEnv, ExecKind, ScriptMeta};
use crate::grants::ScriptStates;
use crate::scheduler::{CronEntry, Due};
use crate::{audit, lab, BlockEvalResult, Perspective, ScriptInfo, ScriptUiEvent, UiBridge};

/// Workflow dispatch backpressure: more pending (merged) events than this and
/// new ones are dropped with an error toast — a stuck consumer must not grow
/// an unbounded queue.
const EVENT_QUEUE_CAP: usize = 100;

/// Wall-clock budgets per execution kind. Scripts that block on the user
/// (grant/prompt dialogs) have the clock paused while waiting.
#[derive(Clone, Copy, Debug)]
pub struct HostConfig {
    pub load_timeout: Duration,
    pub exec_timeout: Duration,
    pub block_timeout: Duration,
    pub memory_limit: usize,
    /// Same-(event, path) dispatches inside this window collapse into one
    /// workflow fire (the 1 s autosave would otherwise machine-gun
    /// note_modified).
    pub event_merge: Duration,
}

impl Default for HostConfig {
    fn default() -> Self {
        HostConfig {
            load_timeout: Duration::from_secs(2),
            exec_timeout: Duration::from_secs(5),
            block_timeout: Duration::from_secs(2),
            memory_limit: 64 * 1024 * 1024,
            event_merge: Duration::from_millis(500),
        }
    }
}

enum Job {
    Reload { reply: Option<Sender<()>> },
    SetEnabled { name: String, enabled: bool, reply: Sender<Result<(), String>> },
    RevokeWriteGrant { name: String, reply: Sender<Result<(), String>> },
    RunCommand { script: String, command: String, current_note: Option<String>, reply: Option<Sender<Result<(), String>>> },
    DispatchEvent { event: VaultEvent },
    EvalBlock { code: String, current_note: Option<String>, params: Option<serde_json::Value>, reply: Sender<BlockEvalResult> },
    #[cfg(test)]
    FireSchedulesNow { reply: Sender<()> },
    #[cfg(test)]
    Barrier { reply: Sender<()> },
}

/// Run queued QuickJS promise jobs (microtasks) under the caller's deadline.
/// The sync runtime never pumps these on its own — without this, `.then()`
/// continuations and the bodies of async callbacks past their first `await`
/// would silently never execute. The interrupt fuse still applies inside each
/// job, and the explicit deadline check stops self-replenishing job chains.
fn drain_jobs(ctx: &rquickjs::Ctx<'_>, deadline: &Mutex<Option<Instant>>) {
    loop {
        if deadline.lock().unwrap().is_some_and(|t| Instant::now() >= t) {
            return;
        }
        if !ctx.execute_pending_job() {
            return;
        }
    }
}

/// Merge key for workflow dispatch: event type + affected path.
fn event_key(event: &VaultEvent) -> Option<(&'static str, String)> {
    match event {
        VaultEvent::NoteAdded { id } => Some(("note_added", id.to_string())),
        VaultEvent::NoteModified { id } => Some(("note_modified", id.to_string())),
        VaultEvent::NoteRemoved { id } => Some(("note_removed", id.to_string())),
        VaultEvent::FilesChanged => Some(("files_changed", String::new())),
        _ => None,
    }
}

struct PendingEvent {
    key: (&'static str, String),
    event: VaultEvent,
    due: Instant,
}

/// Cloneable handle to the script thread. Reads (`list`) come from a shared
/// snapshot so the UI never blocks on a running script.
#[derive(Clone)]
pub struct ScriptHost {
    tx: Sender<Job>,
    snapshot: Arc<Mutex<Vec<ScriptInfo>>>,
}

impl ScriptHost {
    pub fn spawn(
        vault: Arc<Mutex<Option<Vault>>>,
        bridge: Arc<dyn UiBridge>,
        perspective: Perspective,
        config: HostConfig,
    ) -> ScriptHost {
        let (tx, rx) = mpsc::channel();
        let snapshot: Arc<Mutex<Vec<ScriptInfo>>> = Arc::new(Mutex::new(vec![]));
        let snap = Arc::clone(&snapshot);
        std::thread::Builder::new()
            .name("vemry-script".into())
            .spawn(move || worker_main(rx, vault, bridge, perspective, config, snap))
            .expect("spawn script thread");
        ScriptHost { tx, snapshot }
    }

    /// Fire-and-forget reload (vault switch, hot reload). The UI hears the
    /// outcome via `CommandsChanged`.
    pub fn reload(&self) {
        let _ = self.tx.send(Job::Reload { reply: None });
    }

    /// Reload and wait for completion — deterministic paths (tests, explicit
    /// "reload" button) where the caller reads state right after. `Err` when
    /// the host is stopped or still busy after 30s (the reload stays queued,
    /// but the caller must not present stale state as fresh).
    pub fn reload_blocking(&self) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Job::Reload { reply: Some(tx) })
            .map_err(|_| "script host stopped".to_string())?;
        rx.recv_timeout(Duration::from_secs(30)).map_err(|_| "script host busy".to_string())
    }

    /// Latest published script list — non-blocking snapshot.
    pub fn list(&self) -> Vec<ScriptInfo> {
        self.snapshot.lock().unwrap().clone()
    }

    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Job::SetEnabled { name: name.into(), enabled, reply: tx })
            .map_err(|_| "script host stopped".to_string())?;
        rx.recv_timeout(Duration::from_secs(30)).map_err(|_| "script host busy".to_string())?
    }

    pub fn revoke_write_grant(&self, name: &str) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Job::RevokeWriteGrant { name: name.into(), reply: tx })
            .map_err(|_| "script host stopped".to_string())?;
        rx.recv_timeout(Duration::from_secs(30)).map_err(|_| "script host busy".to_string())?
    }

    /// Queue a script command execution. Errors surface as `ScriptUiEvent::
    /// Error` (and the audit log); the palette caller doesn't wait.
    pub fn run_command(&self, script: &str, command: &str, current_note: Option<String>) -> Result<(), String> {
        self.tx
            .send(Job::RunCommand { script: script.into(), command: command.into(), current_note, reply: None })
            .map_err(|_| "script host stopped".to_string())
    }

    /// Synchronous variant for tests: returns the execution outcome.
    #[cfg(test)]
    pub fn run_command_blocking(&self, script: &str, command: &str, current_note: Option<String>) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Job::RunCommand { script: script.into(), command: command.into(), current_note, reply: Some(tx) })
            .map_err(|_| "script host stopped".to_string())?;
        rx.recv_timeout(Duration::from_secs(60)).map_err(|_| "script host busy".to_string())?
    }

    /// Feed a vault event into workflow dispatch. Only ever called from the
    /// app's `broadcast` fan-out (user IPC writes + watcher external
    /// changes) — script-initiated writes bypass this by construction.
    pub fn dispatch_event(&self, event: &VaultEvent) {
        if event_key(event).is_some() {
            let _ = self.tx.send(Job::DispatchEvent { event: event.clone() });
        }
    }

    /// Evaluate a note's `js` code block in a one-shot, read-only context.
    /// Only ever user-clicked — there is no auto-run path anywhere. `params` are
    /// the current values of the lab controls the block declared on an earlier
    /// run (the editor hands them back as the user moves a slider); `None` on
    /// the first run.
    pub fn eval_block(&self, code: &str, current_note: Option<String>, params: Option<serde_json::Value>) -> BlockEvalResult {
        let (tx, rx) = mpsc::channel();
        if self
            .tx
            .send(Job::EvalBlock { code: code.into(), current_note, params, reply: tx })
            .is_err()
        {
            return BlockEvalResult { output: String::new(), error: Some("script host stopped".into()), lab: None };
        }
        rx.recv_timeout(Duration::from_secs(30))
            .unwrap_or_else(|_| BlockEvalResult { output: String::new(), error: Some("script host busy".into()), lab: None })
    }

    /// Test hook: treat every registered cron entry as due right now.
    #[cfg(test)]
    pub fn fire_schedules_blocking(&self) {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(Job::FireSchedulesNow { reply: tx }).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(30));
        }
    }

    /// Test hook: wait until every previously queued job has been processed.
    #[cfg(test)]
    pub fn barrier(&self) {
        let (tx, rx) = mpsc::channel();
        if self.tx.send(Job::Barrier { reply: tx }).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(30));
        }
    }
}

struct Slot {
    name: String,
    enabled: bool,
    context: Option<Context>,
    meta: Rc<RefCell<ScriptMeta>>,
    last_error: Option<String>,
    crons: Vec<CronEntry>,
}

struct Worker {
    vault: Arc<Mutex<Option<Vault>>>,
    bridge: Arc<dyn UiBridge>,
    perspective: Perspective,
    config: HostConfig,
    runtime: Runtime,
    deadline: Arc<Mutex<Option<Instant>>>,
    grants: Rc<RefCell<ScriptStates>>,
    slots: Vec<Slot>,
    /// Root the current slots were loaded from. Audit lines for these scripts
    /// go here — after a vault switch (Reload still queued) they must not
    /// land in the newly opened vault's log.
    loaded_root: Option<PathBuf>,
    snapshot: Arc<Mutex<Vec<ScriptInfo>>>,
    pending: Vec<PendingEvent>,
    /// One toast per overflow stretch, not one per dropped event.
    overflow_toasted: bool,
}

fn worker_main(
    rx: Receiver<Job>,
    vault: Arc<Mutex<Option<Vault>>>,
    bridge: Arc<dyn UiBridge>,
    perspective: Perspective,
    config: HostConfig,
    snapshot: Arc<Mutex<Vec<ScriptInfo>>>,
) {
    let runtime = match Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            bridge.emit(ScriptUiEvent::Error { script: "<host>".into(), message: format!("QuickJS failed to initialise: {e}") });
            return;
        }
    };
    runtime.set_memory_limit(config.memory_limit);
    runtime.set_max_stack_size(512 * 1024);
    let deadline: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    {
        let deadline = Arc::clone(&deadline);
        runtime.set_interrupt_handler(Some(Box::new(move || {
            deadline.lock().unwrap().is_some_and(|t| Instant::now() >= t)
        })));
    }

    let mut worker = Worker {
        vault,
        bridge,
        perspective,
        config,
        runtime,
        deadline,
        grants: Rc::new(RefCell::new(ScriptStates::default())),
        slots: vec![],
        loaded_root: None,
        snapshot,
        pending: vec![],
        overflow_toasted: false,
    };

    // The loop is both executor and scheduler: sleep until the next merged
    // event flush or cron fire, waking early for any incoming job.
    loop {
        let job = match worker.next_wake() {
            Some(at) => match rx.recv_timeout(at.saturating_duration_since(Instant::now())) {
                Ok(job) => Some(job),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            None => match rx.recv() {
                Ok(job) => Some(job),
                Err(_) => break,
            },
        };
        if let Some(job) = job {
            worker.handle(job);
        }
        worker.tick();
    }
}

impl Worker {
    fn handle(&mut self, job: Job) {
        match job {
            Job::Reload { reply } => {
                self.reload();
                if let Some(tx) = reply {
                    let _ = tx.send(());
                }
            }
            Job::SetEnabled { name, enabled, reply } => {
                let _ = reply.send(self.set_enabled(&name, enabled));
            }
            Job::RevokeWriteGrant { name, reply } => {
                let _ = reply.send(self.revoke_write_grant(&name));
            }
            Job::RunCommand { script, command, current_note, reply } => {
                let result = self.run_command(&script, &command, current_note);
                if let Err(e) = &result {
                    self.bridge.emit(ScriptUiEvent::Error { script: script.clone(), message: e.clone() });
                    if let Some(root) = self.loaded_root.clone() {
                        audit::append(&root, "error", json!({ "script": script, "command": command, "message": e }));
                    }
                }
                if let Some(tx) = reply {
                    let _ = tx.send(result);
                }
            }
            Job::DispatchEvent { event } => self.enqueue_event(event),
            Job::EvalBlock { code, current_note, params, reply } => {
                let _ = reply.send(self.eval_block(&code, current_note, params));
            }
            #[cfg(test)]
            Job::FireSchedulesNow { reply } => {
                let fires: Vec<(usize, usize)> = self
                    .slots
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.enabled)
                    .flat_map(|(i, s)| (0..s.crons.len()).map(move |j| (i, j)))
                    .collect();
                for (i, j) in fires {
                    self.fire_schedule(i, j);
                }
                let _ = reply.send(());
            }
            #[cfg(test)]
            Job::Barrier { reply } => {
                let _ = reply.send(());
            }
        }
    }

    /// Earliest reason to wake: a merged event's flush time or a cron fire.
    fn next_wake(&self) -> Option<Instant> {
        let event_due = self.pending.iter().map(|p| p.due).min();
        let cron_due = self
            .slots
            .iter()
            .filter(|s| s.enabled)
            .flat_map(|s| s.crons.iter())
            .filter_map(|c| c.next_fire())
            .min()
            .map(|dt| {
                let wait = (dt - Utc::now()).to_std().unwrap_or(Duration::ZERO);
                Instant::now() + wait
            });
        match (event_due, cron_due) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Merge into the pending queue (same key = one fire), enforce the cap.
    fn enqueue_event(&mut self, event: VaultEvent) {
        let Some(key) = event_key(&event) else { return };
        // nobody subscribed → nothing to queue; a bulk external change (git
        // checkout touching hundreds of notes) must not overflow-toast users
        // who have no workflow scripts at all
        let subscribed = self
            .slots
            .iter()
            .any(|s| s.enabled && s.meta.borrow().event_triggers.iter().any(|t| t == key.0));
        if !subscribed {
            return;
        }
        if self.pending.iter().any(|p| p.key == key) {
            return;
        }
        if self.pending.len() >= EVENT_QUEUE_CAP {
            if !self.overflow_toasted {
                self.overflow_toasted = true;
                self.bridge.emit(ScriptUiEvent::Toast {
                    message: format!("the workflow event queue is full ({EVENT_QUEUE_CAP}); new events are being dropped"),
                    level: "error".into(),
                });
            }
            return;
        }
        self.pending.push(PendingEvent { key, event, due: Instant::now() + self.config.event_merge });
    }

    /// Flush due merged events and fire due cron entries.
    fn tick(&mut self) {
        let now = Instant::now();
        let mut due = Vec::new();
        self.pending.retain_mut(|p| {
            if p.due <= now {
                due.push(p.event.clone());
                false
            } else {
                true
            }
        });
        if self.pending.is_empty() {
            self.overflow_toasted = false;
        }
        for event in due {
            self.fire_event(&event);
        }

        let now_utc = Utc::now();
        let mut fires = Vec::new();
        let mut missed = Vec::new();
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if !slot.enabled {
                continue;
            }
            for (j, entry) in slot.crons.iter_mut().enumerate() {
                match entry.check(now_utc) {
                    Some(Due::Fire) => fires.push((i, j)),
                    Some(Due::Missed { scheduled }) => missed.push((slot.name.clone(), entry.expr.clone(), scheduled)),
                    None => {}
                }
            }
        }
        if let Some(root) = (!missed.is_empty()).then(|| self.loaded_root.clone()).flatten() {
            for (script, cron, scheduled) in missed {
                audit::append(&root, "schedule_missed", json!({ "script": script, "cron": cron, "scheduled": scheduled.to_rfc3339() }));
            }
        }
        for (i, j) in fires {
            self.fire_schedule(i, j);
        }
    }

    /// Run every subscribed script's handlers for one vault event.
    fn fire_event(&self, event: &VaultEvent) {
        let Some((etype, _)) = event_key(event) else { return };
        let payload = serde_json::to_string(event).unwrap_or_else(|_| "{}".into());
        for slot in &self.slots {
            if !slot.enabled || !slot.meta.borrow().event_triggers.iter().any(|t| t == etype) {
                continue;
            }
            let Some(context) = slot.context.as_ref() else { continue };
            {
                let mut meta = slot.meta.borrow_mut();
                meta.exec = ExecKind::Workflow;
                meta.trigger = format!("event:{etype}");
            }
            let result = self.fire_js(context, "__vemry_fire_event", (etype.to_string(), payload.clone()));
            self.reset_meta(slot);
            if let Err(e) = result {
                self.report_script_error(&slot.name, &format!("workflow ({etype}): {e}"));
            }
        }
    }

    fn fire_schedule(&self, slot_idx: usize, cron_idx: usize) {
        let slot = &self.slots[slot_idx];
        let Some(context) = slot.context.as_ref() else { return };
        {
            let mut meta = slot.meta.borrow_mut();
            meta.exec = ExecKind::Schedule;
            meta.trigger = "cron".into();
        }
        let result = self.fire_js(context, "__vemry_fire_schedule", (cron_idx as u32,));
        self.reset_meta(slot);
        if let Err(e) = result {
            let expr = &slot.crons[cron_idx].expr;
            self.report_script_error(&slot.name, &format!("scheduled task ({expr}): {e}"));
        }
    }

    /// Call one of the glue's fire functions under the exec deadline.
    fn fire_js<A>(&self, context: &Context, entry: &str, args: A) -> Result<(), String>
    where
        A: for<'js> rquickjs::function::IntoArgs<'js>,
    {
        context.with(|ctx| {
            let fire: Function = ctx.globals().get(entry).map_err(|e| format!("internal error: {e}"))?;
            let (result, timed_out) = self.with_deadline(self.config.exec_timeout, || {
                let r = fire.call::<_, ()>(args).catch(&ctx).map_err(|e| e.to_string());
                drain_jobs(&ctx, &self.deadline);
                r
            });
            result.map_err(|e| if timed_out { format!("execution timed out (>{:?}): {e}", self.config.exec_timeout) } else { e })
        })
    }

    /// One-shot evaluation for a note's `js` block: fresh context, read-only
    /// API (writes refuse, prompt resolves null, registration rejected),
    /// console captured into the result, context discarded after. The lab
    /// params go in as a JSON global the glue reads while building `vemry.lab`
    /// (their presence is what enables the lab API — a script context never
    /// sets it), and the spec the block built comes back out through
    /// `__vemry_lab_take` once the code has run.
    fn eval_block(&self, code: &str, current_note: Option<String>, params: Option<serde_json::Value>) -> BlockEvalResult {
        let meta = Rc::new(RefCell::new(ScriptMeta {
            exec: ExecKind::Block,
            trigger: "block".into(),
            current_note,
            ..Default::default()
        }));
        let context = match Context::full(&self.runtime) {
            Ok(c) => c,
            Err(e) => return BlockEvalResult { output: String::new(), error: Some(format!("context: {e}")), lab: None },
        };
        let params_json = params.unwrap_or_else(|| serde_json::json!({})).to_string();
        let env = BindEnv {
            vault: Arc::clone(&self.vault),
            root: self.current_root(),
            perspective: self.perspective,
            script: "<js-block>".into(),
            meta: Rc::clone(&meta),
            // empty grant table on purpose — blocks have no stable identity to
            // grant against, and call_write refuses Block contexts anyway
            grants: Rc::new(RefCell::new(ScriptStates::default())),
            bridge: Arc::clone(&self.bridge),
            deadline: Arc::clone(&self.deadline),
        };
        let result: Result<(Option<String>, Option<String>), String> = context.with(|ctx| {
            ctx.globals().set("__vemry_lab_params", params_json).map_err(|e| format!("bindings: {e}"))?;
            bindings::install(&ctx, env).map_err(|e| format!("bindings: {e}"))?;
            let (result, timed_out) = self.with_deadline(self.config.block_timeout, || {
                let value = ctx
                    .eval::<rquickjs::Value, _>(code.as_bytes().to_vec())
                    .catch(&ctx)
                    .map_err(|e| e.to_string());
                // flush queued promise jobs so async console output lands in
                // the block result instead of vanishing
                drain_jobs(&ctx, &self.deadline);
                let value = value?;
                if value.is_undefined() {
                    return Ok(None);
                }
                let repr: Function = ctx
                    .eval("(v) => { try { return typeof v === 'string' ? v : JSON.stringify(v); } catch { return String(v); } }")
                    .catch(&ctx)
                    .map_err(|e| e.to_string())?;
                repr.call::<_, Option<String>>((value,)).catch(&ctx).map_err(|e| e.to_string())
            });
            let repr = result.map_err(|e| {
                if timed_out {
                    format!("the code block timed out (>{:?}): {e}", self.config.block_timeout)
                } else {
                    e
                }
            })?;
            // The lab the block built, as JSON (null when it never called vemry.lab)
            let take: Function = ctx.globals().get("__vemry_lab_take").catch(&ctx).map_err(|e| e.to_string())?;
            let lab: Option<String> = take.call(()).catch(&ctx).map_err(|e| e.to_string())?;
            Ok((repr, lab))
        });
        let mut lines = meta.borrow().block_output.clone();
        match result {
            Ok((repr, lab_json)) => {
                if let Some(repr) = repr {
                    lines.push(format!("⇒ {repr}"));
                }
                match lab_json.as_deref().map(lab::parse) {
                    None => BlockEvalResult { output: lines.join("\n"), error: None, lab: None },
                    Some(Ok(spec)) => BlockEvalResult { output: lines.join("\n"), error: None, lab: Some(spec) },
                    Some(Err(e)) => BlockEvalResult { output: lines.join("\n"), error: Some(e), lab: None },
                }
            }
            Err(e) => BlockEvalResult { output: lines.join("\n"), error: Some(e), lab: None },
        }
    }

    fn reset_meta(&self, slot: &Slot) {
        let mut meta = slot.meta.borrow_mut();
        meta.exec = ExecKind::Load;
        meta.trigger = "load".into();
        meta.current_note = None;
    }

    fn report_script_error(&self, script: &str, message: &str) {
        self.bridge.emit(ScriptUiEvent::Error { script: script.into(), message: message.into() });
        if let Some(root) = self.loaded_root.clone() {
            audit::append(&root, "error", json!({ "script": script, "message": message }));
        }
    }

    fn current_root(&self) -> Option<PathBuf> {
        self.vault.lock().unwrap().as_ref().map(|v| v.root().to_path_buf())
    }

    /// Drop every context and re-evaluate `.vemry/scripts/*.js` from the
    /// current vault. Whole-world reload: simple semantics, no stale state.
    fn reload(&mut self) {
        self.slots.clear();
        self.pending.clear();
        self.loaded_root = self.current_root();
        let Some(root) = self.loaded_root.clone() else {
            self.publish();
            return;
        };
        *self.grants.borrow_mut() = ScriptStates::load(&root);

        let dir = root.join(SCRIPTS_DIR);
        let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|x| x == "js"))
                    .filter_map(|e| {
                        let stem = e.path().file_stem()?.to_string_lossy().to_string();
                        let source = std::fs::read_to_string(e.path()).ok()?;
                        Some((stem, source))
                    })
                    .collect()
            })
            .unwrap_or_default();
        files.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, source) in files {
            let enabled = self.grants.borrow().enabled(&name);
            let meta: Rc<RefCell<ScriptMeta>> = Rc::new(RefCell::new(ScriptMeta { trigger: "load".into(), ..Default::default() }));
            let mut slot = Slot { name: name.clone(), enabled, context: None, meta: Rc::clone(&meta), last_error: None, crons: vec![] };
            if enabled {
                match self.load_script(&name, &source, meta, &root) {
                    Ok(context) => {
                        slot.context = Some(context);
                        let now = Utc::now();
                        slot.crons = slot.meta.borrow().schedules.iter().filter_map(|e| CronEntry::new(e, now)).collect();
                    }
                    Err(message) => {
                        // one bad script must not take the others down
                        slot.last_error = Some(message.clone());
                        self.bridge.emit(ScriptUiEvent::Error { script: name.clone(), message: message.clone() });
                        audit::append(&root, "error", json!({ "script": name, "message": message }));
                    }
                }
            }
            self.slots.push(slot);
        }
        self.publish();
    }

    fn load_script(&self, name: &str, source: &str, meta: Rc<RefCell<ScriptMeta>>, root: &Path) -> Result<Context, String> {
        let context = Context::full(&self.runtime).map_err(|e| format!("context: {e}"))?;
        let env = BindEnv {
            vault: Arc::clone(&self.vault),
            root: Some(root.to_path_buf()),
            perspective: self.perspective,
            script: name.to_string(),
            meta,
            grants: Rc::clone(&self.grants),
            bridge: Arc::clone(&self.bridge),
            deadline: Arc::clone(&self.deadline),
        };
        let load = context.with(|ctx| {
            bindings::install(&ctx, env).map_err(|e| format!("bindings: {e}"))?;
            let (result, timed_out) = self.with_deadline(self.config.load_timeout, || {
                let r = ctx.eval::<(), _>(source.as_bytes().to_vec()).catch(&ctx).map_err(|e| e.to_string());
                drain_jobs(&ctx, &self.deadline);
                r
            });
            result.map_err(|e| if timed_out { format!("loading the script timed out (>{:?}): {e}", self.config.load_timeout) } else { e })
        });
        load.map(|()| context)
    }

    /// Arm the interrupt fuse for one execution; report whether it fired.
    fn with_deadline<T>(&self, budget: Duration, f: impl FnOnce() -> T) -> (T, bool) {
        *self.deadline.lock().unwrap() = Some(Instant::now() + budget);
        let out = f();
        let timed_out = self.deadline.lock().unwrap().take().is_some_and(|t| Instant::now() >= t);
        (out, timed_out)
    }

    fn set_enabled(&mut self, name: &str, enabled: bool) -> Result<(), String> {
        let root = self.current_root().ok_or("no vault open")?;
        let mut grants = ScriptStates::load(&root);
        grants.set_enabled(name, enabled);
        grants.save(&root)?;
        self.reload();
        Ok(())
    }

    fn revoke_write_grant(&mut self, name: &str) -> Result<(), String> {
        let root = self.current_root().ok_or("no vault open")?;
        let mut grants = ScriptStates::load(&root);
        grants.set_write_grant(name, false);
        grants.save(&root)?;
        *self.grants.borrow_mut() = grants;
        audit::append(&root, "grant_revoked", json!({ "script": name }));
        self.publish();
        Ok(())
    }

    fn run_command(&mut self, script: &str, command: &str, current_note: Option<String>) -> Result<(), String> {
        let slot = self
            .slots
            .iter()
            .find(|s| s.name == script)
            .ok_or_else(|| format!("no such script: {script}"))?;
        if !slot.enabled {
            return Err(format!("script is disabled: {script}"));
        }
        let context = slot.context.as_ref().ok_or_else(|| format!("script not loaded (it failed to load): {script}"))?;
        if !slot.meta.borrow().commands.iter().any(|c| c.id == command) {
            return Err(format!("no such command: {script}:{command}"));
        }
        {
            let mut meta = slot.meta.borrow_mut();
            meta.exec = ExecKind::Command;
            meta.trigger = "command".into();
            meta.current_note = current_note.clone();
        }
        let ctx_json = json!({ "currentNote": current_note }).to_string();
        let result = context.with(|ctx| {
            let fire: Function = ctx
                .globals()
                .get("__vemry_run_command")
                .map_err(|e| format!("internal error: {e}"))?;
            let (result, timed_out) = self.with_deadline(self.config.exec_timeout, || {
                let r = fire.call::<_, ()>((command.to_string(), ctx_json.clone())).catch(&ctx).map_err(|e| e.to_string());
                drain_jobs(&ctx, &self.deadline);
                r
            });
            result.map_err(|e| if timed_out { format!("the command timed out (>{:?}): {e}", self.config.exec_timeout) } else { e })
        });
        self.reset_meta(slot);
        result
    }

    /// Rebuild the public snapshot and tell the UI to resync.
    fn publish(&self) {
        let infos: Vec<ScriptInfo> = self
            .slots
            .iter()
            .map(|s| {
                let meta = s.meta.borrow();
                ScriptInfo {
                    name: s.name.clone(),
                    enabled: s.enabled,
                    write_grant: self.grants.borrow().write_grant(&s.name),
                    commands: meta.commands.clone(),
                    event_triggers: meta.event_triggers.clone(),
                    schedules: meta.schedules.clone(),
                    last_error: s.last_error.clone(),
                }
            })
            .collect();
        *self.snapshot.lock().unwrap() = infos.clone();
        self.bridge.emit(ScriptUiEvent::CommandsChanged { scripts: infos });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use vemry_core::VaultEvent;

    struct TestBridge {
        events: Mutex<Vec<ScriptUiEvent>>,
        vault_events: Mutex<Vec<VaultEvent>>,
        grant_answer: bool,
        grant_calls: Mutex<Vec<(String, String)>>,
        prompt_answer: Option<String>,
    }

    impl TestBridge {
        fn new(grant_answer: bool) -> Arc<TestBridge> {
            Arc::new(TestBridge {
                events: Mutex::new(vec![]),
                vault_events: Mutex::new(vec![]),
                grant_answer,
                grant_calls: Mutex::new(vec![]),
                prompt_answer: Some("回答".into()),
            })
        }
        fn errors(&self) -> Vec<String> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter_map(|e| match e {
                    ScriptUiEvent::Error { message, .. } => Some(message.clone()),
                    _ => None,
                })
                .collect()
        }
        fn toasts(&self) -> Vec<String> {
            self.events
                .lock()
                .unwrap()
                .iter()
                .filter_map(|e| match e {
                    ScriptUiEvent::Toast { message, .. } => Some(message.clone()),
                    _ => None,
                })
                .collect()
        }
    }

    impl UiBridge for TestBridge {
        fn emit(&self, event: ScriptUiEvent) {
            self.events.lock().unwrap().push(event);
        }
        fn vault_event(&self, event: VaultEvent) {
            self.vault_events.lock().unwrap().push(event);
        }
        fn request_grant(&self, script: &str, path: &str) -> bool {
            self.grant_calls.lock().unwrap().push((script.into(), path.into()));
            self.grant_answer
        }
        fn request_prompt(&self, _script: &str, _message: &str, _placeholder: Option<&str>) -> Option<String> {
            self.prompt_answer.clone()
        }
    }

    fn fast_config() -> HostConfig {
        HostConfig {
            load_timeout: Duration::from_millis(300),
            exec_timeout: Duration::from_millis(300),
            block_timeout: Duration::from_millis(300),
            memory_limit: 32 * 1024 * 1024,
            event_merge: Duration::from_millis(100),
        }
    }

    /// Vault with one public note (a task) and one private note.
    fn fixture(scripts: &[(&str, &str)]) -> (tempfile::TempDir, Arc<Mutex<Option<Vault>>>) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("Todo.md"), "# Todo\n\n- [ ] 读书 📅 2026-08-20\n").unwrap();
        fs::write(root.join("Secret.md"), "---\nprivate: true\n---\n机密内容 SENTINEL_XYZ\n").unwrap();
        fs::create_dir_all(root.join(SCRIPTS_DIR)).unwrap();
        for (name, source) in scripts {
            fs::write(root.join(SCRIPTS_DIR).join(format!("{name}.js")), source).unwrap();
        }
        let vault = Vault::open(root).unwrap();
        (dir, Arc::new(Mutex::new(Some(vault))))
    }

    fn spawn_host(vault: &Arc<Mutex<Option<Vault>>>, bridge: &Arc<TestBridge>, hide_private: bool) -> ScriptHost {
        let host = ScriptHost::spawn(
            Arc::clone(vault),
            Arc::clone(bridge) as Arc<dyn UiBridge>,
            Perspective { hide_private },
            fast_config(),
        );
        host.reload_blocking().unwrap();
        host
    }

    fn info<'a>(infos: &'a [ScriptInfo], name: &str) -> &'a ScriptInfo {
        infos.iter().find(|i| i.name == name).unwrap()
    }

    #[test]
    fn sandbox_has_no_io_surface() {
        let (_d, vault) = fixture(&[(
            "probe",
            r#"
            const leaks = ["require", "process", "fetch", "XMLHttpRequest", "WebSocket", "os", "std"]
              .filter((n) => typeof globalThis[n] !== "undefined");
            if (leaks.length) throw new Error("leaked: " + leaks.join(","));
            vemry.commands.register({ id: "ok", title: "clean" }, () => {});
            "#,
        )]);
        let bridge = TestBridge::new(false);
        let host = spawn_host(&vault, &bridge, false);
        let infos = host.list();
        assert_eq!(info(&infos, "probe").last_error, None);
        assert_eq!(info(&infos, "probe").commands[0].title, "clean");
    }

    #[test]
    fn infinite_loop_at_load_is_fused() {
        let (_d, vault) = fixture(&[("spin", "while (true) {}")]);
        let bridge = TestBridge::new(false);
        let host = spawn_host(&vault, &bridge, false);
        let infos = host.list();
        let err = info(&infos, "spin").last_error.clone().unwrap();
        assert!(err.contains("timed out"), "expected timeout marker, got: {err}");
        // host is still alive and serves further work
        host.reload_blocking().unwrap();
        assert_eq!(host.list().len(), 1);
    }

    #[test]
    fn memory_bomb_errors_without_killing_host() {
        let (_d, vault) = fixture(&[
            ("bomb", "let s = 'x'; while (true) { s += s; }"),
            ("good", "vemry.commands.register({ id: 'c', title: 'still here' }, () => {});"),
        ]);
        let bridge = TestBridge::new(false);
        let host = spawn_host(&vault, &bridge, false);
        let infos = host.list();
        assert!(info(&infos, "bomb").last_error.is_some());
        assert_eq!(info(&infos, "good").last_error, None);
        assert_eq!(info(&infos, "good").commands[0].title, "still here");
    }

    #[test]
    fn bindings_read_query_list_search() {
        let (_d, vault) = fixture(&[(
            "probe",
            r#"
            const tasks = vemry.query({ kind: "task" });
            const notes = vemry.list();
            const hits = vemry.search("读书");
            const body = vemry.read("Todo.md");
            const alt = vemry.read("Todo"); // extensionless = note
            if (body !== alt) throw new Error("read mismatch");
            let escaped = "no";
            try { vemry.read("../outside"); } catch (e) { escaped = "caught"; }
            vemry.commands.register(
              { id: "r", title: `t${tasks.length} n${notes.length} h${hits.length} e:${escaped}` },
              () => {}
            );
            "#,
        )]);
        let bridge = TestBridge::new(false);
        let host = spawn_host(&vault, &bridge, false);
        let infos = host.list();
        assert_eq!(info(&infos, "probe").last_error, None);
        // user perspective: private note visible → 2 notes
        assert_eq!(info(&infos, "probe").commands[0].title, "t1 n2 h1 e:caught");
    }

    #[test]
    fn hide_private_redacts_private_blocks() {
        let probe = r#"
            const text = vemry.read("Mixed.md");
            const hits = vemry.search("BLOCKSECRET");
            const q = vemry.query({ text: "BLOCKSECRET" });
            vemry.commands.register(
              { id: "p", title: `t:${text.includes("BLOCKSECRET") ? "leak" : "clean"} h${hits.length} q${q.length}` },
              () => {}
            );
            "#;
        let (_d, vault) = fixture(&[("probe", probe)]);
        vault
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .write_note(&vemry_core::NoteId::new("Mixed.md"), "public\n\n> [!private] BLOCKSECRET\n")
            .unwrap();
        let bridge = TestBridge::new(false);
        let hidden = spawn_host(&vault, &bridge, true);
        assert_eq!(info(&hidden.list(), "probe").commands[0].title, "t:clean h0 q0");
        drop(hidden);
        // the user's own perspective still sees everything
        let open = spawn_host(&vault, &bridge, false);
        assert_eq!(info(&open.list(), "probe").commands[0].title, "t:leak h1 q1");
    }

    #[test]
    fn hide_private_perspective_filters_everything() {
        let (_d, vault) = fixture(&[(
            "probe",
            r#"
            const notes = vemry.list();
            const hits = vemry.search("SENTINEL_XYZ");
            const paras = vemry.query({ text: "SENTINEL_XYZ" });
            let read = "no";
            try { vemry.read("Secret.md"); } catch (e) { read = e.message.includes("not found") ? "hidden" : "other"; }
            let wrote = "no";
            try { vemry.write("Secret.md", "x"); } catch (e) { wrote = e.message.includes("not found") ? "hidden" : e.message; }
            vemry.commands.register(
              { id: "p", title: `n${notes.length} h${hits.length} q${paras.length} r:${read} w:${wrote}` },
              () => {}
            );
            "#,
        )]);
        let bridge = TestBridge::new(true); // grants write so the write reaches the privacy check
        let host = spawn_host(&vault, &bridge, true);
        let infos = host.list();
        assert_eq!(info(&infos, "probe").last_error, None);
        assert_eq!(info(&infos, "probe").commands[0].title, "n1 h0 q0 r:hidden w:hidden");
    }

    #[test]
    fn write_grant_flow_and_audit() {
        let (_d, vault) = fixture(&[(
            "writer",
            r##"
            vemry.write("out/Log.md", "# 日志\n");
            vemry.append("out/Log.md", "- 第二行");
            "##,
        )]);
        let bridge = TestBridge::new(true);
        let host = spawn_host(&vault, &bridge, false);
        let infos = host.list();
        assert_eq!(info(&infos, "writer").last_error, None);
        assert!(info(&infos, "writer").write_grant, "grant persisted and reflected");
        // asked exactly once, then the grant covers the second write
        assert_eq!(bridge.grant_calls.lock().unwrap().len(), 1);

        let root = { vault.lock().unwrap().as_ref().unwrap().root().to_path_buf() };
        let content = fs::read_to_string(root.join("out/Log.md")).unwrap();
        assert_eq!(content, "# 日志\n- 第二行\n");
        // the write went through write_note → indexed
        assert!(vault.lock().unwrap().as_ref().unwrap().index().contains(&vemry_core::NoteId::new("out/Log.md")));
        // vault events went straight to the UI bridge
        assert!(!bridge.vault_events.lock().unwrap().is_empty());

        let log = audit::read(&root);
        for kind in ["grant_asked", "grant_given", "write"] {
            assert!(log.contains(&format!("\"kind\":\"{kind}\"")), "missing {kind} in audit:\n{log}");
        }
        assert!(log.contains("\"trigger\":\"load\""));

        // reload: grant already persisted — no new dialog
        host.reload_blocking().unwrap();
        assert_eq!(bridge.grant_calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn write_denied_and_hidden_dirs_blocked() {
        let (_d, vault) = fixture(&[(
            "denied",
            r#"
            let msg = "no";
            try { vemry.write("A.md", "x"); } catch (e) { msg = "denied"; }
            let hidden = "no";
            try { vemry.write(".vemry/scripts.json", "{}"); } catch (e) { hidden = "blocked"; }
            vemry.commands.register({ id: "d", title: `${msg}/${hidden}` }, () => {});
            "#,
        )]);
        let bridge = TestBridge::new(false); // user denies
        let host = spawn_host(&vault, &bridge, false);
        let infos = host.list();
        assert_eq!(info(&infos, "denied").commands[0].title, "denied/blocked");
        assert!(!info(&infos, "denied").write_grant);
        let root = { vault.lock().unwrap().as_ref().unwrap().root().to_path_buf() };
        assert!(!root.join("A.md").exists());
        let log = audit::read(&root);
        assert!(log.contains("\"kind\":\"grant_denied\""));
        // the hidden-dir block never even asks for a grant (denied earlier, so
        // exactly one ask from the first write)
        assert_eq!(bridge.grant_calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn registrations_collected_and_cron_validated() {
        let (_d, vault) = fixture(&[
            (
                "auto",
                r#"
                vemry.events.on("note_added", () => {});
                vemry.schedule("*/5 * * * *", () => {});
                vemry.commands.register({ id: "a", title: "A", description: "说明", hotkey: "mod+9" }, () => {});
                "#,
            ),
            ("badcron", r#"vemry.schedule("not a cron", () => {});"#),
            ("badevent", r#"vemry.events.on("privacy_changed", () => {});"#),
        ]);
        let bridge = TestBridge::new(false);
        let host = spawn_host(&vault, &bridge, false);
        let infos = host.list();
        let auto = info(&infos, "auto");
        assert_eq!(auto.event_triggers, vec!["note_added"]);
        assert_eq!(auto.schedules, vec!["*/5 * * * *"]);
        assert_eq!(auto.commands[0].description.as_deref(), Some("说明"));
        assert!(info(&infos, "badcron").last_error.as_deref().unwrap().contains("cron"));
        assert!(info(&infos, "badevent").last_error.as_deref().unwrap().contains("unknown event type"));
    }

    #[test]
    fn enable_disable_persists_and_reloads() {
        let (_d, vault) = fixture(&[("s", "vemry.commands.register({ id: 'c', title: 'T' }, () => {});")]);
        let bridge = TestBridge::new(false);
        let host = spawn_host(&vault, &bridge, false);
        assert_eq!(info(&host.list(), "s").commands.len(), 1);

        host.set_enabled("s", false).unwrap();
        let infos = host.list();
        assert!(!info(&infos, "s").enabled);
        assert!(info(&infos, "s").commands.is_empty(), "disabled script registers nothing");
        let root = { vault.lock().unwrap().as_ref().unwrap().root().to_path_buf() };
        assert!(fs::read_to_string(root.join(".vemry/scripts.json")).unwrap().contains("\"enabled\": false"));

        host.set_enabled("s", true).unwrap();
        assert_eq!(info(&host.list(), "s").commands.len(), 1);
    }

    #[test]
    fn run_command_executes_with_context() {
        let (_d, vault) = fixture(&[(
            "cmd",
            r#"
            vemry.commands.register({ id: "cap", title: "记录当前笔记" }, (ctx) => {
              vemry.write("Captured.md", "from: " + (ctx.currentNote ?? vemry.currentNote() ?? "none"));
              vemry.toast("已记录");
            });
            "#,
        )]);
        let bridge = TestBridge::new(true);
        let host = spawn_host(&vault, &bridge, false);
        host.run_command_blocking("cmd", "cap", Some("Todo.md".into())).unwrap();
        let root = { vault.lock().unwrap().as_ref().unwrap().root().to_path_buf() };
        assert_eq!(fs::read_to_string(root.join("Captured.md")).unwrap(), "from: Todo.md");
        assert_eq!(bridge.toasts(), vec!["已记录"]);
        let log = audit::read(&root);
        assert!(log.contains("\"trigger\":\"command\""));
        // unknown command / script errors are proper Errs
        assert!(host.run_command_blocking("cmd", "nope", None).is_err());
        assert!(host.run_command_blocking("ghost", "cap", None).is_err());
    }

    #[test]
    fn command_timeout_fuses_and_host_survives() {
        let (_d, vault) = fixture(&[(
            "spin",
            r#"vemry.commands.register({ id: "loop", title: "死循环" }, () => { while (true) {} });"#,
        )]);
        let bridge = TestBridge::new(false);
        let host = spawn_host(&vault, &bridge, false);
        let err = host.run_command_blocking("spin", "loop", None).unwrap_err();
        assert!(err.contains("timed out"), "got: {err}");
        assert!(bridge.errors().iter().any(|e| e.contains("timed out")));
        // context still usable afterwards
        host.reload_blocking().unwrap();
        assert_eq!(host.list().len(), 1);
    }

    #[test]
    fn prompt_null_outside_command_and_answered_inside() {
        let (_d, vault) = fixture(&[(
            "p",
            r#"
            const atLoad = vemry.prompt("载入时问");
            vemry.commands.register({ id: "ask", title: "load:" + String(atLoad) }, () => {
              const v = vemry.prompt("跑命令时问");
              vemry.write("PromptOut.md", "got: " + String(v));
            });
            "#,
        )]);
        let bridge = TestBridge::new(true);
        let host = spawn_host(&vault, &bridge, false);
        assert_eq!(info(&host.list(), "p").commands[0].title, "load:null");
        host.run_command_blocking("p", "ask", None).unwrap();
        let root = { vault.lock().unwrap().as_ref().unwrap().root().to_path_buf() };
        assert_eq!(fs::read_to_string(root.join("PromptOut.md")).unwrap(), "got: 回答");
    }

    #[test]
    fn registration_rejected_at_runtime() {
        let (_d, vault) = fixture(&[(
            "late",
            r#"
            vemry.commands.register({ id: "go", title: "Go" }, () => {
              let msg = "no";
              try { vemry.commands.register({ id: "x", title: "X" }, () => {}); } catch (e) { msg = "rejected"; }
              vemry.toast(msg);
            });
            "#,
        )]);
        let bridge = TestBridge::new(false);
        let host = spawn_host(&vault, &bridge, false);
        host.run_command_blocking("late", "go", None).unwrap();
        assert_eq!(bridge.toasts(), vec!["rejected"]);
    }

    #[test]
    fn revoke_write_grant_asks_again() {
        let (_d, vault) = fixture(&[(
            "w",
            r#"vemry.commands.register({ id: "save", title: "S" }, () => { vemry.write("R.md", "x"); });"#,
        )]);
        let bridge = TestBridge::new(true);
        let host = spawn_host(&vault, &bridge, false);
        host.run_command_blocking("w", "save", None).unwrap();
        assert_eq!(bridge.grant_calls.lock().unwrap().len(), 1);
        host.run_command_blocking("w", "save", None).unwrap();
        assert_eq!(bridge.grant_calls.lock().unwrap().len(), 1, "granted: no second ask");

        host.revoke_write_grant("w").unwrap();
        assert!(!info(&host.list(), "w").write_grant);
        host.run_command_blocking("w", "save", None).unwrap();
        assert_eq!(bridge.grant_calls.lock().unwrap().len(), 2, "revoked: asks again");
        let root = { vault.lock().unwrap().as_ref().unwrap().root().to_path_buf() };
        assert!(audit::read(&root).contains("\"kind\":\"grant_revoked\""));
    }

    #[test]
    fn workflow_fires_once_and_cannot_cascade() {
        // The core loop-prevention test: a workflow subscribed to
        // note_modified writes THE SAME note in its handler. The write goes
        // through the bridge (never back into dispatch), so it must run
        // exactly once.
        let (_d, vault) = fixture(&[(
            "wf",
            r#"
            vemry.events.on("note_modified", (e) => {
              vemry.write(e.id, "已被工作流改写\n");
            });
            "#,
        )]);
        let bridge = TestBridge::new(true);
        let host = spawn_host(&vault, &bridge, false);
        host.dispatch_event(&VaultEvent::NoteModified { id: vemry_core::NoteId::new("Todo.md") });
        std::thread::sleep(Duration::from_millis(400)); // merge window (100ms) + fire
        host.barrier();
        let root = { vault.lock().unwrap().as_ref().unwrap().root().to_path_buf() };
        assert_eq!(fs::read_to_string(root.join("Todo.md")).unwrap(), "已被工作流改写\n");
        let write_lines = audit::read(&root).matches("\"kind\":\"write\"").count();
        assert_eq!(write_lines, 1, "workflow write must not re-trigger the workflow");
        // …and it stays quiet
        std::thread::sleep(Duration::from_millis(300));
        host.barrier();
        assert_eq!(audit::read(&root).matches("\"kind\":\"write\"").count(), 1);
        assert!(audit::read(&root).contains("\"trigger\":\"event:note_modified\""));
    }

    #[test]
    fn same_key_events_merge_distinct_do_not() {
        let (_d, vault) = fixture(&[(
            "log",
            r#"vemry.events.on("note_modified", (e) => console.log("fired:" + e.id));"#,
        )]);
        let bridge = TestBridge::new(false);
        let host = spawn_host(&vault, &bridge, false);
        let a = VaultEvent::NoteModified { id: vemry_core::NoteId::new("A.md") };
        let b = VaultEvent::NoteModified { id: vemry_core::NoteId::new("B.md") };
        host.dispatch_event(&a);
        host.dispatch_event(&a); // merged
        host.dispatch_event(&a); // merged
        host.dispatch_event(&b); // distinct key
        std::thread::sleep(Duration::from_millis(400));
        host.barrier();
        let root = { vault.lock().unwrap().as_ref().unwrap().root().to_path_buf() };
        let log = audit::read(&root);
        assert_eq!(log.matches("fired:A.md").count(), 1, "same (event, path) merges into one fire");
        assert_eq!(log.matches("fired:B.md").count(), 1);
    }

    #[test]
    fn event_queue_cap_drops_with_toast() {
        let (_d, vault) = fixture(&[(
            "log",
            r#"vemry.events.on("note_modified", (e) => console.log("fired:" + e.id));"#,
        )]);
        let bridge = TestBridge::new(false);
        let host = spawn_host(&vault, &bridge, false);
        for i in 0..150 {
            host.dispatch_event(&VaultEvent::NoteModified { id: vemry_core::NoteId::new(&format!("n{i}.md")) });
        }
        std::thread::sleep(Duration::from_millis(500));
        host.barrier();
        let root = { vault.lock().unwrap().as_ref().unwrap().root().to_path_buf() };
        let fired = audit::read(&root).matches("fired:").count();
        assert_eq!(fired, 100, "beyond the cap events are dropped");
        assert!(bridge.toasts().iter().any(|t| t.contains("dropped")));
    }

    #[test]
    fn schedule_fires_with_cron_trigger() {
        let (_d, vault) = fixture(&[(
            "daily",
            r#"vemry.schedule("0 9 * * *", () => { vemry.write("Daily.md", "早安\n"); });"#,
        )]);
        let bridge = TestBridge::new(true);
        let host = spawn_host(&vault, &bridge, false);
        host.fire_schedules_blocking();
        let root = { vault.lock().unwrap().as_ref().unwrap().root().to_path_buf() };
        assert_eq!(fs::read_to_string(root.join("Daily.md")).unwrap(), "早安\n");
        assert!(audit::read(&root).contains("\"trigger\":\"cron\""));
    }

    #[test]
    fn workflow_prompt_resolves_null() {
        let (_d, vault) = fixture(&[(
            "wf",
            r#"
            vemry.events.on("note_added", () => {
              const v = vemry.prompt("无人值守");
              console.log("prompt:" + String(v));
            });
            "#,
        )]);
        let bridge = TestBridge::new(false); // prompt_answer would be Some, but workflow context short-circuits
        let host = spawn_host(&vault, &bridge, false);
        host.dispatch_event(&VaultEvent::NoteAdded { id: vemry_core::NoteId::new("X.md") });
        std::thread::sleep(Duration::from_millis(400));
        host.barrier();
        let root = { vault.lock().unwrap().as_ref().unwrap().root().to_path_buf() };
        assert!(audit::read(&root).contains("prompt:null"));
    }

    #[test]
    fn eval_block_reads_and_reports() {
        let (_d, vault) = fixture(&[]);
        let bridge = TestBridge::new(true);
        let host = spawn_host(&vault, &bridge, false);
        let r = host.eval_block(
            r#"const t = vemry.query({ kind: "task" }); console.log("tasks=" + t.length); t.length"#,
            Some("Todo.md".into()),
            None,
        );
        assert_eq!(r.error, None);
        assert_eq!(r.output, "tasks=1\n⇒ 1");
        // currentNote flows into the one-shot context
        let r = host.eval_block("vemry.currentNote()", Some("Todo.md".into()), None);
        assert_eq!(r.output, "⇒ Todo.md");
        // undefined result → console only, no ⇒ line
        let r = host.eval_block("console.log('只有输出')", None, None);
        assert_eq!(r.output, "只有输出");
    }

    #[test]
    fn eval_block_is_read_only_and_fused() {
        let (_d, vault) = fixture(&[]);
        let bridge = TestBridge::new(true); // even a granting user can't make blocks write
        let host = spawn_host(&vault, &bridge, false);
        let r = host.eval_block(r#"vemry.write("X.md", "x")"#, None, None);
        assert!(r.error.as_deref().unwrap().contains("read-only sandbox"), "got: {:?}", r.error);
        let root = { vault.lock().unwrap().as_ref().unwrap().root().to_path_buf() };
        assert!(!root.join("X.md").exists());
        assert!(bridge.grant_calls.lock().unwrap().is_empty(), "blocks never even ask");
        // append refused the same way; prompt resolves null
        assert!(host.eval_block(r#"vemry.append("Todo.md", "x")"#, None, None).error.is_some());
        assert_eq!(host.eval_block(r#"String(vemry.prompt("q"))"#, None, None).output, "⇒ null");
        // timeout fuses without killing the host
        let r = host.eval_block("while (true) {}", None, None);
        assert!(r.error.as_deref().unwrap().contains("timed out"));
        assert_eq!(host.eval_block("1 + 1", None, None).output, "⇒ 2");
    }

    #[test]
    fn async_callbacks_run_to_completion() {
        // the sync runtime needs an explicit job pump: without it the body of
        // an async callback past its first await would silently never run
        let (_d, vault) = fixture(&[(
            "a",
            r#"
            vemry.commands.register({ id: "go", title: "G" }, async () => {
              await 0;
              vemry.write("Async.md", "done");
            });
            "#,
        )]);
        let bridge = TestBridge::new(true);
        let host = spawn_host(&vault, &bridge, false);
        host.run_command_blocking("a", "go", None).unwrap();
        let root = { vault.lock().unwrap().as_ref().unwrap().root().to_path_buf() };
        assert_eq!(fs::read_to_string(root.join("Async.md")).unwrap(), "done");
        // js blocks flush queued microtasks into their captured output
        let r = host.eval_block("Promise.resolve(41).then(v => console.log('got:' + (v + 1)))", None, None);
        assert_eq!(r.error, None);
        assert!(r.output.contains("got:42"), "got: {}", r.output);
    }

    #[test]
    fn lab_blocks_declare_controls_and_take_params_back() {
        let (_d, vault) = fixture(&[]);
        let bridge = TestBridge::new(true);
        let host = spawn_host(&vault, &bridge, false);
        let code = r#"
            const p = vemry.lab.controls({ d_k: { slider: [4, 64], step: 4, value: 16 }, scale: { toggle: true }, head: { select: ["h1", "h2"] } });
            const Q = vemry.lab.matrix("Q", [[1, 0], [0, 1]]);
            vemry.lab.heatmap("attention", [[p.d_k, 0], [0, Q[1][1]]], { rows: ["a", "b"], cols: ["a", "b"], digits: 2 });
            vemry.lab.line("pe", [0, 1, 0]);
            vemry.lab.bars("bleu", { base: 27.3, big: 28.4 });
            vemry.lab.table("t", [{ n: 1, s: "x" }, { n: 2 }]);
            vemry.lab.step("softmax", ["scores", { label: "exp", value: [1, 2] }]);
            console.log("scale=" + p.scale + " head=" + p.head);
        "#;
        // first run: the defaults
        let r = host.eval_block(code, None, None);
        assert_eq!(r.error, None, "{:?}", r.output);
        assert_eq!(r.output, "scale=true head=h1");
        let spec = r.lab.expect("a lab spec");
        assert_eq!(spec.controls.len(), 4);
        assert_eq!(spec.views.len(), 5);
        match &spec.views[0] {
            lab::LabView::Heatmap { data, rows, .. } => {
                assert_eq!(data[0][0], Some(16.0));
                assert_eq!(rows.as_deref(), Some(&["a".to_string(), "b".to_string()][..]));
            }
            other => panic!("{other:?}"),
        }
        match &spec.views[3] {
            lab::LabView::Table { columns, rows, .. } => {
                assert_eq!(columns, &["n", "s"]);
                assert_eq!(rows[1], vec![lab::Cell::Number(2.0), lab::Cell::Empty(None)]);
            }
            other => panic!("{other:?}"),
        }
        // a re-run with the moved controls: the slider clamps to its range, a select
        // outside its options falls back, a matrix of the wrong shape is ignored
        let params = serde_json::json!({ "d_k": 1000, "scale": false, "head": "zzz", "Q": [[5]] });
        let r = host.eval_block(code, None, Some(params));
        assert_eq!(r.error, None);
        assert_eq!(r.output, "scale=false head=h1");
        let spec = r.lab.unwrap();
        match &spec.controls[0] {
            lab::LabControl::Slider { value, .. } => assert_eq!(*value, 64.0),
            other => panic!("{other:?}"),
        }
        match &spec.views[0] {
            lab::LabView::Heatmap { data, .. } => assert_eq!((data[0][0], data[1][1]), (Some(64.0), Some(1.0))),
            other => panic!("{other:?}"),
        }
        let params = serde_json::json!({ "Q": [[2, 0], [0, 3]] });
        let r = host.eval_block(code, None, Some(params));
        match &r.lab.unwrap().views[0] {
            lab::LabView::Heatmap { data, .. } => assert_eq!(data[1][1], Some(3.0)),
            other => panic!("{other:?}"),
        }
        // a block that never touches the lab has none; an over-cap view is an error, not a cut
        assert!(host.eval_block("1 + 1", None, None).lab.is_none());
        let r = host.eval_block("vemry.lab.heatmap('h', Array.from({length: 70}, () => [1]))", None, None);
        assert!(r.error.as_deref().unwrap().contains("the limit is 64"), "{:?}", r.error);
        assert!(r.lab.is_none());
        // a bad call shape is a plain error naming the call
        let r = host.eval_block("vemry.lab.controls({ x: { slider: [5, 1] } })", None, None);
        assert!(r.error.as_deref().unwrap().contains("`x`.slider"), "{:?}", r.error);
    }

    #[test]
    fn demo_vault_lab_block_runs_and_moves() {
        // the chapter the demo vault ships (examples/demo-vault/papers/…) is what a
        // reader presses Run on first: its fence must run clean, declare its
        // controls, draw its views, and answer a moved slider
        let note = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/demo-vault/papers/vaswani-2017/chapters/01 Scaled dot-product attention.md"),
        )
        .unwrap();
        let start = note.find("```js\n").expect("a js fence") + 6;
        let end = start + note[start..].find("\n```").expect("the closing fence");
        let code = &note[start..end];
        let (_d, vault) = fixture(&[]);
        let bridge = TestBridge::new(false);
        let host = spawn_host(&vault, &bridge, false);
        let r = host.eval_block(code, None, None);
        assert_eq!(r.error, None, "{}", r.output);
        let spec = r.lab.expect("a lab");
        assert_eq!(spec.controls.iter().map(|c| c.name()).collect::<Vec<_>>(), ["d_k", "scale", "Q", "K"]);
        assert_eq!(spec.views.iter().map(|v| v.title()).collect::<Vec<_>>(), ["scores", "attention weights", "the first row, step by step"]);
        // scale off, d_k up: the softmax saturates — the first row's largest weight is close to one
        let r = host.eval_block(code, None, Some(serde_json::json!({ "d_k": 512, "scale": false })));
        assert_eq!(r.error, None);
        match &r.lab.unwrap().views[1] {
            lab::LabView::Heatmap { data, .. } => {
                let top = data[0].iter().flatten().cloned().fold(0.0_f64, f64::max);
                assert!(top > 0.99, "{top}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn lab_api_is_absent_from_script_contexts() {
        // the lab is enabled by the params global the block path sets; a script's
        // context never has it, so the API refuses rather than accumulating a spec
        // nothing would ever read
        let (_d, vault) = fixture(&[("l", r#"vemry.lab.heatmap("h", [[1]]);"#)]);
        let bridge = TestBridge::new(false);
        let host = spawn_host(&vault, &bridge, false);
        let infos = host.list();
        let err = info(&infos, "l").last_error.clone().unwrap();
        assert!(err.contains("only available inside a note's js block"), "{err}");
    }

    #[test]
    fn vault_switch_invalidates_inflight_context() {
        // a switch only queues a Reload behind the running job — until it
        // runs, calls from the old vault's scripts must fail instead of
        // writing into (or pulling grant state from) the new vault
        let (_d, vault) = fixture(&[(
            "w",
            r#"vemry.commands.register({ id: "save", title: "S" }, () => { vemry.write("R.md", "x"); });"#,
        )]);
        let bridge = TestBridge::new(true);
        let host = spawn_host(&vault, &bridge, false);
        host.run_command_blocking("w", "save", None).unwrap(); // grant given in vault A

        let dir_b = tempfile::tempdir().unwrap();
        fs::write(dir_b.path().join("B.md"), "# B\n").unwrap();
        let vault_b = Vault::open(dir_b.path()).unwrap();
        let root_b = vault_b.root().to_path_buf();
        *vault.lock().unwrap() = Some(vault_b);

        let err = host.run_command_blocking("w", "save", None).unwrap_err();
        assert!(err.contains("vault switched"), "got: {err}");
        assert!(!root_b.join("R.md").exists(), "old vault's grant must not reach the new vault");
        assert!(!root_b.join(".vemry").exists(), "no grant/audit state lands in the new vault");
    }

    #[test]
    fn no_vault_open_is_graceful() {
        let vault: Arc<Mutex<Option<Vault>>> = Arc::new(Mutex::new(None));
        let bridge = TestBridge::new(false);
        let host = ScriptHost::spawn(Arc::clone(&vault), Arc::clone(&bridge) as Arc<dyn UiBridge>, Perspective::default(), fast_config());
        host.reload_blocking().unwrap();
        assert!(host.list().is_empty());
        assert!(host.set_enabled("x", true).is_err());
    }
}
