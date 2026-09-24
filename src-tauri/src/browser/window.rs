//! The browser surface: tabs as child webviews, the docked panel geometry, the
//! detached window, the warm-up shell and the navigation commands. See mod.rs for
//! the shape model.
//!
//! Threading: building a webview and creating a window go through the main thread
//! (on macOS the WKWebViewConfiguration with the ad-blocking rules is !Send and has
//! to be made there). tauri-runtime-wry handles a message sent from the main thread
//! inline, so `add_child`, `reparent` and `WindowBuilder::build` are safe both on
//! the main thread (inside run_on_main_thread) and from a command thread; the
//! bounds, hide and show messages are fire-and-forget from anywhere.

use super::{emit, BrowserEntry, BrowserEvent, BrowserInner, PanelLayout, Shape};
use crate::AppState;
use serde::Serialize;
use tauri::webview::{NewWindowResponse, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, State, Url, WebviewUrl, WindowEvent};

/// The standalone window of the detached shape (no webview of its own: the tabs and
/// the chrome bar are its children).
pub const DETACHED_LABEL: &str = "browserwin";
/// The chrome bar at the top of the detached window: our own bundle, authorised by
/// label through capabilities/browserbar.json.
pub const BAR_LABEL: &str = "browserbar";
/// The chrome bar's height (logical px): a tab strip row and an address row.
pub const BAR_H: f64 = 72.0;
/// The bar grown downwards over the page while its destination picker is open.
pub const BAR_EXPANDED_H: f64 = 360.0;

/// The main window: `get_webview_window("main")` is None as soon as the main window
/// has a child webview (a WebviewWindow is a window whose only webview carries its
/// own label), so every "the main window" access goes through here.
pub(crate) fn main_window(app: &AppHandle) -> Option<tauri::Window> {
    app.get_webview("main").map(|w| w.window())
}

/// An action relayed from a secondary window's webview (the chat window, the browser
/// bar) to the main window's frontend; with `focus` the main window is also brought
/// to the front (for a navigation action the user wants to see the result).
pub(crate) fn relay_to_main(app: &AppHandle, event: &str, action: serde_json::Value, focus: bool) -> Result<(), String> {
    use tauri::Emitter;
    app.emit_to("main", event, action).map_err(|e| e.to_string())?;
    if focus {
        if let Some(w) = main_window(app) {
            let _ = w.set_focus();
        }
    }
    Ok(())
}

/// The navigation guard's verdict, plus the bookkeeping for an allowed navigation:
/// the registry's URL, the Navigated event and the history record. Shared by the
/// builder hook and the SPA backstop in the title hook — both have to record the
/// visit the same way.
fn note_navigation(app: &AppHandle, label: &str, url: &Url) -> bool {
    // Only http(s), and never the app's own origin: a remote page must not navigate
    // itself into the app's bundle (where it would still be zero IPC, but it should
    // not even be shown)
    if !matches!(url.scheme(), "http" | "https") || is_app_origin(url) {
        return false;
    }
    let was_blank = update_entry(app, label, |e| std::mem::replace(&mut e.url, url.to_string()) == BLANK);
    emit(app, &BrowserEvent::Navigated { label: label.to_string(), url: url.to_string() });
    super::history::record_visit(app, url.as_str());
    // A blank tab is kept hidden under the panel's new-tab page; the first real
    // navigation brings the view out. Any later navigation changes no tab's
    // visibility, so the tabs are left where they are
    if was_blank == Some(true) {
        apply_layout(app);
    }
    true
}

/// The blank page a new tab starts on (the shell's own page): never recorded, never shown.
pub const BLANK: &str = "about:blank";

/// Turn user input into a URL (the one rule lives in vemry-core's weburl: the clip
/// reverse lookup and the agent's clips_lookup tool compare by the same spelling).
pub use vemry_core::normalize_url;

/// The app's own origin (the dev server, the bundled scheme): never shown as a page.
pub(crate) fn is_app_origin(url: &Url) -> bool {
    match url.scheme() {
        "http" | "https" => {
            let host = url.host_str().unwrap_or_default();
            (host == "localhost" || host == "127.0.0.1" || host == "[::1]") && url.port() == Some(1420)
                || host.ends_with(".localhost")
        }
        _ => true, // tauri:// and everything else that is not http(s) counts as the app side and is refused
    }
}

/// A snapshot entry = label plus the registry entry verbatim (serde flatten).
#[derive(Serialize)]
pub struct BrowserTabView {
    pub label: String,
    #[serde(flatten)]
    pub entry: BrowserEntry,
}

#[derive(Serialize)]
pub struct BrowserSnapshot {
    pub tabs: Vec<BrowserTabView>,
    pub active: Option<String>,
    pub shape: Shape,
}

// ---------------------------------------------------------------------------
// opening
// ---------------------------------------------------------------------------

/// Open a URL: navigate the active tab in place, or (`new_tab`, or no tab yet) open
/// a new tab. Returns the tab's label.
/// `focus=false` is for cases like "opened from a citation": the page appears
/// without taking the keyboard focus, so the user can stay in the conversation.
#[tauri::command]
pub async fn browser_open(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    new_tab: Option<bool>,
    focus: Option<bool>,
) -> Result<String, String> {
    let url = normalize_url(&url)?;
    // The same verdict as on_navigation, refused early: the app's own origin passes
    // normalize_url but would be stopped by the navigation guard — without refusing
    // it here, the registry and the chrome would record a URL the tab never showed
    if is_app_origin(&url) {
        return Err(format!("that address may not be opened: {url}"));
    }
    // Resolving first: it runs alongside the claim, so by the time the navigation really starts the DNS answer is most likely in the system cache
    prefetch_dns(&url);
    super::adblock::ensure_rules_soon(&app);
    let focus = focus.unwrap_or(true);

    // With an active tab and no request for a new one: navigate in place
    if !new_tab.unwrap_or(false) {
        let target = state.browser.0.lock().unwrap().target(None);
        if let Some(label) = target {
            if let Some(webview) = app.get_webview(&label) {
                webview.navigate(url.clone()).map_err(|e| e.to_string())?;
                activate_on_main(&app, &label, focus);
                // The URL record and the Navigated event are emitted together by
                // on_navigation when the navigation is allowed; nothing is written
                // to the registry ahead of time
                return Ok(label);
            }
        }
    }

    open_on_main(&app, Some(url), focus).await
}

/// Open a tab through the warm-up shell on the main thread and wait for its label.
/// Building and claiming happen on the main thread as a whole: on macOS the
/// WKWebViewConfiguration (!Send) has to have the adblock rules attached before
/// the webview is built, so the first frame already blocks.
async fn open_on_main(app: &AppHandle, url: Option<Url>, focus: bool) -> Result<String, String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    {
        let app2 = app.clone();
        app.run_on_main_thread(move || {
            let _ = tx.send(open_in_shell(&app2, url.as_ref(), focus));
        })
        .map_err(|e| e.to_string())?;
    }
    let result = rx.await.map_err(|_| "tab creation was interrupted".to_string())?;
    // Queue the next warm-up shell, on success and failure alike — the failure path
    // consumed the shell too, and without refilling, one transient failure would
    // leave every tab this session opening on the cold path
    ensure_pool_soon(app, 1500);
    result
}

/// Open a blank tab (the tab strip's `+`, ⌘T): the tab exists at once, its page stays
/// hidden under the panel's new-tab page (an address input, bookmarks, recents), and
/// the first navigation — from that page, ⌘L or a link — fills it. Returns the label.
#[tauri::command]
pub async fn browser_new_tab(app: AppHandle) -> Result<String, String> {
    open_on_main(&app, None, false).await
}

/// Close every tab — the panel's × (and the detached window with it). Reopening is
/// ⌘L or "open browser", which reopens the last page from the history.
#[tauri::command]
pub fn browser_close_all(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    // Every tab leaves the registry in one step: closing them one at a time would
    // place, announce and focus each successor only to close it next
    let (labels, detached) = {
        let mut inner = state.browser.0.lock().unwrap();
        let mut v: Vec<(u32, String)> = inner.tabs.iter().map(|(l, e)| (e.id, l.clone())).collect();
        v.sort();
        let labels: Vec<String> = v.into_iter().map(|(_, l)| l).collect();
        for label in &labels {
            forget_tab(&mut inner, label);
        }
        (labels, inner.shape == Shape::Detached)
    };
    if labels.is_empty() {
        return Ok(());
    }
    for label in labels {
        if let Some(wv) = app.get_webview(&label) {
            let _ = wv.close();
        }
        emit(&app, &BrowserEvent::Closed { label });
    }
    after_last_tab(&app, detached);
    Ok(())
}

/// Main thread: open a new tab — claiming the warm-up shell first (a hidden
/// about:blank child webview whose WKWebView, subprocess and ad-blocking rules are
/// already built, so opening is left with navigating and showing). With no shell
/// it takes the cold path and builds one. Returns the tab's label.
fn open_in_shell(app: &AppHandle, url: Option<&Url>, focus: bool) -> Result<String, String> {
    let state = app.state::<AppState>();
    let pooled = state.browser.0.lock().unwrap().pool.take();
    let (label, id) = match pooled.filter(|(l, _)| app.get_webview(l).is_some()) {
        Some((label, id)) => (label, id),
        None => {
            let (label, id) = alloc_label(&state);
            build_tab(app, &label)?;
            (label, id)
        }
    };
    claim_tab(app, id, &label, url, focus)?;
    Ok(label)
}

fn alloc_label(state: &State<'_, AppState>) -> (String, u32) {
    let mut inner = state.browser.0.lock().unwrap();
    inner.next_id += 1;
    (format!("browser-{}", inner.next_id), inner.next_id)
}

/// The single implementation of "open the browser" (shared by the menu bar, the ⌘P
/// command and an empty ⌘L return): with a tab open → bring it out and focus it;
/// with none → reopen the most recent page from the history; with neither →
/// Ok(None) and the frontend shows a toast.
#[tauri::command]
pub async fn browser_open_last(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let existing = state.browser.0.lock().unwrap().target(None);
    if let Some(label) = existing {
        activate_on_main(&app, &label, true);
        return Ok(Some(label));
    }
    match super::history::last_url(&app) {
        Some(url) => browser_open(app, state, url, Some(false), Some(true)).await.map(Some),
        None => Ok(None),
    }
}

/// Main thread: build a hidden tab webview (about:blank) as a child of the main
/// window — the navigation guard, the ad-blocking rules, the injected scripts and
/// the gestures are all in place, but it does not enter the registry and is not
/// shown. Registering, navigating, placing and showing happen in claim_tab (the
/// cold path claims it immediately; the warm-up path builds it in advance).
fn build_tab(app: &AppHandle, label: &str) -> Result<(), String> {
    let main = main_window(app).ok_or_else(|| "the main window is gone".to_string())?;
    let nav_app = app.clone();
    let nav_label = label.to_string();
    let title_app = app.clone();
    let new_win_app = app.clone();
    let new_win_label = label.to_string();

    let blank = Url::parse("about:blank").expect("about:blank always parses");
    // Both `builder =` reassignments below are inside cfg(macos) blocks, so off mac the mut is unused
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut builder = WebviewBuilder::new(label, WebviewUrl::External(blank))
        .focused(false)
        // The selection cache: after a page popup or script clears the selection, clipping can still fall back to the most recent one
        .initialization_script(super::clip::SELECTION_CACHE_JS)
        .on_navigation(move |target| {
            // The shell's initial blank page (and a remote page navigating itself back
            // to blank): allowed but not recorded — it is not a real visit
            if target.as_str() == "about:blank" {
                return true;
            }
            note_navigation(&nav_app, &nav_label, target)
        })
        .on_new_window(move |target, _features| {
            // Popup suppression: target=_blank and window.open all open in the same tab
            if matches!(target.scheme(), "http" | "https") && !is_app_origin(&target) {
                if let Some(w) = new_win_app.get_webview(&new_win_label) {
                    let _ = w.navigate(target);
                }
            }
            NewWindowResponse::Deny
        })
        .on_document_title_changed(move |webview, title| {
            let label = webview.label().to_string();
            // The SPA backstop: a pushState navigation does not trigger on_navigation,
            // and the registry's URL would be left on the old page (the chrome and the
            // bookmark ★ lighting up wrongly, a page missing from the history). An SPA
            // page change almost always changes the title — so the webview's real URL
            // is read back at this moment, and when it differs from the registry a
            // Navigated plus a history record follow (guarded by the same verdict as
            // on_navigation)
            if let Ok(current) = webview.url() {
                let known = update_entry(&title_app, &label, |e| e.url.clone());
                if known.is_some_and(|u| u != current.as_str()) {
                    note_navigation(&title_app, &label, &current);
                }
            }
            // url and title arrive a beat apart: the history entry is located by
            // label → the registry's url to amend the title
            let url = update_entry(&title_app, &label, |e| {
                e.title = title.clone();
                e.url.clone()
            });
            if let Some(url) = url {
                super::history::amend_title(&title_app, &url, &title);
            }
            // The detached window's title follows its active tab, so it can be told apart in ⌘Tab and the window menu
            let (detached, active) = title_app
                .try_state::<AppState>()
                .map(|s| {
                    let inner = s.browser.0.lock().unwrap();
                    (inner.shape == Shape::Detached, inner.active.clone())
                })
                .unwrap_or((false, None));
            if detached && active.as_deref() == Some(label.as_str()) {
                sync_detached_title(&title_app, &title);
            }
            emit(&title_app, &BrowserEvent::TitleChanged { label, title });
        });

    // Smoothing a discrete scroll wheel (macOS only: a WKWebView's wheel jumps
    // instantly, while WebView2 smooths it itself). The script is dormant by default
    // and woken by wheel.rs's device classification
    #[cfg(target_os = "macos")]
    {
        builder = builder.initialization_script(super::wheel::SMOOTH_WHEEL_JS);
    }

    // With the rules compiled and the switch on → they are attached to the
    // configuration before the webview is built, so no ad slips through the first
    // frame. Should the switch or the rules change after the shell was built,
    // rebuild_pool_if_present rebuilds it.
    #[cfg(target_os = "macos")]
    if crate::settings::load(app).adblock_enabled {
        if let Some(cfg) = super::adblock::webview_configuration_with_rules() {
            builder = builder.with_webview_configuration(cfg);
        }
    }

    // Built tiny and hidden: claim_tab places it. A child webview covers the DOM
    // wherever it is, so it must never be visible before it has its real bounds
    let webview = main
        .add_child(builder, LogicalPosition::new(0.0, 0.0), LogicalSize::new(10.0, 10.0))
        .map_err(|e| e.to_string())?;
    let _ = webview.hide();

    #[cfg(target_os = "macos")]
    {
        use objc2_web_kit::WKWebView;
        let app2 = app.clone();
        let label2 = label.to_string();
        let _ = webview.with_webview(move |pw| {
            let raw = pw.inner();
            // The native view pointer, for the menu bar's "is the keyboard focus inside a page" check
            if let Some(s) = app2.try_state::<AppState>() {
                s.browser.0.lock().unwrap().views.insert(label2, raw as usize);
            }
            // A two-finger swipe is WKWebView's native back and forward: a tab has zero
            // IPC, so the native gesture is the only navigation the page carries on its own
            let wk = raw.cast::<WKWebView>();
            if let Some(wk) = unsafe { wk.as_ref() } {
                unsafe { wk.setAllowsBackForwardNavigationGestures(true) };
            }
        });
    }
    Ok(())
}

/// Main thread: claim a shell as a real tab — registration, navigation (none for a
/// blank tab: the shell already shows the blank page), the move into the detached
/// window when that is the shape, placement, showing.
fn claim_tab(app: &AppHandle, id: u32, label: &str, url: Option<&Url>, focus: bool) -> Result<(), String> {
    let webview = app.get_webview(label).ok_or_else(|| "the tab was destroyed".to_string())?;
    let url_str = url.map(|u| u.to_string()).unwrap_or_else(|| BLANK.to_string());
    let Some(s) = app.try_state::<AppState>() else { return Err("no state".into()) };
    let detached = s.browser.0.lock().unwrap().shape == Shape::Detached;
    // The detached window comes first: a window that fails to build must not leave a
    // registered tab that no chrome can reach
    let win = if detached {
        match ensure_detached_window(app) {
            Ok(win) => Some(win),
            Err(e) => {
                s.browser.0.lock().unwrap().views.remove(label);
                let _ = webview.close();
                return Err(e);
            }
        }
    } else {
        None
    };
    {
        let mut inner = s.browser.0.lock().unwrap();
        inner.tabs.insert(
            label.to_string(),
            BrowserEntry { id, url: url_str.clone(), title: String::new() },
        );
        inner.active = Some(label.to_string());
    }
    // Registered before navigating: the update_entry/Navigated that on_navigation
    // performs when it allows the navigation has to find the registry entry
    if let Some(url) = url {
        if let Err(e) = webview.navigate(url.clone()) {
            // A tab that cannot even start navigating is rare but must not be left: hidden plus registered = a ghost entry
            {
                let mut inner = s.browser.0.lock().unwrap();
                forget_tab(&mut inner, label);
                inner.active = inner.tabs.iter().max_by_key(|(_, e)| e.id).map(|(l, _)| l.clone());
            }
            let _ = webview.close();
            return Err(e.to_string());
        }
    }
    emit(app, &BrowserEvent::Opened { label: label.to_string(), url: url_str });
    if let Some(win) = win {
        // The tab was built as a child of the main window (the shell always is) and moves into the detached window now
        let _ = webview.reparent(&win);
        raise_bar(app);
    }
    activate(app, label, focus);
    Ok(())
}

// ---------------------------------------------------------------------------
// the warm-up shell
// ---------------------------------------------------------------------------

/// Refill the warm-up shell: the build is delayed to avoid the main thread's busy
/// stretches at startup and during a page load. lib.rs's startup and each claim
/// queue one.
pub fn ensure_pool_soon(app: &AppHandle, delay_ms: u64) {
    super::on_main_after(app, delay_ms, ensure_pool);
}

/// Main thread: make sure one warm-up shell is standing by. Idempotent — every call
/// is serialised through the main thread, and an occupied slot is a no-op. A failure
/// is only logged (the next browser_open takes the cold path, and nothing breaks).
fn ensure_pool(app: &AppHandle) {
    // The main window is destroyed (we are quitting): a refill timer arriving late must not build anything
    if main_window(app).is_none() {
        return;
    }
    let Some(s) = app.try_state::<AppState>() else { return };
    if s.browser.0.lock().unwrap().pool.is_some() {
        return;
    }
    let (label, id) = alloc_label(&s);
    match build_tab(app, &label) {
        Ok(()) => s.browser.0.lock().unwrap().pool = Some((label, id)),
        Err(e) => eprintln!("browser pool prewarm failed: {e}"),
    }
}

/// Rebuild the warm-up shell after the ad-blocking rules roll over or the switch is
/// toggled: a shell's rule list is fixed into its configuration at build time, and
/// only a rebuild picks up the current configuration (a runtime attachment can only
/// add, never unload). With no shell it is a no-op.
pub fn rebuild_pool_if_present(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if take_pool_webview(&app2).is_none() {
            return;
        }
        ensure_pool(&app2);
    });
}

/// Take the warm-up shell and close it; returns whether there really was one.
fn take_pool_webview(app: &AppHandle) -> Option<()> {
    let s = app.try_state::<AppState>()?;
    let taken = s.browser.0.lock().unwrap().pool.take();
    let (label, _) = taken?;
    s.browser.0.lock().unwrap().views.remove(&label);
    if let Some(w) = app.get_webview(&label) {
        let _ = w.close();
    }
    Some(())
}

/// The detached window goes when the main window is destroyed (as does the
/// detached chat window): it would outlive it with nowhere to relay to, and hold
/// up "closing every window quits". The tabs that are children of the main window
/// die with it.
pub fn close_all(app: &AppHandle) {
    if let Some(s) = app.try_state::<AppState>() {
        let mut inner = s.browser.0.lock().unwrap();
        inner.tabs.clear();
        inner.views.clear();
        inner.active = None;
        inner.pool = None;
    }
    if let Some(w) = app.get_window(DETACHED_LABEL) {
        let _ = w.close();
    }
}

// ---------------------------------------------------------------------------
// DNS prefetch
// ---------------------------------------------------------------------------

/// DNS prefetch: push the host name resolution into the system cache (on macOS
/// getaddrinfo goes through mDNSResponder and is shared system-wide), so WebKit's
/// network process skips the resolution step on the real connection that follows.
/// Only hosts that "look like a domain" (containing a dot) are taken — a half-typed
/// ⌘L input with a scheme filled in (`exam`, say) must not leak to DNS. The same
/// host is deduplicated within 30s.
fn prefetch_dns(url: &Url) {
    use std::net::ToSocketAddrs;
    use std::sync::LazyLock;
    use std::time::{Duration, Instant};
    static RECENT: LazyLock<std::sync::Mutex<std::collections::HashMap<String, Instant>>> =
        LazyLock::new(Default::default);
    let Some(host) = url.host_str().map(str::to_string) else { return };
    if !host.contains('.') {
        return;
    }
    let port = url.port_or_known_default().unwrap_or(443);
    {
        let mut recent = RECENT.lock().unwrap();
        let now = Instant::now();
        if recent.get(&host).is_some_and(|t| now.duration_since(*t) < Duration::from_secs(30)) {
            return;
        }
        recent.retain(|_, t| now.duration_since(*t) < Duration::from_secs(30));
        recent.insert(host.clone(), now);
    }
    tauri::async_runtime::spawn_blocking(move || {
        let _ = (host.as_str(), port).to_socket_addrs();
    });
}

/// The DNS prefetch for a pause in ⌘L input or a highlighted bookmark/history row
/// (called by the frontend after its debounce). Malformed input is ignored silently.
#[tauri::command]
pub fn browser_prefetch(url: String) {
    if let Ok(u) = normalize_url(&url) {
        if !is_app_origin(&u) {
            prefetch_dns(&u);
        }
    }
}

// ---------------------------------------------------------------------------
// layout: where the native tab views go
// ---------------------------------------------------------------------------

fn logical_inner(win: &tauri::Window) -> Option<LogicalSize<f64>> {
    let scale = win.scale_factor().ok()?;
    Some(win.inner_size().ok()?.to_logical::<f64>(scale))
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> tauri::Rect {
    tauri::Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(w.max(1.0), h.max(1.0)).into(),
    }
}

/// Place the active tab and hide the others. Docked: the panel column the frontend
/// reported (right-aligned, under the chrome, down to the window's bottom edge),
/// shown only while the panel is on screen and no overlay is open. Detached: under
/// the chrome bar, the full width of the detached window (the bar itself is placed
/// here too, grown over the page while its picker is open).
/// Called after every state change; a window resize goes through `relayout` (the
/// anchors make the resize a pure recomputation — no round trip to the frontend, so a
/// live resize does not lag).
pub fn apply_layout(app: &AppHandle) {
    place_tabs(app, true);
}

/// The main window or the detached window resized (lib.rs's hooks): re-place the
/// active tab (and the chrome bar) from the anchors. A resize changes no tab's
/// visibility, so the other tabs are left alone — this runs once per resize frame.
pub fn relayout(app: &AppHandle) {
    place_tabs(app, false);
}

/// `full`: also settle every tab's visibility (the active one shown or hidden by the
/// overlay and blank rules, the rest hidden); otherwise only the active tab's bounds.
fn place_tabs(app: &AppHandle, full: bool) {
    let Some(s) = app.try_state::<AppState>() else { return };
    let (shape, active, panel, labels, bar_expanded, active_blank) = {
        let inner = s.browser.0.lock().unwrap();
        (
            inner.shape,
            inner.active.clone(),
            inner.panel,
            if full { inner.tabs.keys().cloned().collect::<Vec<_>>() } else { inner.active.iter().cloned().collect() },
            inner.bar_expanded,
            inner.active.as_ref().and_then(|l| inner.tabs.get(l)).is_some_and(|e| e.url == BLANK),
        )
    };
    // Every main-window resize frame comes through here, browser or no browser: with
    // nothing to place, the window and title-bar queries below are skipped
    if labels.is_empty() {
        return;
    }
    let (bounds, show_active) = match shape {
        Shape::Docked => {
            let Some(main) = main_window(app) else { return };
            let Some(inner) = logical_inner(&main) else { return };
            // A child view's origin is the window's top-left *including the title bar*
            // (the content view is full-size, and unlike the main webview a child gets
            // no automatic content inset), while the frontend measures from its own
            // viewport, which starts under the title bar — so the bar's height is added
            let top = title_bar_height(&main);
            let w = panel.width.clamp(100.0, inner.width);
            let y = (top + panel.chrome_h).min(inner.height);
            (rect(inner.width - w, y, w, inner.height - y), panel.visible && !panel.veiled)
        }
        Shape::Detached => {
            let Some(win) = app.get_window(DETACHED_LABEL) else { return };
            let Some(inner) = logical_inner(&win) else { return };
            if let Some(bar) = app.get_webview(BAR_LABEL) {
                let h = if bar_expanded { BAR_EXPANDED_H.min(inner.height) } else { BAR_H };
                let _ = bar.set_bounds(rect(0.0, 0.0, inner.width, h));
            }
            (rect(0.0, BAR_H, inner.width, inner.height - BAR_H), true)
        }
    };
    // A blank tab stays hidden: the panel draws its new-tab page in the slot instead
    let show_active = show_active && !active_blank;
    for label in labels {
        let Some(wv) = app.get_webview(&label) else { continue };
        if active.as_deref() == Some(label.as_str()) {
            let _ = wv.set_bounds(bounds);
            if !full {
                continue;
            }
            if show_active {
                let _ = wv.show();
            } else {
                let _ = wv.hide();
            }
        } else {
            let _ = wv.hide();
        }
    }
}

/// macOS: how tall the window's title bar is (logical px) — the part of the full-size
/// content view that contentLayoutRect leaves out. Zero where the content view starts
/// under the title bar anyway.
#[cfg(target_os = "macos")]
fn title_bar_height(win: &tauri::Window) -> f64 {
    use objc2_app_kit::NSWindow;
    let Ok(ptr) = win.ns_window() else { return 0.0 };
    let ns: &NSWindow = unsafe { &*ptr.cast() };
    let Some(content) = ns.contentView() else { return 0.0 };
    let full = content.frame().size.height;
    let layout = ns.contentLayoutRect().size.height;
    (full - layout).max(0.0)
}

#[cfg(not(target_os = "macos"))]
fn title_bar_height(_win: &tauri::Window) -> f64 {
    0.0
}

/// The frontend reports the docked panel column: on screen or collapsed, its width
/// and its chrome height (logical px). Sent when any of them changes — never per
/// frame; a live resize of the main window is handled from the anchors in Rust.
#[tauri::command]
pub fn browser_panel_layout(
    app: AppHandle,
    state: State<'_, AppState>,
    visible: bool,
    width: f64,
    chrome_h: f64,
) {
    {
        let mut inner = state.browser.0.lock().unwrap();
        inner.panel = PanelLayout { visible, width, chrome_h, veiled: inner.panel.veiled };
    }
    apply_layout(&app);
}

/// A full-window overlay opened or closed in the main window (the command palette,
/// a dialog, the lightbox, a sash drag): a native view would cover it, so the
/// active tab is hidden underneath and shown again afterwards. The frontend paints
/// a placeholder in the panel meanwhile.
#[tauri::command]
pub fn browser_panel_veil(app: AppHandle, state: State<'_, AppState>, veiled: bool) {
    let changed = {
        let mut inner = state.browser.0.lock().unwrap();
        let changed = inner.panel.veiled != veiled;
        inner.panel.veiled = veiled;
        changed
    };
    if changed {
        apply_layout(&app);
    }
}

// ---------------------------------------------------------------------------
// tabs
// ---------------------------------------------------------------------------

/// Make a tab the shown one (a registry change, the layout, the event; `focus` also
/// hands it the keyboard). A no-op for an unknown label.
fn activate(app: &AppHandle, label: &str, focus: bool) {
    let (detached, title, blank) = {
        let Some(s) = app.try_state::<AppState>() else { return };
        let mut inner = s.browser.0.lock().unwrap();
        let Some(entry) = inner.tabs.get(label) else { return };
        let (title, blank) = (entry.title.clone(), entry.url == BLANK);
        inner.active = Some(label.to_string());
        (inner.shape == Shape::Detached, title, blank)
    };
    apply_layout(app);
    if detached {
        sync_detached_title(app, &title);
        if let Some(win) = app.get_window(DETACHED_LABEL) {
            let _ = win.show();
            if focus {
                let _ = win.set_focus();
            }
        }
    }
    emit(app, &BrowserEvent::Activated { label: label.to_string() });
    // A blank tab's view stays hidden and the keyboard belongs to the chrome's address
    // input: focusing the hidden view would only take it away from there
    if focus && !blank {
        if let Some(wv) = app.get_webview(label) {
            let _ = wv.set_focus();
        }
    }
}

/// activate() from an async command: those run on a runtime thread, and the layout
/// reads the window's title bar through AppKit, which is main-thread only.
fn activate_on_main(app: &AppHandle, label: &str, focus: bool) {
    let app2 = app.clone();
    let label = label.to_string();
    let _ = app.run_on_main_thread(move || activate(&app2, &label, focus));
}

/// Show a tab (a click on the tab strip).
#[tauri::command]
pub fn browser_activate(app: AppHandle, state: State<'_, AppState>, label: String) -> Result<(), String> {
    let label = state.browser.0.lock().unwrap().target(Some(&label)).ok_or("no such tab")?;
    activate(&app, &label, true);
    Ok(())
}

/// Close a tab (the active one when no label is given). The succession: the next
/// tab to the right by open order, else the nearest to the left; closing the last
/// tab of the detached shape closes the detached window, and the keyboard focus
/// returns to the main window.
#[tauri::command]
pub fn browser_close_tab(app: AppHandle, state: State<'_, AppState>, label: Option<String>) -> Result<(), String> {
    let label = state.browser.0.lock().unwrap().target(label.as_deref()).ok_or("no browser tab is open")?;
    close_tab(&app, &label);
    Ok(())
}

/// macOS: the menu bar's ⌘W when the keyboard focus is inside a page (there is
/// no window menu elsewhere, so the only caller is cfg macos too).
#[cfg(target_os = "macos")]
pub fn close_active_tab(app: &AppHandle) {
    let active = app.try_state::<AppState>().and_then(|s| s.browser.0.lock().unwrap().target(None));
    if let Some(label) = active {
        close_tab(app, &label);
    }
}

/// Take a tab out of the registry: its entry, its native view pointer, and the
/// active slot when it held it (the caller decides the succession). Returns the
/// entry and whether it was the active tab; None for an unknown label.
fn forget_tab(inner: &mut BrowserInner, label: &str) -> Option<(BrowserEntry, bool)> {
    let closed = inner.tabs.remove(label)?;
    inner.views.remove(label);
    let was_active = inner.active.as_deref() == Some(label);
    if was_active {
        inner.active = None;
    }
    Some((closed, was_active))
}

fn close_tab(app: &AppHandle, label: &str) {
    let Some(s) = app.try_state::<AppState>() else { return };
    let (next, last, detached) = {
        let mut inner = s.browser.0.lock().unwrap();
        let Some((closed, was_active)) = forget_tab(&mut inner, label) else { return };
        let next = if was_active {
            let right = inner.tabs.iter().filter(|(_, e)| e.id > closed.id).min_by_key(|(_, e)| e.id);
            let left = inner.tabs.iter().filter(|(_, e)| e.id < closed.id).max_by_key(|(_, e)| e.id);
            right.or(left).map(|(l, _)| l.clone())
        } else {
            None
        };
        if was_active {
            inner.active = next.clone();
        }
        (next, inner.tabs.is_empty(), inner.shape == Shape::Detached)
    };
    if let Some(wv) = app.get_webview(label) {
        let _ = wv.close();
    }
    emit(app, &BrowserEvent::Closed { label: label.to_string() });
    if last {
        after_last_tab(app, detached);
    } else if let Some(next) = next {
        activate(app, &next, true);
    }
}

/// After the last tab closed: the detached window goes with it, and (focus
/// etiquette) the keyboard returns to the main window's own webview.
fn after_last_tab(app: &AppHandle, detached: bool) {
    if detached {
        close_detached_window(app);
    }
    if let Some(main) = app.get_webview("main") {
        let _ = main.window().set_focus();
        let _ = main.set_focus();
    }
}

// ---------------------------------------------------------------------------
// shape: docked ⇄ detached
// ---------------------------------------------------------------------------

/// The shape switch: detaching moves every tab into the standalone window
/// (`reparent`, no reload — the page, its scroll position and its history come
/// along), docking moves them back under the main window's panel. Every toggle is
/// persisted ("remember the last choice"), and the next tab opens in that shape.
#[tauri::command]
pub async fn browser_set_docked(app: AppHandle, docked: bool) -> Result<(), String> {
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    {
        let app2 = app.clone();
        app.run_on_main_thread(move || {
            let _ = tx.send(set_shape(&app2, if docked { Shape::Docked } else { Shape::Detached }));
        })
        .map_err(|e| e.to_string())?;
    }
    // A switch that failed (the detached window could not be built, say) is not
    // remembered: the setting must say where the tabs really are
    rx.await.map_err(|_| "the shape switch was interrupted".to_string())??;
    let _ = crate::settings::update(&app, |s| {
        let changed = s.browser_docked != docked;
        s.browser_docked = docked;
        changed
    });
    Ok(())
}

/// Main thread: apply the shape.
fn set_shape(app: &AppHandle, shape: Shape) -> Result<(), String> {
    let Some(s) = app.try_state::<AppState>() else { return Ok(()) };
    let (current, labels) = {
        let inner = s.browser.0.lock().unwrap();
        (inner.shape, inner.tabs.keys().cloned().collect::<Vec<_>>())
    };
    if current == shape {
        return Ok(());
    }
    match shape {
        Shape::Docked => {
            let main = main_window(app).ok_or_else(|| "the main window is gone".to_string())?;
            for label in &labels {
                if let Some(wv) = app.get_webview(label) {
                    let _ = wv.reparent(&main);
                }
            }
            s.browser.0.lock().unwrap().shape = Shape::Docked;
            // Once the tabs are the main window's again, the detached window's close
            // hook finds none of its own and just lets it go
            close_detached_window(app);
            apply_layout(app);
            let _ = main.set_focus();
        }
        Shape::Detached => {
            if !labels.is_empty() {
                let win = ensure_detached_window(app)?;
                for label in &labels {
                    if let Some(wv) = app.get_webview(label) {
                        let _ = wv.reparent(&win);
                    }
                }
                raise_bar(app);
                let title = {
                    let mut inner = s.browser.0.lock().unwrap();
                    inner.shape = Shape::Detached;
                    inner.active.as_ref().and_then(|l| inner.tabs.get(l)).map(|e| e.title.clone()).unwrap_or_default()
                };
                sync_detached_title(app, &title);
                apply_layout(app);
                let _ = win.show();
                let _ = win.set_focus();
            } else {
                s.browser.0.lock().unwrap().shape = Shape::Detached;
            }
        }
    }
    emit(app, &BrowserEvent::ShapeChanged { shape });
    Ok(())
}

/// Put the chrome bar above the tabs: a reparented view lands on top of its
/// siblings, so after tabs move into the detached window the bar is re-added to
/// stay in front (it overlaps the page while its picker is expanded).
fn raise_bar(app: &AppHandle) {
    if let (Some(bar), Some(win)) = (app.get_webview(BAR_LABEL), app.get_window(DETACHED_LABEL)) {
        let _ = bar.reparent(&win);
    }
}

/// The detached window's title follows its active tab (so it can be told apart in
/// ⌘Tab and the window menu), with the catalog's name for an untitled page.
fn sync_detached_title(app: &AppHandle, title: &str) {
    let Some(win) = app.get_window(DETACHED_LABEL) else { return };
    let fallback = crate::i18n::t(crate::i18n::current(), "window.browserTitle");
    let _ = win.set_title(if title.trim().is_empty() { &fallback } else { title });
}

/// Close the detached window, remembering its geometry first (written to disk on Destroyed).
fn close_detached_window(app: &AppHandle) {
    if let Some(win) = app.get_window(DETACHED_LABEL) {
        record_detached_geom(app, &win);
        let _ = win.close();
    }
}

/// The detached window, built on demand (hidden; the caller shows it once the tabs
/// are placed): a plain window with no webview of its own, the chrome bar as its
/// first child. Its geometry restores the last one (settings.browser_window) or is
/// centred at nearly full height on the main window's screen.
fn ensure_detached_window(app: &AppHandle) -> Result<tauri::Window, String> {
    if let Some(w) = app.get_window(DETACHED_LABEL) {
        return Ok(w);
    }
    let remembered = app
        .try_state::<AppState>()
        .and_then(|s| s.browser.0.lock().unwrap().detached_geom)
        .or_else(|| crate::settings::load(app).browser_window);
    let geom = independent_geom(app, remembered);
    // The mut is only reassigned inside the cfg(macos) block
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut builder = tauri::window::WindowBuilder::new(app, DETACHED_LABEL)
        .title(crate::i18n::t(crate::i18n::current(), "window.browserTitle"))
        .min_inner_size(480.0, 360.0)
        .visible(false);
    builder = match geom {
        Some(g) => builder.position(g.x, g.y).inner_size(g.w, g.h),
        None => builder.inner_size(1080.0, 760.0).center(),
    };
    #[cfg(target_os = "macos")]
    {
        // The traffic lights float over the chrome bar's left end, and the bar's tab
        // strip is the drag region (the page below is a WKWebView that would swallow a drag)
        builder = builder.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
    }
    let win = builder.build().map_err(|e| e.to_string())?;
    let width = logical_inner(&win).map(|s| s.width).unwrap_or(1080.0);
    win.add_child(
        WebviewBuilder::new(BAR_LABEL, WebviewUrl::App("index.html".into())).focused(false),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(width, BAR_H),
    )
    .map_err(|e| e.to_string())?;

    let handle = app.clone();
    win.on_window_event(move |event| match event {
        WindowEvent::Resized(_) => {
            relayout(&handle);
            if let Some(w) = handle.get_window(DETACHED_LABEL) {
                record_detached_geom(&handle, &w);
            }
        }
        WindowEvent::Moved(_) => {
            if let Some(w) = handle.get_window(DETACHED_LABEL) {
                record_detached_geom(&handle, &w);
            }
        }
        WindowEvent::CloseRequested { .. } => {
            // The red light or ⌘W on the last tab: every tab still living in this
            // window goes with it (after a dock the tabs are the main window's again
            // and nothing is touched)
            let mine: Vec<String> = handle
                .try_state::<AppState>()
                .map(|s| s.browser.0.lock().unwrap().tabs.keys().cloned().collect::<Vec<String>>())
                .unwrap_or_default()
                .into_iter()
                .filter(|l| handle.get_webview(l).is_some_and(|w| w.window().label() == DETACHED_LABEL))
                .collect();
            for label in mine {
                if let Some(s) = handle.try_state::<AppState>() {
                    forget_tab(&mut s.browser.0.lock().unwrap(), &label);
                }
                emit(&handle, &BrowserEvent::Closed { label });
            }
        }
        WindowEvent::Destroyed => {
            if let Some(s) = handle.try_state::<AppState>() {
                let mut inner = s.browser.0.lock().unwrap();
                inner.bar_expanded = false;
                if let Some(g) = inner.detached_geom {
                    drop(inner);
                    // update: exclusive with browser_set_docked's write (a bare load→save would overwrite the other's fields)
                    let _ = crate::settings::update(&handle, |s| {
                        let changed = s.browser_window != Some(g);
                        s.browser_window = Some(g);
                        changed
                    });
                }
            }
            if let Some(main) = main_window(&handle) {
                let _ = main.set_focus();
            }
        }
        _ => {}
    });
    Ok(win)
}

/// Remember the detached window's normal-state geometry (written to disk on Destroyed).
fn record_detached_geom(app: &AppHandle, win: &tauri::Window) {
    if let Some(g) = crate::agent_window::read_geom(win) {
        if let Some(s) = app.try_state::<AppState>() {
            s.browser.0.lock().unwrap().detached_geom = Some(g);
        }
    }
}

/// The build geometry of the detached window: the position it was last closed at
/// comes first, otherwise it is centred at nearly full height in the visible area
/// (work_area, with the menu bar, Dock and taskbar already removed) of the main
/// window's screen. None when the main window or the monitors cannot be read, and
/// the caller falls back to builder.center().
fn independent_geom(
    app: &AppHandle,
    remembered: Option<crate::agent_window::WindowGeom>,
) -> Option<crate::agent_window::WindowGeom> {
    use crate::agent_window::WindowGeom;
    if let Some(g) = remembered {
        // The remembered geometry may belong to an external display that has been
        // unplugged: when the title bar lands on no monitor at all it is discarded in
        // favour of the "centred on the main window's screen" below — a window must
        // never open off-screen
        if grabbable_on_some_monitor(app, &g) {
            return Some(g);
        }
    }
    let main = main_window(app)?;
    let monitor = main.current_monitor().ok()??;
    let scale = monitor.scale_factor();
    let wa = monitor.work_area();
    let pos = wa.position.to_logical::<f64>(scale);
    let size = wa.size.to_logical::<f64>(scale);
    let (x, y, w, h) = initial_browser_rect(pos.x, pos.y, size.width, size.height);
    Some(WindowGeom { x, y, w, h })
}

/// The usability check for a remembered geometry: the window's title bar (a point in
/// the middle of its top edge) landing inside any monitor's visible area counts as
/// grabbable — the user can at least drag it back. With no monitor list available it
/// does not block (better to trust the memory).
pub(crate) fn grabbable_on_some_monitor(app: &AppHandle, g: &crate::agent_window::WindowGeom) -> bool {
    let Ok(monitors) = app.available_monitors() else { return true };
    if monitors.is_empty() {
        return true;
    }
    // The middle of the title bar, 20 logical px into the window
    let (px, py) = (g.x + g.w / 2.0, g.y + 20.0);
    monitors.iter().any(|m| {
        let scale = m.scale_factor();
        let pos = m.work_area().position.to_logical::<f64>(scale);
        let size = m.work_area().size.to_logical::<f64>(scale);
        px >= pos.x && px <= pos.x + size.width && py >= pos.y && py <= pos.y + size.height
    })
}

/// The default geometry of the first detached window (pure geometry, all logical px):
/// centred at nearly full height in the visible area. The width of 1080 keeps the
/// desktop layout breakpoint (narrower and plenty of sites render their mobile
/// version); the height takes as much as it can, capped at 1200 on a very tall
/// screen, with a floor on both axes for a very small screen.
fn initial_browser_rect(vx: f64, vy: f64, vw: f64, vh: f64) -> (f64, f64, f64, f64) {
    let w = 1080.0_f64.min(vw - 64.0).max(360.0);
    let h = (vh - 64.0).clamp(420.0, 1200.0);
    (vx + (vw - w) / 2.0, vy + (vh - h) / 2.0, w, h)
}

/// macOS: the tab whose native view holds the keyboard focus (the first responder
/// of the window the tabs live in), or None when the focus is in our own DOM. The
/// menu bar routes ⌘W and friends by this: inside a page they mean the tab, outside
/// it they keep their editor meaning.
#[cfg(target_os = "macos")]
pub fn focused_tab(app: &AppHandle) -> Option<String> {
    use objc2::rc::Retained;
    use objc2_app_kit::{NSView, NSWindow};
    let (shape, views, veiled) = {
        let s = app.try_state::<AppState>()?;
        let inner = s.browser.0.lock().unwrap();
        (inner.shape, inner.views.clone(), inner.panel.veiled)
    };
    if views.is_empty() {
        return None;
    }
    let win = match shape {
        // Under an overlay the docked view is hidden, and the keyboard is the overlay's
        // whatever responder the window still remembers
        Shape::Docked if veiled => return None,
        Shape::Docked => main_window(app)?,
        Shape::Detached => app.get_window(DETACHED_LABEL)?,
    };
    // A window keeps its first responder while it is not the key window: a tab in the
    // detached window must not answer for ⌘W pressed in the main one
    if !win.is_focused().unwrap_or(false) {
        return None;
    }
    let ptr = win.ns_window().ok()?;
    let ns: &NSWindow = unsafe { &*ptr.cast() };
    let responder = ns.firstResponder()?;
    let mut view: Option<Retained<NSView>> = responder.downcast::<NSView>().ok();
    while let Some(v) = view {
        let p = Retained::as_ptr(&v) as usize;
        if let Some((label, _)) = views.iter().find(|(_, vp)| **vp == p) {
            return Some(label.clone());
        }
        view = unsafe { v.superview() };
    }
    None
}

// ---------------------------------------------------------------------------
// registry access and the navigation commands
// ---------------------------------------------------------------------------

/// Modify one registry entry inside the lock, with the closure's return value passed
/// through (None when the tab is not in the registry).
fn update_entry<R>(app: &AppHandle, label: &str, f: impl FnOnce(&mut BrowserEntry) -> R) -> Option<R> {
    let s = app.try_state::<AppState>()?;
    let mut inner = s.browser.0.lock().unwrap();
    inner.tabs.get_mut(label).map(f)
}

/// The target tab (an explicit label, or the active one); an error when it does not exist.
pub(crate) fn target_webview(
    app: &AppHandle,
    state: &State<'_, AppState>,
    label: Option<String>,
) -> Result<tauri::Webview, String> {
    let label = state.browser.0.lock().unwrap().target(label.as_deref()).ok_or("no browser tab is open")?;
    app.get_webview(&label).ok_or_else(|| "the browser tab is closed".into())
}

#[tauri::command]
pub fn browser_navigate(
    app: AppHandle,
    state: State<'_, AppState>,
    label: Option<String>,
    url: String,
) -> Result<(), String> {
    let url = normalize_url(&url)?;
    let webview = target_webview(&app, &state, label)?;
    webview.navigate(url).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn browser_back(app: AppHandle, state: State<'_, AppState>, label: Option<String>) -> Result<(), String> {
    // The webview carries its own history; with none this is a no-op (the degradation of not knowing canGoBack is accepted)
    target_webview(&app, &state, label)?.eval("history.back()").map_err(|e| e.to_string())
}

#[tauri::command]
pub fn browser_forward(app: AppHandle, state: State<'_, AppState>, label: Option<String>) -> Result<(), String> {
    target_webview(&app, &state, label)?.eval("history.forward()").map_err(|e| e.to_string())
}

#[tauri::command]
pub fn browser_reload(app: AppHandle, state: State<'_, AppState>, label: Option<String>) -> Result<(), String> {
    target_webview(&app, &state, label)?.reload().map_err(|e| e.to_string())
}

/// Hand the keyboard back to a tab (the focus-return contract: after any chrome
/// interaction, space-bar paging and keyboard scrolling keep working).
#[tauri::command]
pub fn browser_focus(app: AppHandle, state: State<'_, AppState>, label: String) -> Result<(), String> {
    let webview = target_webview(&app, &state, Some(label))?;
    if state.browser.0.lock().unwrap().shape == Shape::Detached {
        if let Some(win) = app.get_window(DETACHED_LABEL) {
            let _ = win.set_focus();
        }
    }
    webview.set_focus().map_err(|e| e.to_string())
}

/// A backstop full-state fetch for when the frontend starts or reloads (a snapshot outside the event stream).
#[tauri::command]
pub fn browser_state(state: State<'_, AppState>) -> BrowserSnapshot {
    let inner = state.browser.0.lock().unwrap();
    let mut tabs: Vec<BrowserTabView> = inner
        .tabs
        .iter()
        .map(|(label, e)| BrowserTabView { label: label.clone(), entry: e.clone() })
        .collect();
    // The open order, matching the frontend's event-stream registry (a label's string order goes wrong past tab 10)
    tabs.sort_by_key(|w| w.entry.id);
    BrowserSnapshot { tabs, active: inner.active.clone(), shape: inner.shape }
}

/// The single destination of an http(s) link click: it branches by the setting into the
/// reading projection, the built-in browser or the system browser. The editor, the AI
/// panel and the source cards all come through here. `invert=true` (⌘click) gives "the
/// other way of opening it": Reading/System → Internal, Internal → System — so ⌘click
/// always produces a real browser tab (the projection's escape-hatch semantics).
#[tauri::command]
pub async fn url_open(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    invert: Option<bool>,
    focus: Option<bool>,
) -> Result<(), String> {
    use crate::settings::LinkOpenMode;
    let mut mode = crate::settings::load(&app).open_links_in;
    if invert.unwrap_or(false) {
        mode = match mode {
            LinkOpenMode::Reading | LinkOpenMode::System => LinkOpenMode::Internal,
            LinkOpenMode::Internal => LinkOpenMode::System,
        };
    }
    match mode {
        // The projection is state in the main window's frontend: this only sends the
        // signal, and the frontend calls page_read back for the content
        LinkOpenMode::Reading => {
            let parsed = normalize_url(&url)?;
            super::reader::emit_reading(
                &app,
                &super::reader::ReadingEvent::Open { url: parsed.to_string() },
            );
            Ok(())
        }
        LinkOpenMode::Internal => browser_open(app, state, url, Some(false), focus).await.map(|_| ()),
        LinkOpenMode::System => {
            let parsed = normalize_url(&url)?;
            opener::open(parsed.as_str()).map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_adds_https_scheme() {
        assert_eq!(normalize_url("example.com").unwrap().as_str(), "https://example.com/");
        assert_eq!(
            normalize_url("  en.wikipedia.org/wiki/Rust ").unwrap().as_str(),
            "https://en.wikipedia.org/wiki/Rust"
        );
    }

    #[test]
    fn normalize_rejects_non_http() {
        assert!(normalize_url("file:///etc/hosts").is_err());
        assert!(normalize_url("tauri://localhost").is_err());
        assert!(normalize_url("javascript:alert(1)").is_err());
        assert!(normalize_url("").is_err());
    }

    #[test]
    fn app_origin_is_blocked() {
        assert!(is_app_origin(&Url::parse("http://localhost:1420/").unwrap()));
        assert!(is_app_origin(&Url::parse("http://127.0.0.1:1420/x").unwrap()));
        assert!(!is_app_origin(&Url::parse("https://example.com/").unwrap()));
    }

    /// The first detached window: on a 14-inch screen (visible area 1512×947) it is
    /// centred at nearly full height; on a very tall external display the height caps
    /// at 1200; on a narrow screen the width gives way and stays inside the visible
    /// area.
    #[test]
    fn initial_rect_fills_height_and_caps() {
        let (x, y, w, h) = initial_browser_rect(0.0, 25.0, 1512.0, 947.0);
        assert_eq!((w, h), (1080.0, 883.0));
        assert_eq!((x, y), (216.0, 57.0));
        let (_, _, _, h) = initial_browser_rect(0.0, 0.0, 2560.0, 1415.0);
        assert_eq!(h, 1200.0);
        let (x, _, w, _) = initial_browser_rect(0.0, 0.0, 1024.0, 768.0);
        assert_eq!(w, 960.0);
        assert_eq!(x, 32.0);
    }
}
