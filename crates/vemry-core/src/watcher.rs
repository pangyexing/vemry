//! Filesystem watching with debounce. Emits batches of changed paths; the
//! caller (Tauri layer) feeds them to `Vault::handle_fs_change`.

use notify::{RecursiveMode, Watcher};
use notify_debouncer_full::{new_debouncer, DebounceEventResult};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::{Result, VemryError};

pub struct VaultWatcher {
    // Kept alive for the watcher's lifetime; dropping stops watching.
    _debouncer: notify_debouncer_full::Debouncer<notify::RecommendedWatcher, notify_debouncer_full::FileIdMap>,
}

pub fn watch(root: &Path, on_change: impl Fn(Vec<PathBuf>) + Send + 'static) -> Result<VaultWatcher> {
    let mut debouncer = new_debouncer(Duration::from_millis(500), None, move |res: DebounceEventResult| {
        if let Ok(events) = res {
            let mut paths: Vec<PathBuf> = events.into_iter().flat_map(|e| e.event.paths.clone()).collect();
            paths.sort();
            paths.dedup();
            if !paths.is_empty() {
                on_change(paths);
            }
        }
    })
    .map_err(|e| VemryError::Watch(e.to_string()))?;

    debouncer
        .watcher()
        .watch(root, RecursiveMode::Recursive)
        .map_err(|e| VemryError::Watch(e.to_string()))?;
    debouncer.cache().add_root(root, RecursiveMode::Recursive);

    Ok(VaultWatcher { _debouncer: debouncer })
}
