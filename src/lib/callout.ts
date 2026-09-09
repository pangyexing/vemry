// The callout vocabulary shared by everything that renders one: the editor's live
// preview, the `> [!` completion list, and the read-only renderers (embeds, the
// reading projection, chat), so a `[!tip]` looks the same wherever it appears.

import { hasKey, t } from "$lib/i18n/i18n.svelte";
import { privacyGlyphSvg } from "$lib/privacyGlyph";

/** The head's title when the author wrote none: the type's name in the UI language (a
 *  `[!warning]` reads 警告 under a Chinese UI); a type the catalog does not know shows
 *  its own name. The file keeps only the type — nothing is written back. */
export function calloutDefaultTitle(type: string): string {
  const key = `callout.${type.toLowerCase()}`;
  return hasKey(key) ? t(key) : type;
}

/** The tint group of each callout type. */
export const CALLOUT_GROUP: Record<string, string> = {
  note: "info", info: "info", todo: "info", abstract: "info", summary: "info", tldr: "info",
  // a flashcard (the Review view reads it; the parser sees a paragraph)
  card: "info",
  tip: "tip", hint: "tip", important: "tip",
  success: "success", check: "success", done: "success",
  question: "question", help: "question", faq: "question",
  warning: "warn", caution: "warn", attention: "warn",
  danger: "danger", error: "danger", failure: "danger", fail: "danger", missing: "danger", bug: "danger",
  example: "example",
  quote: "quote", cite: "quote",
  // a decision is a claim about what to do: the parser indexes it as one
  claim: "claim", decision: "claim", evidence: "evidence",
  // the block-level privacy zone: its own colour and the privacy badge for an icon
  private: "private",
};

/** A callout head as it reads after the `>` marker: `[!type]`, an optional fold marker,
 *  then the title. */
export const CALLOUT_HEAD_RE = /^\[!([\w-]+)\]([+-])?[ \t]*(.*)$/;

/** The icon of each tint group: lucide-style stroke paths in a 24 viewBox. `private`
 *  is absent on purpose — it draws the privacy badge (privacyGlyph.ts). */
export const CALLOUT_ICONS: Record<string, string> = {
  info: '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4M12 8h.01"/>',
  tip: '<path d="M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z"/>',
  success: '<path d="M20 6 9 17l-5-5"/>',
  question: '<circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3M12 17h.01"/>',
  warn: '<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 20h16a2 2 0 0 0 1.73-2ZM12 9v4M12 17h.01"/>',
  danger: '<path d="M13 2 3 14h9l-1 8 10-12h-9l1-8z"/>',
  example: '<path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"/>',
  // the bar on the left is what makes three lines read as a quotation
  quote: '<path d="M17 6H3M21 12H8M21 18H8M3 12v6"/>',
  claim: '<path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1zM4 22v-7"/>',
  evidence:
    '<path d="m16 16 3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1ZM2 16l3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1ZM7 21h10M12 3v18M3 7h2c2 0 5-1 7-2 2 1 5 2 7 2h2"/>',
};

const ICON_CIRCLE_X = '<circle cx="12" cy="12" r="10"/><path d="m15 9-6 6M9 9l6 6"/>';
const ICON_CLIPBOARD_LIST =
  '<rect width="8" height="4" x="8" y="2" rx="1" ry="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2M12 11h4M12 16h4M8 11h.01M8 16h.01"/>';

/** A finer icon for a type whose group icon says too little: the tint stays the
 *  group's, only the glyph changes (a `[!bug]` is red and draws a bug). Types absent
 *  here draw their group's icon. */
export const CALLOUT_TYPE_ICONS: Record<string, string> = {
  todo: '<path d="M21 10.5V19a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h12.5M9 11l3 3L22 4"/>',
  abstract: ICON_CLIPBOARD_LIST,
  summary: ICON_CLIPBOARD_LIST,
  tldr: ICON_CLIPBOARD_LIST,
  card: '<path d="M12.83 2.18a2 2 0 0 0-1.66 0L2.6 6.08a1 1 0 0 0 0 1.83l8.58 3.91a2 2 0 0 0 1.66 0l8.58-3.9a1 1 0 0 0 0-1.83zM2 12a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 12M2 17a1 1 0 0 0 .58.91l8.6 3.91a2 2 0 0 0 1.65 0l8.58-3.9A1 1 0 0 0 22 17"/>',
  // an error or a failure is a negative result, not a hazard: a crossed circle, not a bolt
  error: ICON_CIRCLE_X,
  failure: ICON_CIRCLE_X,
  fail: ICON_CIRCLE_X,
  // something that should be there and is not: a dashed circle
  missing:
    '<path d="M10.1 2.182a10 10 0 0 1 3.8 0M13.9 21.818a10 10 0 0 1-3.8 0M17.609 3.721a10 10 0 0 1 2.69 2.7M2.182 13.9a10 10 0 0 1 0-3.8M20.279 17.609a10 10 0 0 1-2.7 2.69M21.818 10.1a10 10 0 0 1 0 3.8M3.721 6.391a10 10 0 0 1 2.7-2.69M6.391 20.279a10 10 0 0 1-2.69-2.7"/>',
  bug: '<path d="m8 2 1.88 1.88M14.12 3.88 16 2M9 7.13v-1a3.003 3.003 0 1 1 6 0v1M12 20c-3.3 0-6-2.7-6-6v-3a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v3c0 3.3-2.7 6-6 6M12 20v-9M6.53 9C4.6 8.8 3 7.1 3 5M6 13H2M3 21c0-2.1 1.7-3.9 3.8-4M20.97 5c0 2.1-1.6 3.8-3.5 4M22 13h-4M17.2 17c2.1.1 3.8 1.9 3.8 4"/>',
  // a gavel: the claim has been ruled on
  decision: '<path d="m14.5 12.5-8 8a2.119 2.119 0 1 1-3-3l8-8M16 16l6-6M8 8l6-6M9 7l8 8M21 11l-8-8"/>',
};

/** The head icon of a callout as inline SVG, 15px, in `currentColor`: the type's own
 *  icon if it has one, else its group's, else info's; the private block draws the
 *  privacy badge. One drawing for the editor's widget and the read-only renderers. */
export function calloutIconSvg(type: string, group: string): string {
  if (group === "private") return privacyGlyphSvg(15);
  // hasOwn, not a bare lookup: the type is whatever the author typed, and a `[!constructor]`
  // would otherwise resolve up the prototype chain and stringify a function into the <svg>
  const own = <T,>(table: Record<string, T>, key: string) => (Object.hasOwn(table, key) ? table[key] : undefined);
  const paths = own(CALLOUT_TYPE_ICONS, type.toLowerCase()) ?? own(CALLOUT_ICONS, group) ?? CALLOUT_ICONS.info;
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${paths}</svg>`;
}
