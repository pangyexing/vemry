//! Instruction-level skills: playbooks the agent reads on demand. Two layers —
//! the built-in ones ship inside the binary (`skills/*.md` beside this crate's
//! `src/`), and the vault's own `.vemry/skills/*.md`, which add to them or, by
//! carrying the same name, override one. VEMRY.md is the always-loaded
//! constitution; skills load lazily — the system prompt carries only
//! `name — description`, the model pulls the full text via the `read_skill`
//! tool when the task matches (or the user names one from the composer).
//!
//! A built-in never lands in the vault by itself: a vault that wants to change
//! one gets a copy ("customise") which then shadows it by name, and switching a
//! built-in off is a name in the vault config's `skills_disabled` list. So an
//! update to the binary reaches every vault that has not customised the skill,
//! and a customised copy is the user's file for good (Settings flags it when
//! the built-in behind it has moved on, by the `version` in the frontmatter).
//!
//! Privacy note: `.vemry` is normally invisible to the agent (dot-path =
//! built-in privacy zone). Skills are a deliberate, narrow exception — their
//! whole purpose is to be read by the agent. `read_skill` resolves the name
//! against the scanned listing (never joins caller input into a path), so
//! only `.md` files directly in the skills dir are reachable. Hide one from
//! the agent with `disabled: true` or by deleting the file.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;
use vemry_core::parser::extract_frontmatter;

/// Skills live in `<meta_dir>/skills` (i.e. `.vemry/skills/`).
pub const SKILLS_SUBDIR: &str = "skills";
/// The key in `.vemry/config.json` listing the built-in skills the user has
/// switched off (a vault skill is switched off by its own frontmatter).
pub const DISABLED_BUILTINS_KEY: &str = "skills_disabled";
/// Same reading budget as `read_note`.
const READ_SKILL_MAX_CHARS: usize = 30_000;

/// The built-in skills: the file name a customised copy takes in the vault, and
/// the whole file (frontmatter included — `builtin_skills` parses it once).
/// English, because a skill is read by the model and the system prompt already
/// tells it to answer in the user's language (design constraint 3 in CLAUDE.md).
const BUILTIN_FILES: &[(&str, &str)] = &[
    ("research.md", include_str!("../skills/research.md")),
    ("daily-start.md", include_str!("../skills/daily-start.md")),
    ("daily-close.md", include_str!("../skills/daily-close.md")),
    ("meeting-actions.md", include_str!("../skills/meeting-actions.md")),
    ("weekly-report.md", include_str!("../skills/weekly-report.md")),
    ("tidy.md", include_str!("../skills/tidy.md")),
    ("tidy-note.md", include_str!("../skills/tidy-note.md")),
    ("prd.md", include_str!("../skills/prd.md")),
    ("decision-log.md", include_str!("../skills/decision-log.md")),
    ("feedback-triage.md", include_str!("../skills/feedback-triage.md")),
    ("iteration-plan.md", include_str!("../skills/iteration-plan.md")),
    ("retro.md", include_str!("../skills/retro.md")),
    ("brief.md", include_str!("../skills/brief.md")),
    ("tutor.md", include_str!("../skills/tutor.md")),
];

/// Where a listed skill comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkillSource {
    /// Shipped in the binary; no file in the vault.
    Builtin,
    /// The vault's own file, with a name no built-in has.
    Vault,
    /// The vault's own file shadowing a built-in of the same name.
    Override,
}

impl SkillSource {
    pub fn as_str(self) -> &'static str {
        match self {
            SkillSource::Builtin => "builtin",
            SkillSource::Vault => "vault",
            SkillSource::Override => "override",
        }
    }
}

#[derive(Debug, Default)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    disabled: bool,
    /// `version:` — an integer the built-ins carry so a customised copy can be
    /// told apart from the built-in it was copied from. 0 when absent.
    version: u64,
    /// `triggers:` — the words a user would type for this skill, in any
    /// language; the composer's `/` picker matches them besides the name and
    /// the description. A YAML list, or one comma-separated string.
    triggers: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
}

/// One built-in skill, parsed once for the process.
#[derive(Debug)]
pub struct BuiltinSkill {
    pub file: &'static str,
    pub name: String,
    pub description: String,
    pub version: u64,
    pub triggers: Vec<String>,
    /// The whole file — what "customise" copies into the vault.
    pub text: &'static str,
    /// The instructions alone (frontmatter stripped) — what `read_skill` returns.
    pub body: &'static str,
}

/// Frontmatter + body of one skill file, via the same parser every note goes
/// through (`vemry_core::parser::extract_frontmatter`) — a second hand-rolled
/// delimiter scanner would drift on the edges core already handles (`--- `
/// with trailing whitespace, `...` terminators, CRLF), and a mis-parsed
/// `disabled: true` must never leave a skill visible. Malformed YAML degrades
/// to an empty frontmatter — a skill must never break prompt assembly.
fn parse(raw: &str) -> (SkillFrontmatter, &str) {
    let (fm, body_offset) = extract_frontmatter(raw);
    let text = |key: &str| fm.extra.get(key).and_then(|v| v.as_str()).map(str::to_string);
    let version = fm
        .extra
        .get("version")
        .and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())))
        .unwrap_or(0);
    let triggers = match fm.extra.get("triggers") {
        Some(v) if v.is_sequence() => {
            v.as_sequence().unwrap().iter().filter_map(|t| t.as_str()).map(|t| t.trim().to_string()).collect()
        }
        Some(v) => split_triggers(v.as_str().unwrap_or_default()),
        None => vec![],
    };
    let mut skill = SkillFrontmatter {
        name: text("name"),
        description: text("description"),
        disabled: fm.extra.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false),
        version,
        triggers,
    };
    // A block was there but yielded none of the four keys: the usual cause is
    // YAML the parser refuses as a whole (an unquoted description with a colon
    // in it), which must not cost the skill its description — nor its
    // `disabled: true` its effect. The four keys are plain scalars, so a line
    // scan reads them from the same block.
    let empty = skill.name.is_none() && skill.description.is_none() && !skill.disabled && skill.version == 0 && skill.triggers.is_empty();
    if body_offset > 0 && empty {
        skill = scan_lines(&raw[..body_offset]);
    }
    (skill, &raw[body_offset..])
}

/// The fallback reader of a frontmatter block serde rejected: `key: value` per
/// line, quotes around the value dropped; `head` is the text up to the body
/// (the opening line and the closing fence are skipped).
fn scan_lines(head: &str) -> SkillFrontmatter {
    let mut out = SkillFrontmatter::default();
    let lines: Vec<&str> = head.lines().collect();
    if lines.len() < 2 {
        return out;
    }
    for line in &lines[1..lines.len() - 1] {
        let Some((key, value)) = line.split_once(':') else { continue };
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'').trim();
        match key.trim() {
            "name" if !value.is_empty() => out.name = Some(value.to_string()),
            "description" if !value.is_empty() => out.description = Some(value.to_string()),
            "disabled" => out.disabled = value == "true",
            "version" => out.version = value.parse().unwrap_or(0),
            // a flow list `[a, b]` or a plain comma-separated line
            "triggers" => out.triggers = split_triggers(value.trim_matches(|c| c == '[' || c == ']')),
            _ => {}
        }
    }
    out
}

/// A comma-separated trigger list, each word trimmed of space and quotes, empties dropped.
fn split_triggers(s: &str) -> Vec<String> {
    s.split(',').map(|t| t.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_string()).filter(|t| !t.is_empty()).collect()
}

/// The listed name of a skill file: the frontmatter `name`, else the file stem.
fn name_of(fm: &SkillFrontmatter, stem: &str) -> String {
    fm.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| stem.to_string())
}

/// The built-in skills, parsed on first use and kept for the process.
pub fn builtin_skills() -> &'static [BuiltinSkill] {
    static PARSED: OnceLock<Vec<BuiltinSkill>> = OnceLock::new();
    PARSED.get_or_init(|| {
        BUILTIN_FILES
            .iter()
            .map(|(file, text)| {
                let (fm, body) = parse(text);
                let stem = file.strip_suffix(".md").unwrap_or(file);
                BuiltinSkill {
                    file,
                    name: name_of(&fm, stem),
                    description: fm.description.unwrap_or_default(),
                    version: fm.version,
                    triggers: fm.triggers,
                    text,
                    body: body.trim(),
                }
            })
            .collect()
    })
}

/// The built-in skill of that name, if any.
pub fn builtin_skill(name: &str) -> Option<&'static BuiltinSkill> {
    builtin_skills().iter().find(|b| b.name == name)
}

/// The stored `skills_disabled` value decoded: the names in it, in the order the
/// file has them. The one reader of that shape — the prompt side (through the config
/// file, below) and the Settings toggle (through `vault_config_get`) must not
/// interpret it independently.
pub fn disabled_names(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|d| d.as_array())
        .map(|a| a.iter().filter_map(|n| n.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

/// The names in the vault config's `skills_disabled` list. Read here rather than
/// handed in: the prompt assembly and the `read_skill` tool both need it, and a
/// missing or unreadable config means nothing is switched off.
fn disabled_builtins(meta_dir: &Path) -> HashSet<String> {
    let cfg = fs::read_to_string(meta_dir.join("config.json")).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok());
    disabled_names(cfg.as_ref().and_then(|v| v.get(DISABLED_BUILTINS_KEY))).into_iter().collect()
}

/// One skill file of the vault, as scanned.
struct VaultSkill {
    file: String,
    name: String,
    fm: SkillFrontmatter,
    body: String,
}

/// The one directory scan of `.vemry/skills/`: regular `.md` files only — a
/// symlink here could point into the privacy zone and this is an agent-visible
/// read path. Unsorted; the callers order what they list.
fn vault_skills(meta_dir: &Path) -> Vec<VaultSkill> {
    let dir = meta_dir.join(SKILLS_SUBDIR);
    fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                .filter_map(|e| {
                    let stem = e.path().file_stem()?.to_string_lossy().to_string();
                    let file = e.file_name().to_string_lossy().to_string();
                    let raw = fs::read_to_string(e.path()).ok()?;
                    let (fm, body) = parse(&raw);
                    let name = name_of(&fm, &stem);
                    Some(VaultSkill { file, name, fm, body: body.trim().to_string() })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One skill from the management point of view (disabled ones are listed too).
/// `file` is the vault file name — for a built-in, the name a customised copy
/// would take.
#[derive(Clone, Debug)]
pub struct SkillFileEntry {
    pub file: String,
    pub name: String,
    pub description: String,
    pub disabled: bool,
    pub source: SkillSource,
    pub version: u64,
    /// An override whose `version` is behind the built-in it shadows (a copy
    /// without one counts as 0): the built-in has been updated since the copy.
    pub outdated: bool,
    pub triggers: Vec<String>,
}

/// Every skill (including disabled), sorted by name: the vault's files, then
/// the built-ins no vault file shadows. The one listing both views share —
/// the settings page shows this directly, `list_skills` (the agent-facing
/// view) filters it, so the symlink refusal and frontmatter edge cases can't
/// drift between them.
pub fn list_skills_all(meta_dir: &Path) -> Vec<SkillFileEntry> {
    let off = disabled_builtins(meta_dir);
    let vault = vault_skills(meta_dir);
    let mut skills: Vec<SkillFileEntry> = vault
        .iter()
        .map(|s| {
            let shadowed = builtin_skill(&s.name);
            SkillFileEntry {
                file: s.file.clone(),
                name: s.name.clone(),
                description: s.fm.description.clone().unwrap_or_default(),
                disabled: s.fm.disabled,
                source: if shadowed.is_some() { SkillSource::Override } else { SkillSource::Vault },
                version: s.fm.version,
                outdated: shadowed.is_some_and(|b| b.version > s.fm.version),
                triggers: s.fm.triggers.clone(),
            }
        })
        .collect();
    for b in builtin_skills() {
        if vault.iter().any(|s| s.name == b.name) {
            continue;
        }
        skills.push(SkillFileEntry {
            file: b.file.to_string(),
            name: b.name.clone(),
            description: b.description.clone(),
            disabled: off.contains(&b.name),
            source: SkillSource::Builtin,
            version: b.version,
            outdated: false,
            triggers: b.triggers.clone(),
        });
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// Enabled skills, sorted by name (the prompt-injection view).
pub fn list_skills(meta_dir: &Path) -> Vec<SkillMeta> {
    list_skills_all(meta_dir)
        .into_iter()
        .filter(|s| !s.disabled)
        .map(|s| SkillMeta { name: s.name, description: s.description })
        .collect()
}

/// Full instructions (body, frontmatter stripped) of an enabled skill,
/// resolved by exact listed name: the vault's file first (a disabled override
/// hides the built-in behind it too — the row the user switched off is the
/// skill), then the built-in. `None` covers missing and disabled alike —
/// callers report both as `skill not found`.
pub fn read_skill(meta_dir: &Path, name: &str) -> Option<String> {
    let body = match vault_skills(meta_dir).into_iter().find(|s| s.name == name) {
        Some(s) => {
            if s.fm.disabled {
                return None;
            }
            s.body
        }
        None => {
            let b = builtin_skill(name)?;
            if disabled_builtins(meta_dir).contains(name) {
                return None;
            }
            b.body.to_string()
        }
    };
    Some(
        crate::truncate_chars(&body, READ_SKILL_MAX_CHARS, &format!("[... truncated at {READ_SKILL_MAX_CHARS} chars]"))
            .unwrap_or(body),
    )
}

/// A new vault skill's file: the skeleton every built-in follows, with the
/// section names in place and one line under each saying what goes there.
/// Written when the user creates a skill from Settings; the file is theirs
/// from then on.
pub fn skill_skeleton(name: &str) -> String {
    let name = name.trim();
    format!(
        "---\nname: {name}\ndescription: One line of at most 12 words on when to think of this skill\ntriggers: []\n---\n\n\
## When\n\n(The requests this covers, and the ones it does not — a question with one clear answer needs no method.)\n\n\
## Plan\n\n(The steps, verbatim, in the user's language — this list is the plan the UI shows:)\n\n1. \n2. \n3. \n\n\
## Inputs\n\n(What to read first: the working note, the attached references, which queries.)\n\n\
## Do\n\n(The method itself — how each step is done, when to stop.)\n\n\
## Landing\n\n(Where the result goes: the working note, a named note, a new note at a fixed path. Which writes need no dialog.)\n\n\
## Shape\n\n(What the result looks like: frontmatter keys, the sections in order, the block kinds.)\n\n\
## Never\n\n(The hard rules.)\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join("skills");
        fs::create_dir_all(&skills).unwrap();
        fs::write(
            skills.join("weekly-review.md"),
            "---\ndescription: 每周回顾怎么写\n---\n# 周报流程\n\n1. 查询本周完成任务\n",
        )
        .unwrap();
        fs::write(
            skills.join("lit-notes.md"),
            "---\nname: 文献笔记\ndescription: 文献笔记的整理约定\n---\n正文指令\n",
        )
        .unwrap();
        fs::write(skills.join("off.md"), "---\ndescription: 停用的\ndisabled: true\n---\n看不见\n").unwrap();
        fs::write(skills.join("plain.md"), "无 frontmatter 的技能\n").unwrap();
        fs::write(skills.join("notes.txt"), "不是 md\n").unwrap();
        dir
    }

    fn vault_names(meta_dir: &Path) -> Vec<String> {
        list_skills_all(meta_dir).into_iter().filter(|s| s.source != SkillSource::Builtin).map(|s| s.name).collect()
    }

    #[test]
    fn builtins_parse_with_unique_names_matching_their_files() {
        let all = builtin_skills();
        assert_eq!(all.len(), BUILTIN_FILES.len());
        let mut names: Vec<&str> = all.iter().map(|b| b.name.as_str()).collect();
        for b in all {
            assert_eq!(b.name, b.file.strip_suffix(".md").unwrap(), "the listed name is the file stem");
            assert!(!b.description.is_empty(), "{} has no description", b.file);
            assert!(b.version >= 1, "{} carries no version", b.file);
            assert!(b.body.starts_with('#'), "{} body should start at its heading, not the frontmatter", b.file);
            assert!(!b.body.contains("description:"), "frontmatter stripped from {}", b.file);
        }
        names.sort();
        names.dedup();
        assert_eq!(names.len(), all.len(), "built-in names are unique");
        assert!(builtin_skill("research").is_some());
        assert!(builtin_skill("ghost").is_none());
    }

    #[test]
    fn list_sorted_named_and_disabled_excluded() {
        let dir = fixture();
        let skills = list_skills(dir.path());
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        // sorted, fm name override wins, disabled and non-md excluded, the
        // built-ins in the same list
        let mut expected: Vec<String> = builtin_skills().iter().map(|b| b.name.clone()).collect();
        expected.extend(["plain", "weekly-review", "文献笔记"].map(String::from));
        expected.sort();
        assert_eq!(names, expected.iter().map(String::as_str).collect::<Vec<_>>());
        let weekly = skills.iter().find(|s| s.name == "weekly-review").unwrap();
        assert_eq!(weekly.description, "每周回顾怎么写");
        // the vault-sourced view alone (management rows carry the source)
        assert_eq!(vault_names(dir.path()), vec!["off", "plain", "weekly-review", "文献笔记"]);
    }

    #[test]
    fn read_skill_by_listed_name_only() {
        let dir = fixture();
        let body = read_skill(dir.path(), "weekly-review").unwrap();
        assert!(body.starts_with("# 周报流程"));
        assert!(!body.contains("description:"), "frontmatter stripped");
        // fm-declared name resolves; the file stem behind it does not
        assert!(read_skill(dir.path(), "文献笔记").is_some());
        assert!(read_skill(dir.path(), "lit-notes").is_none());
        // missing and disabled are indistinguishable
        assert!(read_skill(dir.path(), "off").is_none());
        assert!(read_skill(dir.path(), "ghost").is_none());
        // path-shaped names can never match a scanned stem
        assert!(read_skill(dir.path(), "../secret").is_none());
        assert!(read_skill(dir.path(), "skills/weekly-review").is_none());
        // a built-in reads without any file in the vault
        let research = read_skill(dir.path(), "research").unwrap();
        assert!(research.starts_with("# Research method"), "{research}");
    }

    #[test]
    fn missing_dir_lists_builtins_only() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list_skills(dir.path()).iter().all(|s| builtin_skill(&s.name).is_some()));
        assert_eq!(list_skills(dir.path()).len(), builtin_skills().len());
        assert!(list_skills_all(dir.path()).iter().all(|s| s.source == SkillSource::Builtin && !s.disabled));
        assert!(read_skill(dir.path(), "any").is_none());
        assert!(read_skill(dir.path(), "tidy").is_some());
    }

    #[test]
    fn vault_file_overrides_the_builtin_of_the_same_name() {
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join("skills");
        fs::create_dir_all(&skills).unwrap();
        // the file name does not matter, the listed name does; the copy carries the
        // built-in's own version, as a fresh "customise" would
        let version = builtin_skill("research").unwrap().version;
        fs::write(skills.join("my-research.md"), format!("---\nname: research\ndescription: mine\nversion: {version}\n---\nMY WAY\n")).unwrap();
        let all = list_skills_all(dir.path());
        let rows: Vec<&SkillFileEntry> = all.iter().filter(|s| s.name == "research").collect();
        assert_eq!(rows.len(), 1, "one row per name");
        assert_eq!(rows[0].source, SkillSource::Override);
        assert_eq!(rows[0].description, "mine");
        assert_eq!(rows[0].file, "my-research.md");
        assert!(!rows[0].outdated, "same version as the built-in");
        assert_eq!(read_skill(dir.path(), "research").unwrap(), "MY WAY");
        // the prompt view sees one research too
        assert_eq!(list_skills(dir.path()).iter().filter(|s| s.name == "research").count(), 1);
    }

    #[test]
    fn a_copy_without_a_version_is_outdated_and_a_disabled_override_hides_the_builtin() {
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join("skills");
        fs::create_dir_all(&skills).unwrap();
        fs::write(skills.join("research.md"), "---\ndescription: old copy\n---\nOLD\n").unwrap();
        let row = list_skills_all(dir.path()).into_iter().find(|s| s.name == "research").unwrap();
        assert!(row.outdated, "a copy that carries no version predates versioning");
        assert_eq!(row.version, 0);
        fs::write(skills.join("research.md"), "---\ndescription: old copy\ndisabled: true\n---\nOLD\n").unwrap();
        let row = list_skills_all(dir.path()).into_iter().find(|s| s.name == "research").unwrap();
        assert!(row.disabled && row.source == SkillSource::Override);
        assert!(read_skill(dir.path(), "research").is_none(), "the built-in does not show through a disabled override");
        assert!(!list_skills(dir.path()).iter().any(|s| s.name == "research"));
    }

    #[test]
    fn builtin_switched_off_in_the_vault_config() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("config.json"), r#"{"write_policy":"ask","skills_disabled":["research","ghost"]}"#).unwrap();
        let all = list_skills_all(dir.path());
        let research = all.iter().find(|s| s.name == "research").unwrap();
        assert!(research.disabled && research.source == SkillSource::Builtin, "listed for re-enabling, off for the agent");
        assert!(all.iter().filter(|s| s.disabled).count() == 1, "an unknown name in the list switches nothing off");
        assert!(read_skill(dir.path(), "research").is_none());
        assert!(read_skill(dir.path(), "tidy").is_some());
        assert!(!list_skills(dir.path()).iter().any(|s| s.name == "research"));
        // a malformed config switches nothing off
        fs::write(dir.path().join("config.json"), "{not json").unwrap();
        assert!(read_skill(dir.path(), "research").is_some());
    }

    #[test]
    fn triggers_read_as_a_list_or_a_line_and_through_the_fallback() {
        let (fm, _) = parse("---\nname: a\ntriggers: [research, 调研, look into]\n---\nB\n");
        assert_eq!(fm.triggers, vec!["research", "调研", "look into"]);
        let (fm, _) = parse("---\nname: a\ntriggers: research, 调研\n---\nB\n");
        assert_eq!(fm.triggers, vec!["research", "调研"]);
        // a colon in the description sends the block through the line scan
        let (fm, _) = parse("---\ndescription: a: b\ntriggers: [\"x y\", z]\n---\nB\n");
        assert_eq!(fm.description.as_deref(), Some("a: b"));
        assert_eq!(fm.triggers, vec!["x y", "z"]);
    }

    #[test]
    fn skeleton_has_every_section_and_parses_as_a_skill() {
        let s = skill_skeleton(" my-routine ");
        let (fm, body) = parse(&s);
        assert_eq!(fm.name.as_deref(), Some("my-routine"));
        assert!(!fm.disabled);
        for section in ["## When", "## Plan", "## Inputs", "## Do", "## Landing", "## Shape", "## Never"] {
            assert!(body.contains(section), "missing {section}");
        }
    }

    #[test]
    fn a_colon_in_the_description_keeps_the_description_and_the_disabled_flag() {
        // `description: The routine: what is due` is not YAML serde accepts; the
        // whole block used to degrade to nothing — no description in the prompt
        // and, worse, a `disabled: true` below it lost
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join("skills");
        fs::create_dir_all(&skills).unwrap();
        fs::write(skills.join("morning.md"), "---\ndescription: The routine: what is due today\nversion: 3\n---\nBODY\n").unwrap();
        fs::write(skills.join("gone.md"), "---\ndescription: The routine: what is due today\ndisabled: true\n---\nHIDDEN\n").unwrap();
        let all = list_skills_all(dir.path());
        let morning = all.iter().find(|s| s.name == "morning").unwrap();
        assert_eq!(morning.description, "The routine: what is due today");
        assert_eq!(morning.version, 3);
        assert_eq!(read_skill(dir.path(), "morning").unwrap(), "BODY");
        let gone = all.iter().find(|s| s.name == "gone").unwrap();
        assert!(gone.disabled, "disabled must survive the YAML edge");
        assert!(read_skill(dir.path(), "gone").is_none());
    }

    #[test]
    fn frontmatter_disabled_toggle_roundtrip() {
        use vemry_core::set_frontmatter_disabled;
        // With frontmatter: inserted before the closing line; removed again when
        // re-enabled, with the other keys untouched
        let raw = "---\nname: research\ndescription: d\n---\n正文\n";
        let off = set_frontmatter_disabled(raw, true);
        assert_eq!(off, "---\nname: research\ndescription: d\ndisabled: true\n---\n正文\n");
        let on = set_frontmatter_disabled(&off, false);
        assert_eq!(on, raw);
        // Without frontmatter: disabling adds one, enabling is a no-op
        let plain = "纯正文技能\n";
        let off = set_frontmatter_disabled(plain, true);
        assert_eq!(off, "---\ndisabled: true\n---\n纯正文技能\n");
        assert_eq!(set_frontmatter_disabled(plain, false), plain);
        // Idempotent: disabling twice does not produce a second line
        let twice = set_frontmatter_disabled(&set_frontmatter_disabled(raw, true), true);
        assert_eq!(twice.matches("disabled").count(), 1);
    }

    #[test]
    fn frontmatter_edge_closers_still_hide_disabled() {
        // trailing whitespace on the closing fence and `...` terminators are
        // valid YAML frontmatter — `disabled: true` must not be lost to a
        // parser edge, or the skill leaks back into the prompt
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join("skills");
        fs::create_dir_all(&skills).unwrap();
        fs::write(skills.join("sp.md"), "---\ndescription: d\ndisabled: true\n--- \nBODY_SP\n").unwrap();
        fs::write(skills.join("dots.md"), "---\ndisabled: true\n...\nBODY_DOTS\n").unwrap();
        assert!(vault_names(dir.path()).iter().all(|n| n == "sp" || n == "dots"));
        assert!(!list_skills(dir.path()).iter().any(|s| s.name == "sp" || s.name == "dots"));
        assert!(read_skill(dir.path(), "sp").is_none());
        assert!(read_skill(dir.path(), "dots").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_skills_are_refused() {
        // a symlink in .vemry/skills/ could point at privacy-zone content;
        // this agent-visible read path must not follow it
        let dir = tempfile::tempdir().unwrap();
        let skills = dir.path().join("skills");
        fs::create_dir_all(&skills).unwrap();
        let secret = dir.path().join("Diary.md");
        fs::write(&secret, "---\nprivate: true\n---\nTOPSECRET\n").unwrap();
        std::os::unix::fs::symlink(&secret, skills.join("leak.md")).unwrap();
        assert!(vault_names(dir.path()).is_empty(), "symlink must not be listed");
        assert!(read_skill(dir.path(), "leak").is_none(), "symlink must not be readable");
    }
}
