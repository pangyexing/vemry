//! User-facing copy for the surfaces Rust owns: the native menu bar, IPC error
//! strings, window titles. Reads the **same** `locales/*.json` files the frontend
//! reads — one catalog, two consumers, the api.ts ↔ commands.rs mirror discipline
//! applied to wording. English and Chinese are complete; the other languages are
//! overlays that fall back to English key by key (see locales/src/README.md).
//!
//! Only keys Rust actually uses need to resolve here; `tests` asserts every one
//! of them exists in the two complete catalogs, so a key renamed on the TS side
//! breaks the build rather than silently rendering its own name in a menu.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use vemry_core::VemryError;

use crate::settings::LangSetting;

/// The interface languages, in the order the settings picker lists them. Mirrored
/// by LANGS in scripts/gen-locales.py and src/lib/i18n/i18n.svelte.ts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhCn,
    Ja,
    Ko,
    Fr,
    De,
    Es,
}

pub const LANGS: [Lang; 7] = [Lang::En, Lang::ZhCn, Lang::Ja, Lang::Ko, Lang::Fr, Lang::De, Lang::Es];

impl Lang {
    /// The BCP 47 tag, as the settings wire value and the frontend spell it.
    pub fn tag(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::ZhCn => "zh-CN",
            Lang::Ja => "ja",
            Lang::Ko => "ko",
            Lang::Fr => "fr",
            Lang::De => "de",
            Lang::Es => "es",
        }
    }

    fn index(self) -> usize {
        LANGS.iter().position(|l| *l == self).unwrap_or(0)
    }

    /// The language a BCP 47 tag resolves to: `zh-*` is Simplified Chinese for now,
    /// the others match on the primary subtag, anything else is English. The
    /// frontend's `langForTag` applies the same rule to navigator.language.
    pub fn for_tag(tag: &str) -> Lang {
        let lower = tag.to_ascii_lowercase();
        let primary = lower.split(['-', '_']).next().unwrap_or("");
        match primary {
            "zh" => Lang::ZhCn,
            "ja" => Lang::Ja,
            "ko" => Lang::Ko,
            "fr" => Lang::Fr,
            "de" => Lang::De,
            "es" => Lang::Es,
            _ => Lang::En,
        }
    }
}

const JSON: [&str; 7] = [
    include_str!("../../locales/en.json"),
    include_str!("../../locales/zh-CN.json"),
    include_str!("../../locales/ja.json"),
    include_str!("../../locales/ko.json"),
    include_str!("../../locales/fr.json"),
    include_str!("../../locales/de.json"),
    include_str!("../../locales/es.json"),
];

fn catalog(lang: Lang) -> &'static HashMap<String, String> {
    static CATALOGS: [OnceLock<HashMap<String, String>>; 7] = [const { OnceLock::new() }; 7];
    let i = lang.index();
    CATALOGS[i].get_or_init(|| serde_json::from_str(JSON[i]).unwrap_or_else(|e| panic!("locales/{}.json: {e}", lang.tag())))
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

/// Whether the catalog carries a key — for a key assembled at runtime (the
/// hosted relay's error codes, `ipc.hosted.<code>`), where "unknown" must fall
/// back to something else rather than render the key.
pub fn has_key(key: &str) -> bool {
    catalog(Lang::En).contains_key(key)
}

/// Resolve the stored preference. `Auto` follows the OS locale — the frontend
/// resolves `auto` against `navigator.language` separately, and the two agree
/// in every case that matters (both ask the same OS).
pub fn resolve(setting: LangSetting) -> Lang {
    match setting {
        LangSetting::En => Lang::En,
        LangSetting::ZhCn => Lang::ZhCn,
        LangSetting::Ja => Lang::Ja,
        LangSetting::Ko => Lang::Ko,
        LangSetting::Fr => Lang::Fr,
        LangSetting::De => Lang::De,
        LangSetting::Es => Lang::Es,
        LangSetting::Auto => system_lang(),
    }
}

/// The process-wide current language. Errors bubble up through dozens of
/// `map_err` sites that have no `AppHandle` in scope; threading one through all
/// of them to render a message would be noise. Language is a process-level
/// setting, so it lives here — set once at startup and again on every change.
static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set_current(lang: Lang) {
    CURRENT.store(lang.index() as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    LANGS.get(CURRENT.load(Ordering::Relaxed) as usize).copied().unwrap_or(Lang::En)
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
    sys_locale::get_locale().map(|l| Lang::for_tag(&l)).unwrap_or(Lang::En)
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
        "menu.trayOpen",
        "menu.trayQuit",
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
        "cmd.view-review.title",
        "cmd.view-reading.title",
        "cmd.translate-toggle.title",
        "translate.privateNote",
        "translate.nothingToTranslate",
        "translate.pageExpired",
        "translate.privateFile",
        "translate.pdfEmpty",
        "translate.noSuchJob",
        "translate.untranslated",
        "translate.pausedHere",
        "translate.continue",
        "translate.translateAll",
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
        "window.agentTitle",
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
        "ipc.notAVault",
        "ipc.imageFormatUnsupported",
        "ipc.copyImageUnsupported",
        "ipc.httpOnly",
        "ipc.agentRunning",
        "ipc.noModel",
        "ipc.noEndpointFor",
        "ipc.noApiKeyFor",
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
        "ipc.skillNotFound",
        "ipc.skillNameEmpty",
        "ipc.rulesUpdateRunning",
        "ipc.noRuleSource",
        "ipc.noRuleFile",
        "ipc.notAFilterList",
        // The hosted account and relay (account.rs): the sign-in states, the relay's
        // error codes (`ipc.hosted.<code>`, looked up through has_key), the fallbacks
        "ipc.hosted.signIn",
        "ipc.hosted.expired",
        "ipc.hosted.network",
        "ipc.hosted.failed",
        "ipc.hosted.auth_invalid",
        "ipc.hosted.insufficient_balance",
        "ipc.hosted.hosted_not_allowed",
        "ipc.hosted.rate_limited",
        "ipc.hosted.daily_cap",
        "ipc.hosted.upstream_unavailable",
        "ipc.hosted.upstream_busy",
        "ipc.hosted.spend_cap",
        "ipc.hosted.hosted_disabled",
        "ipc.hosted.model_unknown",
        "ipc.hosted.payload_too_large",
        "ipc.hosted.moderation_refused",
        "ipc.hosted.email_invalid",
        "ipc.hosted.otp_invalid",
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
        "err.symlink",
        "err.empty",
        "err.blockNotFound",
        "err.blockChanged",
        "err.blockNotIdable",
        "window.captureTitle",
        "ipc.captureNoMainWindow",
        "ipc.captureHotkeyInvalid",
    ];

    #[test]
    fn both_catalogs_parse_and_carry_every_key_rust_uses() {
        for key in RUST_KEYS {
            for lang in [Lang::En, Lang::ZhCn] {
                assert!(catalog(lang).contains_key(*key), "missing {key} in {lang:?}");
            }
        }
    }

    /// The English keys a translated key answers to: the key itself, or for a
    /// plural form the English `_other` (and `_one` for a language's `one`, which
    /// may spell out "1" as the English does). A form English lacks, such as
    /// French `_many`, still belongs to an English plural family.
    fn english_refs(key: &str) -> Vec<&'static String> {
        let en = catalog(Lang::En);
        let plural = key
            .rsplit_once('_')
            .filter(|(base, form)| ["zero", "one", "two", "few", "many", "other"].contains(form) && en.contains_key(&format!("{base}_other")));
        let Some((base, form)) = plural else { return en.get(key).into_iter().collect() };
        let mut refs = vec![&en[&format!("{base}_other")]];
        if form == "one" {
            refs.extend(en.get(&format!("{base}_one")));
        }
        refs
    }

    /// Every overlay parses, and carries no key English does not — a stale key
    /// in an overlay would be dead weight the generator already refuses.
    #[test]
    fn overlays_are_subsets_of_english() {
        for lang in LANGS {
            for key in catalog(lang).keys() {
                assert!(!english_refs(key).is_empty(), "{key} in {lang:?} is not an English key");
            }
        }
    }

    /// The VEMRY.md section a language scaffolds is one `memory_append` can find
    /// again, and the scaffold's own template opens that same section.
    #[test]
    fn every_language_scaffolds_a_memory_section_core_knows() {
        for lang in LANGS {
            let section = t(lang, "memoryFile.section");
            assert!(
                vemry_core::vault::MEMORY_SECTIONS.contains(&section.as_str()),
                "{lang:?}: memoryFile.section {section:?} is missing from vault.rs MEMORY_SECTIONS"
            );
            let template = t(lang, "memoryFile.template");
            assert!(template.contains(&format!("\n## {section}\n")), "{lang:?}: the template has no \"## {section}\" heading");
        }
    }

    #[test]
    fn tags_round_trip() {
        for lang in LANGS {
            assert_eq!(Lang::for_tag(lang.tag()), lang);
        }
        assert_eq!(Lang::for_tag("zh-TW"), Lang::ZhCn);
        assert_eq!(Lang::for_tag("de_AT"), Lang::De);
        assert_eq!(Lang::for_tag("es-MX"), Lang::Es);
        assert_eq!(Lang::for_tag("en-GB"), Lang::En);
        assert_eq!(Lang::for_tag("pt-BR"), Lang::En);
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
        for lang in LANGS {
            for (key, text) in catalog(lang) {
                let refs = english_refs(key);
                assert!(
                    refs.iter().any(|en_text| names(en_text) == names(text)),
                    "placeholder mismatch in {key} ({lang:?})"
                );
            }
        }
    }

    #[test]
    fn catalogs_have_identical_key_sets() {
        let en: std::collections::BTreeSet<_> = catalog(Lang::En).keys().collect();
        let zh: std::collections::BTreeSet<_> = catalog(Lang::ZhCn).keys().collect();
        assert_eq!(en, zh, "locales/en.json and locales/zh-CN.json must carry the same keys");
    }
}
