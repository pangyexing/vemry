//! The clip toolbar window (label `clipbar-N`, paired one to one with
//! `browser-N`).
//!
//! A browser window is a remote page with zero IPC (the security boundary), so no
//! clipping UI can live inside it — the toolbar is **a window of our own** (like the
//! agent window: the same bundle, authorised by label through
//! capabilities/clipbar.json) laid over the bottom of the browser window's frame
//! (the positional language of a browser's status bar: the top belongs to the page,
//! the bottom to vemry; the top also carries the floating traffic lights, which must
//! not be covered).
//!
//! It only displays and forwards: a clip action is relayed back through
//! `clipbar_to_main` to be executed by the main window (delivery has to go through
//! the main window's editor buffer, and the record of where things land is collected
//! there too), and the receipt comes back through `clipbar://result` to flash in
//! place. "Not having to go back to the main window" is this window's entire point,
//! so the relay does not raise the main window by default ("open the note" being the
//! one exception).
//!
//! Window relations: on macOS it is attached as a native child of the **browser**
//! window (which under docking makes it "a child of a child"; AppKit passes movement
//! down nested children, and should that ever glitch, the way out is attaching it to
//! the main window when docked — one file to change); off macOS it is built with
//! `.parent(the browser window)` to keep the z-order, and follows the browser
//! window's Moved/Resized by hand. Its lifetime follows the browser window entirely:
//! when that is Destroyed, window.rs closes the paired toolbar with it, and the
//! toolbar never enters the BrowserState registry (the same as the reader window —
//! the registry means "pages", and a toolbar is only chrome).

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// The collapsed height (logical px); it grows when the destination picker expands (anchored at the bottom edge, growing upwards).
pub const CLIPBAR_H: f64 = 40.0;
pub const CLIPBAR_EXPANDED_H: f64 = 320.0;

pub fn clipbar_label_of(browser_label: &str) -> String {
    format!("clipbar-{}", browser_label.strip_prefix("browser-").unwrap_or(browser_label))
}

/// The reverse of the pairing: a toolbar label → the label of the browser window it is stuck to.
pub fn browser_label_of(clipbar_label: &str) -> String {
    format!("browser-{}", clipbar_label.strip_prefix("clipbar-").unwrap_or(clipbar_label))
}

/// Run spawn_clipbar a beat later: building the toolbar's webview competes for the
/// main thread, and doing it right at the browser window's first frame slows the page
/// load. The bar appears after roughly 0.4s — nobody can "select and clip" within
/// that, so nothing is lost. Should the browser window already be gone, spawn_inner
/// gives up silently.
pub fn spawn_clipbar_soon(app: &AppHandle, browser_label: &str) {
    let label = browser_label.to_string();
    super::on_main_after(app, 400, move |app| spawn_clipbar(app, &label));
}

/// Build the toolbar window and stick it to the bottom of the browser window. Called
/// on the main thread (where spawn_clipbar_soon's delay lands); a failure is only
/// logged — a missing toolbar does not block browsing.
pub fn spawn_clipbar(app: &AppHandle, browser_label: &str) {
    if let Err(e) = spawn_inner(app, browser_label) {
        eprintln!("clipbar spawn failed for {browser_label}: {e}");
    }
}

fn spawn_inner(app: &AppHandle, browser_label: &str) -> Result<(), String> {
    let label = clipbar_label_of(browser_label);
    if app.get_webview_window(&label).is_some() {
        return Ok(());
    }
    // The browser window is already closed (⌘W during the build delay): give up silently, this is not an error
    let Some(browser) = app.get_webview_window(browser_label) else {
        return Ok(());
    };
    let scale = browser.scale_factor().unwrap_or(1.0);
    // inner_size: on Windows the outer size includes an invisible drag border (~7px),
    // so building against outer would make the bar wider than the visible frame; on
    // macOS the two are the same width and it makes no difference
    let bw = browser.inner_size().map(|s| s.width as f64 / scale).unwrap_or(1080.0);

    #[allow(unused_mut)]
    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title(crate::i18n::t(crate::i18n::current(), "window.clipbarTitle"))
        .inner_size(bw, CLIPBAR_H)
        .decorations(false)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        // A transparent window with the frontend painting the background and the
        // bottom corners: a square-cornered bar over a rounded window's bottom edge
        // would poke out (macOS needs the macos-private-api feature, which is on)
        .transparent(true)
        // The bar sits inside the browser window's frame, where its own window shadow would draw a line across the page
        .shadow(false)
        .visible(false)
        .focused(false)
        .skip_taskbar(true);
    #[cfg(target_os = "macos")]
    {
        // The first click counts even when unfocused: pressing a button does not pay the "focus first, then click again" tax
        builder = builder.accept_first_mouse(true);
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Windows owner / Linux transient: the z-order stays above the browser window,
        // and it hides or is destroyed along with it. macOS does not use
        // builder.parent (which would addChildWindow at build time) and keeps to the
        // "position → attach → show" order
        builder = builder.parent(&browser).map_err(|e| e.to_string())?;
    }
    let bar = builder.build().map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    enable_hover_when_inactive(&bar);

    position_clipbar(app, browser_label);
    ensure_clipbar_attached(app, browser_label);
    let _ = bar.show();
    Ok(())
}

/// macOS: key-follows-mouse into the bar. A WKWebView in a window that is not key
/// receives no mouseMoved (AppKit's move event stream feeds the key window alone; the
/// first version added an ActiveInActiveApp tracking area to the webview, which
/// measurably did not save hover) — while the rollover channel of
/// mouseEntered/Exited is reliable for a non-key window. So it goes the other way: a
/// custom owner takes mouseEntered, and when the mouse enters the bar **while the
/// paired browser window is key** it makes the bar window key — WebKit's own tracking
/// areas activate immediately, and hover, cursor and clicks all respond natively
/// without paying the "click once first" tax.
/// The "the browser window is key" gate keeps the keyboard focus from being taken
/// away when the mouse sweeps across the toolbar while the user is typing in the main
/// window.
/// The way back is in the frontend (ClipBar.svelte's onBarLeave): when the mouse
/// leaves and nothing is being typed, refocusBrowser gives key back to the browser
/// window; while an address or a destination is being edited, it does not.
#[cfg(target_os = "macos")]
fn enable_hover_when_inactive(bar: &tauri::WebviewWindow) {
    let _ = bar.with_webview(|pw| {
        use objc2::AnyThread;
        use objc2_app_kit::{NSTrackingArea, NSTrackingAreaOptions, NSView};
        use objc2_foundation::NSRect;
        let view = pw.inner().cast::<NSView>();
        let Some(view) = (unsafe { view.as_ref() }) else { return };
        let owner = hover_key::HoverKeyOwner::new();
        let opts = NSTrackingAreaOptions::MouseEnteredAndExited
            | NSTrackingAreaOptions::InVisibleRect
            | NSTrackingAreaOptions::ActiveInActiveApp;
        let owner_obj: &objc2::runtime::AnyObject = &owner;
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::ZERO, // InVisibleRect: the real area always follows the visible rect, so this one does not matter
                opts,
                Some(owner_obj),
                None,
            )
        };
        view.addTrackingArea(&area);
        // NSTrackingArea does not retain its owner: it has to stay alive for the
        // window's lifetime. Each browser window has at most one toolbar, so this
        // few-dozen-byte object is leaked deliberately, saving the lifetime bookkeeping.
        std::mem::forget(owner);
    });
}

#[cfg(target_os = "macos")]
mod hover_key {
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{define_class, msg_send, AnyThread};
    use objc2_app_kit::NSEvent;

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "VemryClipbarHoverKey"]
        pub struct HoverKeyOwner;

        impl HoverKeyOwner {
            #[unsafe(method(mouseEntered:))]
            fn mouse_entered(&self, event: &NSEvent) {
                // Tracking-area events are only ever delivered on the main thread
                let Some(mtm) = objc2::MainThreadMarker::new() else { return };
                let Some(bar) = event.window(mtm) else { return };
                // Taken over only while the paired browser window holds key: moving
                // into the toolbar while reading makes it live; sweeping the mouse
                // across the bar while working in the main window or elsewhere does not
                // steal the keyboard focus
                let Some(parent) = bar.parentWindow() else { return };
                if parent.isKeyWindow() && !bar.isKeyWindow() {
                    bar.makeKeyWindow();
                }
            }

            // The MouseEnteredAndExited option will try to deliver exited, so it has to
            // be answerable; the way back lives in the frontend (which knows whether
            // something is being typed), so an empty implementation is enough here
            #[unsafe(method(mouseExited:))]
            fn mouse_exited(&self, _event: &NSEvent) {}
        }
    );

    impl HoverKeyOwner {
        pub fn new() -> Retained<Self> {
            unsafe { msg_send![Self::alloc(), init] }
        }
    }
}

/// Pure geometric following (all physical px, in the coordinate space of the browser
/// window's screen): x and width follow the browser window, and the bottom edge sticks
/// to its bottom edge; the height keeps the toolbar's current one, so dragging the
/// browser window while the picker is expanded does not collapse it.
/// It uses inner_* (the client area): on Windows the outer size includes the invisible
/// drag border, and sticking to outer would overhang the visible frame on three sides;
/// on macOS the client area is the same width and its bottom edge is the same line, so
/// nothing changes.
pub fn position_clipbar(app: &AppHandle, browser_label: &str) {
    let label = clipbar_label_of(browser_label);
    let Some(browser) = app.get_webview_window(browser_label) else { return };
    let Some(bar) = app.get_webview_window(&label) else { return };
    let (Ok(bpos), Ok(bsize)) = (browser.inner_position(), browser.inner_size()) else { return };
    let scale = browser.scale_factor().unwrap_or(1.0);
    let bar_h =
        bar.outer_size().map(|s| s.height).unwrap_or((CLIPBAR_H * scale).round() as u32);
    let _ = bar.set_size(tauri::PhysicalSize::new(bsize.width, bar_h));
    let _ = bar.set_position(tauri::PhysicalPosition::new(
        bpos.x,
        bpos.y + bsize.height as i32 - bar_h as i32,
    ));
}

/// macOS: attach the toolbar as a native child of the browser window (so it follows
/// its movement, minimisation and layering). Idempotent — every time the browser
/// window is focused it goes through a backstop re-attachment (minimising breaks the
/// child-window relation, the same two-level backstop as reattach_docked between the
/// main window and a docked one).
#[cfg(target_os = "macos")]
pub fn ensure_clipbar_attached(app: &AppHandle, browser_label: &str) {
    // The attachment itself (including "stay out of the window menu" and "cannot be
    // dragged away") is the same thing as attaching a browser window to the main
    // window, so it shares window.rs's attachment primitive; only the parent-child
    // pair differs here
    super::window::set_child_window(app, browser_label, &clipbar_label_of(browser_label), true);
}

#[cfg(not(target_os = "macos"))]
pub fn ensure_clipbar_attached(_app: &AppHandle, _browser_label: &str) {}

/// Expand or collapse the toolbar's destination picker: the height changes anchored at
/// the bottom edge (growing upwards). The label comes from the `window` parameter (the
/// caller is the toolbar itself) and never from what the frontend passes.
#[tauri::command]
pub fn clipbar_set_expanded(
    app: AppHandle,
    window: tauri::WebviewWindow,
    expanded: bool,
) -> Result<(), String> {
    let label = window.label().to_string();
    if !label.starts_with("clipbar-") {
        return Err("only the clip toolbar can change this".into());
    }
    let app2 = app.clone();
    app.run_on_main_thread(move || {
        let Some(bar) = app2.get_webview_window(&label) else { return };
        let scale = bar.scale_factor().unwrap_or(1.0);
        let h = ((if expanded { CLIPBAR_EXPANDED_H } else { CLIPBAR_H }) * scale).round() as u32;
        let (Ok(pos), Ok(size)) = (bar.outer_position(), bar.outer_size()) else { return };
        let bottom = pos.y + size.height as i32;
        let _ = bar.set_size(tauri::PhysicalSize::new(size.width, h));
        let _ = bar.set_position(tauri::PhysicalPosition::new(pos.x, bottom - h as i32));
    })
    .map_err(|e| e.to_string())
}

/// The toolbar → main window action relay (the same as agent_window_to_main). `focus`
/// is true for "open the note" alone — every other action finishes inside the browser
/// window without interrupting the reading.
#[tauri::command]
pub fn clipbar_to_main(
    app: AppHandle,
    window: tauri::WebviewWindow,
    mut action: serde_json::Value,
    focus: bool,
) -> Result<(), String> {
    use tauri::Emitter;
    let origin = window.label().to_string();
    if !origin.starts_with("clipbar-") {
        return Err("only the clip toolbar can relay".into());
    }
    // The `label` carried by the three clip actions and by "summon the AI" means
    // "which browser window": it is stamped in from the calling window's own label
    // rather than taken from what the frontend passed (the same distrust as
    // clipbar_set_expanded).
    // Only actions that already carry the field are overwritten; no other action has
    // one conjured onto it.
    if let Some(obj) = action.as_object_mut() {
        if obj.contains_key("label") {
            obj.insert("label".into(), browser_label_of(&origin).into());
        }
    }
    app.emit_to("main", "clipbar://action", action).map_err(|e| e.to_string())?;
    if focus {
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.set_focus();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_pair_up() {
        assert_eq!(clipbar_label_of("browser-7"), "clipbar-7");
        assert_eq!(browser_label_of("clipbar-7"), "browser-7");
        assert_eq!(browser_label_of(&clipbar_label_of("browser-42")), "browser-42");
    }
}
