mod agent_bridge;
mod agent_window;
mod browser;
mod commands;
mod config_store;
mod i18n;
mod menu;
mod pasteboard;
mod script_bridge;
mod settings;

use std::sync::{Arc, Mutex};
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
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // In-app updates: the check is user-initiated by default (a setting can turn
        // on one silent check at startup), while downloading and installing always
        // need the user's confirmation; the installer is verified by a minisign
        // signature.
        // Rule 5 of the controlled network egress, see "Network egress list" in
        // docs/ARCHITECTURE.md.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        // The projection image proxy: the main webview never talks to a remote host
        // directly, and images are fetched by Rust through this protocol (SSRF
        // validation, no cookies, a disk cache)
        .register_asynchronous_uri_scheme_protocol("vemry-asset", browser::assets::handle)
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
        })
        .setup(|app| {
            let settings = settings::load(app.handle());
            // The language is settled first; the menu and the error wording both read it
            i18n::set_current(i18n::resolve(settings.language));
            // The browser's shape is the last session's choice: the registry starts
            // there, so the first tab already opens in it
            if !settings.browser_docked {
                tauri::Manager::state::<AppState>(app).browser.0.lock().unwrap().shape = browser::Shape::Detached;
            }
            // The native menu bar (macOS only): a menu item's id is a frontend command
            // id, routed back through the event bridge to executeCommand; a failure
            // does not block startup (every entry point exists in-app as well)
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
            // The docked browser tabs are native child views of the main window: on a
            // resize they are re-placed from the panel anchors (no round trip to the
            // frontend). `get_webview_window("main")` is None once the main window has a
            // child webview, so the window is reached through its webview
            if let Some(main) = browser::window::main_window(app.handle()) {
                let handle = app.handle().clone();
                main.on_window_event(move |event| match event {
                    tauri::WindowEvent::Resized(_) => browser::window::relayout(&handle),
                    // The main window is gone: the detached chat window should not outlive
                    // it (it would have nowhere to send anything back), and the same goes
                    // for the detached browser window (nowhere to relay to, every button
                    // mute, and it would hold up "closing every window quits")
                    tauri::WindowEvent::Destroyed => {
                        agent_window::force_close(&handle);
                        browser::window::close_all(&handle);
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
            commands::provenance_revert_plan,
            commands::provenance_record,
            commands::fields_list,
            commands::types_list,
            commands::type_create,
            commands::type_set_disabled,
            commands::settings_get,
            commands::settings_set,
            commands::agent_start,
            commands::agent_retry,
            commands::agent_history_truncate,
            commands::agent_permission_respond,
            commands::agent_plan_respond,
            commands::agent_cancel,
            commands::agent_follow_set,
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
            commands::agent_session_delete,
            commands::agent_session_rename,
            commands::agent_session_fork,
            commands::session_read,
            commands::init_memory_file,
            commands::memory_append,
            commands::init_research_skill,
            commands::ensure_research_skill,
            commands::skills_list,
            commands::skill_set_enabled,
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
            browser::adblock::adblock_update_rules,
            browser::adblock::adblock_import_rules
        ])
        .run(tauri::generate_context!())
        .expect("error while running vemry");
}
