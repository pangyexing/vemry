//! The built-in knowledge browser (v0.4). A remote page opens in its own
//! `browser-N` window — that label matches no capability, so remote content gets
//! not one IPC; the main window's control panel keeps the address, title and focus
//! state in sync through `browser://event`. By default it opens as a detached,
//! centred window (batch D); once "snapped" it loses its title bar and is attached
//! as a native child of the main window (macOS, the "pseudo-merge", v0.5.2), and
//! the shape switch remembers the last choice through settings.browser_docked. The
//! main window's page card and the clip toolbar (clipbar) at the bottom of the
//! browser window are two entry points that coexist.

pub mod adblock;
pub mod assets;
pub mod bookmarks;
pub mod clip;
pub mod clipbar;
pub mod history;
pub mod reader;
#[cfg(target_os = "macos")]
pub mod wheel;
pub mod window;

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter};

/// The browser window registry. A std Mutex: every access completes synchronously
/// once the lock is held and never spans an await.
#[derive(Default)]
pub struct BrowserState(pub Mutex<BrowserInner>);

#[derive(Default)]
pub struct BrowserInner {
    pub windows: HashMap<String, BrowserEntry>,
    /// The browser window focused last — the default target of the control panel and the clip commands.
    pub active: Option<String>,
    pub next_id: u32,
    /// The warm-up shell window (label, id): a hidden about:blank browser window
    /// whose WKWebView, subprocess and ad-blocking rules are already in place when
    /// it was built, so browser_open only has to navigate and show it once it
    /// claims it. It is not in the windows registry (as far as the UI is concerned
    /// it does not exist yet), and there is at most one.
    pub pool: Option<(String, u32)>,
}

impl BrowserInner {
    /// The target window of a clip or navigation command: an explicit label first, otherwise the one focused last.
    pub fn target(&self, label: Option<&str>) -> Option<String> {
        match label {
            Some(l) if self.windows.contains_key(l) => Some(l.to_string()),
            Some(_) => None,
            None => self.active.clone().filter(|l| self.windows.contains_key(l)),
        }
    }
}

// Default is not derived: an id has no meaningful default, and the id=0 Default
// would produce sorts before every real window in the active-window succession
// (min_by_key) and in the snapshot ordering.
#[derive(Clone, Debug, Serialize)]
pub struct BrowserEntry {
    /// The open sequence number (the N of the label `browser-N`). The authority on
    /// open order: the active-window succession on close and the snapshot ordering
    /// both follow it rather than parsing the label string again.
    #[serde(skip)]
    pub id: u32,
    /// Snapped to the main window's right edge with no title bar; on macOS it is
    /// attached as a native child of the main window and follows it.
    /// False after pressing "detach" on the page card (the title bar comes back and
    /// it can be placed freely).
    pub docked: bool,
    /// How much of the main window's right edge the docked window covers (logical
    /// px, 0 = side by side without overlap). With the main window full-screen or
    /// against the screen's right edge, the browser window can only sit on top of
    /// it, and the frontend moves out of the way by this much (a merged split view).
    pub inset: f64,
    pub url: String,
    pub title: String,
    /// A detached window's most recent normal-state geometry (refreshed on
    /// Moved/Resized; written to settings.browser_window on Destroyed). A docked
    /// window is not recorded — its position is driven by redock.
    #[serde(skip)]
    pub geom: Option<crate::agent_window::WindowGeom>,
}

/// The payload of `browser://event`. A tagged union, mirrored one by one in the frontend's api.ts.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BrowserEvent {
    /// docked travels with the event: the shape a window is built in is decided by
    /// settings.browser_docked, so the frontend registry no longer hardcodes a
    /// default (the source of truth is in Rust alone).
    Opened { label: String, url: String, docked: bool },
    Navigated { label: String, url: String },
    TitleChanged { label: String, title: String },
    Focused { label: String },
    Closed { label: String },
    DockChanged { label: String, docked: bool },
    /// The dock layout changed: inset = how many logical pixels of the main window's right edge the browser window covers.
    DockLayout { label: String, inset: f64 },
    AdblockStatus { status: adblock::AdblockStatus },
    /// A bookmark change broadcast carrying the full snapshot (the ★ mirrors of the
    /// main window's page card and of the clipbar are replaced wholesale, so
    /// neither has to fetch again). Non-window state riding on browser://event, as
    /// AdblockStatus did first.
    BookmarksChanged { bookmarks: Vec<bookmarks::Bookmark> },
}

pub fn emit(app: &AppHandle, event: &BrowserEvent) {
    let _ = app.emit("browser://event", event);
}

/// "Wait ms milliseconds, then run on the main thread". Building a WKWebView (the
/// warm-up shell window, the clip toolbar) needs the main thread, and doing it
/// right at a page's first frame slows down what is being rendered — both delayed
/// window builds share this one waiting primitive.
pub fn on_main_after(app: &AppHandle, ms: u64, f: impl FnOnce(&AppHandle) + Send + 'static) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
        let app2 = app.clone();
        let _ = app.run_on_main_thread(move || f(&app2));
    });
}
