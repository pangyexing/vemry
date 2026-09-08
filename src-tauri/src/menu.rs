//! The native macOS menu bar: the conventional entry point for settings, new,
//! search, view and the rest (⌘, sits in the application menu — a user who does
//! not know ⌘P can still find it). **A menu item's id is a command id from the
//! frontend registry**, and a click travels through the `menu://command` event
//! bridge back to the frontend's executeCommand — the menu never grows a second
//! set of semantics; the wording and the frontend command titles come from the
//! same `locales/*.json` (`cmd.<id>.title`) rather than being maintained twice by
//! hand.
//!
//! Accelerators fall into three classes (paired with the isMac gate on
//! App.svelte's keydown chain):
//! 1. Menu-only (the accelerator is set here, and the matching frontend branch
//!    gets a !isMac gate so it cannot fire twice): ⌘, ⌘N ⌘O ⌘⇧F ⌘⇧C ⌘⇧S — the
//!    application-level combinations WKWebView and CodeMirror do not claim;
//!    ⌘W belongs here too (a special case in Rust that never reaches the
//!    frontend, which has no binding for it; the chat window's frontend ⌘W
//!    serves non-mac alone — on mac the menu takes it first, and the special case
//!    puts the agent through the same close() handshake);
//! 2. Frontend-only (the menu item sets no accelerator but is still clickable):
//!    ⌘P ⌘L ⌘B ⌘. ⌘⌥←/→ — ⌘B may be claimed by WKWebView's contenteditable, so
//!    handling it in the frontend is the most reliable;
//! 3. Predefined-only: the edit menu's undo/redo/cut/copy/paste/selectAll go
//!    through the native selector straight to the first responder (which is how
//!    CodeMirror receives them) and must never be turned into a custom emit —
//!    that would take ⌘Z/⌘C away from the editor.
//!
//! No window menu is installed on Windows/Linux: the frontend keydown chain
//! remains the single source of truth for keys off mac, which avoids two
//! behaviour matrices for two platforms (the whole module is cfg macos and
//! compiles to nothing elsewhere).

#[cfg(target_os = "macos")]
pub use imp::install;

/// No-op off macOS, where there is no window menu at all: callers (startup and
/// the language switch in `settings_set`) stay cfg-free.
#[cfg(not(target_os = "macos"))]
pub fn install(_app: &tauri::AppHandle) -> tauri::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
mod imp {
    use tauri::menu::{AboutMetadata, MenuBuilder, MenuItem, MenuItemBuilder, SubmenuBuilder};
    use tauri::{AppHandle, Emitter};

    use crate::i18n::{self, Lang};

    /// The help menu's special-case id: not a registry command, it opens the system browser directly.
    const HELP_GITHUB: &str = "menu:help-github";
    const HELP_URL: &str = "https://github.com/pangyexing/vemry";
    /// The ⌘W special-case id: it closes the browser tab that holds the keyboard
    /// focus, or the chat window, and closes the current editor tab in the main
    /// window — "clip it and ⌘W away" is the closing gesture of the clipping loop,
    /// and the user should not be forced to aim at the red light.
    const CLOSE_WINDOW: &str = "menu:close-window";
    /// The browser's keyboard entry points are menu accelerators (class 1): a page
    /// is a native webview that swallows every key while it has the focus, so the
    /// frontend's keydown chain never sees ⌘L/⌘T/⌘R there. With the detached browser
    /// window focused the command goes to its chrome bar, otherwise to the main
    /// window's registry like every other menu item.
    const BROWSER_BAR_COMMANDS: [&str; 2] = ["open-url", "browser-new-tab"];

    /// `key` is a `locales/*.json` key; menu items that mirror a palette command
    /// pass that command's own `cmd.<id>.title` so the two can never disagree.
    fn item(app: &AppHandle, lang: Lang, id: &str, key: &str) -> tauri::Result<MenuItem<tauri::Wry>> {
        MenuItemBuilder::with_id(id, i18n::t(lang, key)).build(app)
    }
    fn item_key(
        app: &AppHandle,
        lang: Lang,
        id: &str,
        key: &str,
        accel: &str,
    ) -> tauri::Result<MenuItem<tauri::Wry>> {
        MenuItemBuilder::with_id(id, i18n::t(lang, key)).accelerator(accel).build(app)
    }

    /// Builds (or rebuilds) the whole menu. Called again when the language
    /// changes — `set_menu` replaces the previous one wholesale.
    pub fn install(app: &AppHandle) -> tauri::Result<()> {
        let lang = i18n::resolve(crate::settings::load(app).language);
        let cmd = |id: &str| format!("cmd.{id}.title");
        let app_menu = SubmenuBuilder::new(app, "vemry")
            .about(Some(AboutMetadata::default()))
            .separator()
            .item(&item_key(app, lang, "settings", "menu.settings", "Cmd+,")?)
            .item(&item(app, lang, "check-updates", "menu.checkUpdates")?)
            .separator()
            .hide()
            .hide_others()
            .show_all()
            .separator()
            .quit()
            .build()?;

        let file = SubmenuBuilder::new(app, i18n::t(lang, "menu.file"))
            .item(&item_key(app, lang, "new-note", &cmd("new-note"), "Cmd+N")?)
            .item(&item_key(app, lang, "open-note", &cmd("open-note"), "Cmd+O")?)
            .separator()
            .item(&item_key(app, lang, "quick-capture", "menu.quickCapture", "Cmd+Shift+C")?)
            .separator()
            .item(&item(app, lang, "create-vault", &cmd("create-vault"))?)
            .item(&item(app, lang, "open-vault", &cmd("open-vault"))?)
            .item(&item(app, lang, "switch-vault", &cmd("switch-vault"))?)
            .build()?;

        let edit = SubmenuBuilder::new(app, i18n::t(lang, "menu.edit"))
            .undo()
            .redo()
            .separator()
            .cut()
            .copy()
            .paste()
            .select_all()
            .separator()
            .item(&item_key(app, lang, "search", &cmd("search"), "Cmd+Shift+F")?)
            .build()?;

        let view = SubmenuBuilder::new(app, i18n::t(lang, "menu.view"))
            .item(&item(app, lang, "view-agenda", &cmd("view-agenda"))?)
            .item(&item(app, lang, "view-kanban", &cmd("view-kanban"))?)
            .item(&item(app, lang, "view-timeline", &cmd("view-timeline"))?)
            .item(&item(app, lang, "view-claims", &cmd("view-claims"))?)
            .item(&item(app, lang, "view-evidence", &cmd("view-evidence"))?)
            .item(&item(app, lang, "view-graph", &cmd("view-graph"))?)
            .item(&item(app, lang, "view-sessions", &cmd("view-sessions"))?)
            .item(&item(app, lang, "view-review", &cmd("view-review"))?)
            .item(&item(app, lang, "view-reading", &cmd("view-reading"))?)
            .item(&item_key(app, lang, "translate-toggle", &cmd("translate-toggle"), "Cmd+Shift+T")?)
            .separator()
            .item(&item(app, lang, "toggle-left", &cmd("toggle-left"))?)
            .item(&item(app, lang, "toggle-right", &cmd("toggle-right"))?)
            .item(&item(app, lang, "toggle-sidebar-side", &cmd("toggle-sidebar-side"))?)
            .separator()
            .item(&item(app, lang, "show-agent", &cmd("show-agent"))?)
            .item(&item(app, lang, "show-outline", &cmd("show-outline"))?)
            .build()?;

        let research = SubmenuBuilder::new(app, i18n::t(lang, "menu.research"))
            .item(&item(app, lang, "focus-agent", &cmd("focus-agent"))?) // ⌘J is frontend-only (class 2 above), so the menu item sets no accelerator
            .item(&item(app, lang, "clear-agent", &cmd("clear-agent"))?)
            .separator()
            .item(&item(app, lang, "open-browser", &cmd("open-browser"))?)
            .item(&item_key(app, lang, "open-url", &cmd("open-url"), "Cmd+L")?)
            .item(&item_key(app, lang, "browser-new-tab", &cmd("browser-new-tab"), "Cmd+T")?)
            .item(&item_key(app, lang, "browser-reload", &cmd("browser-reload"), "Cmd+R")?)
            .separator()
            .item(&item_key(app, lang, "clip-page", &cmd("clip-page"), "Cmd+Shift+S")?)
            .item(&item(app, lang, "clip-link", &cmd("clip-link"))?)
            .separator()
            .item(&item(app, lang, "open-memory", &cmd("open-memory"))?)
            .item(&item(app, lang, "init-memory", &cmd("init-memory"))?)
            .build()?;

        let window = SubmenuBuilder::new(app, i18n::t(lang, "menu.window"))
            .minimize()
            .maximize()
            .fullscreen()
            .separator()
            .item(&item_key(app, lang, CLOSE_WINDOW, "menu.closeWindow", "Cmd+W")?)
            .build()?;

        let help = SubmenuBuilder::new(app, i18n::t(lang, "menu.help")).item(&item(app, lang, HELP_GITHUB, "menu.helpGithub")?).build()?;

        let menu = MenuBuilder::new(app)
            .items(&[&app_menu, &file, &edit, &view, &research, &window, &help])
            .build()?;
        app.set_menu(menu)?;
        app.on_menu_event(|app, e| {
            let id = e.id().0.clone();
            if id == HELP_GITHUB {
                // An explicit click by the user, opened in the system browser (the same exit as url_open_system)
                let _ = opener::open(HELP_URL);
            } else if id == CLOSE_WINDOW {
                close_focused_window(app);
            } else if BROWSER_BAR_COMMANDS.contains(&id.as_str()) && browser_window_focused(app) {
                let _ = app.emit_to(crate::browser::window::BAR_LABEL, "browserbar://command", id);
            } else {
                // The payload convention: a string is a registry command id (a
                // Predefined item goes through the native selector and never reaches here)
                let _ = app.emit("menu://command", id);
            }
        });
        Ok(())
    }

    /// The window holding the keyboard focus (`windows()`, not `webview_windows()`:
    /// the main window stops being a WebviewWindow once a browser tab is its child).
    fn focused_window(app: &AppHandle) -> Option<String> {
        use tauri::Manager;
        app.windows().into_iter().find(|(_, w)| w.is_focused().unwrap_or(false)).map(|(l, _)| l)
    }

    fn browser_window_focused(app: &AppHandle) -> bool {
        focused_window(app).as_deref() == Some(crate::browser::window::DETACHED_LABEL)
    }

    /// ⌘W: dispatched by where the keyboard focus is — inside a browser tab (the
    /// detached window, or a docked page holding the first responder) it closes that
    /// tab; the chat window goes through close(), which triggers the existing
    /// CloseRequested handshake; the main window otherwise closes the current editor
    /// tab — bridged through menu://command back to the frontend registry's
    /// close-tab command, a no-op in the frontend when there is no tab, and the main
    /// window itself is never closed by ⌘W.
    fn close_focused_window(app: &AppHandle) {
        use tauri::Manager;
        let Some(label) = focused_window(app) else { return };
        if label == crate::browser::window::DETACHED_LABEL {
            crate::browser::window::close_active_tab(app);
        } else if label == "agent" {
            if let Some(w) = app.get_webview_window("agent") {
                let _ = w.close();
            }
        } else if label == "main" {
            if crate::browser::window::focused_tab(app).is_some() {
                crate::browser::window::close_active_tab(app);
            } else {
                let _ = app.emit("menu://command", "close-tab");
            }
        }
    }
}
