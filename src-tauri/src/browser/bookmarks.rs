//! Bookmarked addresses, stored in app_config_dir/bookmarks.json — application-level
//! bookmarks, shared across vaults and kept out of the vault ("research material of
//! this vault" is what a clips/ note is for; these are ordinary browser bookmarks).
//! The single writer is this module's toggle/remove commands; after a successful
//! change a full BookmarksChanged snapshot is broadcast through browser://event, so
//! the ★ mirrors of the main window's page card and of the clip toolbar stay in
//! sync without each fetching for itself.

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::{emit, BrowserEvent};
use crate::config_store;

const FILE: &str = "bookmarks.json";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bookmark {
    pub url: String,
    #[serde(default)]
    pub title: String,
    /// When it was bookmarked, in unix milliseconds.
    #[serde(default)]
    pub added: i64,
}

/// An in-process cache (the same as settings::CACHE). with_cache is the only way in
/// or out, and reading and writing both happen inside its lock.
static CACHE: std::sync::RwLock<Option<Vec<Bookmark>>> = std::sync::RwLock::new(None);

pub fn list(app: &AppHandle) -> Vec<Bookmark> {
    with_cache(app, |l| l.clone())
}

/// Read and write in place inside the cache (filling it from disk when empty), with
/// the closure's return value passed through. The same as history's.
fn with_cache<R>(app: &AppHandle, f: impl FnOnce(&mut Vec<Bookmark>) -> R) -> R {
    let mut guard = CACHE.write().unwrap();
    let list = guard.get_or_insert_with(|| config_store::read(app, FILE));
    f(list)
}

/// Modify inside the lock, take the snapshot inside the lock, and write to disk plus
/// broadcast the full snapshot outside it (the mirrors of both webviews are replaced
/// wholesale). The snapshot is taken inside the lock, so what reaches disk is
/// certainly what was just modified; a closure returning None means nothing changed
/// and both the write and the broadcast are skipped; with Some(r), r is passed
/// straight through (toggle has to give the new state back to the frontend without
/// smuggling it out through a captured variable).
fn mutate_and_broadcast<R>(
    app: &AppHandle,
    f: impl FnOnce(&mut Vec<Bookmark>) -> Option<R>,
) -> Result<Option<R>, String> {
    let Some((r, list)) = with_cache(app, |l| f(l).map(|r| (r, l.clone()))) else {
        return Ok(None);
    };
    config_store::save(app, FILE, &list)?;
    emit(app, &BrowserEvent::BookmarksChanged { bookmarks: list });
    Ok(Some(r))
}

#[tauri::command]
pub fn bookmark_list(app: AppHandle) -> Vec<Bookmark> {
    list(&app)
}

/// Pure logic: flip one bookmark in the list and return the new state (true =
/// bookmarked). A new one is inserted at the front; the URL is matched exactly
/// against the registry's entry.url (the same rule as the frontend's isBookmarked).
fn toggle_in(list: &mut Vec<Bookmark>, url: &str, title: &str, now_ms: i64) -> bool {
    if let Some(pos) = list.iter().position(|b| b.url == url) {
        list.remove(pos);
        false
    } else {
        list.insert(
            0,
            Bookmark { url: url.to_string(), title: title.trim().to_string(), added: now_ms },
        );
        true
    }
}

/// Bookmark or unbookmark (the single entry point of the ☆ button); returns the new state (true = bookmarked).
#[tauri::command]
pub fn bookmark_toggle(app: AppHandle, url: String, title: String) -> Result<bool, String> {
    // A blank tab (the palette's entry point reads the active tab without looking) has no page to keep
    if url == super::window::BLANK {
        return Err("the tab has no page yet".into());
    }
    let starred = mutate_and_broadcast(&app, |l| {
        Some(toggle_in(l, &url, &title, chrono::Utc::now().timestamp_millis()))
    })?;
    Ok(starred.unwrap_or(false))
}

/// The "unbookmark" inside a ⌘L list row. A URL that is not in the list counts as done, not an error.
#[tauri::command]
pub fn bookmark_remove(app: AppHandle, url: String) -> Result<(), String> {
    mutate_and_broadcast(&app, |l| {
        let before = l.len();
        l.retain(|b| b.url != url);
        (l.len() != before).then_some(())
    })
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_round_trips_and_inserts_at_front() {
        let mut l = vec![Bookmark { url: "https://old/".into(), title: "旧".into(), added: 1 }];
        assert!(toggle_in(&mut l, "https://a/", "  A  ", 9));
        assert_eq!(l[0].url, "https://a/");
        assert_eq!(l[0].title, "A"); // the title is trimmed
        assert_eq!(l[0].added, 9);
        assert_eq!(l.len(), 2);
        // Toggling the same URL again unbookmarks it, and the list is back as it was
        assert!(!toggle_in(&mut l, "https://a/", "A", 10));
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].url, "https://old/");
    }

    /// An old file missing fields takes the defaults instead of blowing up deserialisation.
    #[test]
    fn serde_missing_fields_get_defaults() {
        let l: Vec<Bookmark> = serde_json::from_str(r#"[{"url":"https://a/"}]"#).unwrap();
        assert_eq!(l[0].title, "");
        assert_eq!(l[0].added, 0);
    }
}
