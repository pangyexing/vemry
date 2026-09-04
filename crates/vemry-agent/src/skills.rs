//! Instruction-level skills (v0.3): `.vemry/skills/*.md` teach the agent
//! vault-specific playbooks on demand. VEMRY.md is the always-loaded
//! constitution; skills load lazily — the system prompt carries only
//! `name — description`, the model pulls the full text via the `read_skill`
//! tool when the task matches.
//!
//! Privacy note: `.vemry` is normally invisible to the agent (dot-path =
//! built-in privacy zone). Skills are a deliberate, narrow exception — their
//! whole purpose is to be read by the agent. `read_skill` resolves the name
//! against the scanned listing (never joins caller input into a path), so
//! only `.md` files directly in the skills dir are reachable. Hide one from
//! the agent with `disabled: true` or by deleting the file.

use std::fs;
use std::path::Path;
use vemry_core::parser::extract_frontmatter;

/// Skills live in `<meta_dir>/skills` (i.e. `.vemry/skills/`).
pub const SKILLS_SUBDIR: &str = "skills";
/// Same reading budget as `read_note`.
const READ_SKILL_MAX_CHARS: usize = 30_000;

#[derive(Debug, Default)]
struct SkillFrontmatter {
    name: Option<String>,
    description: Option<String>,
    disabled: bool,
}

#[derive(Clone, Debug)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
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
    let skill = SkillFrontmatter {
        name: text("name"),
        description: text("description"),
        disabled: fm.extra.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false),
    };
    (skill, &raw[body_offset..])
}

/// One skill from the management point of view (`file` = the file name; disabled
/// ones are listed too).
#[derive(Clone, Debug)]
pub struct SkillFileEntry {
    pub file: String,
    pub name: String,
    pub description: String,
    pub disabled: bool,
}

/// All skill files (including disabled), sorted by name; `name` defaults to
/// the file stem. The one directory scan both views share — the settings page
/// lists this directly, `list_skills` (the agent-facing view) filters it, so
/// the symlink refusal and frontmatter edge cases can't drift between them.
pub fn list_skills_all(meta_dir: &Path) -> Vec<SkillFileEntry> {
    let dir = meta_dir.join(SKILLS_SUBDIR);
    let mut skills: Vec<SkillFileEntry> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                // regular files only: a symlink here could point into the
                // privacy zone and this is an agent-visible read path
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                .filter_map(|e| {
                    let stem = e.path().file_stem()?.to_string_lossy().to_string();
                    let file = e.file_name().to_string_lossy().to_string();
                    let raw = fs::read_to_string(e.path()).ok()?;
                    let (fm, _) = parse(&raw);
                    Some(SkillFileEntry {
                        file,
                        name: fm.name.filter(|n| !n.trim().is_empty()).unwrap_or(stem),
                        description: fm.description.unwrap_or_default(),
                        disabled: fm.disabled,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
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
/// resolved by exact listed name. `None` covers missing and disabled alike —
/// callers report both as `skill not found`.
pub fn read_skill(meta_dir: &Path, name: &str) -> Option<String> {
    let dir = meta_dir.join(SKILLS_SUBDIR);
    let entries = fs::read_dir(dir).ok()?;
    for e in entries.filter_map(|e| e.ok()) {
        // same symlink refusal as list_skills: this tool must not become a
        // read path into privacy-zone content
        if !e.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        if !e.path().extension().is_some_and(|x| x == "md") {
            continue;
        }
        let Some(stem) = e.path().file_stem().map(|s| s.to_string_lossy().to_string()) else { continue };
        let Ok(raw) = fs::read_to_string(e.path()) else { continue };
        let (fm, body) = parse(&raw);
        let skill_name = fm.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or(stem);
        if skill_name != name || fm.disabled {
            continue;
        }
        let body = body.trim();
        return Some(
            crate::truncate_chars(body, READ_SKILL_MAX_CHARS, &format!("[... truncated at {READ_SKILL_MAX_CHARS} chars]"))
                .unwrap_or_else(|| body.to_string()),
        );
    }
    None
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

    #[test]
    fn list_sorted_named_and_disabled_excluded() {
        let dir = fixture();
        let skills = list_skills(dir.path());
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        // sorted, fm name override wins, disabled and non-md excluded
        assert_eq!(names, vec!["plain", "weekly-review", "文献笔记"]);
        let weekly = skills.iter().find(|s| s.name == "weekly-review").unwrap();
        assert_eq!(weekly.description, "每周回顾怎么写");
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
    }

    #[test]
    fn missing_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(list_skills(dir.path()).is_empty());
        assert!(list_skills_all(dir.path()).is_empty());
        assert!(read_skill(dir.path(), "any").is_none());
    }

    #[test]
    fn list_all_includes_disabled_with_file_names() {
        let dir = fixture();
        let all = list_skills_all(dir.path());
        let off = all.iter().find(|s| s.file == "off.md").unwrap();
        assert!(off.disabled, "management view keeps disabled entries");
        // The injection view is the management list projected without the disabled ones
        let enabled: Vec<&str> = all.iter().filter(|s| !s.disabled).map(|s| s.name.as_str()).collect();
        let listed: Vec<String> = list_skills(dir.path()).into_iter().map(|s| s.name).collect();
        assert_eq!(listed, enabled);
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
        assert!(list_skills(dir.path()).is_empty());
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
        assert!(list_skills(dir.path()).is_empty(), "symlink must not be listed");
        assert!(read_skill(dir.path(), "leak").is_none(), "symlink must not be readable");
    }
}
