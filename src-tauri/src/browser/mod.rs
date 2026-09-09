//! The built-in knowledge browser (v0.4; the same-window panel since v0.9). A
//! remote page is a **child webview** labelled `browser-N` — that label matches no
//! capability, so remote content gets not one IPC. Every page is a tab of one
//! browser surface which has two shapes: **docked**, the tabs are children of the
//! main window and sit in a panel column at its right edge whose chrome (tab strip,
//! address row, clip and AI entry points) is the main window's own DOM; and
//! **detached**, the tabs are moved (`Webview::reparent`, no reload) into a
//! standalone window `browserwin` that has no webview of its own, with a
//! `browserbar` child webview running our bundle as the chrome at its top. The
//! switch remembers the last choice through settings.browser_docked. The frontend
//! keeps the tab list, the active tab and the shape in sync through
//! `browser://event`; the source of truth is the `BrowserState` registry here.

pub mod adblock;
pub mod pagetr;
pub mod assets;
pub mod bookmarks;
pub mod browserbar;
pub mod clip;
pub mod history;
pub mod reader;
#[cfg(target_os = "macos")]
pub mod wheel;
pub mod window;

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};

/// The browser tab registry. A std Mutex: every access completes synchronously
/// once the lock is held and never spans an await.
#[derive(Default)]
pub struct BrowserState(pub Mutex<BrowserInner>);

/// Which window the tabs live in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    /// Children of the main window, laid out in the panel column the frontend reports.
    #[default]
    Docked,
    /// Children of the standalone `browserwin` window, under its `browserbar` chrome.
    Detached,
}

/// The docked panel's geometry as the frontend reports it (logical px): the tabs
/// are native views laid over the main webview, so Rust has to be told where the
/// panel column is and how tall its chrome is; on a window resize the bounds are
/// recomputed from these anchors without a round trip to the frontend.
#[derive(Clone, Copy, Debug)]
pub struct PanelLayout {
    /// The panel column is on screen (false = collapsed; the tabs stay alive, hidden).
    pub visible: bool,
    pub width: f64,
    pub chrome_h: f64,
    /// A full-window overlay (palette, dialog, lightbox) is open in the main window:
    /// the active tab is hidden underneath it, because a native view would cover it.
    pub veiled: bool,
}

impl Default for PanelLayout {
    fn default() -> Self {
        PanelLayout { visible: false, width: 420.0, chrome_h: window::BAR_H, veiled: false }
    }
}

#[derive(Default)]
pub struct BrowserInner {
    pub tabs: HashMap<String, BrowserEntry>,
    /// The tab shown (the only visible one); the default target of the clip commands.
    pub active: Option<String>,
    pub next_id: u32,
    /// The warm-up shell (label, id): a hidden about:blank child webview of the main
    /// window whose WKWebView, subprocess and ad-blocking rules are already in place,
    /// so opening a page only has to navigate and show it. It is not in the tab
    /// registry (as far as the UI is concerned it does not exist yet), and there is
    /// at most one.
    pub pool: Option<(String, u32)>,
    pub shape: Shape,
    pub panel: PanelLayout,
    /// The detached window's most recent normal-state geometry (refreshed on
    /// Moved/Resized; written to settings.browser_window on Destroyed).
    pub detached_geom: Option<crate::agent_window::WindowGeom>,
    /// The chrome bar of the detached window is expanded (the destination picker is open).
    pub bar_expanded: bool,
    /// label → the native view pointer of a tab's webview (macOS: to tell whether
    /// the keyboard focus is inside a page, for the menu bar's routing).
    pub views: HashMap<String, usize>,
}

impl BrowserInner {
    /// The target of a clip or navigation command: an explicit label first, otherwise the active tab.
    pub fn target(&self, label: Option<&str>) -> Option<String> {
        match label {
            Some(l) if self.tabs.contains_key(l) => Some(l.to_string()),
            Some(_) => None,
            None => self.active.clone().filter(|l| self.tabs.contains_key(l)),
        }
    }
}

// Default is not derived: an id has no meaningful default, and the id=0 Default
// would sort before every real tab in the active-tab succession and the snapshot
// ordering.
#[derive(Clone, Debug, Serialize)]
pub struct BrowserEntry {
    /// The open sequence number (the N of the label `browser-N`). The authority on
    /// tab order: the snapshot ordering and the active-tab succession on close both
    /// follow it rather than parsing the label string again.
    #[serde(skip)]
    pub id: u32,
    pub url: String,
    pub title: String,
}

/// The payload of `browser://event`. A tagged union, mirrored one by one in the frontend's api.ts.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserEvent {
    Opened { label: String, url: String },
    Navigated { label: String, url: String },
    TitleChanged { label: String, title: String },
    /// The tab shown changed (a click on the tab strip, a new tab, the succession after a close).
    Activated { label: String },
    Closed { label: String },
    ShapeChanged { shape: Shape },
    AdblockStatus { status: adblock::AdblockStatus },
    /// A bookmark change broadcast carrying the full snapshot (the ★ mirrors of the
    /// main window's chrome and of the detached bar are replaced wholesale, so
    /// neither has to fetch again). Non-tab state riding on browser://event, as
    /// AdblockStatus did first.
    BookmarksChanged { bookmarks: Vec<bookmarks::Bookmark> },
    /// A tab's in-page translation: on or off, how many of its blocks have their
    /// translation, and the tokens spent on it (the chrome's button reads it).
    Translate { label: String, active: bool, done: usize, total: usize, tokens: u64 },
}

pub fn emit(app: &AppHandle, event: &BrowserEvent) {
    // A closed tab takes its in-page translation with it (every close funnels through this event)
    if let BrowserEvent::Closed { label } = event {
        if let Some(state) = app.try_state::<crate::AppState>() {
            pagetr::tab_closed(&state, label);
        }
    }
    let _ = app.emit("browser://event", event);
}

/// "Wait ms milliseconds, then run on the main thread". Building a WKWebView (the
/// warm-up shell) needs the main thread, and doing it right at a page's first frame
/// slows down what is being rendered.
pub fn on_main_after(app: &AppHandle, ms: u64, f: impl FnOnce(&AppHandle) + Send + 'static) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || f(&app2));
    });
}
