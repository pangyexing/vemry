//! A session's standing consents, kept with the app: the write grants given for the
//! session ("allow for this session", "allow in this folder"), the read gate's "read
//! without asking", and a followed note's writing without asking. They hold for the
//! whole session — across a switch to another session and back, and across a restart
//! — as the buttons promise, and they live in the app's config directory
//! (`session-consents.json`, by vault root and session id), never in the vault. The
//! session's jsonl records each decision too, but nothing is rebuilt from it: a vault
//! copied or cloned from elsewhere, or a collaborator's commit, could have written an
//! `allow_session` line there.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

const FILE: &str = "session-consents.json";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Consents {
    #[serde(default)]
    pub grants: vemry_agent::WriteGrants,
    #[serde(default)]
    pub read_auto: bool,
    /// The followed note takes the agent's local writes without a dialog.
    #[serde(default)]
    pub follow_auto: bool,
}

/// vault root → session id → consents.
type All = BTreeMap<String, BTreeMap<String, Consents>>;

/// A session's consents (none recorded = none given).
pub fn load(app: &AppHandle, root: &Path, session: &str) -> Consents {
    get(&crate::config_store::read::<All>(app, FILE), root, session)
}

/// Record a session's consents; none at all removes its entry.
pub fn save(app: &AppHandle, root: &Path, session: &str, consents: &Consents) {
    let mut all = crate::config_store::read::<All>(app, FILE);
    put(&mut all, root, session, consents);
    let _ = crate::config_store::save(app, FILE, &all);
}

fn key(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

fn get(all: &All, root: &Path, session: &str) -> Consents {
    all.get(&key(root)).and_then(|s| s.get(session)).cloned().unwrap_or_default()
}

fn put(all: &mut All, root: &Path, session: &str, consents: &Consents) {
    let vault = all.entry(key(root)).or_default();
    if *consents == Consents::default() {
        vault.remove(session);
    } else {
        vault.insert(session.to_string(), consents.clone());
    }
    if vault.is_empty() {
        all.remove(&key(root));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consents_are_kept_per_vault_and_session() {
        let (a, b) = (Path::new("/v/a"), Path::new("/v/b"));
        let mut all = All::new();
        assert_eq!(get(&all, a, "s1"), Consents::default());
        let given = Consents { grants: vemry_agent::WriteGrants { all: true, prefixes: vec![] }, read_auto: true, follow_auto: true };
        put(&mut all, a, "s1", &given);
        assert_eq!(get(&all, a, "s1"), given);
        assert_eq!(get(&all, a, "s2"), Consents::default(), "another session");
        assert_eq!(get(&all, b, "s1"), Consents::default(), "another vault");
        // revoking everything removes the entry rather than storing an empty one
        put(&mut all, a, "s1", &Consents::default());
        assert!(all.is_empty());
    }
}
