//! Block types: `.vemry/types/<name>.md` defines a kind of block the user
//! writes often — a Markdown kind (bullet, task, paragraph, …) plus the inline
//! fields it carries and an optional tag — with a template in the body. The
//! editor lists them in the `/` menu and under "turn into ›", and the agent's
//! system prompt names them so it writes such blocks the same way. Nothing in a
//! note points back at a type: a typed block is ordinary Markdown with fields,
//! and deleting the type file changes no note. Like skills, this directory is
//! read from the metadata folder, regular files only (a symlink could point
//! into a privacy zone), `.md` only; a damaged frontmatter degrades to defaults
//! rather than hiding the type.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::parser::extract_frontmatter;

/// Types live in `<meta_dir>/types` (i.e. `.vemry/types/`).
pub const TYPES_SUBDIR: &str = "types";

/// The block kinds a type may declare (the editor's turn-into targets).
pub const TYPE_KINDS: &[&str] =
    &["paragraph", "heading1", "heading2", "heading3", "bullet", "numbered", "task", "quote", "callout", "code", "private"];

/// One inline field a type carries.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct FieldSpec {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Suggested values (the completion offers them first).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    /// `date` / `number` / `text` (a hint for the editor; the parser does not care).
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct BlockType {
    /// The file name (`book.md`).
    pub file: String,
    pub name: String,
    pub description: String,
    /// One of `TYPE_KINDS` (an unknown kind falls back to `paragraph`).
    pub kind: String,
    /// The callout type when `kind` is `callout`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callout: Option<String>,
    /// A `#tag` every block of this type carries (without the `#`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// A short glyph shown in the menus.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub fields: Vec<FieldSpec>,
    /// The body: `{{key}}` is a field's default, `{{date}}` today, `{{cursor}}`
    /// where the caret lands. An empty body gets the fields laid out on one line.
    pub template: String,
    pub disabled: bool,
}

/// The example written by "create an example block type".
pub const BOOK_TYPE_TEMPLATE: &str = "---
name: book
description: A book you read or plan to read
kind: bullet
tag: book
icon: 📚
fields:
  - key: author
  - key: status
    values: [to-read, reading, done]
    default: to-read
  - key: rating
    type: number
---
{{cursor}} [author:: {{author}}] [status:: {{status}}]
";

/// The file a new type starts from ("block type…" given a name with no file yet):
/// every knob once, so the file explains itself when it opens in the editor.
/// `book` gets `BOOK_TYPE_TEMPLATE` instead (the caller's choice).
pub fn type_skeleton(name: &str) -> String {
    let name = name.trim();
    let tag = tag_of(name);
    let tag_line = if tag.is_empty() { String::new() } else { format!("tag: {}\n", yaml_scalar(&tag)) };
    format!(
        "---\nname: {}\ndescription: {}\nkind: bullet\n{tag_line}fields:\n  - key: status\n    values: [open, done]\n    default: open\n  - key: due\n    type: date\n---\n{{{{cursor}}}} [status:: {{{{status}}}}] [due:: ]\n",
        yaml_scalar(name),
        yaml_scalar(&format!("A {name}")),
    )
}

/// A name as a YAML scalar: plain when it is unmistakably one (letters, digits,
/// spaces, `-` and `_`, not starting with an indicator), double-quoted otherwise.
fn yaml_scalar(s: &str) -> String {
    let plain = !s.is_empty()
        && s.chars().all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '_')
        && !s.starts_with(['-', ' '])
        && !s.ends_with(' ');
    if plain {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// The `#tag` derived from a type name: letters, digits, `-` and `_` kept, runs
/// of whitespace joined with `-`, anything else dropped.
fn tag_of(name: &str) -> String {
    let mut tag = String::new();
    for c in name.chars() {
        if c.is_alphanumeric() || c == '-' || c == '_' {
            tag.push(c);
        } else if c.is_whitespace() && !tag.is_empty() && !tag.ends_with('-') {
            tag.push('-');
        }
    }
    tag.trim_matches('-').to_string()
}

fn yaml_str(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn parse_fields(v: Option<&serde_yaml::Value>) -> Vec<FieldSpec> {
    let Some(serde_yaml::Value::Sequence(items)) = v else { return Vec::new() };
    items
        .iter()
        .filter_map(|item| match item {
            serde_yaml::Value::String(key) => Some(FieldSpec { key: key.trim().to_string(), ..Default::default() }),
            serde_yaml::Value::Mapping(m) => {
                let get = |k: &str| m.get(serde_yaml::Value::String(k.into()));
                let key = get("key").and_then(yaml_str)?.trim().to_string();
                let values = match get("values") {
                    Some(serde_yaml::Value::Sequence(vs)) => vs.iter().filter_map(yaml_str).collect(),
                    _ => Vec::new(),
                };
                Some(FieldSpec { key, default: get("default").and_then(yaml_str), values, kind: get("type").and_then(yaml_str) })
            }
            _ => None,
        })
        .filter(|f| crate::parser::is_field_key(&f.key))
        .collect()
}

/// The layout of a type with no body: the caret first, then every field as
/// `[key:: {{key}}]` on the same line.
fn auto_template(fields: &[FieldSpec]) -> String {
    let mut s = String::from("{{cursor}}");
    for f in fields {
        s.push_str(&format!(" [{}:: {{{{{}}}}}]", f.key, f.key));
    }
    s
}

/// Parse one type file. Malformed YAML → an empty frontmatter, so the type is
/// still listed under its file stem with paragraph defaults.
pub fn parse_block_type(file: &str, raw: &str) -> BlockType {
    let (fm, body_offset) = extract_frontmatter(raw);
    let text = |key: &str| fm.extra.get(key).and_then(yaml_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let stem = file.strip_suffix(".md").unwrap_or(file).to_string();
    let kind = text("kind").filter(|k| TYPE_KINDS.contains(&k.as_str())).unwrap_or_else(|| "paragraph".into());
    let fields = parse_fields(fm.extra.get("fields"));
    let body = raw[body_offset..].trim();
    let template = if body.is_empty() { auto_template(&fields) } else { body.to_string() };
    BlockType {
        file: file.to_string(),
        name: text("name").unwrap_or(stem),
        description: text("description").unwrap_or_default(),
        kind,
        callout: text("callout"),
        tag: text("tag").map(|t| t.trim_start_matches('#').to_string()).filter(|t| !t.is_empty()),
        icon: text("icon"),
        fields,
        template,
        disabled: fm.extra.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false),
    }
}

/// Every type file (disabled ones included), sorted by name. Regular `.md`
/// files directly in the directory; a symlink is refused.
pub fn list_block_types(meta_dir: &Path) -> Vec<BlockType> {
    let dir = meta_dir.join(TYPES_SUBDIR);
    let mut types: Vec<BlockType> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                .filter_map(|e| {
                    let file = e.file_name().to_string_lossy().to_string();
                    let raw = fs::read_to_string(e.path()).ok()?;
                    Some(parse_block_type(&file, &raw))
                })
                .collect()
        })
        .unwrap_or_default();
    types.sort_by(|a, b| a.name.cmp(&b.name));
    types
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skeleton_parses_for_plain_and_awkward_names() {
        let t = parse_block_type("meeting.md", &type_skeleton("meeting"));
        assert_eq!(t.name, "meeting");
        assert_eq!(t.description, "A meeting");
        assert_eq!(t.kind, "bullet");
        assert_eq!(t.tag.as_deref(), Some("meeting"));
        assert_eq!(t.fields.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(), ["status", "due"]);
        assert_eq!(t.fields[0].default.as_deref(), Some("open"));
        assert_eq!(t.fields[1].kind.as_deref(), Some("date"));
        assert!(t.template.starts_with("{{cursor}} [status:: {{status}}]"));
        // a name YAML would misread is quoted, and the tag keeps only what a tag can hold
        let t = parse_block_type("x.md", &type_skeleton(" to: \"do\" later "));
        assert_eq!(t.name, "to: \"do\" later");
        assert_eq!(t.tag.as_deref(), Some("to-do-later"));
        assert_eq!(t.fields.len(), 2);
        // nothing tag-worthy → no tag line at all
        assert_eq!(parse_block_type("x.md", &type_skeleton("!!!")).tag, None);
    }

    #[test]
    fn example_type_parses_every_key() {
        let t = parse_block_type("book.md", BOOK_TYPE_TEMPLATE);
        assert_eq!(t.name, "book");
        assert_eq!(t.kind, "bullet");
        assert_eq!(t.tag.as_deref(), Some("book"));
        assert_eq!(t.icon.as_deref(), Some("📚"));
        assert_eq!(t.fields.len(), 3);
        assert_eq!(t.fields[1].values, vec!["to-read", "reading", "done"]);
        assert_eq!(t.fields[1].default.as_deref(), Some("to-read"));
        assert_eq!(t.fields[2].kind.as_deref(), Some("number"));
        assert!(t.template.starts_with("{{cursor}}"));
        assert!(!t.disabled);
    }

    #[test]
    fn missing_body_gets_the_auto_template_and_bad_yaml_degrades() {
        let t = parse_block_type("meeting.md", "---\nkind: task\nfields: [when, who]\n---\n");
        assert_eq!(t.template, "{{cursor}} [when:: {{when}}] [who:: {{who}}]");
        assert_eq!(t.kind, "task");
        let bad = parse_block_type("odd.md", "---\nname: [unclosed\n---\nbody\n");
        assert_eq!(bad.name, "odd");
        assert_eq!(bad.kind, "paragraph");
        // an unknown kind and a bad field key fall back rather than fail
        let t = parse_block_type("x.md", "---\nkind: rocket\nfields: [\"12\", ok]\ndisabled: true\n---\n");
        assert_eq!(t.kind, "paragraph");
        assert_eq!(t.fields.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(), vec!["ok"]);
        assert!(t.disabled);
    }

    #[test]
    fn listing_takes_regular_md_files_only_and_sorts_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join(".vemry");
        let types = meta.join("types");
        fs::create_dir_all(&types).unwrap();
        fs::write(types.join("zeta.md"), "---\nname: zeta\n---\n").unwrap();
        fs::write(types.join("alpha.md"), "---\nname: alpha\n---\n").unwrap();
        fs::write(types.join("notes.txt"), "not a type").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(types.join("alpha.md"), types.join("link.md")).unwrap();
        let got = list_block_types(&meta);
        assert_eq!(got.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), vec!["alpha", "zeta"]);
        assert!(list_block_types(dir.path()).is_empty());
    }
}
