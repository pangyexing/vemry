//! The chrome bar of the detached browser window (`browserbar`): our own bundle,
//! mounted as a child webview at the top of `browserwin` (window.rs builds it). A
//! tab is a remote page with zero IPC, so the chrome cannot live inside it; in the
//! docked shape the chrome is the main window's DOM, and in the detached shape it
//! is this bar.
//!
//! The bar only displays and forwards: a clip action is relayed through
//! `browserbar_to_main` to be executed by the main window (delivery has to go
//! through the main window's editor buffer, and the record of where things land is
//! collected there too), and the receipt comes back through `browserbar://result`
//! to flash in place. "Not having to go back to the main window" is the bar's
//! entire point, so the relay does not raise the main window by default ("open the
//! note" being the one exception).

use super::window::{apply_layout, relay_to_main, BAR_LABEL};
use tauri::{AppHandle, State};

/// Grow or shrink the bar: while its destination picker is open it extends
/// downwards over the page; the label comes from the calling webview and never
/// from what the frontend passes.
#[tauri::command]
pub fn browserbar_set_expanded(
    app: AppHandle,
    state: State<'_, crate::AppState>,
    webview: tauri::Webview,
    expanded: bool,
) -> Result<(), String> {
    if webview.label() != BAR_LABEL {
        return Err("only the browser bar can change this".into());
    }
    state.browser.0.lock().unwrap().bar_expanded = expanded;
    apply_layout(&app);
    Ok(())
}

/// The bar → main window action relay. `focus` is true for "open the note" alone —
/// every other action finishes in the detached window without interrupting the reading.
#[tauri::command]
pub fn browserbar_to_main(
    app: AppHandle,
    webview: tauri::Webview,
    action: serde_json::Value,
    focus: bool,
) -> Result<(), String> {
    if webview.label() != BAR_LABEL {
        return Err("only the browser bar can relay".into());
    }
    relay_to_main(&app, "browserbar://action", action, focus)
}
