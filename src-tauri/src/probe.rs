//! The development probe. A debug build started with `VEMRY_PROBE=<name>` (`pnpm tauri
//! dev` with the variable set) runs a scripted pass over one feature inside the real app:
//! the frontend drives its real stores, IPC and DOM (`src/lib/probe/`), and every check
//! is printed here, to the terminal that launched it, as `[probe:<window>] …`. Nothing
//! runs otherwise, and a release build has no probe at all — `plan` is None there, and
//! every command below answers as if there were nothing to do.

use tauri::{AppHandle, Window};

/// The probe asked for (a comma-separated list of names), in a debug build alone.
pub fn plan() -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::env::var("VEMRY_PROBE").ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

#[tauri::command]
pub fn probe_plan() -> Option<String> {
    plan()
}

/// One line of the probe's report, tagged with the window it came from.
#[tauri::command]
pub fn probe_report(window: Window, line: String) {
    if plan().is_some() {
        eprintln!("[probe:{}] {line}", window.label());
    }
}

/// The top-level keys of settings.json as it is on disk — the names alone, never a
/// value (the file holds API keys).
#[tauri::command]
pub fn probe_settings_keys(app: AppHandle) -> Vec<String> {
    if plan().is_none() {
        return Vec::new();
    }
    crate::config_store::path(&app, "settings.json")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&s).ok())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default()
}

/// A path for a scratch vault: under the system's temporary directory, not there yet
/// (vault_create wants a new or empty folder). The probe works in it so the user's own
/// vaults are never written to.
#[tauri::command]
pub fn probe_scratch_path() -> Result<String, String> {
    if plan().is_none() {
        return Err("no probe".into());
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let path = std::env::temp_dir().join(format!("vemry-probe-{stamp}"));
    Ok(path.to_string_lossy().into_owned())
}

/// The end of a pass: the tally, and — when the plan says `exit` — the app quits with a
/// status that says whether every check passed.
#[tauri::command]
pub fn probe_done(app: AppHandle, passed: u32, failed: u32) {
    let Some(plan) = plan() else { return };
    eprintln!("[probe] done: {passed} passed, {failed} failed");
    if plan.split(',').any(|p| p.trim() == "exit") {
        app.exit(if failed == 0 { 0 } else { 1 });
    }
}

/// A pause for a picture: the calling window is brought to the front and, when
/// `VEMRY_PROBE_SHOTS` names a folder, captured there as `<name>.png` (macOS: that one
/// window, by its window number). Returns the file written, if any. Async, so the wait
/// for the window to come forward and redraw is not spent holding the main thread.
#[tauri::command]
pub async fn probe_shot(window: Window, name: String) -> Option<String> {
    plan()?;
    let _ = window.set_focus();
    let dir = std::env::var("VEMRY_PROBE_SHOTS").ok().filter(|d| !d.is_empty())?;
    let name: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
    let path = std::path::Path::new(&dir).join(format!("{name}.png"));
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    let number = window_number(&window)?;
    let ok = std::process::Command::new("screencapture")
        .args(["-x", "-o", &format!("-l{number}")])
        .arg(&path)
        .status()
        .is_ok_and(|s| s.success());
    ok.then(|| path.to_string_lossy().into_owned())
}

/// The window's number in the window server (what `screencapture -l` takes), read on
/// the main thread as AppKit wants.
#[cfg(target_os = "macos")]
fn window_number(window: &Window) -> Option<isize> {
    let (tx, rx) = std::sync::mpsc::channel();
    let win = window.clone();
    window
        .run_on_main_thread(move || {
            use objc2_app_kit::NSWindow;
            let number = win.ns_window().ok().map(|ptr| {
                let ns: &NSWindow = unsafe { &*ptr.cast() };
                ns.windowNumber()
            });
            let _ = tx.send(number);
        })
        .ok()?;
    rx.recv_timeout(std::time::Duration::from_secs(2)).ok().flatten()
}

#[cfg(not(target_os = "macos"))]
fn window_number(_window: &Window) -> Option<isize> {
    None
}
