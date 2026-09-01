// User-facing copy lives in `locales/{en,zh-CN}.json` — one pair of files read by
// both this module and `src-tauri/src/i18n.rs`, the same mirror discipline as
// api.ts ↔ commands.rs. Nothing user-visible may be hardcoded in a component.
//
// Wording for any new term goes through docs/GLOSSARY.md first: the two READMEs,
// the two catalogs and the UI have to call the same thing the same name.
import { api, emitLanguage, onLanguageChange } from "$lib/api";

import en from "../../../locales/en.json";
import zhCN from "../../../locales/zh-CN.json";

export type Lang = "en" | "zh-CN";
/** What the user picked; "auto" resolves against the browser locale. */
export type LangSetting = "auto" | Lang;

export type MessageKey = keyof typeof en;

// A key missing from zh-CN is a `pnpm check` error…
const EN: Record<MessageKey, string> = en;
const ZH: Record<MessageKey, string> = zhCN;

// …and so is a stale key left behind in zh-CN but gone from en.
type Assert<T extends never> = T;
export type _NoStaleZhKeys = Assert<Exclude<keyof typeof zhCN, MessageKey>>;

const CATALOGS: Record<Lang, Record<MessageKey, string>> = { en: EN, "zh-CN": ZH };

function detectSystemLang(): Lang {
  const nav = typeof navigator === "undefined" ? "" : navigator.language;
  return nav.toLowerCase().startsWith("zh") ? "zh-CN" : "en";
}

class I18nStore {
  /** The stored preference. Every webview keeps its own copy, so a change in
   *  the main window has to be pushed to the others (see host.ts). */
  setting = $state<LangSetting>("auto");
  /** Resolved once at load: navigator.language does not change at runtime. */
  #system: Lang = detectSystemLang();

  get lang(): Lang {
    return this.setting === "auto" ? this.#system : this.setting;
  }

  /** Locale tag for Intl (collators, date formatting). */
  get locale(): string {
    return this.lang;
  }

  set(setting: LangSetting) {
    this.setting = setting;
    if (typeof document !== "undefined") document.documentElement.lang = this.lang;
  }
}

export const i18n = new I18nStore();

/** Look up a message. `{name}` placeholders are replaced from `params`.
 *  Reads `i18n.setting`, so every template call re-renders on a language change. */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
  const s = CATALOGS[i18n.lang][key] ?? EN[key] ?? key;
  if (!params) return s;
  return s.replace(/\{(\w+)\}/g, (m, name: string) => {
    const v = params[name];
    return v === undefined ? m : String(v);
  });
}

/** English needs a plural form where Chinese does not; keys come in
 *  `<base>_one` / `<base>_other` pairs. Callers pass the base key. */
export function tn(base: string, n: number, params?: Record<string, string | number>): string {
  const key = (n === 1 ? `${base}_one` : `${base}_other`) as MessageKey;
  return t(key, { n, ...params });
}

/** Read the stored preference once and keep this webview in sync with the
 *  others. Called from main.ts, so every window gets it. Not awaited: `t()` is
 *  reactive, so a late answer just relabels on the next frame. */
export async function initI18n(): Promise<void> {
  try {
    i18n.set((await api.settingsGet()).language);
  } catch {
    // No IPC (or no settings yet) — stay on the system language.
  }
  void onLanguageChange((lang) => i18n.set(lang));
}

/** Apply a language everywhere. The native menu is relabelled by Rust inside
 *  `settings_set`; this covers the webviews. */
export function setLanguage(lang: LangSetting) {
  i18n.set(lang);
  emitLanguage(lang);
}
