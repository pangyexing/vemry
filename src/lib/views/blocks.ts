// The Blocks view's pure logic: any block of the vault as a row — kind, words, note,
// fields — grouped and sorted the way the user asks. The data is one query_blocks
// call (kind and field conditions go to Rust, the authority on the field grammar);
// everything here is presentation.
import type { BlockHit } from "$lib/api";
import { t } from "$lib/i18n/i18n.svelte";
import { stripFields } from "$lib/fields";
import { plainText } from "$lib/markdown";

export type KindChoice = "any" | "paragraph" | "heading" | "list_item" | "task" | "claim" | "evidence" | "code_block" | "private";
export const KIND_CHOICES: KindChoice[] = ["any", "paragraph", "heading", "list_item", "task", "claim", "evidence", "code_block", "private"];

export type GroupBy = "none" | "note" | "kind" | `field:${string}`;
export type SortBy = "note" | "kind" | "text" | `field:${string}`;

/** The rows a run shows at most (the query itself is capped at 5000). */
export const MAX_ROWS = 2000;
/** How many characters of a block's words a row shows. */
const TEXT_CHARS = 120;

export interface BlockRow {
  hit: BlockHit;
  kind: string;
  /** The block's words: prefix markers, markdown ornaments and inline fields stripped, one line, capped. */
  text: string;
  /** Lowercase key → first value. */
  fields: Record<string, string>;
  chips: [string, string][];
}

/** The glyph a kind gets in front of its row. */
export function kindGlyph(kind: string): string {
  switch (kind) {
    case "heading":
      return "#";
    case "list_item":
      return "•";
    case "task":
      return "☐";
    case "claim":
      return "!";
    case "evidence":
      return "≡";
    case "code_block":
      return "{}";
    case "private":
      return "✦";
    default:
      return "¶";
  }
}

const KIND_KEYS = ["paragraph", "heading", "list_item", "task", "claim", "evidence", "code_block", "private"] as const;

export function kindLabel(kind: string): string {
  return (KIND_KEYS as readonly string[]).includes(kind) ? t(`blocks.kind.${kind as (typeof KIND_KEYS)[number]}`) : kind;
}

const PREFIX_RE = /^[ \t]*(?:>[ \t]*)*(?:(?:[-*+]|\d{1,9}[.)])[ \t]+(?:\[[ xX/]\][ \t]*)?)?(?:#{1,6}[ \t]+)?(?:\[![\w-]+\][+-]?[ \t]*)?/;

export function rowOf(hit: BlockHit): BlockRow {
  const fields: Record<string, string> = {};
  for (const f of hit.block.fields ?? []) {
    const k = f.key.toLowerCase();
    if (!(k in fields)) fields[k] = f.value;
  }
  const kind = hit.block.kind.type;
  let text = stripFields(hit.block.content)
    .split("\n")
    .map((l) => l.replace(PREFIX_RE, ""))
    .filter((l) => l.trim() !== "")
    .join(" ");
  text = kind === "code_block" ? text.replace(/`{3,}[^\s]*/g, "").trim() : plainText(text);
  if (text.length > TEXT_CHARS) text = `${text.slice(0, TEXT_CHARS)}…`;
  return { hit, kind, text, fields, chips: Object.entries(fields).slice(0, 3) };
}

/** The key of a row for the DOM: the block's byte span start, physically unique within a note. */
export const rowKey = (r: BlockRow) => `${r.hit.note}:${r.hit.block.span.start}`;

/** What the filter box matches against. */
export const rowFilterKey = (r: BlockRow) =>
  `${r.text} ${r.hit.note_title} ${r.hit.note} ${Object.entries(r.fields)
    .map(([k, v]) => `${k} ${v}`)
    .join(" ")}`;

/** The field keys the rows carry, most used first (for the group and sort menus). */
export function fieldKeysOf(rows: BlockRow[]): string[] {
  const counts = new Map<string, number>();
  for (const r of rows) for (const k of Object.keys(r.fields)) counts.set(k, (counts.get(k) ?? 0) + 1);
  return [...counts.keys()].sort((a, b) => counts.get(b)! - counts.get(a)! || a.localeCompare(b));
}

function cmpValue(a: string | undefined, b: string | undefined): number {
  if (a === undefined && b === undefined) return 0;
  if (a === undefined) return 1; // a missing value sorts last
  if (b === undefined) return -1;
  const na = Number(a);
  const nb = Number(b);
  if (a !== "" && b !== "" && !Number.isNaN(na) && !Number.isNaN(nb)) return na - nb;
  return a.localeCompare(b);
}

export function sortRows(rows: BlockRow[], by: SortBy, desc: boolean): BlockRow[] {
  const byNote = (a: BlockRow, b: BlockRow) => a.hit.note.localeCompare(b.hit.note) || a.hit.block.span.start - b.hit.block.span.start;
  let cmp: (a: BlockRow, b: BlockRow) => number;
  if (by === "note") cmp = byNote;
  else if (by === "kind") cmp = (a, b) => a.kind.localeCompare(b.kind) || byNote(a, b);
  else if (by === "text") cmp = (a, b) => a.text.localeCompare(b.text) || byNote(a, b);
  else {
    const key = by.slice("field:".length).toLowerCase();
    cmp = (a, b) => cmpValue(a.fields[key], b.fields[key]) || byNote(a, b);
  }
  const sorted = [...rows].sort(cmp);
  return desc ? sorted.reverse() : sorted;
}

export interface RowGroup {
  key: string;
  title: string;
  rows: BlockRow[];
}

/** Groups in first-seen order of the (already sorted) rows; a missing field value
 *  goes under "none", last. */
export function groupRows(rows: BlockRow[], by: GroupBy): RowGroup[] {
  if (by === "none") return [{ key: "all", title: "", rows }];
  const groups = new Map<string, RowGroup>();
  const none: BlockRow[] = [];
  for (const r of rows) {
    let key: string | undefined;
    let title: string;
    if (by === "note") {
      key = r.hit.note;
      title = r.hit.note_title;
    } else if (by === "kind") {
      key = r.kind;
      title = kindLabel(r.kind);
    } else {
      key = r.fields[by.slice("field:".length).toLowerCase()];
      title = key ?? "";
    }
    if (key === undefined) {
      none.push(r);
      continue;
    }
    let g = groups.get(key);
    if (!g) {
      g = { key, title, rows: [] };
      groups.set(key, g);
    }
    g.rows.push(r);
  }
  const out = [...groups.values()];
  if (none.length) out.push({ key: "__none__", title: t("blocks.none"), rows: none });
  return out;
}
