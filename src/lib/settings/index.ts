// The settings index: every setting the dialog shows, by a stable id, with the page it
// lives on and the catalog keys of its label and hint. It is the one definition three
// readers share — the row itself (SettingRow takes an id and reads its words from here),
// the dialog's search, and the command palette's "Settings" rows — so a setting cannot be
// findable under one name and shown under another. The controls stay in the page
// components; this holds no values and no behaviour.

import { fold } from "$lib/commands/registry";
import { t, tEnglish, type MessageKey } from "$lib/i18n/i18n.svelte";
import { isMac, k } from "$lib/shortcut";

export type SettingsPage = "general" | "editing" | "ai" | "account" | "browser" | "capture" | "privacy" | "skills";

/** The pages in navigation order; the last two act on the open vault and are disabled without one. */
export const PAGES: { id: SettingsPage; needsVault?: boolean }[] = [
  { id: "general" },
  { id: "editing" },
  { id: "ai" },
  { id: "account" },
  { id: "browser" },
  { id: "capture" },
  { id: "privacy", needsVault: true },
  { id: "skills", needsVault: true },
];

export const pageLabel = (p: SettingsPage): string => t(`settings.tab.${p}` as MessageKey);

const NEEDS_VAULT = new Set(PAGES.filter((p) => p.needsVault).map((p) => p.id));
/** Whether a page can be shown: the vault's own pages need an open vault. */
export const pageAvailable = (p: SettingsPage, hasVault: boolean): boolean => hasVault || !NEEDS_VAULT.has(p);

/** What decides whether a row is on its page at all — the few facts a page shows or hides rows by. */
export interface SettingsState {
  hasVault: boolean;
  signedIn: boolean;
  provider: "hosted" | "custom";
  captureTarget: string;
}
/** Whether a search may offer the setting: its page can be shown and the row is on it now. */
export const settingAvailable = (s: SettingEntry, st: SettingsState): boolean =>
  pageAvailable(s.page, st.hasVault) && (s.shownWhen?.(st) ?? true);

export interface SettingEntry {
  id: string;
  page: SettingsPage;
  /** A function where the wording follows the platform (the menu bar vs the system tray). */
  label: MessageKey | (() => MessageKey);
  hint?: MessageKey | (() => MessageKey);
  /** The hint's placeholders (a shortcut drawn for this platform). */
  hintParams?: () => Record<string, string>;
  /** Extra words a search should find it by, in any language (space-separated). */
  keywords?: string;
  /** The row is on its page only in this state (the endpoint only with your own key):
   *  a search offers it only then, so it never points at a row that is not there. */
  shownWhen?: (st: SettingsState) => boolean;
  /** A list or a card rather than one control: in search results it is a row that jumps to
   *  the page instead of being rendered in place (its content loads with the page). */
  jump?: boolean;
}

const residentLabel = (): MessageKey => (isMac ? "settings.resident" : "settings.residentTray");
const residentHint = (): MessageKey => (isMac ? "settings.residentHint" : "settings.residentHintTray");

export const SETTINGS: SettingEntry[] = [
  { id: "general.language", page: "general", label: "settings.language.label", hint: "settings.language.hint", keywords: "语言 language 中文 English" },
  { id: "general.resident", page: "general", label: residentLabel, hint: residentHint, hintParams: () => ({ quit: k("Mod+Q") }), keywords: "tray menu bar 托盘 菜单栏 后台" },
  { id: "general.version", page: "general", label: "settings.version", keywords: "update 更新 升级 version" },
  { id: "general.autoCheck", page: "general", label: "settings.autoCheck", hint: "settings.autoCheckHint", keywords: "update 更新" },

  { id: "editing.fontSize", page: "editing", label: "settings.fontSize", hint: "settings.fontSizeHint", keywords: "字号 字体 font size 大小" },
  { id: "editing.lineWidth", page: "editing", label: "settings.readableLineWidth", hint: "settings.readableLineWidthHint", keywords: "行宽 宽度 width" },
  { id: "editing.clickOpensLinks", page: "editing", label: "settings.clickOpensLinks", hint: "settings.clickOpensLinksHint", hintParams: () => ({ modClick: k("Mod+click") }), keywords: "链接 link click" },
  { id: "editing.openLinksIn", page: "editing", label: "settings.openLinksIn", hint: "settings.openLinksHint", hintParams: () => ({ key: k("Mod+click") }), keywords: "链接 link reader browser 浏览器" },
  { id: "editing.readingImages", page: "editing", label: "settings.readingImages", hint: "settings.readingImagesHint", keywords: "图片 image reader" },
  { id: "editing.formatClips", page: "editing", label: "settings.formatClips", hint: "settings.formatClipsHint", keywords: "格式化 format paste 粘贴" },
  { id: "editing.cjkSpacing", page: "editing", label: "settings.formatCjkSpacing", hint: "settings.formatCjkSpacingHint", keywords: "空格 space 格式化 format" },
  { id: "editing.punctuation", page: "editing", label: "settings.formatPunctuation", hint: "settings.formatPunctuationHint", keywords: "标点 全角 punctuation 格式化 format" },

  { id: "ai.provider", page: "ai", label: "settings.provider", hint: "settings.providerHint", keywords: "托管 hosted key provider 提供方" },
  { id: "ai.model", page: "ai", label: "settings.model", keywords: "模型 model glm deepseek" },
  { id: "ai.endpoint", page: "ai", shownWhen: (st) => st.provider === "custom", label: "settings.endpoint", hint: "settings.endpointHint", keywords: "端点 endpoint url openai ollama" },
  { id: "ai.apiKey", page: "ai", shownWhen: (st) => st.provider === "custom", label: "settings.apiKey", hint: "settings.apiKeyNote", keywords: "key 密钥 api" },
  { id: "ai.effort", page: "ai", label: "settings.reasoningEffort", hint: "settings.reasoningHint", keywords: "思考 thinking reasoning effort 深度" },
  { id: "ai.webSearch", page: "ai", label: "settings.webSearch", hint: "settings.webSearchHint", keywords: "搜索 联网 search web" },
  { id: "ai.planConfirm", page: "ai", label: "settings.planConfirm", hint: "settings.planConfirmHint", keywords: "计划 plan" },
  { id: "ai.provenanceMarks", page: "ai", label: "settings.provenanceMarks", hint: "settings.provenanceMarksHint", keywords: "审阅 review 出处 provenance" },
  { id: "ai.formatAgentWrites", page: "ai", label: "settings.formatAgentWrites", hint: "settings.formatAgentWritesHint", keywords: "格式化 format" },
  { id: "ai.writeGrant", page: "ai", shownWhen: () => false, label: "settings.writeGrant", hint: "settings.writeGrantHint", keywords: "免审 授权 permission allow" },
  { id: "ai.translateTarget", page: "ai", label: "settings.translate.target", keywords: "翻译 translate 语言 language" },
  { id: "ai.translateModel", page: "ai", label: "settings.translate.model", hint: "settings.translate.modelHint", keywords: "翻译 translate 模型 model" },
  { id: "ai.translateAddendum", page: "ai", label: "settings.translate.addendum", hint: "settings.translate.addendumHint", keywords: "翻译 translate" },

  { id: "account.panel", page: "account", label: "settings.tab.account", hint: "settings.accountHint", keywords: "登录 邮箱 配额 算力包 兑换码 订阅 sign in email quota credit redeem subscription", jump: true },

  { id: "browser.adblock", page: "browser", label: "settings.adblock", hint: "settings.adblockHint", keywords: "广告 ad block 拦截" },
  { id: "browser.ruleUrls", page: "browser", label: "settings.ruleUrls", hint: "settings.ruleUrlsHint", keywords: "广告 ad block 规则 rules" },
  { id: "browser.clipImages", page: "browser", label: "settings.clipLocalizeImages", hint: "settings.clipLocalizeImagesHint", keywords: "图片 image 剪藏 clip" },

  { id: "capture.hotkey", page: "capture", label: "settings.capture.hotkey", hint: "settings.capture.hotkeyHint", keywords: "速记 capture 快捷键 shortcut hotkey" },
  { id: "capture.target", page: "capture", label: "settings.capture.target", hint: "settings.capture.targetHint", hintParams: () => ({ key: k("Mod+Enter") }), keywords: "速记 capture inbox 日志 journal" },
  { id: "capture.pinnedNote", page: "capture", shownWhen: (st) => st.captureTarget === "pinned" && st.hasVault, label: "settings.capture.pinnedNote", hint: "settings.capture.pinnedNoteHint", keywords: "速记 capture 钉 pin" },
  { id: "capture.pinnedHeading", page: "capture", shownWhen: (st) => st.captureTarget === "pinned" && st.hasVault, label: "settings.capture.pinnedHeading", hint: "settings.capture.pinnedHeadingHint", keywords: "速记 capture 小节 section" },
  { id: "capture.phone", page: "capture", label: "settings.capture.phone", hint: "settings.capture.phoneHint", keywords: "手机 phone 配对 pair 二维码 qr", jump: true },
  { id: "capture.phoneVault", page: "capture", shownWhen: (st) => st.signedIn, label: "settings.capture.phoneVault", hint: "settings.capture.phoneVaultHint", keywords: "手机 phone 知识库 vault" },

  { id: "privacy.rules", page: "privacy", label: "settings.privacyRules", hint: "settings.privacyRulesHint", keywords: "隐私 privacy private 规则 rules", jump: true },
  { id: "privacy.marked", page: "privacy", label: "settings.privacyMarkedNotes", hint: "settings.privacyMarkedHint", keywords: "隐私 privacy private", jump: true },

  { id: "skills.skills", page: "skills", label: "settings.skills", hint: "settings.skillsHint", keywords: "技能 skill", jump: true },
  { id: "skills.typeExamples", page: "skills", label: "settings.typeExamples", hint: "settings.typeExamplesHint", keywords: "块类型 block type 示例 example", jump: true },
  { id: "skills.scripts", page: "skills", label: "settings.userScripts", hint: "settings.userScriptsHint", keywords: "脚本 script js", jump: true },
];

const BY_ID = new Map(SETTINGS.map((s) => [s.id, s]));

export function settingById(id: string): SettingEntry | undefined {
  return BY_ID.get(id);
}

const key = (k: MessageKey | (() => MessageKey)): MessageKey => (typeof k === "function" ? k() : k);
export const settingLabel = (s: SettingEntry): string => t(key(s.label));
export const settingHint = (s: SettingEntry): string | undefined => (s.hint ? t(key(s.hint), s.hintParams?.()) : undefined);

/** Everything a setting can be found by: its words in the interface language and in
 *  English (typing English finds a setting in any language), the page's name, and the
 *  keywords. The palette's command matching folds the same way. */
function haystack(s: SettingEntry): string {
  return fold(
    [settingLabel(s), settingHint(s) ?? "", tEnglish(key(s.label)), s.hint ? tEnglish(key(s.hint)) : "", pageLabel(s.page), s.keywords ?? ""].join(" "),
  );
}

/** The settings a query finds — the one matcher behind the dialog's search and the
 *  command palette's "Settings" rows: every whitespace-separated term appears somewhere
 *  in the haystack (substring, not subsequence — a hint is a sentence, and scattered
 *  letters across it would match nearly anything). A label hit ranks before a hint-only one. */
export function searchSettings(query: string): SettingEntry[] {
  const terms = fold(query).split(/\s+/).filter(Boolean);
  if (!terms.length) return [];
  const hits: { s: SettingEntry; rank: number }[] = [];
  SETTINGS.forEach((s, i) => {
    const hay = haystack(s);
    if (!terms.every((term) => hay.includes(term))) return;
    const label = fold(`${settingLabel(s)} ${tEnglish(key(s.label))}`);
    hits.push({ s, rank: (terms.every((term) => label.includes(term)) ? 0 : 1000) + i });
  });
  return hits.sort((a, b) => a.rank - b.rank).map((h) => h.s);
}
