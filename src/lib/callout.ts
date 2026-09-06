// The callout vocabulary shared by everything that renders one: the editor's live
// preview, the `> [!` completion list, and the read-only renderers (embeds, the
// reading projection, chat), so a `[!tip]` looks the same wherever it appears.

import { hasKey, t } from "$lib/i18n/i18n.svelte";

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
  tip: "tip", hint: "tip", important: "tip",
  success: "success", check: "success", done: "success",
  question: "question", help: "question", faq: "question",
  warning: "warn", caution: "warn", attention: "warn",
  danger: "danger", error: "danger", failure: "danger", fail: "danger", missing: "danger", bug: "danger",
  example: "example",
  quote: "quote", cite: "quote",
  claim: "claim", evidence: "evidence",
  // the block-level privacy zone: its own colour and the privacy badge for an icon
  private: "private",
};

/** A callout head as it reads after the `>` marker: `[!type]`, an optional fold marker,
 *  then the title. */
export const CALLOUT_HEAD_RE = /^\[!([\w-]+)\]([+-])?[ \t]*(.*)$/;
