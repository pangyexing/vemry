//! Vault events. This bus is the future mount point for hooks/workflows
//! (v0.3): a workflow trigger is just another subscriber.

use serde::Serialize;

use crate::note::NoteId;

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VaultEvent {
    NoteAdded { id: NoteId },
    NoteModified { id: NoteId },
    NoteRemoved { id: NoteId },
    IndexReady { note_count: usize },
    /// Non-note files/folders changed on disk (added, removed, or renamed) —
    /// the file tree should refresh. At most one per watcher batch.
    FilesChanged,
    /// Privacy rules or marks changed — the tree badges and the agent-visible set
    /// must refresh.
    PrivacyChanged,
    /// A file under `.vemry/scripts/` changed on disk — the script host must
    /// reload. At most one per watcher batch.
    ScriptsChanged,
    /// A file under `.vemry/types/` changed on disk — the block-type menus and
    /// the prompt snapshot must refresh. At most one per watcher batch.
    TypesChanged,
}

type Subscriber = Box<dyn Fn(&VaultEvent) + Send + Sync>;

#[derive(Default)]
pub struct EventBus {
    subscribers: Vec<Subscriber>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn subscribe(&mut self, f: impl Fn(&VaultEvent) + Send + Sync + 'static) {
        self.subscribers.push(Box::new(f));
    }

    pub fn emit(&self, event: &VaultEvent) {
        for s in &self.subscribers {
            s(event);
        }
    }
}
