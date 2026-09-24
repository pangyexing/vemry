mod account;
mod agent_bridge;
mod agent_window;
mod browser;
mod capture_window;
mod commands;
mod config_store;
mod i18n;
mod inbox;
mod menu;
mod pasteboard;
mod script_bridge;
mod settings;
mod translate;
mod tray;
mod type_templates;
mod window_geom;

use std::sync::{Arc, Mutex, MutexGuard};

/// A mutex of this crate's window and tray state, taken even when poisoned: what
/// it guards is a few flags a panicked holder cannot leave half-written.
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
use vemry_core::Vault;

/// Shared app state behind Tauri's managed state. The vault is Arc'd because
/// the agent runner, the script host and the fs watcher access it from their
/// own threads.
pub struct AppState {
    pub vault: Arc<Mutex<Option<Vault>>>,
    pub watcher: Mutex<Option<vemry_core::watcher::VaultWatcher>>,
    pub agent: agent_bridge::AgentState,
    /// Lazily spawned on the first vault open (needs an AppHandle for the
    /// UI bridge); lives for the rest of the app.
    pub scripts: Mutex<Option<vemry_script::ScriptHost>>,
    pub script_bridge: Arc<script_bridge::ScriptBridgeState>,
    /// The built-in browser's window registry (vault-independent, so it survives a vault switch).
    pub browser: browser::BrowserState,
    /// The reading projection's fetch state: the request generation of the hidden
    /// reader window plus the projection page cache.
    pub reader: browser::reader::ReaderState,
    /// A mirror of the ad-blocking state (the Send-safe side; the rule list itself
    /// lives in a main-thread thread_local).
    pub adblock: browser::adblock::AdblockState,
    /// Handoff, close handshake and geometry relay for the detached AI chat window
    /// (see agent_window.rs).
    pub agent_window: agent_window::AgentWindowState,
    /// The quick capture window: build, show/hide, the shortcut's registration and
    /// the receipt relay (see capture_window.rs).
    pub capture_window: capture_window::CaptureWindowState,
    /// The resident state's menu bar / system tray icon (see tray.rs).
    pub tray: tray::TrayState,
    /// The phone inbox's pull loop: what is in flight to the main window and
    /// what is waiting for a retry (see inbox.rs).
    pub inbox: inbox::InboxState,
    /// The one side-by-side translation job at a time (see translate.rs).
    pub translate: translate::TranslateState,
    /// The browser tabs being translated in place (see browser/pagetr.rs).
    pub browser_translate: browser::pagetr::PageTranslations,
    /// The quit handshake: ⌘Q, the main window's close button and the last window
    /// going all reach `ExitRequested`; the first request is refused and the main
    /// webview is asked to write its editor buffer out (`vemry://quit-requested`),
    /// and it answers with `quit_confirmed`, which exits for real. Set once the
    /// question has been asked, so a second request — the webview dead, or the
    /// user pressing again — is let through rather than refused forever.
    pub quit_asked: std::sync::atomic::AtomicBool,
}

pub fn run() {
    // Before any HTTP client exists (account, ad-block rule downloads, the image
    // proxy, the agent): see the note on the function
    vemry_agent::client::install_tls_provider();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // In-app updates: the check is user-initiated by default (a setting can turn
        // on one silent check at startup), while downloading and installing always
        // need the user's confirmation in the update dialog; the installer is
        // verified by a minisign signature.
        // Rule 5 of the controlled network egress, see "Network egress list" in
        // docs/ARCHITECTURE.md.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // The quick capture window's global shortcut (one combination, registered
        // from the settings in setup; capture_window.rs)
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        // The projection image proxy: the main webview never talks to a remote host
        // directly, and images are fetched by Rust through this protocol (SSRF
        // validation, no cookies, a disk cache)
        .register_asynchronous_uri_scheme_protocol("vemry-asset", browser::assets::handle)
        // The main window's page starting to load, at launch or on a reload: its event
        // listeners are gone, so the phone inbox stops handing items to it until the new
        // page reports ready (inbox.rs, "The ready gate")
        .on_page_load(|webview, payload| {
            use tauri::Manager;
            if webview.label() == "main" && matches!(payload.event(), tauri::webview::PageLoadEvent::Started) {
                inbox::page_reloading(webview.app_handle());
            }
        })
        .manage(AppState {
            vault: Arc::new(Mutex::new(None)),
            watcher: Mutex::new(None),
            agent: agent_bridge::AgentState::default(),
            scripts: Mutex::new(None),
            script_bridge: Arc::new(script_bridge::ScriptBridgeState::default()),
            browser: browser::BrowserState::default(),
            reader: browser::reader::ReaderState::default(),
            adblock: browser::adblock::AdblockState::default(),
            agent_window: agent_window::AgentWindowState::default(),
            capture_window: capture_window::CaptureWindowState::default(),
            tray: tray::TrayState::default(),
            inbox: inbox::InboxState::default(),
            translate: translate::TranslateState::default(),
            browser_translate: browser::pagetr::PageTranslations::default(),
            quit_asked: std::sync::atomic::AtomicBool::new(false),
        })
        .setup(|app| {
            let settings = settings::load(app.handle());
            // The language is settled first; the menu and the error wording both read it
            i18n::set_current(i18n::resolve(settings.language));
            // The main window starts hidden (tauri.conf): it is sized and placed for the
            // display it opens on — where it last was, or 85% of the display the first
            // time — and only then shown, so it never visibly jumps (window_geom.rs)
            window_geom::restore_main(app.handle());
            // The account file into its in-process copy: the hosted provider is built
            // from it on every request without a disk read
            account::warm(app.handle());
            // The browser's shape is the last session's choice: the registry starts
            // there, so the first tab already opens in it
            if !settings.browser_docked {
                tauri::Manager::state::<AppState>(app).browser.0.lock().unwrap().shape = browser::Shape::Detached;
            }
            // The one menu event listener (the application menu's and the tray
            // menu's items alike; menu.rs), then the native menu bar itself (macOS
            // only): a menu item's id is a frontend command id, routed back through
            // the event bridge to executeCommand; a failure does not block startup
            // (every entry point exists in-app as well)
            menu::listen(app.handle());
            #[cfg(target_os = "macos")]
            if let Err(e) = menu::install(app.handle()) {
                eprintln!("menu install failed: {e}");
            }
            // Ad-blocking rules are precompiled in the background at startup (ready in
            // seconds when the store's disk cache hits), so the rules are in hand
            // before a browser window is built and the first frame already blocks
            browser::adblock::reconcile(app.handle());
            // The reader window's heavy-resource blocking rules (image/media/font) are precompiled at startup too
            browser::adblock::ensure_reader_rules(app.handle());
            // Smoothing a discrete scroll wheel: a local NSEvent monitor classifies the
            // input device by hasPreciseScrollingDeltas and pushes that to the browser
            // window's injected script, which decides whether to take over the scroll
            // animation
            #[cfg(target_os = "macos")]
            browser::wheel::install_monitor(app.handle());
            // The browser's warm-up shell window: once the startup rush has passed, build
            // a hidden about:blank window so opening the first page does not pay the few
            // hundred milliseconds of building a window and cold-starting a WebKit
            // subprocess (the memory cost is roughly one blank-page WebContent process)
            browser::window::ensure_pool_soon(app.handle(), 3000);
            // The quick capture window: its global shortcut is registered now (a
            // failure is recorded for the settings page, never fatal), and the hidden
            // window itself is built once the startup rush has passed, so the first
            // press shows it as instantly as every later one
            capture_window::install_hotkey(app.handle());
            capture_window::ensure_soon(app.handle(), 2500);
            // The menu bar / tray icon of the resident state, when the setting asks for it
            tray::reconcile(app.handle());
            // The phone inbox: one long-poll loop for the life of the app, idle while
            // signed out; what it pulls lands through the main window (inbox.rs)
            inbox::start(app.handle());
            // The docked browser tabs are native child views of the main window: on a
            // resize they are re-placed from the panel anchors (no round trip to the
            // frontend). `get_webview_window("main")` is None once the main window has a
            // child webview, so the window is reached through its webview
            if let Some(main) = browser::window::main_window(app.handle()) {
                let handle = app.handle().clone();
                let win = main.clone();
                main.on_window_event(move |event| match event {
                    tauri::WindowEvent::Resized(_) => {
                        browser::window::relayout(&handle);
                        window_geom::record_main(&win);
                    }
                    tauri::WindowEvent::Moved(_) => window_geom::record_main(&win),
                    // The close button: in the resident state it only hides the window (the
                    // tray icon and the Dock bring it back; tray.rs); otherwise it is a quit —
                    // the other windows cannot outlive the main one — through the same
                    // handshake as ⌘Q, so the editor buffer is written out first
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        // Hidden or quitting, the geometry is written now (the exit below
                        // writes it again, for the quit that never passes through here)
                        window_geom::persist_main(&handle);
                        if tray::resident(&handle) {
                            api.prevent_close();
                            tray::hide_main(&handle);
                        } else if commands::ask_before_quit(&handle) {
                            api.prevent_close();
                        }
                    }
                    // The main window is gone: the detached chat window should not outlive
                    // it (it would have nowhere to send anything back), and the same goes
                    // for the detached browser window (nowhere to relay to, every button
                    // mute, and it would hold up "closing every window quits")
                    tauri::WindowEvent::Destroyed => {
                        agent_window::force_close(&handle);
                        browser::window::close_all(&handle);
                        capture_window::force_close(&handle);
                    }
                    _ => {}
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::vault_open,
            commands::vault_create,
            commands::vault_recent,
            commands::vault_recent_remove,
            commands::vault_last,
            commands::notes_list,
            commands::note_read,
            commands::note_write,
            commands::note_create,
            commands::note_insert,
            commands::note_history_list,
            commands::note_history_read,
            commands::note_history_restore,
            commands::quit_confirmed,
            commands::search,
            commands::links_get,
            commands::query_blocks,
            commands::link_targets,
            commands::folders_list,
            commands::folder_create,
            commands::files_list,
            commands::file_create,
            commands::file_read,
            commands::file_read_binary,
            commands::file_write,
            commands::attachment_save,
            commands::attachments_migrate,
            commands::clipboard_paste,
            commands::files_import,
            commands::clipboard_read_text,
            commands::clipboard_copy_image,
            commands::path_rename,
            commands::path_delete,
            commands::file_open_system,
            commands::url_open_system,
            commands::resolve_link,
            commands::resolve_ref,
            commands::block_ensure_id,
            commands::block_id_plan,
            commands::provenance_get,
            commands::provenance_accept,
            commands::provenance_pending_notes,
            commands::provenance_accept_notes,
            commands::provenance_revert_plan,
            commands::provenance_record,
            commands::fields_list,
            commands::types_list,
            commands::type_create,
            commands::type_set_disabled,
            commands::settings_get,
            commands::settings_set,
            account::account_status,
            account::account_login_start,
            account::account_login_verify,
            account::account_logout,
            account::account_refresh,
            account::account_redeem,
            inbox::inbox_ack,
            inbox::inbox_failed,
            inbox::inbox_ready,
            inbox::inbox_pair_start,
            inbox::inbox_doors,
            inbox::inbox_door_revoke,
            commands::agent_start,
            commands::agent_retry,
            commands::agent_history_truncate,
            commands::agent_permission_respond,
            commands::agent_plan_respond,
            commands::agent_cancel,
            commands::agent_follow_set,
            commands::agent_actions_respond,
            commands::agent_read_respond,
            commands::agent_undo_turn,
            commands::format_markdown,
            commands::html_to_markdown,
            commands::type_examples_list,
            commands::type_install_example,
            commands::agent_is_running,
            commands::agent_session_new,
            commands::agent_session_list,
            commands::agent_session_select,
            commands::agent_session_current,
            commands::vault_current,
            agent_window::agent_window_open,
            agent_window::agent_window_close,
            agent_window::agent_window_is_open,
            agent_window::agent_window_take_handoff,
            agent_window::agent_window_take_handback,
            agent_window::agent_window_set_note,
            agent_window::agent_window_set_selection,
            agent_window::agent_window_to_main,
            agent_window::agent_window_to_agent,
            capture_window::capture_window_show,
            capture_window::capture_window_hide,
            capture_window::capture_window_ready,
            capture_window::capture_window_resize,
            capture_window::capture_to_main,
            capture_window::capture_reply,
            commands::clipboard_probe,
            commands::clipboard_image,
            commands::agent_session_delete,
            commands::agent_session_rename,
            commands::agent_session_fork,
            commands::session_read,
            commands::init_memory_file,
            commands::memory_append,
            commands::skills_list,
            commands::skill_set_enabled,
            commands::skill_customize,
            commands::skill_restore,
            commands::skill_create,
            commands::vault_write_policy_get,
            commands::vault_write_policy_set,
            commands::privacy_list,
            commands::privacy_mark,
            commands::privacy_unmark,
            commands::privacy_rules,
            commands::privacy_rule_add,
            commands::privacy_rule_remove,
            commands::graph_get,
            commands::scripts_list,
            commands::scripts_reload,
            commands::script_set_enabled,
            commands::script_command_run,
            commands::script_grant_respond,
            commands::script_grant_revoke,
            commands::script_prompt_respond,
            commands::script_eval_block,
            browser::window::browser_open,
            browser::window::browser_new_tab,
            browser::window::browser_close_all,
            browser::window::browser_navigate,
            browser::window::browser_back,
            browser::window::browser_forward,
            browser::window::browser_reload,
            browser::window::browser_close_tab,
            browser::window::browser_activate,
            browser::window::browser_focus,
            browser::window::browser_state,
            browser::window::browser_set_docked,
            browser::window::browser_panel_layout,
            browser::window::browser_panel_veil,
            browser::window::browser_open_last,
            browser::window::browser_prefetch,
            browser::window::url_open,
            browser::history::browser_history_list,
            browser::history::browser_history_remove,
            browser::history::browser_history_clear,
            browser::bookmarks::bookmark_list,
            browser::bookmarks::bookmark_toggle,
            browser::bookmarks::bookmark_remove,
            browser::browserbar::browserbar_to_main,
            browser::browserbar::browserbar_set_expanded,
            browser::clip::clip_selection,
            browser::clip::clip_page,
            browser::clip::clip_localize_note,
            browser::clip::clip_link,
            browser::clip::clip_url,
            browser::clip::clips_lookup,
            browser::clip::browser_page_context,
            browser::reader::page_read,
            browser::reader::reading_clip,
            browser::reader::reading_quote,
            translate::translate_open,
            translate::translate_request,
            translate::translate_cancel,
            translate::translate_save,
            translate::translate_continue,
            browser::pagetr::browser_translate_toggle,
            browser::adblock::adblock_update_rules,
            browser::adblock::adblock_import_rules
        ])
        .build(tauri::generate_context!())
        .expect("error while building vemry")
        .run(|app, event| {
            // A quit by user interaction (⌘Q, the dock) carries no code; the exit the
            // handshake ends in (`quit_confirmed` → `app.exit`) and a restart carry one
            // and pass — a restart's prevent_exit would be ignored anyway
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = &event {
                if commands::ask_before_quit(app) {
                    api.prevent_exit();
                }
            }
            // The Dock icon clicked while the main window is hidden (the resident
            // state's close): the window comes back
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = &event {
                tray::reopen(app);
            }
            if let tauri::RunEvent::Exit = &event {
                window_geom::persist_main(app);
            }
        });
}
