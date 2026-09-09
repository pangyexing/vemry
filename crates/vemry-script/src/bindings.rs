//! The `vemry.*` API surface. One Rust dispatch function (`__vemry_call`)
//! crosses the JS boundary with JSON strings both ways; a JS glue script
//! builds the ergonomic API on top and keeps callbacks (commands, event
//! handlers, cron jobs) on the JS side, so no `Persistent` function handles
//! are needed — the engine fires them via `__vemry_run_command` /
//! `__vemry_fire_event` / `__vemry_fire_schedule` globals.
//!
//! Errors travel as `{"err": "..."}` envelopes and re-throw as JS `Error`s in
//! the glue, so scripts can `try/catch` naturally.

use rquickjs::{Ctx, Function};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use vemry_core::{reject, PathReject, Vault, VaultEvent, VemryError};

use crate::grants::ScriptStates;
use crate::{audit, CommandInfo, Perspective, ScriptUiEvent, UiBridge};

/// What kind of execution the context is currently running — set by the
/// engine before it calls into JS. Registration is load-only; `prompt` is
/// command-only; code blocks are read-only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExecKind {
    #[default]
    Load,
    Command,
    Workflow,
    Schedule,
    Block,
}

/// Per-script mutable state shared between the engine and the dispatch
/// closure (single-threaded: the script thread owns both ends).
#[derive(Default)]
pub struct ScriptMeta {
    pub commands: Vec<CommandInfo>,
    pub event_triggers: Vec<String>,
    pub schedules: Vec<String>,
    pub exec: ExecKind,
    /// Audit label for writes: "load" | "command" | "event:<type>" | "cron".
    pub trigger: String,
    pub current_note: Option<String>,
    /// Captured console output for code-block evaluation (M4).
    pub block_output: Vec<String>,
}

/// Everything a dispatch call needs. Moved into the `__vemry_call` closure.
pub struct BindEnv {
    pub vault: Arc<Mutex<Option<Vault>>>,
    /// Root of the vault this context was loaded for (`None` only when no
    /// vault was open at bind time). A vault switch merely queues a Reload
    /// behind any in-flight job, so an old vault's script can still be
    /// executing when the shared handle already points at the new vault —
    /// every call checks this pin and fails instead of reading, writing, or
    /// auditing into a vault that never authorized this script.
    pub root: Option<std::path::PathBuf>,
    pub perspective: Perspective,
    pub script: String,
    pub meta: Rc<RefCell<ScriptMeta>>,
    pub grants: Rc<RefCell<ScriptStates>>,
    pub bridge: Arc<dyn UiBridge>,
    /// The execution deadline cell shared with the runtime's interrupt
    /// handler. Paused while blocking on the user (grant/prompt dialogs).
    pub deadline: Arc<Mutex<Option<Instant>>>,
}

const ALLOWED_EVENTS: &[&str] = &["note_added", "note_modified", "note_removed", "files_changed"];
const CONSOLE_MAX_CHARS: usize = 2000;

/// Install `__vemry_call` and evaluate the JS glue that builds `vemry.*`.
pub fn install(ctx: &Ctx<'_>, env: BindEnv) -> Result<(), rquickjs::Error> {
    let dispatch =
        Function::new(ctx.clone(), move |method: String, payload: String| -> String { dispatch_call(&env, &method, &payload) })?;
    ctx.globals().set("__vemry_call", dispatch)?;
    ctx.eval::<(), _>(GLUE_JS)?;
    Ok(())
}

fn ok(v: Value) -> String {
    json!({ "ok": v }).to_string()
}

fn err(msg: impl Into<String>) -> String {
    json!({ "err": msg.into() }).to_string()
}

fn dispatch_call(env: &BindEnv, method: &str, payload: &str) -> String {
    let p: Value = match serde_json::from_str(payload) {
        Ok(v) => v,
        Err(e) => return err(format!("bad payload: {e}")),
    };
    match method {
        "query" => call_query(env, p),
        "read" => call_read(env, p),
        "list" => call_list(env, p),
        "search" => call_search(env, p),
        "current_note" => ok(json!(env.meta.borrow().current_note)),
        "register_command" => call_register_command(env, p),
        "on_event" => call_on_event(env, p),
        "schedule" => call_schedule(env, p),
        "write" => call_write(env, p, false),
        "append" => call_write(env, p, true),
        "toast" => call_toast(env, p),
        "prompt" => call_prompt(env, p),
        "console" => call_console(env, p),
        other => err(format!("unknown method: {other}")),
    }
}

/// Run `f` against the open vault, or error like every other consumer. The
/// vault must still be the one this context was loaded for (see `BindEnv::
/// root`); after a switch the call fails until the queued reload rebinds.
fn with_vault<T>(env: &BindEnv, f: impl FnOnce(&Vault) -> Result<T, String>) -> Result<T, String> {
    let Some(pinned) = &env.root else { return Err("no vault open".into()) };
    let guard = env.vault.lock().unwrap();
    let vault = guard.as_ref().ok_or("no vault open")?;
    if vault.root() != pinned {
        return Err("vault switched; the script call was cancelled".into());
    }
    f(vault)
}

fn with_vault_mut<T>(env: &BindEnv, f: impl FnOnce(&mut Vault) -> Result<T, String>) -> Result<T, String> {
    let Some(pinned) = &env.root else { return Err("no vault open".into()) };
    let mut guard = env.vault.lock().unwrap();
    let vault = guard.as_mut().ok_or("no vault open")?;
    if vault.root() != pinned {
        return Err("vault switched; the script call was cancelled".into());
    }
    f(vault)
}

/// Vault root for audit/grant persistence — the pinned root, and only while
/// it is still the open vault, so a switch mid-execution can never write
/// grant/audit state into a vault that didn't authorize this script.
fn pinned_root(env: &BindEnv) -> Result<std::path::PathBuf, String> {
    let Some(pinned) = env.root.clone() else { return Err("no vault open".into()) };
    let guard = env.vault.lock().unwrap();
    match guard.as_ref() {
        Some(v) if v.root() == pinned => Ok(pinned),
        Some(_) => Err("vault switched; the script call was cancelled".into()),
        None => Err("no vault open".into()),
    }
}

/// Best-effort variant for paths that silently skip on failure (console log).
fn current_root(env: &BindEnv) -> Option<std::path::PathBuf> {
    pinned_root(env).ok()
}

/// Block on the UI without burning execution budget: the deadline is lifted
/// while the user thinks and restored with its remaining budget after.
fn with_paused_deadline<T>(env: &BindEnv, f: impl FnOnce() -> T) -> T {
    let remaining = env
        .deadline
        .lock()
        .unwrap()
        .take()
        .map(|t| t.saturating_duration_since(Instant::now()));
    let out = f();
    if let Some(rem) = remaining {
        *env.deadline.lock().unwrap() = Some(Instant::now() + rem);
    }
    out
}

/// Note-vs-file rule shared by read/write: `.md` and extensionless paths are
/// notes (note_id appends `.md`), anything else is a plain vault file.
fn is_note_path(path: &str) -> bool {
    let last = path.rsplit('/').next().unwrap_or(path);
    path.ends_with(".md") || !last.contains('.')
}

fn call_query(env: &BindEnv, p: Value) -> String {
    let q: vemry_core::BlockQuery = match serde_json::from_value(p) {
        Ok(q) => q,
        Err(e) => return err(format!("bad query: {e}")),
    };
    let hide = env.perspective.hide_private;
    match with_vault(env, |v| {
        // The script's perspective decides whether it reads as the agent does.
        let hits = if hide { v.query_for_agent(&q).0 } else { v.index().query(&q) };
        serde_json::to_value(hits).map_err(|e| e.to_string())
    }) {
        Ok(v) => ok(v),
        Err(e) => err(e),
    }
}

fn call_read(env: &BindEnv, p: Value) -> String {
    let Some(path) = p["path"].as_str().map(str::to_string) else {
        return err("read: missing `path`");
    };
    let hide = env.perspective.hide_private;
    let result = with_vault(env, |v| {
        if is_note_path(&path) {
            let id = v.note_id(&path).map_err(|e| e.to_string())?;
            if hide && v.is_private(&id) {
                // indistinguishable from a missing note, verbatim
                return Err(VemryError::NoteNotFound(id.to_string()).to_string());
            }
            if hide {
                // the agent's read: private blocks removed
                return v.read_note_for_agent(&id).map(|r| r.text).map_err(|e| e.to_string());
            }
            v.read_note(&id).map_err(|e| e.to_string())
        } else {
            if hide && v.is_private_rel(&path, false) {
                return Err(reject(PathReject::NotFound, path.clone()).to_string());
            }
            v.read_file(&path).map_err(|e| e.to_string())
        }
    });
    match result {
        Ok(content) => ok(json!(content)),
        Err(e) => err(e),
    }
}

fn call_list(env: &BindEnv, p: Value) -> String {
    let folder = p["folder"].as_str().map(|f| f.trim_matches('/').to_string()).filter(|f| !f.is_empty());
    let hide = env.perspective.hide_private;
    match with_vault(env, |v| {
        let mut summaries = v.index().summaries();
        if let Some(folder) = &folder {
            let prefix = format!("{folder}/");
            summaries.retain(|s| s.id.as_str().starts_with(&prefix));
        }
        if hide {
            summaries.retain(|s| !v.is_private(&s.id));
        }
        serde_json::to_value(summaries).map_err(|e| e.to_string())
    }) {
        Ok(v) => ok(v),
        Err(e) => err(e),
    }
}

fn call_search(env: &BindEnv, p: Value) -> String {
    let Some(query) = p["query"].as_str().map(str::to_string) else {
        return err("search: missing `query`");
    };
    let limit = p["limit"].as_u64().map(|n| n as usize).unwrap_or(30);
    let hide = env.perspective.hide_private;
    match with_vault(env, |v| {
        let mut hits = if hide { v.search_for_agent(&query, limit) } else { v.search(&query, limit) };
        if hide {
            hits.retain(|h| !v.is_private(&h.note));
        }
        serde_json::to_value(hits).map_err(|e| e.to_string())
    }) {
        Ok(v) => ok(v),
        Err(e) => err(e),
    }
}

fn call_register_command(env: &BindEnv, p: Value) -> String {
    if env.meta.borrow().exec != ExecKind::Load {
        return err("commands.register may only be called while the script is loading");
    }
    let Some(id) = p["id"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string) else {
        return err("commands.register: missing `id`");
    };
    let Some(title) = p["title"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string) else {
        return err("commands.register: missing `title`");
    };
    let mut meta = env.meta.borrow_mut();
    if meta.commands.iter().any(|c| c.id == id) {
        return err(format!("duplicate command id: {id}"));
    }
    meta.commands.push(CommandInfo {
        id,
        title,
        description: p["description"].as_str().map(str::to_string),
        hotkey: p["hotkey"].as_str().map(str::to_string),
    });
    ok(Value::Null)
}

fn call_on_event(env: &BindEnv, p: Value) -> String {
    if env.meta.borrow().exec != ExecKind::Load {
        return err("events.on may only be called while the script is loading");
    }
    let Some(kind) = p["type"].as_str() else {
        return err("events.on: missing `type`");
    };
    if !ALLOWED_EVENTS.contains(&kind) {
        return err(format!("events.on: unknown event type {kind} (available: {})", ALLOWED_EVENTS.join(", ")));
    }
    env.meta.borrow_mut().event_triggers.push(kind.to_string());
    ok(Value::Null)
}

fn call_schedule(env: &BindEnv, p: Value) -> String {
    if env.meta.borrow().exec != ExecKind::Load {
        return err("schedule may only be called while the script is loading");
    }
    let Some(expr) = p["cron"].as_str().map(str::trim).filter(|s| !s.is_empty()) else {
        return err("schedule: missing cron expression");
    };
    // the same translation CronEntry uses: seconds field prepended, POSIX
    // day-of-week numbering (0/7 = Sunday) mapped to the crate's 1-7
    let Some(translated) = crate::scheduler::to_crate_expr(expr) else {
        return err(format!("schedule: cron expression must have the standard 5 fields (minute hour day month weekday), got: {expr}"));
    };
    if let Err(e) = translated.parse::<cron::Schedule>() {
        return err(format!("schedule: invalid cron expression: {e}"));
    }
    env.meta.borrow_mut().schedules.push(expr.to_string());
    ok(Value::Null)
}

fn call_write(env: &BindEnv, p: Value, append: bool) -> String {
    let what = if append { "append" } else { "write" };
    let exec = env.meta.borrow().exec;
    if exec == ExecKind::Block {
        return err("a js code block is a read-only sandbox; put writes in a .vemry/scripts/ script");
    }
    let Some(path) = p["path"].as_str().map(str::to_string) else {
        return err(format!("{what}: missing `path`"));
    };
    let content_key = if append { "text" } else { "content" };
    let Some(content) = p[content_key].as_str().map(str::to_string) else {
        return err(format!("{what}: missing `{content_key}`"));
    };
    // host state stays out of reach: no writes into hidden directories
    // (`.vemry/scripts.json` tampering would be self-granting)
    if path.split('/').any(|seg| seg.starts_with('.')) {
        return err(format!("{what}: cannot write into a hidden directory: {path}"));
    }
    if append && !is_note_path(&path) {
        return err("append only supports notes (.md)");
    }

    let root = match pinned_root(env) {
        Ok(root) => root,
        Err(e) => return err(e),
    };
    // First write: block on the user's grant dialog (deadline paused, no
    // locks held), persist the answer, audit both the ask and the outcome.
    if !env.grants.borrow().write_grant(&env.script) {
        audit::append(&root, "grant_asked", json!({ "script": env.script, "path": path }));
        let granted = with_paused_deadline(env, || env.bridge.request_grant(&env.script, &path));
        if !granted {
            audit::append(&root, "grant_denied", json!({ "script": env.script }));
            return err(format!("write not granted: the user has not allowed script {} to write to this vault", env.script));
        }
        env.grants.borrow_mut().set_write_grant(&env.script, true);
        if let Err(e) = env.grants.borrow().save(&root) {
            return err(format!("could not save the grant: {e}"));
        }
        audit::append(&root, "grant_given", json!({ "script": env.script }));
    }

    let hide = env.perspective.hide_private;
    let result = with_vault_mut(env, |v| {
        if is_note_path(&path) {
            let id = v.note_id(&path).map_err(|e| e.to_string())?;
            if hide && v.is_private(&id) {
                return Err(VemryError::NoteNotFound(id.to_string()).to_string());
            }
            let ev = if append {
                v.append_note(&id, &content).map_err(|e| e.to_string())?
            } else {
                v.write_note(&id, &content).map_err(|e| e.to_string())?
            };
            Ok(ev)
        } else {
            if hide && v.is_private_rel(&path, false) {
                return Err(reject(PathReject::NotFound, path.clone()).to_string());
            }
            v.write_file(&path, &content).map_err(|e| e.to_string())?;
            Ok(VaultEvent::FilesChanged)
        }
    });
    match result {
        Ok(event) => {
            let trigger = env.meta.borrow().trigger.clone();
            audit::append(
                &root,
                "write",
                json!({ "script": env.script, "path": path, "bytes": content.len(), "append": append, "trigger": trigger }),
            );
            // straight to the UI — never into the workflow dispatcher
            env.bridge.vault_event(event);
            ok(Value::Null)
        }
        Err(e) => err(e),
    }
}

fn call_toast(env: &BindEnv, p: Value) -> String {
    let message = p["message"].as_str().unwrap_or_default().to_string();
    if message.is_empty() {
        return err("toast: missing `message`");
    }
    let level = match p["level"].as_str() {
        Some("error") => "error",
        _ => "info",
    };
    env.bridge.emit(ScriptUiEvent::Toast { message, level: level.into() });
    ok(Value::Null)
}

fn call_prompt(env: &BindEnv, p: Value) -> String {
    // Only a user-invoked command has someone at the keyboard to answer;
    // unattended contexts (load, workflows, cron) resolve to null instead of
    // hanging a background run on a dialog nobody will see.
    if env.meta.borrow().exec != ExecKind::Command {
        return ok(Value::Null);
    }
    let Some(message) = p["message"].as_str() else {
        return err("prompt: missing `message`");
    };
    let placeholder = p["placeholder"].as_str();
    let answer = with_paused_deadline(env, || env.bridge.request_prompt(&env.script, message, placeholder));
    ok(json!(answer))
}

fn call_console(env: &BindEnv, p: Value) -> String {
    let level = p["level"].as_str().unwrap_or("info").to_string();
    let mut text = p["text"].as_str().unwrap_or_default().to_string();
    if text.chars().count() > CONSOLE_MAX_CHARS {
        text = text.chars().take(CONSOLE_MAX_CHARS).collect::<String>() + "…";
    }
    if env.meta.borrow().exec == ExecKind::Block {
        env.meta.borrow_mut().block_output.push(text);
    } else if let Some(root) = current_root(env) {
        audit::append(&root, "console", json!({ "script": env.script, "level": level, "text": text }));
    }
    ok(Value::Null)
}

/// The ergonomic `vemry.*` API, `console`, and the engine-facing fire
/// functions. Callbacks live in JS-side maps keyed by what Rust knows
/// (command id / event type / schedule index).
const GLUE_JS: &str = r#"
(() => {
  const raw = globalThis.__vemry_call;
  delete globalThis.__vemry_call;
  const invoke = (method, payload) => {
    const r = JSON.parse(raw(method, JSON.stringify(payload ?? {})));
    if (r.err !== undefined) throw new Error(r.err);
    return r.ok;
  };
  const cmds = new Map();
  const handlers = new Map();
  const crons = [];

  // ---- vemry.lab: the declarative experiment of a note's js block ----
  // Enabled only when the engine set __vemry_lab_params before installing the
  // glue, which it does for block evaluation alone — a script context has no lab.
  // The block declares controls (and gets their current values back) and draws
  // views; the editor renders the spec __vemry_lab_take hands out, and re-runs
  // the block with the moved controls as params. Shapes are normalised here into
  // the canonical JSON lab.rs types and caps (crates/vemry-script/src/lab.rs).
  const labRaw = globalThis.__vemry_lab_params;
  delete globalThis.__vemry_lab_params;
  const labEnabled = typeof labRaw === "string";
  let labParams = {};
  if (labEnabled) {
    try { labParams = JSON.parse(labRaw); } catch { labParams = {}; }
    if (!labParams || typeof labParams !== "object" || Array.isArray(labParams)) labParams = {};
  }
  const labState = { controls: [], views: [] };
  let labUsed = false;
  const labOn = (what) => {
    if (!labEnabled) throw new Error("lab." + what + " is only available inside a note's js block");
    labUsed = true;
  };
  const isNum = (v) => typeof v === "number" && Number.isFinite(v);
  const numOrNull = (v) => (isNum(v) ? v : null);
  const strOr = (v, fallback) => (v == null ? fallback : String(v));
  const numList = (a, what) => {
    if (!Array.isArray(a)) throw new Error(what + " must be an array of numbers");
    return a.map(numOrNull);
  };
  const numGrid = (g, what) => {
    if (!Array.isArray(g) || !g.every(Array.isArray)) throw new Error(what + " must be an array of rows (arrays of numbers)");
    return g.map((r) => numList(r, what));
  };
  const digitsOf = (v) => (isNum(v) ? Math.max(0, Math.min(6, Math.round(v))) : null);
  const labControls = (spec) => {
    labOn("controls");
    if (!spec || typeof spec !== "object" || Array.isArray(spec)) throw new Error("lab.controls: pass an object of { name: spec }");
    const values = {};
    for (const [name, c] of Object.entries(spec)) {
      if (!c || typeof c !== "object") throw new Error("lab.controls: `" + name + "` needs a spec object");
      const label = c.label == null ? null : String(c.label);
      const given = labParams[name];
      if (Array.isArray(c.slider)) {
        const min = Number(c.slider[0]);
        const max = Number(c.slider[1]);
        if (!isNum(min) || !isNum(max) || !(max > min)) throw new Error("lab.controls: `" + name + "`.slider must be [min, max] with max > min");
        const step = isNum(c.step) && c.step > 0 ? c.step : (max - min) / 100;
        let value = isNum(c.value) ? c.value : min;
        if (isNum(given)) value = given;
        value = Math.min(max, Math.max(min, value));
        labState.controls.push({ kind: "slider", name, label, min, max, step, value });
        values[name] = value;
      } else if (typeof c.toggle === "boolean") {
        const value = typeof given === "boolean" ? given : c.toggle;
        labState.controls.push({ kind: "toggle", name, label, value });
        values[name] = value;
      } else if (Array.isArray(c.select)) {
        const options = c.select.map(String);
        if (!options.length) throw new Error("lab.controls: `" + name + "`.select needs at least one option");
        let value = c.value == null ? options[0] : String(c.value);
        if (typeof given === "string" && options.includes(given)) value = given;
        if (!options.includes(value)) value = options[0];
        labState.controls.push({ kind: "select", name, label, options, value });
        values[name] = value;
      } else if (c.presets != null) {
        // presets: [{ label, set: { name: value } }] or { label: { name: value } } — buttons that set other controls
        const raw = Array.isArray(c.presets) ? c.presets : Object.entries(c.presets).map(([label, set]) => ({ label, set }));
        const items = raw.map((it, i) => {
          if (!it || typeof it !== "object") throw new Error("lab.controls: `" + name + "`.presets needs { label, set } items");
          const label = String(it.label ?? it.name ?? i + 1);
          const set = it.set && typeof it.set === "object" ? it.set : Object.fromEntries(Object.entries(it).filter(([k]) => k !== "label" && k !== "name"));
          return { label, set };
        });
        if (!items.length) throw new Error("lab.controls: `" + name + "`.presets needs at least one item");
        const value = typeof given === "string" && items.some((it) => it.label === given) ? given : null;
        labState.controls.push({ kind: "presets", name, label, items, value });
        values[name] = value;
      } else {
        throw new Error("lab.controls: `" + name + "` needs slider: [min, max], toggle: true|false, select: [options], or presets: [{ label, set }]");
      }
    }
    return values;
  };
  const labMatrix = (name, rows, opts) => {
    labOn("matrix");
    name = String(name);
    const base = numGrid(rows, "lab.matrix `" + name + "`").map((r) => r.map((v) => v ?? 0));
    const width = base.length ? base[0].length : 0;
    if (!width || base.some((r) => r.length !== width)) throw new Error("lab.matrix: `" + name + "` must be a rectangular grid with at least one cell");
    const given = labParams[name];
    let value = base;
    if (
      Array.isArray(given) &&
      given.length === base.length &&
      given.every((r) => Array.isArray(r) && r.length === width && r.every(isNum))
    )
      value = given;
    const o = opts ?? {};
    labState.controls.push({
      kind: "matrix",
      name,
      label: o.label == null ? null : String(o.label),
      rows: value,
      min: numOrNull(o.min),
      max: numOrNull(o.max),
      step: isNum(o.step) && o.step > 0 ? o.step : null,
    });
    return value.map((r) => r.slice());
  };
  const labHeatmap = (title, data, opts) => {
    labOn("heatmap");
    const o = opts ?? {};
    labState.views.push({
      kind: "heatmap",
      title: strOr(title, ""),
      data: numGrid(data, "lab.heatmap data"),
      rows: Array.isArray(o.rows) ? o.rows.map(String) : null,
      cols: Array.isArray(o.cols) ? o.cols.map(String) : null,
      min: numOrNull(o.min),
      max: numOrNull(o.max),
      digits: digitsOf(o.digits),
      diverging: o.diverging === true,
    });
  };
  const seriesOf = (input, what) => {
    if (Array.isArray(input) && input.every((v) => v == null || typeof v === "number")) return [{ name: null, y: numList(input, what), x: null }];
    const list = Array.isArray(input) ? input : [input];
    return list.map((s) => {
      if (!s || typeof s !== "object" || !Array.isArray(s.y)) throw new Error(what + ": a series is { name, y: [numbers] } (x optional)");
      return { name: strOr(s.name, null), y: numList(s.y, what), x: Array.isArray(s.x) ? numList(s.x, what) : null };
    });
  };
  const labLine = (title, series, opts) => {
    labOn("line");
    const o = opts ?? {};
    labState.views.push({
      kind: "line",
      title: strOr(title, ""),
      series: seriesOf(series, "lab.line"),
      x: Array.isArray(o.x) ? numList(o.x, "lab.line x") : null,
      x_label: strOr(o.xLabel, null),
      y_label: strOr(o.yLabel, null),
      y_min: numOrNull(o.yMin),
      y_max: numOrNull(o.yMax),
    });
  };
  const labBars = (title, data, opts) => {
    labOn("bars");
    const o = opts ?? {};
    let labels;
    let series;
    if (Array.isArray(data) && data.every((d) => Array.isArray(d) && d.length === 2)) {
      labels = data.map((d) => String(d[0]));
      series = [{ name: null, values: data.map((d) => numOrNull(d[1])) }];
    } else if (data && typeof data === "object" && Array.isArray(data.labels)) {
      labels = data.labels.map(String);
      const list = Array.isArray(data.series) ? data.series : [];
      series = list.map((s) => ({ name: strOr(s && s.name, null), values: numList(s && s.values, "lab.bars values") }));
    } else if (data && typeof data === "object" && !Array.isArray(data)) {
      labels = Object.keys(data);
      series = [{ name: null, values: labels.map((k) => numOrNull(data[k])) }];
    } else {
      throw new Error("lab.bars: pass { label: value }, [[label, value], …] or { labels, series: [{ name, values }] }");
    }
    labState.views.push({ kind: "bars", title: strOr(title, ""), labels, series, y_label: strOr(o.yLabel, null), horizontal: o.horizontal === true });
  };
  const pointsOf = (p, what) => {
    if (!Array.isArray(p)) throw new Error(what + ": points are [[x, y], …]");
    return p.filter((q) => Array.isArray(q) && isNum(q[0]) && isNum(q[1])).map((q) => [q[0], q[1]]);
  };
  const labScatter = (title, points, opts) => {
    labOn("scatter");
    const o = opts ?? {};
    let series;
    if (Array.isArray(points) && points.every(Array.isArray)) series = [{ name: null, points: pointsOf(points, "lab.scatter") }];
    else {
      const list = Array.isArray(points) ? points : [points];
      series = list.map((s) => ({ name: strOr(s && s.name, null), points: pointsOf(s && s.points, "lab.scatter") }));
    }
    labState.views.push({ kind: "scatter", title: strOr(title, ""), series, x_label: strOr(o.xLabel, null), y_label: strOr(o.yLabel, null) });
  };
  const labTable = (title, rows, opts) => {
    labOn("table");
    const o = opts ?? {};
    if (!Array.isArray(rows)) throw new Error("lab.table: rows are an array of arrays or of objects");
    let columns;
    let body;
    if (rows.every(Array.isArray)) {
      columns = Array.isArray(o.columns) ? o.columns.map(String) : (rows[0] ?? []).map((_, i) => String(i + 1));
      body = rows;
    } else {
      columns = Array.isArray(o.columns) ? o.columns.map(String) : [...new Set(rows.flatMap((r) => Object.keys(r ?? {})))];
      body = rows.map((r) => columns.map((c) => (r ?? {})[c]));
    }
    const cell = (v) => (v == null ? null : typeof v === "number" ? numOrNull(v) : String(v));
    const h = o.highlight && typeof o.highlight === "object" ? o.highlight : null;
    labState.views.push({
      kind: "table",
      title: strOr(title, ""),
      columns,
      rows: body.map((r) => columns.map((_, i) => cell(r[i]))),
      digits: digitsOf(o.digits),
      highlight: h ? { row: isNum(h.row) ? h.row : null, col: isNum(h.col) ? h.col : null } : null,
    });
  };
  const labStep = (title, steps) => {
    labOn("step");
    if (!Array.isArray(steps)) throw new Error("lab.step: steps are an array of { label, text, value } (or strings)");
    const items = steps.map((s, i) => {
      if (typeof s === "string") return { label: s, text: null, value: null };
      if (!s || typeof s !== "object") throw new Error("lab.step: step " + (i + 1) + " must be a string or { label, text, value }");
      let value = null;
      if (isNum(s.value)) value = s.value;
      else if (Array.isArray(s.value)) value = s.value.every(Array.isArray) ? numGrid(s.value, "lab.step value") : numList(s.value, "lab.step value");
      return { label: strOr(s.label, "Step " + (i + 1)), text: strOr(s.text, null), value };
    });
    labState.views.push({ kind: "step", title: strOr(title, ""), steps: items });
  };
  globalThis.__vemry_lab_take = () => (labUsed ? JSON.stringify(labState) : null);
  const lab = Object.freeze({
    controls: labControls,
    matrix: labMatrix,
    heatmap: labHeatmap,
    line: labLine,
    bars: labBars,
    scatter: labScatter,
    table: labTable,
    step: labStep,
  });

  const vemry = {
    lab,
    query: (q) => invoke("query", q ?? {}),
    read: (path) => invoke("read", { path: String(path) }),
    list: (folder) => invoke("list", { folder: folder == null ? null : String(folder) }),
    search: (query, limit) => invoke("search", { query: String(query), limit: limit ?? null }),
    currentNote: () => invoke("current_note", {}),
    write: (path, content) => invoke("write", { path: String(path), content: String(content) }),
    append: (path, text) => invoke("append", { path: String(path), text: String(text) }),
    toast: (message, level) => invoke("toast", { message: String(message), level: level ?? "info" }),
    prompt: (message, placeholder) =>
      invoke("prompt", { message: String(message), placeholder: placeholder == null ? null : String(placeholder) }),
    commands: Object.freeze({
      register: (spec, fn) => {
        if (typeof fn !== "function") throw new Error("commands.register: the second argument must be a function");
        invoke("register_command", spec ?? {});
        // the Rust side stores the trimmed id and fires with it — key the
        // callback map identically or the command becomes uninvokable
        cmds.set(String(spec.id).trim(), fn);
      },
    }),
    events: Object.freeze({
      on: (type, fn) => {
        if (typeof fn !== "function") throw new Error("events.on: the second argument must be a function");
        invoke("on_event", { type: String(type) });
        const list = handlers.get(type) ?? [];
        list.push(fn);
        handlers.set(type, list);
      },
    }),
    schedule: (cron, fn) => {
      if (typeof fn !== "function") throw new Error("schedule: the second argument must be a function");
      invoke("schedule", { cron: String(cron) });
      crons.push(fn);
    },
  };
  Object.freeze(vemry);
  globalThis.vemry = vemry;
  const mklog = (level) => (...args) => {
    const text = args
      .map((a) => {
        if (typeof a === "string") return a;
        try { return JSON.stringify(a); } catch { return String(a); }
      })
      .join(" ");
    invoke("console", { level, text });
  };
  globalThis.console = Object.freeze({
    log: mklog("info"),
    info: mklog("info"),
    warn: mklog("warn"),
    error: mklog("error"),
  });
  globalThis.__vemry_run_command = (id, ctxJson) => {
    const fn = cmds.get(id);
    if (!fn) throw new Error("unregistered command: " + id);
    fn(ctxJson ? JSON.parse(ctxJson) : {});
  };
  globalThis.__vemry_fire_event = (type, evJson) => {
    for (const fn of handlers.get(type) ?? []) fn(evJson ? JSON.parse(evJson) : {});
  };
  globalThis.__vemry_fire_schedule = (i) => {
    const fn = crons[i];
    if (fn) fn();
  };
})();
"#;
