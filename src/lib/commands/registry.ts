// The command registry — everything the app can do is a command. Each registers
// here with a title and description, making the app self-documenting; the
// palette, hotkeys, and (v0.3) user scripts all execute through this one
// entry point.

import type { ScriptInfo } from "$lib/api";
import { i18n, t, type MessageKey } from "$lib/i18n/i18n.svelte";

/** The command palette's groups and their order: with an empty query it is shown in
 *  sections by group (with a query it flattens by fuzzy score, and the group takes no
 *  part in the scoring). A group is a presentation property of the palette and does
 *  not affect execution.
 *  Ids are stable; the visible name comes from `group.<id>` in the catalog. */
export type CommandGroup = "nav" | "note" | "clip" | "view" | "research" | "vault" | "script" | "system";
export const GROUP_ORDER: CommandGroup[] = ["nav", "note", "clip", "view", "research", "vault", "script", "system"];

export function groupLabel(g: CommandGroup): string {
  return t(`group.${g}` as MessageKey);
}

export interface Command {
  id: string;
  /** Literal copy, for script commands only — their title comes from the user's
   *  JS and has no catalog entry. Built-ins leave both unset and take their copy
   *  from `cmd.<id>.title` / `cmd.<id>.desc`, so registration and catalog can
   *  never drift apart. */
  title?: string;
  description?: string;
  /** Interpolation for `cmd.<id>.desc` — hotkeys, mostly. */
  descriptionParams?: Record<string, string | number>;
  /** The hint shown beside the row; a function for one that follows a setting (read at render time). */
  hotkey?: string | (() => string);
  /** Defaults to "system". */
  group?: CommandGroup;
  run: () => void | Promise<void>;
}

/** Visible title. Reads `i18n.setting` through `t()`, so palette rows relabel
 *  themselves when the language changes — no re-registration. */
export function commandTitle(c: Command): string {
  return c.title ?? t(`cmd.${c.id}.title` as MessageKey);
}

export function commandHotkey(c: Command): string | undefined {
  return typeof c.hotkey === "function" ? c.hotkey() : c.hotkey;
}

export function commandDescription(c: Command): string | undefined {
  if (c.title !== undefined) return c.description;
  const key = `cmd.${c.id}.desc` as MessageKey;
  const s = t(key, c.descriptionParams);
  return s === key ? undefined : s;
}

const commands = new Map<string, Command>();

export function registerCommand(cmd: Command) {
  commands.set(cmd.id, cmd);
}

/** Script commands live under the `script:` namespace; a (re)load replaces
 *  the whole namespace so removed/renamed commands never linger. */
export function syncScriptCommands(scripts: ScriptInfo[], run: (script: string, command: string) => void) {
  for (const id of [...commands.keys()]) {
    if (id.startsWith("script:")) commands.delete(id);
  }
  for (const s of scripts) {
    if (!s.enabled) continue;
    for (const c of s.commands) {
      registerCommand({
        id: `script:${s.name}:${c.id}`,
        title: c.title,
        description: c.description ?? t("script.fromScript", { name: s.name }),
        hotkey: c.hotkey ?? undefined,
        group: "script",
        run: () => run(s.name, c.id),
      });
    }
  }
}

/** A command's group (defaulting to "system") — the sorting and the palette's group-header check share this one definition. */
export function groupOf(c: Command): CommandGroup {
  return c.group ?? "system";
}

export function allCommands(): Command[] {
  // Group order first, alphabetical within a group: a purely alphabetical global sort
  // would break the "View:" section apart and throw titles beginning with A to both
  // ends of the list (the "it feels chaotic" the user reported), while the group order
  // is stable spatial memory
  const rank = (c: Command) => GROUP_ORDER.indexOf(groupOf(c));
  // Collator follows the UI language: sorting English titles by Chinese rules
  // (and vice versa) scrambles the within-group order users navigate by.
  return [...commands.values()].sort(
    (a, b) => rank(a) - rank(b) || commandTitle(a).localeCompare(commandTitle(b), i18n.locale),
  );
}

export async function executeCommand(id: string) {
  await commands.get(id)?.run();
}

/** One string as matching reads it: NFKC so a half-width kana, a full-width digit and a
 *  composed accent meet their canonical form, then lower case. Both the query and the key
 *  go through it, so the two are always compared in the same form. */
function fold(s: string): string {
  return s.normalize("NFKC").toLowerCase();
}

/** Match score for one already-folded key against an already-folded, non-empty
 *  query: every query character must appear in order, substring matches rank first.
 *  null = no match. The one definition of a match, shared by both entry points.
 *  The query is walked by code point, not by UTF-16 unit, so an astral character
 *  (an extension-B ideograph, an emoji) is one character to match rather than a
 *  surrogate pair that can only ever half-match. */
function fuzzyScore(key: string, chars: string[]): number | null {
  const q = chars.join("");
  if (key.includes(q)) return 1000 - key.indexOf(q) - key.length * 0.01;
  let i = 0;
  for (const ch of key) {
    if (ch === chars[i]) i++;
    if (i === chars.length) break;
  }
  return i === chars.length ? 100 - key.length * 0.01 : null;
}

/** Filtering reordered by match quality — for the command palette alone: "the closest match comes first" is its sorting contract. */
export function fuzzyFilter<T>(items: T[], query: string, keyOf: (item: T) => string): T[] {
  const chars = Array.from(fold(query.trim()));
  if (!chars.length) return items;
  const scored: { item: T; score: number }[] = [];
  for (const item of items) {
    const score = fuzzyScore(fold(keyOf(item)), chars);
    if (score !== null) scored.push({ item, score });
  }
  scored.sort((a, b) => b.score - a.score);
  return scored.map((s) => s.item);
}

/** Order-preserving filtering: the same match verdict, with the hits returned in the
 *  order they came in. The list views use this one — they each have a sorting contract
 *  of their own (by date, by time descending, grouped by note), and reordering by
 *  match score would wash it out: type one character and the order of the note groups
 *  would jump. */
export function fuzzyKeep<T>(items: T[], query: string, keyOf: (item: T) => string): T[] {
  const chars = Array.from(fold(query.trim()));
  if (!chars.length) return items;
  return items.filter((item) => fuzzyScore(fold(keyOf(item)), chars) !== null);
}
