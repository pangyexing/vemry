//! Creating, navigating and the lifetime commands of a browser window.
//!
//! The security boundary: a window's label is `browser-N`, which matches no
//! capability (default.json only authorises the `main` webview) — a remote page
//! physically has no IPC. The navigation guard is a second line of defence: even
//! should a page try to jump to the app's own origin (the dev server, tauri://), it
//! is refused outright.

use serde::Serialize;
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, State, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent};

use super::{emit, BrowserEntry, BrowserEvent};
use crate::AppState;

/// Record one real navigation: the guard (http(s) and not the app's origin) →
/// refresh the registry's URL → broadcast Navigated → record it in the
/// application-level history. Both callers share this one navigation policy:
/// `on_navigation` (whose return value is the verdict) and the SPA backstop in the
/// title hook (pushState does not trigger on_navigation). Adding a banned scheme or
/// filtering the history changes this one place.
fn note_navigation(app: &AppHandle, label: &str, url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https") || is_app_origin(url) {
        return false;
    }
    let url = url.to_string();
    update_entry(app, label, |e| e.url = url.clone());
    emit(app, &BrowserEvent::Navigated { label: label.to_string(), url: url.clone() });
    // The application-level browsing history: the first load and every later
    // navigation come through here; the hidden reader window has no such hook and
    // therefore never enters it
    super::history::record_visit(app, &url);
    true
}

/// User input → a navigable URL: the scheme is filled in, and only http(s) is accepted.
pub fn normalize_url(input: &str) -> Result<Url, String> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err("empty URL".into());
    }
    let candidate =
        if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let url = Url::parse(&candidate).map_err(|e| format!("invalid URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("only http(s) is supported: {url}"));
    }
    if url.host_str().is_none() {
        return Err("invalid URL: the host name is missing".into());
    }
    Ok(url)
}

/// A browser window must never drive into the app's own origin (the main webview's
/// dev server or bundled assets), which stops a remote page taking the local context
/// through a navigation.
pub(crate) fn is_app_origin(url: &Url) -> bool {
    match url.scheme() {
        "http" | "https" => matches!(url.host_str(), Some("localhost") | Some("127.0.0.1")),
        _ => true, // tauri:// and everything else that is not http(s) counts as the app side and is refused
    }
}

/// A snapshot entry = label plus the registry entry verbatim (serde flatten, so the
/// JSON shape on the wire matches the days of copying field by field). A field added
/// to BrowserEntry reaches the snapshot automatically, with no three places to keep
/// in sync.
#[derive(Serialize)]
pub struct BrowserWindowView {
    pub label: String,
    #[serde(flatten)]
    pub entry: BrowserEntry,
}

#[derive(Serialize)]
pub struct BrowserSnapshot {
    pub windows: Vec<BrowserWindowView>,
    pub active: Option<String>,
}

/// Open a URL: reuse the active browser window or build a new one. Returns the
/// window's label.
/// `focus=false` is for cases like "opened from a citation": the window appears
/// without taking the keyboard focus, so the user can stay in the conversation and
/// keep asking.
#[tauri::command]
pub async fn browser_open(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    new_window: Option<bool>,
    focus: Option<bool>,
) -> Result<String, String> {
    let url = normalize_url(&url)?;
    // The same verdict as on_navigation, refused early: the app's own origin
    // (localhost and friends) passes normalize_url but would be stopped by the
    // navigation guard — without refusing it here, the registry and the page card
    // would record a URL the window never showed, and every later clip would land
    // under the wrong source
    if is_app_origin(&url) {
        return Err(format!("that address may not be opened: {url}"));
    }
    // Resolving first: it runs alongside the window build, so by the time the navigation really starts the DNS answer is most likely in the system cache
    prefetch_dns(&url);
    super::adblock::ensure_rules_soon(&app);
    let focus = focus.unwrap_or(true);

    // With an active window and no request for a new one: navigate in place
    if !new_window.unwrap_or(false) {
        let target = state.browser.0.lock().unwrap().target(None);
        if let Some(label) = target {
            if let Some(webview) = app.get_webview_window(&label) {
                webview.navigate(url.clone()).map_err(|e| e.to_string())?;
                if focus {
                    let _ = webview.set_focus();
                }
                // The URL record and the Navigated event are emitted together by
                // on_navigation when the navigation is allowed; nothing is written to
                // the registry ahead of time (which would drift from what is really
                // shown and duplicate the event)
                return Ok(label);
            }
        }
    }

    // Building and claiming a window happen on the main thread as a whole: on macOS
    // the WKWebViewConfiguration (!Send) has to have the adblock rules attached
    // before the window is built, so the first frame already blocks
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<String, String>>();
    {
        let app2 = app.clone();
        let url2 = url.clone();
        app.run_on_main_thread(move || {
            let _ = tx.send(open_in_shell(&app2, &url2, focus));
        })
        .map_err(|e| e.to_string())?;
    }
    let result = rx.await.map_err(|_| "window creation was interrupted".to_string())?;
    // Queue the next warm-up shell window, on success and failure alike — the failure
    // path consumed the shell too (a failed claim closes it), and without refilling,
    // one transient failure would leave every window this session opening on the cold
    // path
    ensure_pool_soon(&app, 1500);
    let label = result?;

    let (docked, inset) = {
        let inner = state.browser.0.lock().unwrap();
        inner.windows.get(&label).map(|e| (e.docked, e.inset)).unwrap_or((false, 0.0))
    };
    emit(&app, &BrowserEvent::Opened { label: label.clone(), url: url.to_string(), docked });
    // The dock at build time happens before Opened, when the frontend registry does not have this window yet — so the layout is sent once more
    if inset > 0.0 {
        emit(&app, &BrowserEvent::DockLayout { label: label.clone(), inset });
    }
    Ok(label)
}

/// Main thread: open a new browser window — claiming the warm-up shell first (a
/// hidden about:blank window whose WKWebView, subprocess and ad-blocking rules are
/// already built, so opening is left with navigating and showing; the same idea as
/// pre-building a renderer process). With no shell, or one that was destroyed, it
/// takes the cold path and builds everything. Returns the window's label.
fn open_in_shell(app: &AppHandle, url: &Url, focus: bool) -> Result<String, String> {
    let state = app.state::<AppState>();
    let pooled = state.browser.0.lock().unwrap().pool.take();
    let (label, id) = match pooled.filter(|(l, _)| app.get_webview_window(l).is_some()) {
        Some((label, id)) => (label, id),
        None => {
            let (label, id) = alloc_label(&state);
            build_shell(app, &label)?;
            (label, id)
        }
    };
    claim_shell(app, id, &label, url, focus)?;
    Ok(label)
}

fn alloc_label(state: &State<'_, AppState>) -> (String, u32) {
    let mut inner = state.browser.0.lock().unwrap();
    inner.next_id += 1;
    (format!("browser-{}", inner.next_id), inner.next_id)
}

/// The single implementation of "open the browser" (shared by the menu bar, the ⌘P
/// command and an empty ⌘L return): with a browser window open → focus it; with none
/// → reopen the most recent page from the history; with neither → Ok(None) and the
/// frontend shows a toast.
#[tauri::command]
pub async fn browser_open_last(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let existing = state.browser.0.lock().unwrap().target(None);
    if let Some(label) = existing {
        if let Some(webview) = app.get_webview_window(&label) {
            let _ = webview.set_focus();
            return Ok(Some(label));
        }
    }
    match super::history::last_url(&app) {
        Some(url) => browser_open(app, state, url, Some(false), Some(true)).await.map(Some),
        None => Ok(None),
    }
}

/// Main thread: build the hidden browser shell window (about:blank) — the builder
/// chain, the navigation guard, the ad-blocking rules, the gestures and the window
/// events are all in place, but it does not enter the registry and is not shown.
/// Registering, navigating, docking and showing happen in claim_shell (the cold path
/// claims it immediately; the warm-up path builds it in advance and waits to be
/// claimed).
fn build_shell(app: &AppHandle, label: &str) -> Result<(), String> {
    let nav_app = app.clone();
    let nav_label = label.to_string();
    let title_app = app.clone();
    let new_win_app = app.clone();
    let new_win_label = label.to_string();

    let blank = Url::parse("about:blank").expect("about:blank always parses");
    // Both `builder =` reassignments below are inside cfg(macos) blocks, so off mac the mut is unused
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut builder = WebviewWindowBuilder::new(app, label, WebviewUrl::External(blank))
        .title(crate::i18n::t(crate::i18n::current(), "window.browserTitle"))
        // Built hidden first: it is shown once claiming has positioned it, which avoids the "appear, then jump" flicker
        .visible(false)
        .focused(false)
        .inner_size(1080.0, 760.0)
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
            // Popup suppression: target=_blank and window.open all open in the same window
            if matches!(target.scheme(), "http" | "https") && !is_app_origin(&target) {
                if let Some(w) = new_win_app.get_webview_window(&new_win_label) {
                    let _ = w.navigate(target);
                }
            }
            NewWindowResponse::Deny
        })
        .on_document_title_changed(move |webview, title| {
            let label = webview.label().to_string();
            // The window title follows the page, so it can be told apart in ⌘Tab and the window menu
            let fallback = crate::i18n::t(crate::i18n::current(), "window.browserTitle");
            let _ = webview.set_title(if title.trim().is_empty() { &fallback } else { title.as_str() });
            // The SPA backstop: a pushState navigation does not trigger on_navigation,
            // and the registry's URL would be left on the old page (the page card and
            // the bookmark ★ lighting up wrongly, a page missing from the history). An
            // SPA page change almost always changes the title — so the webview's real
            // URL is read back at this moment, and when it differs from the registry a
            // Navigated plus a history record follow (guarded by the same verdict as
            // on_navigation).
            if let Ok(current) = webview.url() {
                let known = update_entry(&title_app, &label, |e| e.url.clone());
                if known.is_some_and(|u| u != current.as_str()) {
                    note_navigation(&title_app, &label, &current);
                }
            }
            // url and title arrive a beat apart: the history entry is located by
            // label → the registry's url to amend the title. One trip through the lock
            // sets the title and reads the current URL back, rather than entering the
            // registry twice for it
            let url = update_entry(&title_app, &label, |e| {
                e.title = title.clone();
                e.url.clone()
            });
            if let Some(url) = url {
                super::history::amend_title(&title_app, &url, &title);
            }
            emit(&title_app, &BrowserEvent::TitleChanged { label, title });
        });

    // A shell window is always built in the detached shape (an ordinary title bar),
    // and the docked shape is switched at runtime while claiming by apply_dock_state
    // (the same path as "snap/detach", with the window still hidden, so there is no
    // flicker). The same goes for geometry: set_position/set_size happen while
    // claiming.

    // Smoothing a discrete scroll wheel (macOS only: a WKWebView's wheel jumps
    // instantly, while WebView2 smooths it itself and injecting there would only get
    // in the way). The script is dormant by default and woken by wheel.rs's device
    // classification
    #[cfg(target_os = "macos")]
    {
        builder = builder.initialization_script(super::wheel::SMOOTH_WHEEL_JS);
    }

    // With the rules compiled and the switch on → they are attached to the
    // configuration before the window is built, so no ad slips through the first
    // frame.
    // Should the switch or the rules change after the shell was built,
    // rebuild_pool_if_present rebuilds it.
    #[cfg(target_os = "macos")]
    if crate::settings::load(app).adblock_enabled {
        if let Some(cfg) = super::adblock::webview_configuration_with_rules() {
            builder = builder.with_webview_configuration(cfg);
        }
    }

    let webview = builder.build().map_err(|e| e.to_string())?;

    // A two-finger swipe is WKWebView's native back and forward. A browser window has
    // zero IPC (capabilities authorise the main window alone), so no fallback UI can
    // be injected — the native gesture is the only navigation the window carries on
    // its own
    #[cfg(target_os = "macos")]
    {
        use objc2_web_kit::WKWebView;
        let _ = webview.with_webview(|pw| {
            let wk = pw.inner().cast::<WKWebView>();
            if let Some(wk) = unsafe { wk.as_ref() } {
                unsafe { wk.setAllowsBackForwardNavigationGestures(true) };
            }
        });
    }

    // Focus and destruction tracking can only hang off a window that is built (the builder has no such hook)
    let ev_app = app.clone();
    let ev_label = label.to_string();
    webview.on_window_event(move |event| match event {
        WindowEvent::Focused(true) => {
            if let Some(s) = ev_app.try_state::<AppState>() {
                s.browser.0.lock().unwrap().active = Some(ev_label.clone());
            }
            // The toolbar's backstop re-attachment: minimising a browser window breaks
            // the child-window relation (the same as reattach_docked between the main
            // window and a docked one; this is the second level)
            super::clipbar::ensure_clipbar_attached(&ev_app, &ev_label);
            super::clipbar::position_clipbar(&ev_app, &ev_label);
            emit(&ev_app, &BrowserEvent::Focused { label: ev_label.clone() });
        }
        WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
            // The toolbar follows the bottom edge: on macOS a child window moves with
            // its parent through AppKit, and resetting the same value on every Moved
            // tick of a window drag would be six window-server round trips for nothing,
            // so it is only synced on a resize (with Focused as the backstop that
            // re-attaches and repositions when the attachment lapsed); off mac there is
            // no child-window mechanism, so both moving and resizing rely on this
            #[cfg(target_os = "macos")]
            if matches!(event, WindowEvent::Resized(_)) {
                super::clipbar::position_clipbar(&ev_app, &ev_label);
            }
            #[cfg(not(target_os = "macos"))]
            super::clipbar::position_clipbar(&ev_app, &ev_label);
            // Only a detached window's geometry is recorded (written to disk on
            // Destroyed for "reopen where it was"): a docked window's movement is driven
            // by redock, so its position means nothing. The lock covers the check
            // alone, and reading the geometry (a window API) happens outside it
            let undocked = ev_app.try_state::<AppState>().is_some_and(|s| {
                s.browser.0.lock().unwrap().windows.get(&ev_label).is_some_and(|e| !e.docked)
            });
            if undocked {
                if let Some(w) = ev_app.get_webview_window(&ev_label) {
                    if let Some(g) = crate::agent_window::read_geom(&w) {
                        update_entry(&ev_app, &ev_label, |e| e.geom = Some(g));
                    }
                }
            }
        }
        WindowEvent::Destroyed => {
            // The toolbar's lifetime follows the browser window entirely (it never enters the registry, and this is the only link)
            if let Some(bar) =
                ev_app.get_webview_window(&super::clipbar::clipbar_label_of(&ev_label))
            {
                let _ = bar.close();
            }
            let mut last_closed = false;
            let mut removed: Option<BrowserEntry> = None;
            if let Some(s) = ev_app.try_state::<AppState>() {
                let mut inner = s.browser.0.lock().unwrap();
                // The warm-up shell was destroyed (taken along by the main window quitting, or rebuilt): clearing the slot is enough
                if inner.pool.as_ref().is_some_and(|(l, _)| l == &ev_label) {
                    inner.pool = None;
                }
                removed = inner.windows.remove(&ev_label);
                if inner.active.as_deref() == Some(ev_label.as_str()) {
                    // It has to pick the same window as the frontend registry's
                    // windows[0] (the open order): with HashMap's arbitrary order,
                    // target(None) and the current window the page card shows would
                    // drift apart, and clipping would quietly grab the wrong window. The
                    // open sequence number is data on the entry, so take the earliest
                    inner.active =
                        inner.windows.iter().min_by_key(|(_, e)| e.id).map(|(l, _)| l.clone());
                }
                last_closed = removed.is_some() && inner.windows.is_empty();
            }
            // An unregistered shell makes no noise outwards: no Closed, and no focus back to the main window
            let Some(entry) = removed else { return };
            // A detached window's geometry is written to disk (the same as
            // agent_window; an unchanged value is not written). A docked one is not: the
            // snapshot taken while it was detached stays in the entry, but with
            // docked==true at close it is void and must not pollute the detached
            // geometry
            if !entry.docked {
                if let Some(g) = entry.geom {
                    // update: this can race browser_set_docked's write (the command
                    // thread against the main-thread hook), and a bare load→save would
                    // overwrite the other's fields
                    let _ = crate::settings::update(&ev_app, |s| {
                        let changed = s.browser_window != Some(g);
                        s.browser_window = Some(g);
                        changed
                    });
                }
            }
            emit(&ev_app, &BrowserEvent::Closed { label: ev_label.clone() });
            // Focus etiquette: the main window is refocused once the last browser window closes
            if last_closed {
                if let Some(main) = ev_app.get_webview_window("main") {
                    let _ = main.set_focus();
                }
            }
        }
        _ => {}
    });

    Ok(())
}

/// Main thread: claim a shell as a real browser window — geometry, registration,
/// navigation, docking, showing.
/// The window stays hidden throughout and show comes last, so there is no flicker.
fn claim_shell(app: &AppHandle, id: u32, label: &str, url: &Url, focus: bool) -> Result<(), String> {
    let webview =
        app.get_webview_window(label).ok_or_else(|| "the browser window was destroyed".to_string())?;
    // The shape to claim it in comes from the settings (browser_set_docked persists
    // every toggle = "remember the last choice"); the first time it defaults to
    // detached and centred, with docking as an optional switch (batch D inverted
    // v0.5.2's docked default)
    let settings = crate::settings::load(app);
    let docked = settings.browser_docked;
    let indep_geom = if docked { None } else { independent_geom(app, settings.browser_window) };

    // Geometry: a docked window's size and position are dock_window's business; a
    // detached one restores its last geometry (cascading with several windows) or is
    // centred at nearly full height on the main window's screen (independent_geom
    // computes against the monitor the main window belongs to — with the main window
    // on an external display, a browser window must not open on a different one).
    if let Some(g) = &indep_geom {
        let _ = webview.set_position(tauri::LogicalPosition::new(g.x, g.y));
        let _ = webview.set_size(tauri::LogicalSize::new(g.w, g.h));
    } else if !docked {
        // The fallback when the main window or the monitor cannot be read (the shell defaults to 1080×760)
        let _ = webview.center();
    }

    if let Some(s) = app.try_state::<AppState>() {
        let mut inner = s.browser.0.lock().unwrap();
        inner.windows.insert(
            label.to_string(),
            BrowserEntry {
                id,
                docked,
                inset: 0.0,
                url: url.to_string(),
                title: String::new(),
                geom: indep_geom,
            },
        );
        inner.active = Some(label.to_string());
    }

    // Registered before navigating: the update_entry/Navigated that on_navigation
    // performs when it allows the navigation has to find the registry entry (in the
    // days of the cold path, navigation began at build and registration came after,
    // so the event order was once inverted)
    if let Err(e) = webview.navigate(url.clone()) {
        // A window that cannot even start navigating is rare but must not be left: hidden plus registered = a ghost entry
        if let Some(s) = app.try_state::<AppState>() {
            s.browser.0.lock().unwrap().windows.remove(label);
        }
        let _ = webview.close();
        return Err(e.to_string());
    }
    if docked {
        // Title bar switched to the docked shape → positioned → attached as a native
        // child window → then shown (once attached it follows the main window's
        // movement, minimisation and layering, and addChildWindow may order it in
        // immediately, which is why it is positioned first)
        apply_dock_chrome(app, label, true);
        dock_window(app, label, true);
        set_native_child(app, label, true);
    }
    let _ = webview.show();
    // The clip toolbar comes a beat later: building its webview competes for the main thread, which should not happen at the page's first frame
    super::clipbar::spawn_clipbar_soon(app, label);
    if focus {
        let _ = webview.set_focus();
    }
    Ok(())
}

/// Refill the warm-up shell: the build is delayed to avoid the main thread's busy
/// stretches at startup and during a page load (building a WKWebView competes for the
/// main thread and slows down what is rendering). lib.rs's startup and each claim
/// queue one.
pub fn ensure_pool_soon(app: &AppHandle, delay_ms: u64) {
    super::on_main_after(app, delay_ms, ensure_pool);
}

/// Main thread: make sure one warm-up shell is standing by. Idempotent — every call
/// is serialised through the main thread, and an occupied slot is a no-op. A failure
/// is only logged (the next browser_open takes the cold path, and nothing breaks).
fn ensure_pool(app: &AppHandle) {
    // The main window is destroyed (we are quitting): a refill timer arriving late
    // must not build another hidden shell — destroy_pool runs once on the main
    // window's Destroyed, and a shell born after that has nobody to clean it up, which
    // would leave "closing every window quits" waiting forever
    if app.get_webview_window("main").is_none() {
        return;
    }
    let Some(s) = app.try_state::<AppState>() else { return };
    if s.browser.0.lock().unwrap().pool.is_some() {
        return;
    }
    let (label, id) = alloc_label(&s);
    match build_shell(app, &label) {
        Ok(()) => s.browser.0.lock().unwrap().pool = Some((label, id)),
        Err(e) => eprintln!("browser pool prewarm failed: {e}"),
    }
}

/// Rebuild the warm-up shell after the ad-blocking rules roll over or the switch is
/// toggled: a shell's rule list is fixed into its configuration at build time, and
/// only a rebuild picks up the current configuration (a runtime attachment can only
/// add, never unload).
/// With no shell it is a no-op — refilling is the ordinary ensure_pool_soon's job.
pub fn rebuild_pool_if_present(app: &AppHandle) {
    let app2 = app.clone();
    let _ = app.run_on_main_thread(move || {
        if take_pool_window(&app2).is_none() {
            return;
        }
        ensure_pool(&app2);
    });
}

/// Take the warm-up shell and close it; returns whether there really was one. The
/// slot and the window are disposed of together, and the rebuild and quit paths share
/// it.
fn take_pool_window(app: &AppHandle) -> Option<()> {
    let s = app.try_state::<AppState>()?;
    let taken = s.browser.0.lock().unwrap().pool.take();
    let (label, _) = taken?;
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.close();
    }
    Some(())
}

/// The warm-up shell goes when the main window is destroyed: a hidden window must not
/// outlive every visible one, or Tauri's "closing every window quits" would wait
/// forever (called from lib.rs's main-window Destroyed hook).
pub fn destroy_pool(app: &AppHandle) {
    take_pool_window(app);
}

/// Every browser window goes when the main window is destroyed (as does the detached
/// chat window, agent_window::force_close): since batch D made detached the default,
/// a browser window is no longer a child of the main window and would outlive it —
/// every relay from the clip toolbar would emit to a main that no longer exists, every
/// button would be mute, and the windows would still hold up "closing every window
/// quits". A paired toolbar closes through its browser window's Destroyed hook.
pub fn close_all(app: &AppHandle) {
    for (label, w) in app.webview_windows() {
        if label.starts_with("browser-") {
            let _ = w.close();
        }
    }
}

/// DNS prefetch: push the host name resolution into the system cache (on macOS
/// getaddrinfo goes through mDNSResponder and is shared system-wide), so WebKit's
/// network process skips the resolution step on the real connection that follows.
/// Only hosts that "look like a domain" (containing a dot) are taken — a half-typed ⌘L
/// input with a scheme filled in (`exam`, say) must not leak to DNS. The same host is
/// deduplicated within 30s: hammering the real resolver on every keystroke both
/// disturbs it and leaks privacy.
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
    // Through tauri's blocking pool rather than a std::thread::spawn each time:
    // arrowing through the history in ⌘L fires one for every host that survives the
    // debounce, and building a thread (stack reservation included) costs more than the
    // resolution itself
    tauri::async_runtime::spawn_blocking(move || {
        let _ = (host.as_str(), port).to_socket_addrs();
    });
}

/// The DNS prefetch for a pause in ⌘L input or a highlighted bookmark/history row
/// (called by the frontend after its debounce). Malformed input is ignored silently —
/// a prefetch has no visible consequence, and it never creates a window or a
/// navigation.
#[tauri::command]
pub fn browser_prefetch(url: String) {
    if let Ok(u) = normalize_url(&url) {
        if !is_app_origin(&u) {
            prefetch_dns(&u);
        }
    }
}

/// macOS: attach or detach a browser window as a native child of the main window
/// (`NSWindow addChildWindow:`).
/// Once attached it follows the main window's movement, minimisation and layering as a
/// group and does not appear separately in the window menu — the core of the
/// "pseudo-merge". Returns whether the attachment state really changed. It has to be
/// called on the main thread (the call sites: the main-thread closures of
/// build_browser_window and apply_dock_state, and the main window's event callbacks).
#[cfg(target_os = "macos")]
fn set_native_child(app: &AppHandle, label: &str, attach: bool) -> bool {
    use objc2_app_kit::{NSWindow, NSWindowButton};
    let changed = set_child_window(app, "main", label, attach);
    // In the floating traffic lights, minimise and zoom mean nothing for a docked
    // window, so only the red close button is kept and they are restored on detach.
    // This one is specific to "a browser window attached to the main window" and stays
    // out of the shared attachment primitive
    let Some(browser) = app.get_webview_window(label) else { return changed };
    let Ok(child_ptr) = browser.ns_window() else { return changed };
    let child_ns: &NSWindow = unsafe { &*child_ptr.cast() };
    for b in [NSWindowButton::MiniaturizeButton, NSWindowButton::ZoomButton] {
        if let Some(btn) = child_ns.standardWindowButton(b) {
            btn.setHidden(attach);
        }
    }
    changed
}

/// macOS: attach or detach `child_label` as a native child window of `parent_label`
/// (`NSWindow addChildWindow:`). Once attached it follows the parent's movement,
/// minimisation and layering as a group and does not appear separately in the window
/// menu; its position is entirely the code's to decide, so while attached it may not
/// be dragged away by its frame.
/// Idempotent; returns whether the attachment state really changed. It has to be
/// called on the main thread.
///
/// Shared by both attachments: a browser window onto the main window (the
/// "pseudo-merge" dock) and the clip toolbar onto a browser window.
#[cfg(target_os = "macos")]
pub(super) fn set_child_window(
    app: &AppHandle,
    parent_label: &str,
    child_label: &str,
    attach: bool,
) -> bool {
    use objc2_app_kit::{NSWindow, NSWindowOrderingMode};
    let Some(parent) = app.get_webview_window(parent_label) else { return false };
    let Some(child) = app.get_webview_window(child_label) else { return false };
    let (Ok(parent_ptr), Ok(child_ptr)) = (parent.ns_window(), child.ns_window()) else {
        return false;
    };
    let parent_ns: &NSWindow = unsafe { &*parent_ptr.cast() };
    let child_ns: &NSWindow = unsafe { &*child_ptr.cast() };
    let changed = if attach {
        // Already attached means no second attachment (every focus goes through a backstop re-attachment)
        let newly = child_ns.parentWindow().is_none();
        if newly {
            unsafe { parent_ns.addChildWindow_ordered(child_ns, NSWindowOrderingMode::Above) };
        }
        newly
    } else {
        let was_attached = child_ns.parentWindow().is_some();
        if was_attached {
            parent_ns.removeChildWindow(child_ns);
        }
        was_attached
    };
    child_ns.setExcludedFromWindowsMenu(attach);
    child_ns.setMovable(!attach);
    changed
}

#[cfg(not(target_os = "macos"))]
fn set_native_child(_app: &AppHandle, _label: &str, _attach: bool) -> bool {
    false
}

/// The backstop re-attachment when the main window is focused: on macOS minimising the
/// main window breaks the NSWindow child relation, so once it is back in front, every
/// browser window still marked docked is re-attached and repositioned (called from
/// lib.rs's main-window event hook, whose callbacks run on the main thread). For a
/// window that is still attached this is a zero-cost no-op.
pub fn reattach_docked(app: &AppHandle) {
    let labels: Vec<String> = {
        let Some(s) = app.try_state::<AppState>() else { return };
        let inner = s.browser.0.lock().unwrap();
        inner.windows.iter().filter(|(_, e)| e.docked).map(|(l, _)| l.clone()).collect()
    };
    for label in labels {
        if set_native_child(app, &label, true) {
            dock_window(app, &label, true);
        }
    }
}

/// The build geometry of a detached window: the position it was last closed at comes
/// first (cascaded by how many detached windows there already are, so a new one does
/// not completely cover an old one), otherwise it is centred at nearly full height in
/// the visible area (work_area, with the menu bar, Dock and taskbar already removed)
/// of the main window's screen. None when the main window or the monitors cannot be
/// read, and the caller falls back to builder.center().
fn independent_geom(
    app: &AppHandle,
    remembered: Option<crate::agent_window::WindowGeom>,
) -> Option<crate::agent_window::WindowGeom> {
    use crate::agent_window::WindowGeom;
    let cascade = app
        .try_state::<AppState>()
        .map(|s| s.browser.0.lock().unwrap().windows.len())
        .unwrap_or(0) as f64;
    let off = 28.0 * cascade;
    if let Some(g) = remembered {
        let g = WindowGeom { x: g.x + off, y: g.y + off, ..g };
        // The remembered geometry may belong to an external display that has been
        // unplugged: when the title bar lands on no monitor at all it is discarded in
        // favour of the "centred on the main window's screen" below — a window must
        // never open off-screen
        if grabbable_on_some_monitor(app, &g) {
            return Some(g);
        }
    }
    let main = app.get_webview_window("main")?;
    let monitor = main.current_monitor().ok()??;
    let scale = monitor.scale_factor();
    let wa = monitor.work_area();
    let pos = wa.position.to_logical::<f64>(scale);
    let size = wa.size.to_logical::<f64>(scale);
    let (x, y, w, h) = initial_browser_rect(pos.x, pos.y, size.width, size.height);
    Some(WindowGeom { x: x + off, y: y + off, w, h })
}

/// The usability check for a remembered geometry: the window's title bar (a point in
/// the middle of its top edge) landing inside any monitor's visible area counts as
/// grabbable — the user can at least drag it back. With no monitor list available it
/// does not block (better to trust the memory).
fn grabbable_on_some_monitor(app: &AppHandle, g: &crate::agent_window::WindowGeom) -> bool {
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
/// version); the height takes as much as it can — a window for looking things up gains
/// from vertical space (less scrolling) — capped at 1200 on a very tall screen so it
/// does not become a thin strip, with a floor on both axes for a very small screen.
fn initial_browser_rect(vx: f64, vy: f64, vw: f64, vh: f64) -> (f64, f64, f64, f64) {
    let w = 1080.0_f64.min(vw - 64.0).max(360.0);
    let h = (vh - 64.0).clamp(420.0, 1200.0);
    (vx + (vw - w) / 2.0, vy + (vh - h) / 2.0, w, h)
}

/// How wide a docked window has to be at least (logical px); below that it pins to the
/// screen's right edge instead (partly overlapping the main window).
const MIN_DOCK_WIDTH: f64 = 360.0;
/// How much visible width the main window keeps at least when a window pins to the
/// screen's right edge (the merged split view, logical px): the editor's content area
/// and the research panel (the clipping entry point) both have to stay usable. On a
/// small screen the browser window's width gives way to this — otherwise a 1080-wide
/// browser window by default would cover the main window down to a sliver, with the
/// clip button out of reach. It is matched to the sum of the frontend .layout flexible
/// columns' minimum widths plus the activity bar (tree 140 + content 300 + panel 280 +
/// activity bar 44 = 764, the same as tauri.conf.json's minWidth); changing it here
/// means changing all three.
const MIN_MAIN_VISIBLE: f64 = 804.0;

/// Pure geometry (all physical px): the space available to the right of the main
/// window, the browser window's current width, the main window's width and the
/// screen's width → (the dock width, whether it pins to the screen's right edge). When
/// pinned there, how much of the main window it covers is capped so the main window
/// keeps MIN_MAIN_VISIBLE; on a screen too small to hold both, the browser window
/// keeps its MIN_DOCK_WIDTH floor.
fn fit_dock_width(avail: u32, bwidth: u32, mwidth: u32, monitor_w: u32, scale: f64) -> (u32, bool) {
    let min_dock = (MIN_DOCK_WIDTH * scale).round() as u32;
    if avail >= min_dock {
        return (bwidth.min(avail), false);
    }
    let min_main = (MIN_MAIN_VISIBLE * scale).round() as u32;
    let max_width = avail + mwidth.saturating_sub(min_main);
    (bwidth.min(monitor_w).min(max_width.max(min_dock)), true)
}

/// The dock geometry: the main window's right edge, the same y, the same height; when
/// there is not enough room to the right it narrows or pins to the screen's right edge.
/// The third return value is how many physical pixels of the main window the browser
/// window covers (0 when they sit side by side, > 0 when the main window is full-screen
/// or the browser window is pinned to the screen's right edge, and the frontend moves
/// out of the way by it).
fn dock_geometry(
    app: &AppHandle,
    browser: &tauri::WebviewWindow,
) -> Option<(tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>, u32)> {
    let main = app.get_webview_window("main")?;
    if main.is_minimized().unwrap_or(false) {
        return None;
    }
    let mpos = main.outer_position().ok()?;
    let msize = main.outer_size().ok()?;
    let bsize = browser.outer_size().ok()?;
    let main_right = mpos.x + msize.width as i32;
    let mut x = main_right;
    let mut width = bsize.width;
    if let Ok(Some(monitor)) = main.current_monitor() {
        let right = monitor.position().x + monitor.size().width as i32;
        let avail = (right - x).max(0) as u32;
        // The threshold conversion uses the scale of "the main window's monitor":
        // avail/mwidth/monitor_w are all physical px in that monitor's coordinate
        // space. Under mixed DPI the browser window may still be on the old display
        // (Windows/Linux have no child-window following), and its own scale_factor
        // would convert MIN_DOCK/MIN_MAIN by an entirely wrong factor
        let scale = monitor.scale_factor();
        let (w, pin_right) =
            fit_dock_width(avail, bsize.width, msize.width, monitor.size().width, scale);
        width = w;
        if pin_right {
            x = (right - width as i32).max(monitor.position().x);
        }
    }
    let covered = (main_right - x).clamp(0, width as i32) as u32;
    Some((
        tauri::PhysicalPosition::new(x, mpos.y),
        tauri::PhysicalSize::new(width, msize.height),
        covered,
    ))
}

/// Dock a window against the main window's right edge. `resize` decides whether the
/// size is synced too (dragging the main window only moves it, which avoids the judder
/// of continuous resizing). On macOS a child window has already moved with its parent,
/// so the set_position here is usually a same-value no-op and only really takes effect
/// in the clamping case of pinning to the screen's right edge.
/// When the covered width (inset) changes a DockLayout is pushed and the frontend moves
/// out of the way — with the main window full-screen the browser window lies over its
/// right edge, and once the frontend gives way it becomes a merged split view.
pub(crate) fn dock_window(app: &AppHandle, label: &str, resize: bool) {
    let Some(webview) = app.get_webview_window(label) else { return };
    let Some((pos, size, covered)) = dock_geometry(app, &webview) else { return };
    if resize {
        let _ = webview.set_size(size);
    }
    let _ = webview.set_position(pos);
    let inset = covered as f64 / webview.scale_factor().unwrap_or(1.0);
    let mut changed = false;
    update_entry(app, label, |e| {
        if (e.inset - inset).abs() > 0.5 {
            e.inset = inset;
            changed = true;
        }
    });
    if changed {
        emit(app, &BrowserEvent::DockLayout { label: label.to_string(), inset });
    }
    // The dock geometry moved, so the toolbar follows the bottom edge (on macOS a child window follows a move but not a resize)
    super::clipbar::position_clipbar(app, label);
}

/// Bring every docked browser window along when the main window moves or resizes (called from lib.rs's main-window event hook).
pub fn redock_all(app: &AppHandle, resize: bool) {
    let labels: Vec<String> = {
        let Some(s) = app.try_state::<AppState>() else { return };
        let inner = s.browser.0.lock().unwrap();
        inner.windows.iter().filter(|(_, e)| e.docked).map(|(l, _)| l.clone()).collect()
    };
    for label in labels {
        dock_window(app, &label, resize);
    }
}

/// The dock switch: "detach" = take the child window off and restore the title bar, so
/// the window moves freely; docking again = remove the title bar, attach it as a child
/// window again and reposition it immediately.
#[tauri::command]
pub fn browser_set_docked(
    app: AppHandle,
    state: State<'_, AppState>,
    label: Option<String>,
    docked: bool,
) -> Result<(), String> {
    let label = state
        .browser
        .0
        .lock()
        .unwrap()
        .target(label.as_deref())
        .ok_or("no browser window is open")?;
    update_entry(&app, &label, |e| e.docked = docked);
    // The decoration switch and the child-window attachment run in order on the main
    // thread, so a styleMask change cannot interleave with an attach or detach
    let app2 = app.clone();
    let label2 = label.clone();
    app.run_on_main_thread(move || {
        apply_dock_state(&app2, &label2, docked);
        if !docked {
            // Snapshot the geometry at the moment of detaching (the decoration switch
            // has taken effect): a user who detaches and closes the window without
            // moving it still has a position stored; Moved/Resized keep refreshing it
            // afterwards
            if let Some(w) = app2.get_webview_window(&label2) {
                if let Some(g) = crate::agent_window::read_geom(&w) {
                    update_entry(&app2, &label2, |e| e.geom = Some(g));
                }
            }
        }
    })
    .map_err(|e| e.to_string())?;
    // "Remember the last choice": every toggle is persisted and the next window is built
    // from it (update: exclusive with the window hook's geometry write, see
    // settings::update)
    let _ = crate::settings::update(&app, |s| {
        let changed = s.browser_docked != docked;
        s.browser_docked = docked;
        changed
    });
    emit(&app, &BrowserEvent::DockChanged { label, docked });
    Ok(())
}

/// Main thread: apply the dock state to the window itself. The macOS movability and
/// traffic-light switching live in set_native_child; off macOS there is no
/// child-window mechanism, so it falls back to the decoration switch.
fn apply_dock_state(app: &AppHandle, label: &str, docked: bool) {
    apply_dock_chrome(app, label, docked);
    if docked {
        set_native_child(app, label, true);
        dock_window(app, label, true);
    } else {
        set_native_child(app, label, false);
    }
}

/// Main thread: switch the window decorations for the dock shape (the title bar part
/// alone, without attaching, detaching or positioning — claim_shell has to orchestrate
/// those separately in its "position first, then attach" build order).
fn apply_dock_chrome(app: &AppHandle, label: &str, docked: bool) {
    #[cfg(not(target_os = "macos"))]
    if let Some(webview) = app.get_webview_window(label) {
        let _ = webview.set_decorations(!docked);
    }
    // macOS: docked = a transparent full-width title bar (the no-title-bar shape);
    // detached = an ordinary title bar, a visible drag handle (under Overlay the
    // WKWebView swallows the drag and the window cannot be moved). The title text's
    // visibility has to be switched separately: a styleMask change does not affect
    // titleVisibility
    #[cfg(target_os = "macos")]
    if let Some(webview) = app.get_webview_window(label) {
        let _ = webview.set_title_bar_style(if docked {
            tauri::TitleBarStyle::Overlay
        } else {
            tauri::TitleBarStyle::Visible
        });
        if let Ok(ptr) = webview.ns_window() {
            use objc2_app_kit::{NSWindow, NSWindowTitleVisibility};
            let ns: &NSWindow = unsafe { &*ptr.cast() };
            ns.setTitleVisibility(if docked {
                NSWindowTitleVisibility::Hidden
            } else {
                NSWindowTitleVisibility::Visible
            });
        }
    }
}

/// Modify one registry entry inside the lock, with the closure's return value passed
/// through (None when the window is not in the registry) — "modify and read a field
/// back" needs no second trip through the lock.
fn update_entry<R>(
    app: &AppHandle,
    label: &str,
    f: impl FnOnce(&mut BrowserEntry) -> R,
) -> Option<R> {
    let s = app.try_state::<AppState>()?;
    let mut inner = s.browser.0.lock().unwrap();
    inner.windows.get_mut(label).map(f)
}

/// The target window (an explicit label, or the one focused last); an error when it does not exist.
pub(crate) fn target_webview(
    app: &AppHandle,
    state: &State<'_, AppState>,
    label: Option<String>,
) -> Result<tauri::WebviewWindow, String> {
    let label = state
        .browser
        .0
        .lock()
        .unwrap()
        .target(label.as_deref())
        .ok_or("no browser window is open")?;
    app.get_webview_window(&label).ok_or_else(|| "the browser window is closed".into())
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
    // The webview carries its own history; with none this is a no-op (v0.4 accepts the degradation of not knowing canGoBack)
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

#[tauri::command]
pub fn browser_close(app: AppHandle, state: State<'_, AppState>, label: Option<String>) -> Result<(), String> {
    // The state cleanup happens through the Destroyed event; this only initiates the close
    target_webview(&app, &state, label)?.close().map_err(|e| e.to_string())
}

/// Focus a given browser window (active is updated through the Focused window event).
#[tauri::command]
pub fn browser_focus(app: AppHandle, state: State<'_, AppState>, label: String) -> Result<(), String> {
    target_webview(&app, &state, Some(label))?.set_focus().map_err(|e| e.to_string())
}

/// A backstop full-state fetch for when the frontend starts or reloads (a snapshot outside the event stream).
#[tauri::command]
pub fn browser_state(state: State<'_, AppState>) -> BrowserSnapshot {
    let inner = state.browser.0.lock().unwrap();
    let mut windows: Vec<BrowserWindowView> = inner
        .windows
        .iter()
        .map(|(label, e)| BrowserWindowView { label: label.clone(), entry: e.clone() })
        .collect();
    // The open order, matching the frontend's event-stream registry (a label's string order goes wrong past window 10)
    windows.sort_by_key(|w| w.entry.id);
    BrowserSnapshot { windows, active: inner.active.clone() }
}

/// The single destination of an http(s) link click: it branches by the setting into the
/// reading projection, the built-in browser or the system browser. The editor, the AI
/// panel and the source cards all come through here. `invert=true` (⌘click) gives "the
/// other way of opening it": Reading/System → Internal, Internal → System — so ⌘click
/// always produces a real browser window (the projection's escape-hatch semantics).
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
        // signal, and the frontend calls page_read back for the content.
        // The funnel is still the only entry point — every link click branches inside
        // this one match.
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

    #[test]
    fn dock_width_small_screen_keeps_main_usable() {
        // A 14" laptop screen (1512 logical wide, scale 2): the main window is 1400
        // logical wide, leaving only 112 logical to the right.
        // It pins to the right edge, but the main window keeps ≥804 logical visible —
        // what it covers is w - avail
        let (w, pin) = fit_dock_width(224, 2160, 2800, 3024, 2.0);
        assert!(pin);
        assert!(2800 - (w as i32 - 224) >= 1608, "main window visible {}", 2800 - (w as i32 - 224));
        assert!(w >= 720, "browser window floor width {w}");
        // A roomy screen: docked side by side, the width unchanged
        assert_eq!(fit_dock_width(2400, 2160, 2800, 5120, 2.0), (2160, false));
        // Side by side but slightly narrow on the right: narrowed to the available space
        assert_eq!(fit_dock_width(1800, 2160, 2800, 5120, 2.0), (1800, false));
        // A very small screen (the main window itself is under 640 logical): the browser window keeps the MIN_DOCK_WIDTH floor
        assert_eq!(fit_dock_width(0, 2160, 1200, 1600, 2.0), (720, true));
    }
}
