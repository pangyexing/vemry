//! Per-script state that travels with the vault: `.vemry/scripts.json`.
//! Two flags per script — `enabled` (default true) and `write_grant` (default
//! false; set after the user answers the first-write dialog, revocable).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub const STATE_FILE: &str = ".vemry/scripts.json";

fn default_enabled() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptState {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub write_grant: bool,
}

impl Default for ScriptState {
    fn default() -> Self {
        ScriptState { enabled: true, write_grant: false }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScriptStates(pub BTreeMap<String, ScriptState>);

impl ScriptStates {
    /// Missing or malformed file = empty state (all defaults). A vault is
    /// usable without ever having touched script settings.
    pub fn load(root: &Path) -> ScriptStates {
        fs::read_to_string(root.join(STATE_FILE))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, root: &Path) -> Result<(), String> {
        let path = root.join(STATE_FILE);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&path, json).map_err(|e| e.to_string())
    }

    pub fn enabled(&self, name: &str) -> bool {
        self.0.get(name).is_none_or(|s| s.enabled)
    }

    pub fn write_grant(&self, name: &str) -> bool {
        self.0.get(name).is_some_and(|s| s.write_grant)
    }

    pub fn set_enabled(&mut self, name: &str, enabled: bool) {
        self.0.entry(name.to_string()).or_default().enabled = enabled;
    }

    pub fn set_write_grant(&mut self, name: &str, grant: bool) {
        self.0.entry(name.to_string()).or_default().write_grant = grant;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip_and_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let s = ScriptStates::load(root);
        assert!(s.enabled("anything"));
        assert!(!s.write_grant("anything"));

        let mut s = s;
        s.set_enabled("a", false);
        s.set_write_grant("b", true);
        s.save(root).unwrap();

        let s2 = ScriptStates::load(root);
        assert!(!s2.enabled("a"));
        assert!(!s2.write_grant("a"));
        assert!(s2.enabled("b"));
        assert!(s2.write_grant("b"));
        // unknown names keep defaults
        assert!(s2.enabled("c"));
    }

    #[test]
    fn malformed_file_is_empty_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".vemry")).unwrap();
        std::fs::write(dir.path().join(STATE_FILE), "{ not json").unwrap();
        let s = ScriptStates::load(dir.path());
        assert!(s.0.is_empty());
    }
}
