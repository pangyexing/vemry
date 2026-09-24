// The command registry — everything the app can do is a command. Each registers
// here with a title and description, making the app self-documenting; the
// palette, hotkeys, and (v0.3) user scripts all execute through this one
// entry point.

import type { ScriptInfo } from "$lib/api";
import { i18n, t, tEnglish, type MessageKey } from "$lib/i18n/i18n.svelte";

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
  /** Whether running it now can do anything; absent = always. It reads state the stores
   *  already hold and never calls IPC, since the palette asks every row on every open.
   *  The palette leaves an unavailable command out of its empty list and, when a query
   *  matches one, lists it last with `whyNot` and does not run it. executeCommand does
   *  not consult it: a menu item, a key or a script still reaches the command, whose own
   *  feedback (a toast, a no-op) stays as it was. */
  when?: () => boolean;
  /** Why it is unavailable, shown beside the row: a catalog key, or a function choosing
   *  one when the reason depends on the state (no file open, or a file already private). */
  whyNot?: MessageKey | (() => MessageKey);
  run: () => void | Promise<void>;
}

/** Visible title. Reads `i18n.setting` through `t()`, so palette rows relabel
 *  themselves when the language changes — no re-registration. */
export function commandTitle(c: Command): string {
  return c.title ?? t(`cmd.${c.id}.title` as MessageKey);
}

/** The title in English, whatever the interface language (a script command's own title
 *  as written): the palette's second match key. */
export function commandEnglishTitle(c: Command): string {
  return c.title ?? tEnglish(`cmd.${c.id}.title` as MessageKey);
}

/** A command's `when`, defaulting to available. A predicate that throws (a store not
 *  set up yet) counts as available: hiding a command is worse than a no-op run. */
export function commandAvailable(c: Command): boolean {
  try {
    return c.when?.() ?? true;
  } catch {
    return true;
  }
}

/** The reason an unavailable command gives, in the interface language. */
export function commandWhyNot(c: Command): string | undefined {
  const key = typeof c.whyNot === "function" ? c.whyNot() : c.whyNot;
  return key ? t(key) : undefined;
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
export function fold(s: string): string {
  return s.normalize("NFKC").toLowerCase();
}

/** A word starts after nothing, a space or a punctuation mark or symbol (`-`, `/`, `·`,
 *  a full-width comma). Han and kana carry no such boundary; there every character is
 *  equal and the bonuses below simply never apply. */
const BOUNDARY = /[\s\p{P}\p{S}]/u;
function startsWord(prev: string | undefined): boolean {
  return prev === undefined || BOUNDARY.test(prev);
}

/** Match score for one already-folded key against an already-folded, non-empty
 *  query: every query character must appear in order. null = no match. The one
 *  definition of a match, shared by every entry point.
 *  A substring ranks above any scattered match, earlier above later, and an
 *  occurrence that starts a word beats an earlier one inside a word ("note" in
 *  "Denote notes" is the second). A scattered match earns a little for each
 *  character landing on a word start and for each pair that stays adjacent. The
 *  verdict — match or not — is the plain in-order test, so fuzzyKeep's filtering
 *  never depends on the bonuses.
 *  The query is walked by code point, not by UTF-16 unit, so an astral character
 *  (an extension-B ideograph, an emoji) is one character to match rather than a
 *  surrogate pair that can only ever half-match. */
function fuzzyScore(key: string, chars: string[]): number | null {
  const q = chars.join("");
  const at = key.indexOf(q);
  if (at !== -1) {
    for (let s = at; s !== -1; s = key.indexOf(q, s + 1)) {
      if (startsWord(key[s - 1])) return 1030 - s - key.length * 0.01;
    }
    return 1000 - at - key.length * 0.01;
  }
  let i = 0;
  let bonus = 0;
  let prev: string | undefined;
  let last = -2;
  let pos = 0;
  for (const ch of key) {
    if (ch === chars[i]) {
      if (startsWord(prev)) bonus += 4;
      if (last === pos - 1) bonus += 2;
      last = pos;
      i++;
      if (i === chars.length) break;
    }
    prev = ch;
    pos++;
  }
  return i === chars.length ? 100 + Math.min(bonus, 800) - key.length * 0.01 : null;
}

/** The score of `text` against `query`, both folded here; null = no match (an empty
 *  query matches nothing). For a caller ranking by its own rules — the palette's
 *  title-before-description tiers. */
export function fuzzyScoreText(text: string, query: string): number | null {
  const chars = Array.from(fold(query.trim()));
  return chars.length ? fuzzyScore(fold(text), chars) : null;
}

/** Each code point of `text` folded on its own, with the index of the code point it
 *  came from — the map from a match in folded text back to what is on screen. Folding
 *  per character can differ from folding the whole string (a combining accent composes
 *  only in context), which costs at most a missed highlight, never a wrong row. */
function foldedChars(text: string): { c: string; at: number }[] {
  const out: { c: string; at: number }[] = [];
  Array.from(text).forEach((ch, at) => {
    for (const c of fold(ch)) out.push({ c, at });
  });
  return out;
}

/** Which characters of `text` (code-point indices) a query matched, for drawing them
 *  in bold: the same preference as the score — a substring starting a word, else the
 *  first substring, else the in-order characters. null = no match. */
export function fuzzyPositions(text: string, query: string): number[] | null {
  const chars = Array.from(fold(query.trim()));
  if (!chars.length) return null;
  const f = foldedChars(text);
  const hits = (start: number) => chars.every((c, j) => f[start + j]?.c === c);
  let first = -1;
  for (let s = 0; s + chars.length <= f.length; s++) {
    if (!hits(s)) continue;
    if (first === -1) first = s;
    if (s === 0 || BOUNDARY.test(f[s - 1].c)) {
      first = s;
      break;
    }
  }
  const spans: number[] = [];
  if (first !== -1) {
    for (let j = 0; j < chars.length; j++) spans.push(f[first + j].at);
  } else {
    let i = 0;
    for (const x of f) {
      if (x.c === chars[i]) {
        spans.push(x.at);
        if (++i === chars.length) break;
      }
    }
    if (i < chars.length) return null;
  }
  return [...new Set(spans)];
}

/** Every occurrence of each whitespace-separated term of `query` in `text`, as
 *  code-point indices — a full-text hit's words, drawn in bold in its title and
 *  snippet (where the words need not be in order, unlike a fuzzy match). */
export function termPositions(text: string, query: string): number[] {
  const f = foldedChars(text);
  const marks = new Set<number>();
  for (const term of fold(query).split(/\s+/).filter(Boolean)) {
    const tc = Array.from(term);
    for (let s = 0; s + tc.length <= f.length; s++) {
      if (tc.every((c, j) => f[s + j].c === c)) for (let j = 0; j < tc.length; j++) marks.add(f[s + j].at);
    }
  }
  return [...marks].sort((a, b) => a - b);
}

/** `text` cut into runs by whether each code point is in `marks` — what a template
 *  walks to draw the matched characters in bold. */
export function markRuns(text: string, marks: readonly number[] | null | undefined): { text: string; hit: boolean }[] {
  const chars = Array.from(text);
  if (!marks?.length) return [{ text, hit: false }];
  const set = new Set(marks);
  const runs: { text: string; hit: boolean }[] = [];
  chars.forEach((ch, i) => {
    const hit = set.has(i);
    const last = runs[runs.length - 1];
    if (last && last.hit === hit) last.text += ch;
    else runs.push({ text: ch, hit });
  });
  return runs;
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
