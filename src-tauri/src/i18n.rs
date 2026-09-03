//! User-facing copy for the surfaces Rust owns: the native menu bar, IPC error
//! strings, window titles. Reads the **same** `locales/*.json` pair the frontend
//! reads — one catalog, two consumers, the api.ts ↔ commands.rs mirror discipline
//! applied to wording.
//!
//! Only keys Rust actually uses need to resolve here; `tests` asserts every one
//! of them exists in both catalogs, so a key renamed on the TS side breaks the
//! build rather than silently rendering its own name in a menu.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use vemry_core::VemryError;

use crate::settings::LangSetting;

const EN_JSON: &str = include_str!("../../locales/en.json");
const ZH_JSON: &str = include_str!("../../locales/zh-CN.json");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhCn,
}

fn catalog(lang: Lang) -> &'static HashMap<String, String> {
    static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    static ZH: OnceLock<HashMap<String, String>> = OnceLock::new();
    match lang {
        Lang::En => EN.get_or_init(|| serde_json::from_str(EN_JSON).expect("locales/en.json")),
        Lang::ZhCn => ZH.get_or_init(|| serde_json::from_str(ZH_JSON).expect("locales/zh-CN.json")),
    }
}

/// Look up a message, falling back to English and then to the key itself — a
/// menu with a raw key in it is ugly but still clickable.
pub fn t(lang: Lang, key: &str) -> String {
    catalog(lang)
        .get(key)
        .or_else(|| catalog(Lang::En).get(key))
        .cloned()
        .unwrap_or_else(|| key.to_string())
}

/// Resolve the stored preference. `Auto` follows the OS locale — the frontend
/// resolves `auto` against `navigator.language` separately, and the two agree
/// in every case that matters (both ask the same OS).
pub fn resolve(setting: LangSetting) -> Lang {
    match setting {
        LangSetting::En => Lang::En,
        LangSetting::ZhCn => Lang::ZhCn,
        LangSetting::Auto => system_lang(),
    }
}

/// The process-wide current language. Errors bubble up through dozens of
/// `map_err` sites that have no `AppHandle` in scope; threading one through all
/// of them to render a message would be noise. Language is a process-level
/// setting, so it lives here — set once at startup and again on every change.
static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_current(lang: Lang) {
    CURRENT.store(if lang == Lang::ZhCn { 1 } else { 0 }, Ordering::Relaxed);
}

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == 1 {
        Lang::ZhCn
    } else {
        Lang::En
    }
}

/// Interpolate `{name}` placeholders. Small and allocation-light: messages have
/// one or two parameters.
pub fn tf(lang: Lang, key: &str, params: &[(&str, &str)]) -> String {
    let mut s = t(lang, key);
    for (name, value) in params {
        s = s.replace(&format!("{{{name}}}"), value);
    }
    s
}

/// Render a core error in the user's language. Every IPC boundary that hands a
/// `VemryError` to the frontend goes through this — the frontend only ever
/// receives a finished string, so this is the single place the wording is
/// chosen. Non-path errors keep their `Display` (io/yaml/watch: developer-facing
/// detail with no useful translation).
pub fn err(e: VemryError) -> String {
    match &e {
        VemryError::InvalidPath { reason, path } => tf(current(), reason.key(), &[("path", path)]),
        VemryError::BlockNotFound { local_id } => tf(current(), "err.blockNotFound", &[("id", local_id)]),
        VemryError::BlockChanged { local_id } => tf(current(), "err.blockChanged", &[("id", local_id)]),
        VemryError::BlockNotIdable => tf(current(), "err.blockNotIdable", &[]),
        _ => e.to_string(),
    }
}

fn system_lang() -> Lang {
    match sys_locale::get_locale() {
        Some(l) if l.to_ascii_lowercase().starts_with("zh") => Lang::ZhCn,
        _ => Lang::En,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key Rust looks up, so a rename on the TS side fails the build here
    /// instead of surfacing as a raw key in the menu bar. Two families are not
    /// spelled out at their call sites and are the easy ones to forget when this
    /// list is extended: `cmd.{id}.title`, assembled in menu.rs from the command
    /// ids, and the `err.` keys, which come from `PathReject::key()` in vemry-core.
    const RUST_KEYS: &[&str] = &[
        "menu.file",
        "menu.edit",
        "menu.view",
        "menu.research",
        "menu.window",
        "menu.help",
        "menu.settings",
        "menu.checkUpdates",
        "menu.quickCapture",
        "menu.closeWindow",
        "menu.helpGithub",
        "cmd.new-note.title",
        "cmd.open-note.title",
        "cmd.create-vault.title",
        "cmd.open-vault.title",
        "cmd.switch-vault.title",
        "cmd.search.title",
        "cmd.view-agenda.title",
        "cmd.view-kanban.title",
        "cmd.view-timeline.title",
        "cmd.view-claims.title",
        "cmd.view-evidence.title",
        "cmd.view-graph.title",
        "cmd.view-sessions.title",
        "cmd.view-reading.title",
        "cmd.toggle-left.title",
        "cmd.toggle-right.title",
        "cmd.toggle-sidebar-side.title",
        "cmd.show-agent.title",
        "cmd.show-outline.title",
        "cmd.focus-agent.title",
        "cmd.clear-agent.title",
        "cmd.open-browser.title",
        "cmd.clip-page.title",
        "cmd.clip-link.title",
        "cmd.open-memory.title",
        "cmd.init-memory.title",
        "cmd.init-research-skill.title",
        "window.agentTitle",
        "window.clipbarTitle",
        "window.browserTitle",
        "window.readerTitle",
        "clip.sourceLabel",
        "memoryFile.header",
        "memoryFile.section",
        "memoryFile.template",
        // Command errors handed back over IPC and rendered as a toast. In the order
        // of the `ipc.` table in gen-locales.py, so the two read side by side —
        // `ipc.trashFailed` is missing on purpose: the frontend is what looks that
        // one up.
        "ipc.fileExists",
        "ipc.dirNotEmpty",
        "ipc.imageFormatUnsupported",
        "ipc.copyImageUnsupported",
        "ipc.httpOnly",
        "ipc.agentRunning",
        "ipc.noApiKey",
        "ipc.nothingToRetry",
        "ipc.runningNoEditResend",
        "ipc.sessionEmpty",
        "ipc.runningNoFork",
        "ipc.noSessionToFork",
        "ipc.runningNoNewSession",
        "ipc.runningNoSwitch",
        "ipc.runningNoDelete",
        "ipc.contentEmpty",
        "ipc.typeNameEmpty",
        "ipc.typeNotFound",
        "ipc.typeFrontmatterBroken",
        "ipc.memoryExists",
        "ipc.rulesUpdateRunning",
        "ipc.noRuleSource",
        "ipc.noRuleFile",
        "ipc.notAFilterList",
        // Path rejections, reached through `err()` above rather than by name: the
        // keys live in `PathReject::key()` in vemry-core, so grepping this crate for
        // a string literal will not find them. Renaming one is a two-file edit
        // (core's match arm and the catalog); this list is what fails if it is one.
        "err.notFound",
        "err.notAFile",
        "err.notADirectory",
        "err.tooLarge",
        "err.notUtf8",
        "err.exists",
        "err.attachmentIsMd",
        "err.mdNeedsNoteApi",
        "err.metaNotANote",
        "err.isTheVaultItself",
        "err.folderIntoItself",
        "err.reservedName",
        "err.escapes",
        "err.empty",
        "err.blockNotFound",
        "err.blockChanged",
        "err.blockNotIdable",
    ];

    #[test]
    fn both_catalogs_parse_and_carry_every_key_rust_uses() {
        for key in RUST_KEYS {
            for lang in [Lang::En, Lang::ZhCn] {
                assert!(catalog(lang).contains_key(*key), "missing {key} in {lang:?}");
            }
        }
    }

    /// Same key, same placeholders. A caller passes one set of names, so if the
    /// two translations disagree the other language renders a literal `{foo}` —
    /// invisible to the key-set test above and to `pnpm check`, and it only
    /// shows up in whichever language the developer was not looking at.
    #[test]
    fn placeholders_match_across_languages() {
        fn names(s: &str) -> std::collections::BTreeSet<&str> {
            let mut out = std::collections::BTreeSet::new();
            let mut rest = s;
            while let Some(open) = rest.find('{') {
                rest = &rest[open + 1..];
                let Some(close) = rest.find('}') else { break };
                let name = &rest[..close];
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    out.insert(name);
                }
                rest = &rest[close + 1..];
            }
            out
        }
        let (en, zh) = (catalog(Lang::En), catalog(Lang::ZhCn));
        for (key, en_text) in en {
            let Some(zh_text) = zh.get(key) else { continue };
            assert_eq!(names(en_text), names(zh_text), "placeholder mismatch in {key}");
        }
    }

    #[test]
    fn catalogs_have_identical_key_sets() {
        let en: std::collections::BTreeSet<_> = catalog(Lang::En).keys().collect();
        let zh: std::collections::BTreeSet<_> = catalog(Lang::ZhCn).keys().collect();
        assert_eq!(en, zh, "locales/en.json and locales/zh-CN.json must carry the same keys");
    }
}
