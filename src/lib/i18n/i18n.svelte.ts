// User-facing copy lives in `locales/*.json` — one set of files read by both this
// module and `src-tauri/src/i18n.rs`, the same mirror discipline as api.ts ↔
// commands.rs. Nothing user-visible may be hardcoded in a component.
//
// English and Chinese are written in scripts/gen-locales.py; every other language
// is an overlay (locales/src/<tag>.json) that carries only what has been
// translated, and `t()` falls back to English for the rest. Wording for any new
// term goes through docs/GLOSSARY.md first.
import { api, emitLanguage, onLanguageChange } from "$lib/api";

import en from "../../../locales/en.json";
import zhCN from "../../../locales/zh-CN.json";
import ja from "../../../locales/ja.json";
import ko from "../../../locales/ko.json";
import fr from "../../../locales/fr.json";
import de from "../../../locales/de.json";
import es from "../../../locales/es.json";

/** The interface languages, in the order the settings picker lists them. Mirrored
 *  by LANGS in scripts/gen-locales.py and the Lang enum in src-tauri/src/i18n.rs. */
export const LANGS = ["en", "zh-CN", "ja", "ko", "fr", "de", "es"] as const;
export type Lang = (typeof LANGS)[number];
/** What the user picked; "auto" resolves against the browser locale. */
export type LangSetting = "auto" | Lang;

export type MessageKey = keyof typeof en;

// A key missing from zh-CN is a `pnpm check` error…
const EN: Record<MessageKey, string> = en;
const ZH: Record<MessageKey, string> = zhCN;

// …and so is a stale key left behind in any catalog but gone from en.
type Assert<T extends never> = T;
export type _NoStaleZhKeys = Assert<Exclude<keyof typeof zhCN, MessageKey>>;
export type _NoStaleJaKeys = Assert<Exclude<keyof typeof ja, MessageKey>>;
export type _NoStaleKoKeys = Assert<Exclude<keyof typeof ko, MessageKey>>;
export type _NoStaleFrKeys = Assert<Exclude<keyof typeof fr, MessageKey>>;
export type _NoStaleDeKeys = Assert<Exclude<keyof typeof de, MessageKey>>;
export type _NoStaleEsKeys = Assert<Exclude<keyof typeof es, MessageKey>>;

const CATALOGS: Record<Lang, Partial<Record<MessageKey, string>>> = { en: EN, "zh-CN": ZH, ja, ko, fr, de, es };

/** The interface language a BCP 47 tag resolves to: `zh-*` is Simplified Chinese
 *  for now (the translation feature knows Traditional; the interface does not yet),
 *  the others match on the primary subtag, anything else is English. */
export function langForTag(tag: string): Lang {
  const primary = tag.toLowerCase().split(/[-_]/)[0];
  if (primary === "zh") return "zh-CN";
  const hit = LANGS.find((l) => l === primary);
  return hit ?? "en";
}

function detectSystemLang(): Lang {
  const nav = typeof navigator === "undefined" ? "" : navigator.language;
  return langForTag(nav);
}

// A pseudo-locale for layout audits, development builds only: every English string
// comes back about a third longer and with accented letters, so a label that
// truncates in German or French shows up without waiting for a translation.
// Switched on with `localStorage.vemryPseudo = "1"` and a reload.
const PSEUDO_MAP: Record<string, string> = {
  a: "å", e: "é", i: "î", o: "ö", u: "ü", c: "ç", n: "ñ", y: "ÿ",
  A: "Å", E: "É", I: "Î", O: "Ö", U: "Ü", C: "Ç", N: "Ñ",
};
function pseudo(s: string): string {
  let out = "";
  let inPlaceholder = false;
  let letters = 0;
  for (const ch of s) {
    if (ch === "{") inPlaceholder = true;
    if (!inPlaceholder && PSEUDO_MAP[ch]) {
      out += PSEUDO_MAP[ch];
      letters++;
    } else out += ch;
    if (ch === "}") inPlaceholder = false;
  }
  return `[${out}${" ~".repeat(Math.ceil(letters / 6))}]`;
}
function pseudoOn(): boolean {
  if (!import.meta.env.DEV) return false;
  try {
    return localStorage.getItem("vemryPseudo") === "1";
  } catch {
    return false;
  }
}

class I18nStore {
  /** The stored preference. Every webview keeps its own copy, so a change in
   *  the main window has to be pushed to the others (see host.ts). */
  setting = $state<LangSetting>("auto");
  /** Resolved once at load: navigator.language does not change at runtime. */
  #system: Lang = detectSystemLang();
  readonly pseudo = pseudoOn();

  get lang(): Lang {
    return this.setting === "auto" ? this.#system : this.setting;
  }

  /** Locale tag for Intl (collators, date and number formatting). */
  get locale(): string {
    return this.lang;
  }

  set(setting: LangSetting) {
    this.setting = setting;
    this.applyLang();
  }

  /** The document's language attribute: fonts, line breaking and hyphenation
   *  follow it (`:root[lang]` in app.css). Written at init as well as on change —
   *  index.html carries only a fallback. */
  applyLang() {
    if (typeof document !== "undefined") document.documentElement.lang = this.lang;
  }
}

export const i18n = new I18nStore();

// One collator per language, built on demand: `localeCompare` with a locale
// argument re-parses the locale on every comparison, which a sort over a few
// thousand notes feels. Every user-visible ordering (file tree, lists, chips)
// compares through this so a Japanese, Korean or German vault sorts by its own
// rules instead of by a hardcoded one.
const COLLATORS = new Map<string, Intl.Collator>();

/** The comparator for names shown to the user, in the interface language:
 *  numeric ("note 2" before "note 10"), accent- and case-insensitive at the
 *  first level. */
export function collator(): Intl.Collator {
  const tag = i18n.locale;
  let c = COLLATORS.get(tag);
  if (!c) {
    c = new Intl.Collator(tag, { numeric: true, sensitivity: "base" });
    COLLATORS.set(tag, c);
  }
  return c;
}

/** `a.localeCompare(b)` in the interface language — the one comparator for sorts. */
export function compareNames(a: string, b: string): number {
  return collator().compare(a, b);
}

/** Look up a message. `{name}` placeholders are replaced from `params`.
 *  Reads `i18n.setting`, so every template call re-renders on a language change. */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
  const lang = i18n.lang;
  let s = CATALOGS[lang][key] ?? EN[key] ?? key;
  if (i18n.pseudo && lang === "en") s = pseudo(s);
  if (!params) return s;
  return s.replace(/\{(\w+)\}/g, (m, name: string) => {
    const v = params[name];
    return v === undefined ? m : String(v);
  });
}

/** A message as the English catalog has it, whatever the interface language — a second
 *  key the command palette matches on, so a command's English name finds it in any
 *  interface. No placeholders are filled. */
export function tEnglish(key: MessageKey): string {
  return EN[key] ?? key;
}

/** Whether a key built at runtime (`skill.<name>.title`) is in the catalog — the guard that
 *  lets such a key be passed to `t` without a cast that would hide a missing entry. */
export function hasKey(key: string): key is MessageKey {
  return key in EN;
}

const PLURAL_RULES = new Map<string, Intl.PluralRules>();

/** A count's message. Keys come in `<base>_<form>` families where the forms are
 *  the CLDR categories the language needs (English one/other, French one/many/
 *  other, Japanese other alone); callers pass the base key. A form the catalog
 *  does not carry falls back to `_other`, then `_one`, then to English. */
export function tn(base: string, n: number, params?: Record<string, string | number>): string {
  const lang = i18n.lang;
  let rules = PLURAL_RULES.get(lang);
  if (!rules) {
    rules = new Intl.PluralRules(lang);
    PLURAL_RULES.set(lang, rules);
  }
  const form = rules.select(n);
  const catalog = CATALOGS[lang];
  const candidates = [`${base}_${form}`, `${base}_other`, `${base}_one`] as MessageKey[];
  const key = candidates.find((k) => catalog[k] !== undefined) ?? (n === 1 ? candidates[2] : candidates[1]);
  return t(key, { n, ...params });
}

/** A language's own name (日本語, Deutsch): the same in every catalog. */
export function langName(lang: Lang): string {
  return t(`langName.${lang}` as MessageKey);
}

/** Read the stored preference once and keep this webview in sync with the
 *  others. Called from main.ts, so every window gets it. Not awaited: `t()` is
 *  reactive, so a late answer just relabels on the next frame. */
export async function initI18n(): Promise<void> {
  try {
    i18n.set((await api.settingsGet()).language);
  } catch {
    // No IPC (or no settings yet) — stay on the system language, but still stamp it on the document.
    i18n.applyLang();
  }
  void onLanguageChange((lang) => i18n.set(lang));
}

/** Apply a language everywhere. The native menu is relabelled by Rust inside
 *  `settings_set`; this covers the webviews. */
export function setLanguage(lang: LangSetting) {
  i18n.set(lang);
  emitLanguage(lang);
}
