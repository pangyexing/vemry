//! Privacy zone: paths the AI agent must never see. Rules live in
//! `.vemry/privacy` (gitignore syntax, one glob per line) and travel with the
//! vault — privacy is a property of the vault, not of a session or model
//! endpoint. The per-note half (`private: true` frontmatter) lives in the
//! index; this module compiles the glob half.
//!
//! Read semantics are "does not exist", never "no permission": callers filter
//! private paths out of results instead of reporting them as forbidden,
//! because an error would leak that the path exists.

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::fs;
use std::path::Path;
use unicode_normalization::UnicodeNormalization;

/// Vault-relative path of the rules file. The file itself is inside `.vemry`
/// and therefore always private (hidden segments are built-in rules).
pub const PRIVACY_FILE: &str = ".vemry/privacy";

pub struct PrivacyScope {
    matcher: Gitignore,
    rules: Vec<String>,
}

impl PrivacyScope {
    /// Load `<root>/.vemry/privacy`. Missing file means no user rules; bad
    /// glob lines are skipped — privacy must never make the vault unopenable.
    pub fn load(root: &Path) -> PrivacyScope {
        match fs::read_to_string(root.join(PRIVACY_FILE)) {
            Ok(content) => Self::from_lines(content.lines()),
            Err(_) => Self::from_lines(std::iter::empty::<&str>()),
        }
    }

    pub fn from_lines<'a>(lines: impl IntoIterator<Item = &'a str>) -> PrivacyScope {
        let mut builder = GitignoreBuilder::new("");
        let _ = builder.case_insensitive(true); // APFS opens `SECRET/` for a `secret/` rule
        let mut rules = Vec::new();
        for line in lines {
            let line: String = line.nfc().collect(); // NoteId paths are NFC
            let _ = builder.add_line(None, &line); // bad globs skipped
            if !line.trim().is_empty() {
                rules.push(line);
            }
        }
        let matcher = builder.build().unwrap_or_else(|_| Gitignore::empty());
        PrivacyScope { matcher, rules }
    }

    /// Raw rule lines as stored (comments included), for UI display.
    pub fn rules(&self) -> &[String] {
        &self.rules
    }

    /// Whether a vault-relative path is in the privacy zone. `rel` must be
    /// `/`-separated and NFC-normalized (`NoteId` already is). Any path with
    /// a dot-leading segment is private by construction — that covers
    /// `.vemry/privacy` itself, session logs and `.obsidian`.
    pub fn matches_rel(&self, rel: &str, is_dir: bool) -> bool {
        if rel.split('/').any(|seg| seg.starts_with('.')) {
            return true;
        }
        // `_or_any_parents`: a `secret/` rule must catch `secret/deep/a.md`.
        self.matcher.matched_path_or_any_parents(rel, is_dir).is_ignore()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dir_rule_covers_descendants() {
        let s = PrivacyScope::from_lines(["secret/"]);
        assert!(s.matches_rel("secret", true));
        assert!(s.matches_rel("secret/a.md", false));
        assert!(s.matches_rel("secret/deep/b.md", false));
        assert!(!s.matches_rel("public/a.md", false));
        assert!(!s.matches_rel("secretive.md", false));
        // bare name (no trailing slash) works the same, gitignore-style
        let s = PrivacyScope::from_lines(["secret"]);
        assert!(s.matches_rel("secret/a.md", false));
        assert!(s.matches_rel("secret", true));
    }

    #[test]
    fn star_and_negation() {
        let s = PrivacyScope::from_lines(["diary/**", "!diary/Public.md"]);
        assert!(s.matches_rel("diary/2026.md", false));
        assert!(s.matches_rel("diary/deep/x.md", false));
        assert!(!s.matches_rel("diary/Public.md", false));
    }

    #[test]
    fn case_insensitive_matching() {
        let s = PrivacyScope::from_lines(["secret/"]);
        assert!(s.matches_rel("Secret/a.md", false));
        assert!(s.matches_rel("SECRET/A.MD", false));
    }

    #[test]
    fn rules_are_nfc_normalized() {
        // rule written in NFD ("e" + combining acute), path arrives NFC
        let rule = "cafe\u{0301}/";
        let s = PrivacyScope::from_lines([rule]);
        assert!(s.matches_rel("caf\u{e9}/x.md", false));
    }

    #[test]
    fn hidden_segments_always_private() {
        let s = PrivacyScope::from_lines(std::iter::empty::<&str>());
        assert!(s.matches_rel(".vemry/privacy", false));
        assert!(s.matches_rel(".vemry/sessions/x.jsonl", false));
        assert!(s.matches_rel(".obsidian/a.md", false));
        assert!(s.matches_rel("sub/.hidden/a.md", false));
        assert!(!s.matches_rel("normal.md", false));
    }

    #[test]
    fn bad_and_comment_lines_skipped() {
        let s = PrivacyScope::from_lines(["# comment", "", "a**b**c[", "secret/"]);
        assert!(s.matches_rel("secret/a.md", false));
        assert!(!s.matches_rel("comment", false));
    }
}
