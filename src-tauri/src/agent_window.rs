//! The detached AI chat window (label `agent`): the conversation ⌘J summons is no
//! longer a DOM overlay inside the main window but a real native window — it can
//! be dragged out of the main window and onto another screen.
//!
//! What runs inside it is the same frontend (`index.html`), mounting the
//! AgentWindow root component by window label; it owns IPC
//! (capabilities/agent.json matches the `agent` webview exactly), which does not
//! conflict with the browser window's "a remote page gets zero IPC" boundary —
//! this is a page of our own.
//!
//! The two webviews are two JS heaps and share no Svelte store. Rust does the
//! **relaying and the handoff** and copies no frontend semantics:
//! - The open handoff package `handoff` (draft, context chips, level; the shape is
//!   the frontend's to define, and opaque JSON here) plus the main window's
//!   current note path `current_note` — the chat window takes them once at
//!   startup, and afterwards the main window pushes each note switch through
//!   `agent_window_set_note`; the editor selection snapshot `selection` works the
//!   same way (`agent_window_set_selection`), and it is where the chat window's
//!   `@ current selection` candidate comes from.
//! - The close handshake: when the native ✕ or ⌘W reaches `CloseRequested` it is
//!   prevented first and `agent-window://close-requested` is sent to the webview;
//!   the webview hands the draft back through `agent_window_close(handback)` and
//!   closes again, and the second time it is let through. Should the webview be
//!   dead, one more press of ✕ still closes it (the closing flag is already set).
//!   After `Destroyed` the main window is told through `agent-window://closed`,
//!   takes the `handback` and rebuilds its timeline from the session jsonl — the
//!   source of truth for conversation state is always the session and audit files
//!   on the Rust side, and the two windows are each a projection of them.
//! - Action relaying: "open a note / insert into a note / pin / a main-window
//!   command" from the chat window goes back through `agent_window_to_main` to be
//!   executed by the main window (which alone holds the editor and the clip
//!   target); the main window's "new session" is sent into the chat window through
//!   `agent_window_to_agent`.
//! - Geometry memory: Moved/Resized are kept in memory and written to settings on
//!   Destroyed, so the window reopens where it was.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use crate::AppState;

pub const AGENT_LABEL: &str = "agent";
const EV_CLOSE_REQUESTED: &str = "agent-window://close-requested";
const EV_CLOSED: &str = "agent-window://closed";
const EV_NOTE: &str = "agent-window://note";
const EV_SELECTION: &str = "agent-window://selection";
const EV_ACTION: &str = "agent-window://action";
const EV_COMMAND: &str = "agent-window://command";

/// Window geometry (in logical pixels; converted by scale when moved across
/// screens of different DPI, so no physical values are stored).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowGeom {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The cross-window relay state. One lock for the whole thing: the fields are
/// related to each other (discard decides whether a handback is accepted, and
/// closing and discard are reset together when the window opens or is destroyed),
/// and a lock each would only turn the close handshake into three critical
/// sections whose lock order every reader would have to work out.
#[derive(Default)]
pub struct AgentWindowState(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    /// The main window → chat window handoff package (opaque JSON; the frontend defines the shape).
    handoff: Option<serde_json::Value>,
    /// The chat window → main window package handed back at close.
    handback: Option<serde_json::Value>,
    /// A mirror of the main window's current note path: read once when the window opens, pushed afterwards.
    current_note: Option<String>,
    /// A mirror of the main window's editor selection snapshot (opaque JSON, the
    /// frontend defines the shape; the source of the `@ current selection`
    /// candidate in the chat window): the same as current_note — handed over once
    /// when the window opens, and pushed by the main window through
    /// `agent_window_set_selection` on every selection change.
    selection: Option<serde_json::Value>,
    /// The webview has mounted (take_handoff has run once). emit_to does not queue:
    /// right after the window is built the frontend bundle has not registered its
    /// listeners yet, and a command sent at that moment is simply dropped — the
    /// toolbar's "summon the AI with this page" attach-page beats the bundle load
    /// on a small page.
    /// A command sent before it is ready goes into queued and is handed over with
    /// take_handoff.
    ready: bool,
    /// `agent_window_to_agent` commands queued up before ready (cleared when the window opens, so nothing rises from the dead).
    queued: Vec<serde_json::Value>,
    /// The close handshake: the first CloseRequested asks the webview, and once this is set the second is let through.
    closing: bool,
    /// A close initiated by the main window switching vaults: the handback belongs to the old vault and is dropped whole.
    discard: bool,
    /// The most recent window geometry in this process (newer than settings; written to disk on Destroyed).
    geom: Option<WindowGeom>,
}

impl AgentWindowState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What the chat window takes, once, at startup.
#[derive(Serialize)]
pub struct Handoff {
    pub handoff: Option<serde_json::Value>,
    pub current_note: Option<String>,
    pub selection: Option<serde_json::Value>,
    /// `agent_window_to_agent` commands queued before the webview mounted, in the
    /// order they were sent; the chat window handles them one by one as if they had
    /// just arrived.
    pub queued: Vec<serde_json::Value>,
}

/// Open (or focus) the chat window. Returns whether a window was created: when one
/// is already open (including mid close handshake) it is only focused and no
/// handoff package is taken — a live webview takes its handoff only at startup, so
/// a package stored now would be collected by nobody and would merely rise from the
/// dead at the next open; the caller decides from the return value whether to clear
/// the main window's composer (false = the draft stays in the main window).
///
/// It has to be async: on Windows a synchronous command runs inside WebView2's UI
/// thread callback, and `WebviewWindowBuilder::build` in there deadlocks the whole
/// app (a known Tauri issue; the browser and reader window commands are under the
/// same constraint).
#[tauri::command]
pub async fn agent_window_open(
    app: AppHandle,
    state: State<'_, AppState>,
    handoff: Option<serde_json::Value>,
    current_note: Option<String>,
    selection: Option<serde_json::Value>,
) -> Result<bool, String> {
    let mut st = state.agent_window.lock();
    st.current_note = current_note;
    st.selection = selection;
    if let Some(w) = app.get_webview_window(AGENT_LABEL) {
        drop(st);
        let _ = w.unminimize();
        let _ = w.show();
        w.set_focus().map_err(|e| e.to_string())?;
        return Ok(false);
    }
    if handoff.is_some() {
        st.handoff = handoff;
    }
    st.closing = false;
    st.discard = false;
    // A new window starts a new ledger: a command that never reached the previous life must not rise from the dead in this one
    st.ready = false;
    st.queued.clear();
    let geom = st.geom;
    drop(st);
    build(&app, geom.or_else(|| crate::settings::load(&app).agent_window))?;
    Ok(true)
}

fn build(app: &AppHandle, geom: Option<WindowGeom>) -> Result<(), String> {
    let mut builder = WebviewWindowBuilder::new(app, AGENT_LABEL, WebviewUrl::App("index.html".into()))
        .title(crate::i18n::t(crate::i18n::current(), "window.agentTitle"))
        .min_inner_size(380.0, 420.0)
        .focused(true);
    builder = match geom {
        Some(g) => builder.position(g.x, g.y).inner_size(g.w, g.h),
        None => builder.inner_size(600.0, 760.0).center(),
    };
    let window = builder.build().map_err(|e| e.to_string())?;

    let handle = app.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            let st = handle.state::<AppState>();
            {
                let mut aw = st.agent_window.lock();
                if aw.closing {
                    return; // the webview has handed back (or this is a forced close): let it through
                }
                aw.closing = true;
            }
            api.prevent_close();
            let _ = handle.emit_to(AGENT_LABEL, EV_CLOSE_REQUESTED, ());
        }
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
            if let Some(w) = handle.get_webview_window(AGENT_LABEL) {
                if let Some(g) = read_geom(&w) {
                    handle.state::<AppState>().agent_window.lock().geom = Some(g);
                }
            }
        }
        WindowEvent::Destroyed => {
            let st = handle.state::<AppState>();
            let geom = {
                let mut aw = st.agent_window.lock();
                aw.closing = false;
                aw.discard = false;
                aw.handoff = None; // a handoff package startup never took dies with the window instead of rising again at the next open
                aw.ready = false; // commands arriving after this queue up again (cleared when the window next opens)
                aw.queued.clear();
                aw.geom
            };
            if let Some(g) = geom {
                // update: so a concurrent write of the browser geometry or the dock switch does not overwrite the other's field
                let _ = crate::settings::update(&handle, |s| {
                    let changed = s.agent_window != Some(g);
                    s.agent_window = Some(g);
                    changed
                });
            }
            let _ = handle.emit_to("main", EV_CLOSED, ());
        }
        _ => {}
    });
    Ok(())
}

/// The geometry of a minimised or hidden window cannot be trusted (some platforms
/// report 0 or a large negative), so only the normal state is recorded.
/// The geometry memory of a detached browser window (browser/window.rs) reuses this
/// same guard.
pub(crate) fn read_geom(w: &tauri::WebviewWindow) -> Option<WindowGeom> {
    if w.is_minimized().unwrap_or(false) {
        return None;
    }
    let scale = w.scale_factor().ok()?;
    let pos = w.outer_position().ok()?.to_logical::<f64>(scale);
    let size = w.inner_size().ok()?.to_logical::<f64>(scale);
    if size.width < 100.0 || size.height < 100.0 {
        return None;
    }
    Some(WindowGeom { x: pos.x, y: pos.y, w: size.width, h: size.height })
}

/// Close the window. Shared by three routes:
/// - the chat window answering the handshake itself: `handback=Some` → record the
///   package, set closing, and really close;
/// - the main window's "back to the main window": `handback=None` → only initiate
///   it, and the handshake still asks the webview for its package;
/// - the main window switching vaults: `discard=true` → the package belongs to the
///   old vault and is dropped even when it arrives.
#[tauri::command]
pub fn agent_window_close(
    app: AppHandle,
    state: State<'_, AppState>,
    handback: Option<serde_json::Value>,
    discard: Option<bool>,
) -> Result<(), String> {
    let Some(w) = app.get_webview_window(AGENT_LABEL) else {
        return Ok(());
    };
    {
        let mut aw = state.agent_window.lock();
        if discard.unwrap_or(false) {
            aw.discard = true;
        }
        if let Some(h) = handback {
            if !aw.discard {
                aw.handback = Some(h);
            }
            aw.closing = true;
        }
    }
    w.close().map_err(|e| e.to_string())
}

/// Closed along with the main window being destroyed (no handshake: with the main window gone, a handback has nowhere to go).
pub fn force_close(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(AGENT_LABEL) {
        app.state::<AppState>().agent_window.lock().closing = true;
        let _ = w.close();
    }
}

#[tauri::command]
pub fn agent_window_is_open(app: AppHandle) -> bool {
    app.get_webview_window(AGENT_LABEL).is_some()
}

/// Chat window startup: take the handoff package (once) and the current note path,
/// along with the commands queued before it mounted; from then on
/// `agent_window_to_agent` emits directly (the webview is listening).
#[tauri::command]
pub fn agent_window_take_handoff(state: State<'_, AppState>) -> Handoff {
    let mut aw = state.agent_window.lock();
    aw.ready = true;
    Handoff {
        handoff: aw.handoff.take(),
        current_note: aw.current_note.clone(),
        selection: aw.selection.clone(),
        queued: std::mem::take(&mut aw.queued),
    }
}

/// After the main window receives the closed event: take the handback package (once).
#[tauri::command]
pub fn agent_window_take_handback(state: State<'_, AppState>) -> Option<serde_json::Value> {
    state.agent_window.lock().handback.take()
}

/// The main window switched notes → mirror it and push it to the chat window (with
/// no window open it is only mirrored, and read when one opens).
#[tauri::command]
pub fn agent_window_set_note(app: AppHandle, state: State<'_, AppState>, path: Option<String>) {
    state.agent_window.lock().current_note = path.clone();
    let _ = app.emit_to(AGENT_LABEL, EV_NOTE, path);
}

/// The main window's editor selection changed → mirror it and push it to the chat
/// window (the frontend already debounces; with no window open it is only
/// mirrored, and read with the handoff package when one opens).
#[tauri::command]
pub fn agent_window_set_selection(app: AppHandle, state: State<'_, AppState>, selection: Option<serde_json::Value>) {
    state.agent_window.lock().selection = selection.clone();
    let _ = app.emit_to(AGENT_LABEL, EV_SELECTION, selection);
}

/// An action from the chat window to the main window (open a note, insert, pin, a
/// command); with `focus` it also brings the main window to the front (for a
/// navigation action the user wants to see the result).
#[tauri::command]
pub fn agent_window_to_main(app: AppHandle, action: serde_json::Value, focus: Option<bool>) -> Result<(), String> {
    app.emit_to("main", EV_ACTION, action).map_err(|e| e.to_string())?;
    if focus.unwrap_or(false) {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.set_focus();
        }
    }
    Ok(())
}

/// A command from the main window to the chat window (a new session, say).
#[tauri::command]
pub fn agent_window_to_agent(
    app: AppHandle,
    state: State<'_, AppState>,
    action: serde_json::Value,
) -> Result<(), String> {
    {
        let mut aw = state.agent_window.lock();
        if !aw.ready {
            // The window has not finished mounting (or is still being built): an emit
            // would be dropped, so it queues up and is handed over with take_handoff —
            // the toolbar's "summon the AI" attach-page often beats the bundle mount
            aw.queued.push(action);
            return Ok(());
        }
    }
    app.emit_to(AGENT_LABEL, EV_COMMAND, action).map_err(|e| e.to_string())
}
