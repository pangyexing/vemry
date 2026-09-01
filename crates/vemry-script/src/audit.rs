//! Script audit log: every script write, grant decision, error and console
//! line appends to `.vemry/script-log.jsonl` (one file per vault — scripts
//! are long-lived residents, unlike per-conversation agent sessions).
//! Same line format as the agent session log: `{"ts", "kind", ...}`.

use chrono::Utc;
use serde_json::{json, Value};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

pub const LOG_FILE: &str = ".vemry/script-log.jsonl";

/// Best-effort append — auditing must never take the script host down.
pub fn append(root: &Path, kind: &str, mut payload: Value) {
    if let Some(obj) = payload.as_object_mut() {
        obj.insert("ts".into(), json!(Utc::now().to_rfc3339()));
        obj.insert("kind".into(), json!(kind));
    }
    let path = root.join(LOG_FILE);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{payload}");
    }
}

#[cfg(test)]
pub fn read(root: &Path) -> String {
    std::fs::read_to_string(root.join(LOG_FILE)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_jsonl_with_ts_and_kind() {
        let dir = tempfile::tempdir().unwrap();
        append(dir.path(), "write", json!({ "script": "s", "path": "a.md" }));
        append(dir.path(), "error", json!({ "script": "s", "message": "boom" }));
        let content = read(dir.path());
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["kind"], "write");
        assert_eq!(first["script"], "s");
        assert!(first["ts"].as_str().unwrap().contains('T'));
    }
}
