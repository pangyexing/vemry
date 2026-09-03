// The presentation half of a query block: how its hits are sorted and grouped. The
// query itself (kind, note, tag, field conditions…) goes to Rust; `group` and `sort`
// are display keys and stay in the frontend, so the agent's query_blocks tool never
// sees them and BlockQuery is unchanged. Pure: no CodeMirror, no i18n (labels are
// passed in), so the batch's harness can replay it.
import type { BlockHit } from "$lib/api";

export type GroupKey = { by: "note" } | { by: "kind" } | { by: "field"; key: string };
export type SortKey = { by: "note" } | { by: "kind" } | { by: "text" } | { by: "field"; key: string };
export interface SortSpec {
  key: SortKey;
  desc: boolean;
}

/** A field key as the parser accepts it: letters, digits, `_`, `-`; not all digits. */
const FIELD_KEY_RE = /^(?![\d]+$)[\p{L}\p{N}_-]+$/u;

/** `note` / `kind` / a field key → GroupKey; null when the value is not one of those. */
export function parseGroup(value: string): GroupKey | null {
  const v = value.trim();
  if (v === "note" || v === "kind") return { by: v };
  return FIELD_KEY_RE.test(v) ? { by: "field", key: v } : null;
}

/** `<note|kind|text|field key> [asc|desc]` → SortSpec; null when malformed. */
export function parseSort(value: string): SortSpec | null {
  const parts = value.trim().split(/\s+/);
  if (parts.length < 1 || parts.length > 2) return null;
  const [k, dir] = parts;
  let desc = false;
  if (dir !== undefined) {
    if (dir === "desc") desc = true;
    else if (dir !== "asc") return null;
  }
  if (k === "note" || k === "kind" || k === "text") return { key: { by: k }, desc };
  return FIELD_KEY_RE.test(k) ? { key: { by: "field", key: k }, desc } : null;
}

/** A block's first value of a field, the key already lowercased — for the loops that
 *  compare one key against every row, where lowercasing it per row is the whole cost. */
export function fieldOfLower(hit: BlockHit, lower: string): string | undefined {
  return hit.block.fields?.find((f) => f.key.toLowerCase() === lower)?.value;
}

/** A block's first value of a field (keys compare case-insensitively). */
export function fieldOf(hit: BlockHit, key: string): string | undefined {
  return fieldOfLower(hit, key.toLowerCase());
}

/** Dates and numbers in their own order, everything else alphabetical. The board's
 *  column order and card order read the same rule. */
export function cmpValue(a: string, b: string): number {
  const na = Number(a);
  const nb = Number(b);
  if (a !== "" && b !== "" && !Number.isNaN(na) && !Number.isNaN(nb)) return na - nb;
  return a.localeCompare(b);
}

/** The source order: the note's path, then the block's byte position in it. The tie-break
 *  of every block ordering, so a requery never shuffles two rows that compare equal
 *  (tasks.ts's bySource is this one read off a TaskRow's hit). */
export const byNote = (a: BlockHit, b: BlockHit) => a.note.localeCompare(b.note) || a.block.span.start - b.block.span.start;

/** The one `sort` convention, read by a query block's hits and by the board's cards: a row
 *  without the key's value goes last whichever the direction, and equal values keep `tie`'s
 *  order. `tie` is a comparator rather than a number so it runs only when it decides. */
export function cmpMissingLast<T>(a: T, b: T, va: string | undefined, vb: string | undefined, dir: number, tie: (a: T, b: T) => number): number {
  if (va === undefined || vb === undefined) return va === vb ? tie(a, b) : va === undefined ? 1 : -1;
  return cmpValue(va, vb) * dir || tie(a, b);
}

/** A stable sort; the missing-value and tie-break rules are cmpMissingLast's. */
export function sortHits(hits: BlockHit[], spec: SortSpec): BlockHit[] {
  const k = spec.key;
  const dir = spec.desc ? -1 : 1;
  const lower = k.by === "field" ? k.key.toLowerCase() : "";
  const cmp = (a: BlockHit, b: BlockHit): number => {
    let c: number;
    if (k.by === "note") c = byNote(a, b);
    else if (k.by === "kind") c = a.block.kind.type.localeCompare(b.block.kind.type);
    else if (k.by === "text") c = a.block.content.localeCompare(b.block.content);
    else return cmpMissingLast(a, b, fieldOfLower(a, lower), fieldOfLower(b, lower), dir, byNote);
    return c * dir || byNote(a, b);
  };
  return [...hits].sort(cmp);
}

export interface HitGroup {
  /** The stable identity (a path, a kind, a value); the "none" group's is empty. */
  key: string;
  title: string;
  hits: BlockHit[];
}

/** Groups in first-seen order of the (already sorted) hits; blocks without the
 *  field go under `noneTitle`, last. `kindLabel` names a kind group. */
export function groupHits(hits: BlockHit[], group: GroupKey, noneTitle: string, kindLabel: (kind: string) => string): HitGroup[] {
  const groups = new Map<string, HitGroup>();
  const none: BlockHit[] = [];
  const lower = group.by === "field" ? group.key.toLowerCase() : "";
  for (const h of hits) {
    let key: string | undefined;
    let title: string;
    if (group.by === "note") {
      key = h.note;
      title = h.note_title;
    } else if (group.by === "kind") {
      key = h.block.kind.type;
      title = kindLabel(key);
    } else {
      key = fieldOfLower(h, lower);
      title = key ?? "";
    }
    if (key === undefined) {
      none.push(h);
      continue;
    }
    let g = groups.get(key);
    if (!g) {
      g = { key, title, hits: [] };
      groups.set(key, g);
    }
    g.hits.push(h);
  }
  const out = [...groups.values()];
  if (none.length) out.push({ key: "", title: noneTitle, hits: none });
  return out;
}
