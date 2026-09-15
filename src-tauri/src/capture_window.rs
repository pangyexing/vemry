//! The quick capture window (label `capture`): one global shortcut, pressed in any
//! application, raises a small always-on-top composer on the screen the mouse is
//! on; Enter lands the text (and any pasted or dropped images) in the capture
//! target, and the window goes away again with the focus handed back to the
//! application it was pressed in.
//!
//! The window is the third of our own webviews (after `agent` and `browserbar`):
//! the same bundle, mounting the CaptureWindow root by label, with
//! capabilities/capture.json matching the label exactly. It follows the chat
//! window's skeleton (agent_window.rs) with two differences:
//! - **Hidden, never destroyed.** Esc, a lost focus and Enter all `hide()` it (a
//!   landing that fails shows it again with the text); the webview stays mounted
//!   with its draft, and the next press shows it with no build to pay. It is built
//!   once, hidden, a few seconds after startup (`ensure_soon`), so even the first
//!   press is instant.
//! - **Landing goes through the main window.** This webview is another JS heap and
//!   cannot see the main window's editor buffer: had it written `inbox.md` on disk
//!   while the main window had that note open, the next autosave would overwrite
//!   the capture (the old "a capture is swallowed while inbox.md is open" bug). So
//!   the text is relayed (`capture_to_main` → `capture://action`), the main
//!   window's `vault.capture` decides between the editor buffer and `note_insert`,
//!   and its receipt comes back through `capture_reply` → `capture://result`.
//!   Rust itself never writes a capture. The resident state (tray.rs) keeps to
//!   this: closing the main window there only hides it, so the relay has a main
//!   window to land in for as long as the app runs.
//!
//! The application never activates for it (macOS): the window is turned into a
//! **non-activating panel** after it is built (`make_panel`), so it takes the
//! keyboard the way the system's own search field does — the application the
//! user was in stays active, its windows stay where they are, and hiding the
//! panel gives the keyboard straight back. Activating vemry instead would bring
//! the main window forward on every press and drop it again on every dismissal
//! (a visible flash), and would need a "re-activate the previous application"
//! step whose cooperative-activation rules differ by macOS version. Windows and
//! Linux rely on the window manager giving the focus back to the previous window
//! when ours hides.
//!
//! Keyboard in the panel (macOS): WebKit hands every key to AppKit's keyboard
//! layout pass, which turns an arrow or a delete key into an editing command; when
//! that pass yields nothing it emits an `insertText:` of the key's private-use
//! character instead, and the webview inserts it as text (a box per press). The
//! pass runs through the webview's text input context, which AppKit activates only
//! for the key window of an *active* application — never for this panel. So
//! `make_panel` activates the context by hand whenever the panel becomes key while
//! the application is inactive, puts the webview back as first responder after
//! the style-mask change (which resets it), and tells the window server about the
//! non-activating mask (set after creation, it would otherwise disagree with
//! AppKit). CaptureComposer.svelte keeps a last line of defence: a private-use
//! character arriving as text is turned back into the edit it stood for.
//!
//! The shortcut is one global combination (`capture_hotkey` in the settings,
//! `Alt+Shift+Space` by default): a registered hot key fires while vemry itself is
//! frontmost too, so the main window needs no second binding, and the menu item's
//! accelerator is a hint only — the system consumes a registered hot key before
//! the menu ever sees it.

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::AppState;

pub const CAPTURE_LABEL: &str = "capture";
/// To the capture webview: the window was just shown (refresh the settings and the notes, focus the text).
const EV_SHOWN: &str = "capture://shown";
/// To the main window: land this capture (the payload's shape is the frontend's `CaptureAction`).
const EV_ACTION: &str = "capture://action";
/// To the capture webview: the main window's receipt (`CaptureResult`).
const EV_RESULT: &str = "capture://result";

/// Logical width; the height follows the content (the webview reports it through `capture_window_resize`).
pub const WIDTH: f64 = 520.0;
const MIN_HEIGHT: f64 = 96.0;
const MAX_HEIGHT: f64 = 640.0;
/// The window's top edge sits this far down the screen the mouse is on: high enough
/// to be in the upper third, low enough to clear a menu bar or a notch.
const TOP_RATIO: f64 = 0.22;

#[derive(Default)]
pub struct CaptureWindowState(Mutex<Inner>);

#[derive(Default)]
struct Inner {
    /// The webview has mounted (`capture_window_ready` ran). Before that an emit is
    /// dropped and a show would present a blank window, so it is deferred.
    ready: bool,
    /// A show requested before the webview mounted (the first press beat the warm-up).
    pending_show: bool,
    /// The last content height the webview reported (logical px), reapplied when the window is shown again.
    height: f64,
    /// The registration failure of the shortcut, if any (the settings page shows it, and the main window toasts it once at startup).
    hotkey_error: Option<String>,
}

impl CaptureWindowState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        crate::lock(&self.0)
    }
}

// ---------------------------------------------------------------------------
// the global shortcut
// ---------------------------------------------------------------------------

/// Register the shortcut from the settings at startup. A failure (the combination
/// is held by another application, or the platform has no global shortcuts at all)
/// is recorded rather than fatal: the menu item, the palette command and the
/// activity bar still open the window, and the settings page says what went wrong.
pub fn install_hotkey(app: &AppHandle) {
    let combo = crate::settings::load(app).capture_hotkey;
    if let Err(e) = bind(app, &combo) {
        eprintln!("capture shortcut {combo}: {e}");
        app.state::<AppState>().capture_window.lock().hotkey_error = Some(e);
    }
}

/// Parse and register one combination; the handler toggles the window.
fn bind(app: &AppHandle, combo: &str) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
    let shortcut: Shortcut = combo.parse().map_err(|e| format!("{e}"))?;
    app.global_shortcut()
        .on_shortcut(shortcut, |app, _, event| {
            if event.state() == ShortcutState::Pressed {
                toggle(app);
            }
        })
        .map_err(|e| e.to_string())
}

/// Whether a combination parses (the settings page validates before saving; the
/// registration itself can still fail when another application holds the keys).
pub fn is_valid_hotkey(combo: &str) -> bool {
    combo.parse::<tauri_plugin_global_shortcut::Shortcut>().is_ok()
}

/// Re-register after a settings change. The new combination is tried first; when
/// it cannot be registered the old one is put back and the error returned, so a
/// change never leaves the user with no shortcut at all.
pub fn rebind(app: &AppHandle, old: &str, new: &str) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let _ = app.global_shortcut().unregister_all();
    let outcome = bind(app, new);
    let st = app.state::<AppState>();
    match outcome {
        Ok(()) => {
            st.capture_window.lock().hotkey_error = None;
            Ok(())
        }
        Err(e) => {
            st.capture_window.lock().hotkey_error = bind(app, old).err();
            Err(e)
        }
    }
}

/// The failure of the current registration, for the settings view (None = the shortcut works).
pub fn hotkey_error(app: &AppHandle) -> Option<String> {
    app.state::<AppState>().capture_window.lock().hotkey_error.clone()
}

// ---------------------------------------------------------------------------
// building, showing, hiding
// ---------------------------------------------------------------------------

/// Build the hidden window a moment after startup, so the first press is as fast
/// as every later one (the same warm-up idea as the browser's shell window).
pub fn ensure_soon(app: &AppHandle, delay_ms: u64) {
    crate::browser::on_main_after(app, delay_ms, |app| {
        // Quitting already: a late timer must not build anything
        if crate::browser::window::main_window(app).is_none() || app.get_webview_window(CAPTURE_LABEL).is_some() {
            return;
        }
        if let Err(e) = build(app) {
            eprintln!("capture window prebuild failed: {e}");
        }
    });
}

/// Main thread. The window starts hidden and unfocused; `capture_window_ready`
/// (the webview's mount) is what makes it showable.
fn build(app: &AppHandle) -> Result<(), String> {
    app.state::<AppState>().capture_window.lock().ready = false;
    let builder = WebviewWindowBuilder::new(app, CAPTURE_LABEL, WebviewUrl::App("index.html".into()))
        .title(crate::i18n::t(crate::i18n::current(), "window.captureTitle"))
        .inner_size(WIDTH, MIN_HEIGHT)
        .decorations(false)
        // The page draws its own rounded box on a transparent window; the system
        // shadow follows the opaque region (invalidated again after each resize)
        .transparent(true)
        .shadow(true)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .accept_first_mouse(true)
        .visible(false)
        .focused(false);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.skip_taskbar(true);
    let window = builder.build().map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    make_panel(&window.as_ref().window());

    let handle = app.clone();
    window.on_window_event(move |event| match event {
        // The keyboard went elsewhere (a click in another application, or in the main
        // window): the composer goes away and the draft stays in it
        WindowEvent::Focused(false) => hide(&handle),
        // The native close (⌘W through the menu, a window-manager close) hides instead
        // of destroying; our own close (`force_close`) is a destroy, which never asks
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            hide(&handle);
        }
        WindowEvent::Destroyed => {
            let st = handle.state::<AppState>();
            let mut cw = st.capture_window.lock();
            cw.ready = false;
            cw.pending_show = false;
        }
        _ => {}
    });
    Ok(())
}

/// macOS: the panel class the built window is switched to. The window runtime
/// creates an NSWindow subclass of its own; an NSPanel adds no instance variables
/// over NSWindow, so changing the instance's class in place is sound, and the
/// runtime's one extra variable (behind its `set_focusable`, which this window
/// never calls) is simply left behind. NSPanel's own `canBecomeKeyWindow` refuses
/// a borderless panel, hence the override.
#[cfg(target_os = "macos")]
mod panel {
    use objc2::rc::Retained;
    use objc2::{define_class, msg_send, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSPanel, NSTextInputContext, NSView, NSWindow};

    define_class!(
        #[unsafe(super(NSPanel))]
        #[thread_kind = MainThreadOnly]
        #[name = "VemryCapturePanel"]
        pub struct CapturePanel;

        impl CapturePanel {
            #[unsafe(method(canBecomeKeyWindow))]
            fn can_become_key_window(&self) -> bool {
                true
            }

            #[unsafe(method(canBecomeMainWindow))]
            fn can_become_main_window(&self) -> bool {
                false
            }

            /// The text input context of the first responder (the webview) is what turns
            /// a function key into an editing command; AppKit activates it only for the
            /// key window of an active application, so for an inactive one it is done
            /// here (see the module doc).
            #[unsafe(method(becomeKeyWindow))]
            fn become_key_window(&self) {
                unsafe {
                    let _: () = msg_send![super(self), becomeKeyWindow];
                }
                if !NSApplication::sharedApplication(self.mtm()).isActive() {
                    if let Some(ctx) = self.responder_input_context() {
                        ctx.activate();
                    }
                }
            }

            #[unsafe(method(resignKeyWindow))]
            fn resign_key_window(&self) {
                if !NSApplication::sharedApplication(self.mtm()).isActive() {
                    if let Some(ctx) = self.responder_input_context() {
                        ctx.deactivate();
                    }
                }
                unsafe {
                    let _: () = msg_send![super(self), resignKeyWindow];
                }
            }
        }
    );

    impl CapturePanel {
        fn responder_input_context(&self) -> Option<Retained<NSTextInputContext>> {
            let window: &NSWindow = self;
            let view = window.firstResponder()?.downcast::<NSView>().ok()?;
            view.inputContext()
        }
    }
}

/// macOS, main thread: turn the window into a non-activating floating panel — it
/// takes the keyboard without activating the application (the reason is in the
/// module doc), joins every Space, shows over a full-screen application (the
/// press happens there as often as anywhere), and stays out of Mission Control's
/// window list. NSPanel hides itself when the application deactivates by default;
/// that is switched off so hiding stays the `Focused(false)` hook's decision alone.
#[cfg(target_os = "macos")]
fn make_panel(win: &tauri::Window) {
    use objc2::runtime::{AnyObject, NSObjectProtocol};
    use objc2::{msg_send, sel, ClassType};
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask};
    let Ok(ptr) = win.ns_window() else { return };
    let obj: *mut AnyObject = ptr.cast();
    unsafe { objc2::ffi::object_setClass(obj, panel::CapturePanel::class()) };
    let ns: &NSWindow = unsafe { &*obj.cast() };
    // Changing the style mask resets the first responder; the webview (which the
    // webview runtime made first responder at creation) is put back, as the window
    // runtime's own style-mask helper does
    let responder = ns.firstResponder();
    ns.setStyleMask(ns.styleMask() | NSWindowStyleMask::NonactivatingPanel);
    let _ = ns.makeFirstResponder(responder.as_deref());
    // The window server learns whether a window may activate the application at
    // creation; a non-activating mask set afterwards leaves it and AppKit disagreeing
    // (the panel steals the keyboard, the server still lets a click activate). This
    // private setter is how the two are brought back in line; should it disappear
    // from AppKit, the panel merely keeps the disagreement
    if ns.respondsToSelector(sel!(_setPreventsActivation:)) {
        unsafe {
            let _: () = msg_send![ns, _setPreventsActivation: true];
        }
    }
    ns.setHidesOnDeactivate(false);
    let behavior = ns.collectionBehavior()
        | NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary
        | NSWindowCollectionBehavior::Transient;
    ns.setCollectionBehavior(behavior);
}

/// macOS: the system shadow of a transparent window is computed from what was
/// drawn; after the content changes height the old outline would linger.
#[cfg(target_os = "macos")]
fn invalidate_shadow(win: &tauri::Window) {
    use objc2_app_kit::NSWindow;
    let Ok(ptr) = win.ns_window() else { return };
    let ns: &NSWindow = unsafe { &*ptr.cast() };
    ns.invalidateShadow();
}

/// Main thread: show the window on the screen the mouse is on. With the webview
/// not mounted yet (or no window at all) the show is deferred to
/// `capture_window_ready`; a visible window is only re-focused.
pub(crate) fn show(app: &AppHandle) {
    let st = app.state::<AppState>();
    let Some(w) = app.get_webview_window(CAPTURE_LABEL) else {
        st.capture_window.lock().pending_show = true;
        if let Err(e) = build(app) {
            eprintln!("capture window build failed: {e}");
        }
        return;
    };
    {
        let mut cw = st.capture_window.lock();
        if !cw.ready {
            cw.pending_show = true;
            return;
        }
    }
    place(app, &w);
    // show() orders the window front and makes it key; set_focus() would also
    // activate the application, which the non-activating panel is there to avoid —
    // off macOS it is what gives the window the keyboard
    let _ = w.show();
    #[cfg(not(target_os = "macos"))]
    let _ = w.set_focus();
    let _ = app.emit_to(CAPTURE_LABEL, EV_SHOWN, ());
}

/// Centre the window horizontally in the work area of the monitor under the mouse
/// (the primary one when the cursor cannot be read), its top edge in the upper
/// part of the screen. The position is set in physical pixels: a monitor's
/// coordinates are physical, and converting through a logical position would use
/// the window's *current* monitor's scale, which is the wrong one when the mouse is
/// on another screen.
fn place(app: &AppHandle, w: &WebviewWindow) {
    let height = app.state::<AppState>().capture_window.lock().height.clamp(MIN_HEIGHT, MAX_HEIGHT);
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    if let Some(m) = monitor {
        let area = m.work_area();
        let scale = m.scale_factor();
        let x = area.position.x as f64 + (area.size.width as f64 - WIDTH * scale) / 2.0;
        let y = area.position.y as f64 + area.size.height as f64 * TOP_RATIO;
        let _ = w.set_position(tauri::PhysicalPosition::new(x.round(), y.round()));
    }
    let _ = w.set_size(tauri::LogicalSize::new(WIDTH, height));
}

/// Main thread: hide the window (Esc, a landed capture, a lost focus, ⌘W through
/// the menu). The keyboard goes back to where it was on its own: the application
/// the user was in never stopped being the active one.
pub(crate) fn hide(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(CAPTURE_LABEL) {
        let _ = w.hide();
    }
}

/// The shortcut's action (and the tray icon's click): a visible, focused window goes away; anything else shows it.
pub(crate) fn toggle(app: &AppHandle) {
    match app.get_webview_window(CAPTURE_LABEL) {
        Some(w) if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) => hide(app),
        _ => show(app),
    }
}

/// Closed along with the main window being destroyed (a hidden window would
/// otherwise keep the process alive): a destroy, so the hide-on-close hook above
/// is not asked.
pub fn force_close(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(CAPTURE_LABEL) {
        let _ = w.destroy();
    }
}

// ---------------------------------------------------------------------------
// commands
// ---------------------------------------------------------------------------

/// The in-app entry points (the menu item, the palette command, the activity bar).
/// Routed to the main thread: window creation and the AppKit calls belong there,
/// and a synchronous build inside a command deadlocks on Windows.
#[tauri::command]
pub fn capture_window_show(app: AppHandle) -> Result<(), String> {
    let a = app.clone();
    app.run_on_main_thread(move || show(&a)).map_err(|e| e.to_string())
}

/// Esc, and the moment after a receipt.
#[tauri::command]
pub fn capture_window_hide(app: AppHandle) -> Result<(), String> {
    let a = app.clone();
    app.run_on_main_thread(move || hide(&a)).map_err(|e| e.to_string())
}

/// The webview has mounted and is listening; a show that arrived earlier happens now.
#[tauri::command]
pub fn capture_window_ready(app: AppHandle, state: State<'_, AppState>) {
    let pending = {
        let mut cw = state.capture_window.lock();
        cw.ready = true;
        std::mem::take(&mut cw.pending_show)
    };
    if pending {
        let a = app.clone();
        let _ = app.run_on_main_thread(move || show(&a));
    }
}

/// The webview reports its content height (logical px): the window follows the content, one line to eight.
#[tauri::command]
pub fn capture_window_resize(app: AppHandle, state: State<'_, AppState>, height: f64) -> Result<(), String> {
    let height = height.clamp(MIN_HEIGHT, MAX_HEIGHT);
    state.capture_window.lock().height = height;
    let a = app.clone();
    app.run_on_main_thread(move || {
        if let Some(w) = a.get_webview_window(CAPTURE_LABEL) {
            let _ = w.set_size(tauri::LogicalSize::new(WIDTH, height));
            #[cfg(target_os = "macos")]
            invalidate_shadow(&w.as_ref().window());
        }
    })
    .map_err(|e| e.to_string())
}

/// The capture window asks the main window to land a capture — the only write
/// path (see the module doc). Err when there is no main window to land it in.
#[tauri::command]
pub fn capture_to_main(app: AppHandle, action: serde_json::Value) -> Result<(), String> {
    if crate::browser::window::main_window(&app).is_none() {
        return Err(crate::i18n::t(crate::i18n::current(), "ipc.captureNoMainWindow"));
    }
    crate::browser::window::relay_to_main(&app, EV_ACTION, action, false)
}

/// The main window's receipt for a landed (or failed) capture, back to the capture window.
#[tauri::command]
pub fn capture_reply(app: AppHandle, result: serde_json::Value) -> Result<(), String> {
    app.emit_to(CAPTURE_LABEL, EV_RESULT, result).map_err(|e| e.to_string())
}
