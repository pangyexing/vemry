//! vemry-core — the vault engine.
//!
//! Layering contract: this crate has zero UI/AI/Tauri dependencies. It owns
//! the single source of truth (a folder of Markdown files) and the single
//! semantic layer (a block-level index). Every surface — UI views, scripts,
//! AI agent tools — consumes the same `query`/`search`/`links` APIs.

pub mod attach_migrate;
pub mod block;
pub mod blockid;
pub mod event;
pub mod index;
pub mod insert;
pub mod locate;
pub mod note;
pub mod parser;
pub mod privacy;
pub mod query;
pub mod search;
pub mod vault;
pub mod watcher;

pub use attach_migrate::MigrationReport;
pub use block::{Block, BlockKind, TaskState, WikiLink};
pub use event::VaultEvent;
pub use index::{BlockIndex, GraphData, GraphEdge, GraphNode, NoteSummary, ResolvedLink};
pub use note::{Frontmatter, Note, NoteId, VemryMeta};
pub use privacy::PrivacyScope;
pub use query::{BlockHit, BlockKindFilter, BlockQuery};
pub use search::{InMemorySearch, SearchHit, SearchProvider};
pub use vault::{sanitize_filename, BacklinkEntry, InsertLanded, Vault};

#[derive(thiserror::Error, Debug)]
pub enum VemryError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("note not found: {0}")]
    NoteNotFound(String),
    #[error("{reason}: {path}")]
    InvalidPath { reason: PathReject, path: String },
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("watch error: {0}")]
    Watch(String),
}

/// Why a path was rejected. Typed rather than pre-formatted so the glue layer
/// can render it in the user's language — core carries no copy and knows no
/// language. `Display` is the English fallback, which is also what the script
/// API (a developer-facing surface) shows verbatim.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathReject {
    NotFound,
    NotAFile,
    NotADirectory,
    TooLarge,
    NotUtf8,
    Exists,
    AttachmentIsMd,
    MdNeedsNoteApi,
    IsTheVaultItself,
    FolderIntoItself,
    ReservedName,
    Escapes,
    Empty,
}

impl PathReject {
    /// Catalog key (`locales/*.json`) for the localized wording.
    pub fn key(self) -> &'static str {
        match self {
            PathReject::NotFound => "err.notFound",
            PathReject::NotAFile => "err.notAFile",
            PathReject::NotADirectory => "err.notADirectory",
            PathReject::TooLarge => "err.tooLarge",
            PathReject::NotUtf8 => "err.notUtf8",
            PathReject::Exists => "err.exists",
            PathReject::AttachmentIsMd => "err.attachmentIsMd",
            PathReject::MdNeedsNoteApi => "err.mdNeedsNoteApi",
            PathReject::IsTheVaultItself => "err.isTheVaultItself",
            PathReject::FolderIntoItself => "err.folderIntoItself",
            PathReject::ReservedName => "err.reservedName",
            PathReject::Escapes => "err.escapes",
            PathReject::Empty => "err.empty",
        }
    }
}

impl std::fmt::Display for PathReject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PathReject::NotFound => "does not exist",
            PathReject::NotAFile => "not a file",
            PathReject::NotADirectory => "not a directory",
            PathReject::TooLarge => "file too large",
            PathReject::NotUtf8 => "not UTF-8 text",
            PathReject::Exists => "already exists",
            PathReject::AttachmentIsMd => "an attachment cannot be .md",
            PathReject::MdNeedsNoteApi => ".md must be written through the note API",
            PathReject::IsTheVaultItself => "cannot import the vault itself",
            PathReject::FolderIntoItself => "cannot copy a folder into itself",
            PathReject::ReservedName => "the name uses characters or a reserved word Windows forbids",
            PathReject::Escapes => "path escapes the vault",
            PathReject::Empty => "empty",
        })
    }
}

/// Shorthand for the common `Err(InvalidPath { .. })`.
pub fn reject(reason: PathReject, path: impl Into<String>) -> VemryError {
    VemryError::InvalidPath { reason, path: path.into() }
}

pub type Result<T> = std::result::Result<T, VemryError>;
