//! The resident state — an application behaviour, not a capture one (`tray_resident`,
//! Settings › General, on by default): closing the main window hides it instead
//! of quitting, and a menu bar (system tray) icon stands for the app meanwhile;
//! its menu has quick capture, "Open vemry" and "Quit vemry". What one click on
//! it does follows the platform: on macOS it is the quick capture shortcut's twin
//! (a menu bar icon opens an application's light surface there, and the Dock is
//! what brings the window back); on Windows and Linux it brings the window back
//! (the tray is the hidden window's only way back, and a click restoring it is
//! the convention of those trays). With the switch off nothing here exists: no
//! icon, and the close button quits through the handshake as it always did.
//! Closing is silent, as it is in every menu bar application: no notice, no
//! dialog — the settings hint is where what closing means is written.
//!
//! **Hidden, not destroyed.** The quick capture window lands every capture
//! through the main window's `vault.capture` (see capture_window.rs — only the
//! main webview can tell an editor buffer from a note on disk). A main window
//! that is merely hidden keeps that relay working: its webview, its editor buffer
//! and its vault are all still there. So the resident state needs no second write
//! path in Rust, and the rule that the other windows cannot outlive the main one
//! is untouched — the main window lives until the app quits.
//!
//! **What still quits**: ⌘Q and the application menu's Quit (the `ExitRequested`
//! handshake in lib.rs) and the tray menu's own Quit, all through
//! `commands::ask_before_quit`, so the editor buffer is written out first. The
//! Dock (macOS) brings a hidden window back through `RunEvent::Reopen`; the tray
//! menu does everywhere.
//!
//! The icon is created by the runtime on the main thread. A failure (a desktop
//! without a tray) is recorded for the settings page, and the close button then
//! keeps quitting: a hidden window nobody can bring back must never happen.

use std::sync::Mutex;

use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::settings::Settings;
use crate::{lock, AppState};

const TRAY_ID: &str = "vemry";
/// The tray menu's item ids, distinct from the application menu's (a frontend
/// command id): the one menu event listener (menu.rs `listen`) hands these here.
const M_CAPTURE: &str = "tray:capture";
const M_OPEN: &str = "tray:open";
const M_QUIT: &str = "tray:quit";

#[derive(Default)]
pub struct TrayState {
    /// Why the icon could not be created (the settings page shows it). None: it
    /// exists, or the switch is off.
    error: Mutex<Option<String>>,
}

/// The creation failure, for the settings view.
pub fn error(app: &AppHandle) -> Option<String> {
    lock(&app.state::<AppState>().tray.error).clone()
}

/// A menu event for one of this menu's items; false when the id is not ours.
pub fn on_menu_event(app: &AppHandle, id: &str) -> bool {
    match id {
        M_CAPTURE => {
            let a = app.clone();
            let _ = app.run_on_main_thread(move || crate::capture_window::show(&a));
        }
        M_OPEN => show_main(app),
        M_QUIT => quit(app),
        _ => return false,
    }
    true
}

/// Whether closing the main window hides it: the switch is on *and* the icon is
/// there to bring the window back.
pub fn resident(app: &AppHandle) -> bool {
    crate::settings::load(app).tray_resident && app.tray_by_id(TRAY_ID).is_some()
}

/// Bring the icon in line with the setting: created when the switch is on and it
/// is missing, its menu rebuilt when it is there (the language or the shortcut
/// hint may have changed), removed when the switch is off. Called at startup and
/// after every settings write.
pub fn reconcile(app: &AppHandle) {
    let settings = crate::settings::load(app);
    let st = app.state::<AppState>();
    if !settings.tray_resident {
        let _ = app.remove_tray_by_id(TRAY_ID);
        *lock(&st.tray.error) = None;
        return;
    }
    let outcome = match app.tray_by_id(TRAY_ID) {
        Some(tray) => menu(app, &settings).and_then(|m| tray.set_menu(Some(m))),
        None => build(app, &settings),
    };
    if let Err(e) = &outcome {
        eprintln!("tray icon: {e}");
    }
    *lock(&st.tray.error) = outcome.err().map(|e| e.to_string());
}

fn build(app: &AppHandle, settings: &Settings) -> tauri::Result<()> {
    TrayIconBuilder::with_id(TRAY_ID)
        // The app icon in its own colours (not a template image the system would
        // flatten to black or white): the icon stands for vemry, and reads as it
        .icon(icon()?)
        .icon_as_template(false)
        .tooltip("vemry")
        .menu(&menu(app, settings)?)
        // A click acts (below); the menu is on the secondary button. Its events reach
        // `on_menu_event` through the process's one listener (menu.rs): a listener
        // registered here would be kept by the runtime past this icon's removal and
        // added to again on every rebuild
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                let app = tray.app_handle().clone();
                // The platform split of the module doc: the capture window here, the main window there
                #[cfg(target_os = "macos")]
                {
                    let a = app.clone();
                    let _ = app.run_on_main_thread(move || crate::capture_window::toggle(&a));
                }
                #[cfg(not(target_os = "macos"))]
                show_main(&app);
            }
        })
        .build(app)?;
    Ok(())
}

/// The app icon, in its colours, on every desktop: macOS gets an 18-point
/// rendering of it, the height the menu bar shows (scripts/gen-tray-icon.py);
/// Windows and Linux the 32 px app icon.
fn icon() -> tauri::Result<tauri::image::Image<'static>> {
    #[cfg(target_os = "macos")]
    {
        tauri::image::Image::from_bytes(include_bytes!("../icons/tray-mac.png"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))
    }
}

/// Quick capture (the File menu's item, shortcut hint included), open the window,
/// quit — from the catalog in the current language.
fn menu(app: &AppHandle, settings: &Settings) -> tauri::Result<Menu<tauri::Wry>> {
    use crate::i18n;
    let lang = i18n::resolve(settings.language);
    MenuBuilder::new(app)
        .item(&crate::menu::quick_capture_item(app, lang, M_CAPTURE, &settings.capture_hotkey)?)
        .item(&MenuItemBuilder::with_id(M_OPEN, i18n::t(lang, "menu.trayOpen")).build(app)?)
        .separator()
        .item(&MenuItemBuilder::with_id(M_QUIT, i18n::t(lang, "menu.trayQuit")).build(app)?)
        .build()
}

/// The close button in the resident state: the window goes away, everything in it stays.
pub fn hide_main(app: &AppHandle) {
    if let Some(w) = crate::browser::window::main_window(app) {
        let _ = w.hide();
    }
}

/// The tray menu's "Open vemry", and the Dock on macOS: the hidden window comes back, in front.
pub fn show_main(app: &AppHandle) {
    if let Some(w) = crate::browser::window::main_window(app) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// macOS: the Dock icon clicked with no window showing (`RunEvent::Reopen`).
#[cfg(target_os = "macos")]
pub fn reopen(app: &AppHandle) {
    if let Some(w) = crate::browser::window::main_window(app) {
        if !w.is_visible().unwrap_or(true) {
            show_main(app);
        }
    }
}

/// The tray menu's Quit: the same handshake as ⌘Q. `ask_before_quit` answers
/// false once the question has been asked (or when nobody is there to ask), and
/// then the exit is immediate.
fn quit(app: &AppHandle) {
    if !crate::commands::ask_before_quit(app) {
        app.exit(0);
    }
}
