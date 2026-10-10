//! Shared reading and writing of the few small JSON files under app_config_dir
//! (settings.json / browser_history.json / bookmarks.json). All three used to
//! copy out the same "find the config dir → read and parse → create the parent
//! directory → to_string_pretty → write", which scattered the on-disk invariants
//! (create the parent first, fall back to the default on a parse failure, report
//! one wording for a missing directory) across three files that drifted apart;
//! collected here, they are maintained in one place.
//!
//! Each module's own in-process CACHE stays out of this — their types all differ,
//! and going generic for it would only make something simple complicated. This
//! handles the path, the serialisation and the write, and nothing else.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

pub fn path(app: &AppHandle, name: &str) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join(name))
}

/// Read one config file. A missing file or a parse failure always falls back to
/// the default — neither a first start nor JSON broken by hand should blow up (the
/// semantics of all three implementations before they were collected here).
pub fn read<T: DeserializeOwned + Default>(app: &AppHandle, name: &str) -> T {
    path(app, name)
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Write to a given path (creating the parent directory as needed). The history's
/// single background writer thread holds a path and writes to it directly, which
/// is why this takes a path rather than a name.
pub fn write<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, json).map_err(|e| e.to_string())
}

/// Write by name (the ordinary synchronous path).
pub fn save<T: Serialize + ?Sized>(app: &AppHandle, name: &str, value: &T) -> Result<(), String> {
    write(&path(app, name).ok_or("no config dir")?, value)
}

/// Write by name with the file readable by its owner alone (mode 0600 on Unix;
/// Windows has no mode bits and the app-config directory is per user anyway).
/// For the files that hold a credential: account.json (the device token) and
/// settings.json (the user's own API key).
pub fn save_private<T: Serialize + ?Sized>(app: &AppHandle, name: &str, value: &T) -> Result<(), String> {
    let p = path(app, name).ok_or("no config dir")?;
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    // Created with the mode from the start (rather than written and then chmod'ed),
    // so the token never sits in a world-readable file even for a moment; an
    // existing file's mode is fixed as well.
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut file = opts.open(&p).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    use std::io::Write;
    file.write_all(json.as_bytes()).map_err(|e| e.to_string())
}
