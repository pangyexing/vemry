//! Application-level browsing history, stored in
//! app_config_dir/browser_history.json — shared across vaults and kept out of the
//! vault (the same decision as settings: a vault gets synced and committed).
//!
//! Single-writer discipline: every change goes through this module's record_visit /
//! amend_title / clear alone, and the write points are the browser window's
//! navigation hooks (build_browser_window) and the clear command; the hidden reader
//! window has no such hooks and therefore never enters the history. Deduplication
//! means merging the same URL to the front globally (the same retain+insert(0) as
//! recent_vaults): the list holds only unique URLs, "the last page" is list[0], a
//! title amendment matches a URL uniquely, and the ⌘L list needs no second
//! deduplication. The price is giving up the exact visit ordering — this is "places
//! I have been recently", not a forensic log.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::config_store;

const FILE: &str = "browser_history.json";

/// Capped at 500 entries (~75KB of JSON). Every change is written straight through
/// with no debounce: serialising at most twice per page load (Navigated plus the
/// title amendment) takes well under a millisecond, and debouncing would be
/// over-engineering.
pub const HISTORY_CAP: usize = 500;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub url: String,
    #[serde(default)]
    pub title: String,
    /// The most recent visit, in unix milliseconds.
    #[serde(default)]
    pub ts: i64,
    #[serde(default = "one")]
    pub visits: u32,
}

fn one() -> u32 {
    1
}

/// An in-process cache (the same as settings::CACHE): the navigation hook has to
/// read-modify-write on every page change and should not read and parse the disk
/// each time. with_cache is the only way in or out, and reading and writing both
/// happen inside its lock.
static CACHE: std::sync::RwLock<Option<Vec<HistoryEntry>>> = std::sync::RwLock::new(None);

pub fn list(app: &AppHandle) -> Vec<HistoryEntry> {
    with_cache(app, |l| l.clone())
}

/// Modify in place inside the cache (filling it from disk when empty), with the
/// closure's return value passed through. **Everything** goes through here: the
/// navigation hook takes two beats per page (visit and title), which saves cloning
/// the whole table twice; delete and clear take the same lock, rather than "clone →
/// modify the clone → overwrite the cache with a stale snapshot" (which would eat
/// any visit that arrived while cloning).
fn with_cache<R>(app: &AppHandle, f: impl FnOnce(&mut Vec<HistoryEntry>) -> R) -> R {
    let mut guard = CACHE.write().unwrap();
    let list = guard.get_or_insert_with(|| config_store::read(app, FILE));
    f(list)
}

/// The file-write serialisation lock: the synchronous path (clear/remove) and the
/// background writer thread both write the same browser_history.json, and two
/// interleaved fs::writes would tear the file; worse, the background thread holding
/// a pre-clear snapshot and landing a beat later would write the history the user
/// just cleared back to disk (a privacy problem). Every writer follows "take the
/// lock → take the snapshot as of now → write": what reaches disk is always the
/// cache as it stands at the moment of writing, so neither a stale overwrite nor an
/// interleaving can happen.
static FS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Take the snapshot as of now under FS_LOCK and write it (with the cache never
/// filled there is nothing to write, which is Ok).
fn write_current(path: &std::path::Path) -> Result<(), String> {
    let _fs = FS_LOCK.lock().unwrap();
    let snapshot = CACHE.read().unwrap().clone();
    match snapshot {
        Some(list) => config_store::write(path, &list),
        None => Ok(()),
    }
}

/// Modify inside the lock and then write synchronously (used by explicit user
/// actions like clear/remove: a privacy operation does not get a "write it later").
/// The write takes the latest snapshot as of that moment under FS_LOCK, exclusive
/// with the background thread.
fn mutate_and_save(
    app: &AppHandle,
    f: impl FnOnce(&mut Vec<HistoryEntry>) -> bool,
) -> Result<(), String> {
    let changed = with_cache(app, f);
    if !changed {
        return Ok(());
    }
    let path = config_store::path(app, FILE).ok_or("no config dir")?;
    write_current(&path)
}

/// The background single-writer thread: the navigation hook (WKWebView's
/// decidePolicy) waits synchronously for the callback to return, and serialising and
/// writing should not sit on its critical path. The writer thread naps first to
/// merge a page's visit and title beats, then writes "the cache snapshot as of now"
/// — a serialised single writer, so an out-of-order overwrite cannot happen. The
/// price: a visit in the last ~50ms before the process exits may not reach disk,
/// which a "places I have been recently" list can live with.
fn schedule_flush(app: &AppHandle) {
    use std::sync::mpsc::Sender;
    use std::sync::OnceLock;
    // The path is an invariant of the process: taken once when the thread is built rather than re-sent with each message
    static TX: OnceLock<Sender<()>> = OnceLock::new();
    let Some(path) = config_store::path(app, FILE) else { return };
    let tx = TX.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                std::thread::sleep(std::time::Duration::from_millis(50));
                while rx.try_recv().is_ok() {}
                // The snapshot is taken inside FS_LOCK: exclusive with the
                // synchronous writes of clear/remove, so a stale snapshot cannot
                // overwrite a file that was just cleared
                let _ = write_current(&path);
            }
        });
        tx
    });
    let _ = tx.send(());
}

/// Merge to the front: an existing entry for the same URL is moved to the head
/// (visits+1, the title kept), otherwise a new entry is inserted at the head;
/// anything beyond the cap is truncated from the tail.
fn push_visit(list: &mut Vec<HistoryEntry>, url: &str, now_ms: i64) {
    if let Some(pos) = list.iter().position(|e| e.url == url) {
        let mut e = list.remove(pos);
        e.ts = now_ms;
        e.visits = e.visits.saturating_add(1);
        list.insert(0, e);
    } else {
        list.insert(
            0,
            HistoryEntry { url: url.to_string(), title: String::new(), ts: now_ms, visits: 1 },
        );
        list.truncate(HISTORY_CAP);
    }
}

/// The title amendment (url and title arrive from the webview a beat apart): a blank
/// title is skipped, and a URL that is not in the history is a no-op (it was just
/// cleared, say). Returns whether anything actually changed — when the title matches
/// the stored one exactly, the caller can skip a write.
fn set_title(list: &mut [HistoryEntry], url: &str, title: &str) -> bool {
    let title = title.trim();
    if title.is_empty() {
        return false;
    }
    match list.iter_mut().find(|e| e.url == url) {
        Some(e) if e.title != title => {
            e.title = title.to_string();
            true
        }
        _ => false,
    }
}

/// Called by the navigation hook: record one visit. It touches the in-memory cache
/// only and leaves the write to the background single writer; a failure is swallowed
/// silently — the history must never block navigation.
pub fn record_visit(app: &AppHandle, url: &str) {
    with_cache(app, |l| push_visit(l, url, chrono::Utc::now().timestamp_millis()));
    schedule_flush(app);
}

/// Called by the TitleChanged hook: amend the most recent entry's title. It writes
/// through the background thread like record_visit — but a title that did not really
/// change schedules no write: a page with a live title (an unread count, a clock, a
/// countdown) pushes the same string over and over, and rewriting all 75KB of the
/// history for each would be far too expensive.
pub fn amend_title(app: &AppHandle, url: &str, title: &str) {
    if with_cache(app, |l| set_title(l, url, title)) {
        schedule_flush(app);
    }
}

/// Where "open the browser" lands: the most recently visited page. Only the first entry is needed, so the table is not cloned.
pub fn last_url(app: &AppHandle) -> Option<String> {
    with_cache(app, |l| l.first().map(|e| e.url.clone()))
}

#[tauri::command]
pub fn browser_history_list(app: AppHandle) -> Vec<HistoryEntry> {
    list(&app)
}

/// The "delete" inside a ⌘L list row: remove one entry by URL (one that does not exist counts as done, not an error).
#[tauri::command]
pub fn browser_history_remove(app: AppHandle, url: String) -> Result<(), String> {
    mutate_and_save(&app, |l| {
        let before = l.len();
        l.retain(|e| e.url != url);
        l.len() != before
    })
}

#[tauri::command]
pub fn browser_history_clear(app: AppHandle) -> Result<(), String> {
    mutate_and_save(&app, |l| {
        let had = !l.is_empty();
        l.clear();
        had
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(url: &str, title: &str, ts: i64, visits: u32) -> HistoryEntry {
        HistoryEntry { url: url.into(), title: title.into(), ts, visits }
    }

    #[test]
    fn push_visit_promotes_and_merges_same_url() {
        let mut l = vec![entry("https://a/", "A", 1, 1), entry("https://b/", "B", 2, 3)];
        push_visit(&mut l, "https://b/", 9);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].url, "https://b/");
        assert_eq!(l[0].title, "B"); // merging keeps the old title until the amendment overwrites it
        assert_eq!(l[0].ts, 9);
        assert_eq!(l[0].visits, 4);
        assert_eq!(l[1].url, "https://a/");
    }

    #[test]
    fn push_visit_inserts_new_at_front_and_caps() {
        let mut l: Vec<HistoryEntry> =
            (0..HISTORY_CAP).map(|i| entry(&format!("https://n{i}/"), "", 0, 1)).collect();
        push_visit(&mut l, "https://new/", 5);
        assert_eq!(l.len(), HISTORY_CAP);
        assert_eq!(l[0].url, "https://new/");
        assert_eq!(l[0].visits, 1);
        // What is truncated from the tail is the oldest entry
        assert!(!l.iter().any(|e| e.url == format!("https://n{}/", HISTORY_CAP - 1)));
    }

    #[test]
    fn set_title_matches_url_skips_blank_and_missing() {
        let mut l = vec![entry("https://a/", "", 1, 1), entry("https://b/", "old", 2, 1)];
        assert!(set_title(&mut l, "https://a/", "  Hello  "));
        assert_eq!(l[0].title, "Hello");
        assert!(!set_title(&mut l, "https://b/", "   "));
        assert_eq!(l[1].title, "old"); // blank is skipped
        assert!(!set_title(&mut l, "https://gone/", "X")); // no match is a no-op, not a panic
        // The same title pushed again: not a change, so the caller can skip writing the whole table
        assert!(!set_title(&mut l, "https://a/", "Hello"));
    }

    /// An old file missing fields takes the defaults (empty title, ts 0, visits 1) instead of blowing up deserialisation.
    #[test]
    fn serde_missing_fields_get_defaults() {
        let l: Vec<HistoryEntry> =
            serde_json::from_str(r#"[{"url":"https://a/"}]"#).unwrap();
        assert_eq!(l[0].title, "");
        assert_eq!(l[0].ts, 0);
        assert_eq!(l[0].visits, 1);
    }
}
