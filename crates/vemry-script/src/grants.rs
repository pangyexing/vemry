//! Per-script state, kept outside every vault: one file in the app's config
//! directory holding an entry per vault, keyed by the vault's canonical root. Two
//! flags per script — `enabled` (default true) and `write_grant` (default false; set
//! after the user answers the first-write dialog, revocable). Outside the vault on
//! purpose: a vault copied or cloned from elsewhere could otherwise arrive with its
//! scripts already allowed to write, and a grant is something this machine's user
//! gave. A `.vemry/scripts.json` left in a vault by an earlier version is not read.

use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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

/// Where the state lives: a file of the app's (`HostConfig::state_file`), or memory
/// alone when the host is given none (tests). Owned by the script thread; the
/// bindings share it to record a grant.
pub struct StateStore {
    file: Option<PathBuf>,
    memory: RefCell<BTreeMap<String, ScriptStates>>,
}

impl StateStore {
    pub fn new(file: Option<PathBuf>) -> StateStore {
        StateStore { file, memory: RefCell::new(BTreeMap::new()) }
    }

    /// Every vault's state. A missing or malformed file reads as none: a vault is
    /// usable without ever having touched script settings.
    fn all(&self) -> BTreeMap<String, ScriptStates> {
        match &self.file {
            Some(f) => fs::read_to_string(f).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default(),
            None => self.memory.borrow().clone(),
        }
    }

    /// One vault's state (all defaults when it has none yet).
    pub fn load(&self, root: &Path) -> ScriptStates {
        self.all().remove(&key(root)).unwrap_or_default()
    }

    pub fn save(&self, root: &Path, states: &ScriptStates) -> Result<(), String> {
        let mut all = self.all();
        all.insert(key(root), states.clone());
        let Some(file) = &self.file else {
            *self.memory.borrow_mut() = all;
            return Ok(());
        };
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?;
        // A temporary file renamed over the old one: a crash halfway through must not
        // lose every vault's grants
        let tmp = file.with_extension("json.tmp");
        fs::write(&tmp, json).and_then(|()| fs::rename(&tmp, file)).map_err(|e| e.to_string())
    }
}

/// A vault is known by its canonical root (what `Vault::root` holds), so the same
/// folder opened by another spelling finds its state, and a moved one starts over.
fn key(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip_and_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config/script-state.json");
        let (a, b) = (dir.path().join("vault-a"), dir.path().join("vault-b"));
        let store = StateStore::new(Some(file.clone()));
        let s = store.load(&a);
        assert!(s.enabled("anything"));
        assert!(!s.write_grant("anything"));

        let mut s = s;
        s.set_enabled("a", false);
        s.set_write_grant("b", true);
        store.save(&a, &s).unwrap();

        // a fresh store reads the same file back
        let s2 = StateStore::new(Some(file.clone())).load(&a);
        assert!(!s2.enabled("a"));
        assert!(!s2.write_grant("a"));
        assert!(s2.enabled("b"));
        assert!(s2.write_grant("b"));
        // unknown names keep defaults
        assert!(s2.enabled("c"));
        // another vault's scripts share nothing with this one's
        assert!(!store.load(&b).write_grant("b"));
        // and nothing was written into either vault
        assert!(!a.join(".vemry").exists() && file.exists());
    }

    /// A vault carrying a state file of its own (an earlier version's, or one
    /// planted in a shared vault) grants nothing.
    #[test]
    fn a_state_file_inside_the_vault_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        fs::create_dir_all(vault.join(".vemry")).unwrap();
        fs::write(vault.join(".vemry/scripts.json"), r#"{"w":{"enabled":true,"write_grant":true}}"#).unwrap();
        let store = StateStore::new(Some(dir.path().join("script-state.json")));
        assert!(!store.load(&vault).write_grant("w"));
        assert!(!StateStore::new(None).load(&vault).write_grant("w"));
    }

    #[test]
    fn malformed_file_is_empty_state() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("script-state.json");
        fs::write(&file, "{ not json").unwrap();
        let s = StateStore::new(Some(file)).load(dir.path());
        assert!(s.0.is_empty());
    }
}
