use serde::{Deserialize, Deserializer, Serialize};
use std::path::Path;
use std::time::SystemTime;
use unicode_normalization::UnicodeNormalization;

use crate::block::Block;

/// Identity of a note: its vault-relative path, `/`-separated, NFC-normalized.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NoteId(String);

impl NoteId {
    pub fn new(rel_path: &str) -> Self {
        let normalized: String = rel_path.replace('\\', "/").nfc().collect();
        NoteId(normalized.trim_start_matches('/').to_string())
    }

    pub fn from_rel_path(p: &Path) -> Self {
        // Windows renders a Path with backslashes (watcher events and joins
        // onto a verbatim path especially); new() normalises to "/" (the
        // existing cross-platform convention) so index keys, self-write
        // suppression and stem() cannot go out of sync.
        Self::new(&p.to_string_lossy())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// File stem: "folder/My Note.md" -> "My Note"
    pub fn stem(&self) -> &str {
        let name = self.0.rsplit('/').next().unwrap_or(&self.0);
        name.strip_suffix(".md").unwrap_or(name)
    }

    pub fn folder(&self) -> &str {
        match self.0.rfind('/') {
            Some(i) => &self.0[..i],
            None => "",
        }
    }
}

impl std::fmt::Display for NoteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Provenance metadata vemry writes under the `vemry:` frontmatter namespace,
/// kept separate so it never collides with user fields.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VemryMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>, // "agent" | "capture"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session: Option<String>,
}

/// YAML frontmatter. Unknown fields are preserved verbatim in `extra` so a
/// round-trip never destroys user metadata.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Frontmatter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "string_or_seq", skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, deserialize_with = "string_or_seq", skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vemry: Option<VemryMeta>,
    /// `private: true` puts the note in the privacy zone: invisible to the
    /// AI agent (never read, written, or mentioned), fully visible in the UI.
    #[serde(default, skip_serializing_if = "is_false")]
    pub private: bool,
    #[serde(flatten)]
    pub extra: serde_yaml::Mapping,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Frontmatter {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.tags.is_empty()
            && self.aliases.is_empty()
            && self.created.is_none()
            && self.vemry.is_none()
            && !self.private
            && self.extra.is_empty()
    }

    pub fn to_yaml(&self) -> crate::Result<String> {
        Ok(serde_yaml::to_string(self)?)
    }
}

/// Accepts `tags: foo`, `tags: [a, b]`, or a YAML list.
fn string_or_seq<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
        None(()),
    }
    Ok(match Option::<OneOrMany>::deserialize(d)? {
        Some(OneOrMany::One(s)) => vec![s],
        Some(OneOrMany::Many(v)) => v,
        _ => vec![],
    })
}

#[derive(Clone, Debug)]
pub struct Note {
    pub id: NoteId,
    pub title: String,
    pub frontmatter: Frontmatter,
    /// Full raw file text (frontmatter included). Block spans index into this.
    pub raw: String,
    /// Body text (after frontmatter).
    pub content: String,
    pub blocks: Vec<Block>,
    pub mtime: SystemTime,
}

impl Note {
    pub fn effective_title(&self) -> &str {
        self.frontmatter.title.as_deref().unwrap_or_else(|| self.id.stem())
    }
}

/// A YAML scalar as the text a field carries: strings as written, numbers and
/// booleans rendered; a mapping, a sequence or a null has no scalar form. The one
/// rule for both readers of frontmatter — the index's `field_stats` and a block
/// type's declared fields — so a `=` comparison and a type's `default:` cannot
/// come to read the same value differently.
pub fn yaml_text(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}
