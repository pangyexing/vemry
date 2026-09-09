// One block as a row of a projection: the words, the state, the dates and the chips,
// derived once from a BlockHit. A query block's rows read it, and it is the one place
// that decides what of a block's metadata is words and what is a chip — before it, a
// query row folded the whole block through plainText (a `[status:: unread]` read as
// the text "status: unread"), Agenda stripped the fields out and the board showed two
// of them as chips: the same block, three answers.
// Pure apart from the types it is handed: the state vocabulary comes from fields.ts's
// stateField (a `type: state` key of a block type), never from a store.
import type { BlockHit, BlockType, TaskState } from "$lib/api";
import { stripAnchor, stripFields, stateField, stateIndex } from "$lib/fields";
import { plainText } from "$lib/markdown";
import { calloutText } from "$lib/views/claims";
import { isClosed, TASK_ATTR_KEYS, STAMP_RE, taskText } from "$lib/views/tasks";
import { TASK_STEPS, taskStep, type StateStep } from "$lib/chips";

/** A chip on a row: a state field's value with its step, a plain field, or a tag. */
export type RowChip =
  | { kind: "state"; key: string; value: string; step: number; of: number }
  | { kind: "field"; key: string; value: string }
  | { kind: "tag"; tag: string };

/** The chips derived from a block's inline fields: every RowChip but a tag, which comes
 *  off the block's tags and not its fields. */
export type FieldChip = Exclude<RowChip, { kind: "tag" }>;

export interface BlockRow {
  hit: BlockHit;
  /** The words: the block's text without its markers, fields and anchor; the note's
   *  title when nothing but tags and fields is left (a `#chapter [status:: unread]`
   *  line names its note, not itself). */
  text: string;
  titleAsText: boolean;
  /** A task's state; absent on every other kind. */
  state?: TaskState;
  due?: string;
  scheduled?: string;
  /** An unfinished task whose due date has passed. */
  overdue: boolean;
  /** The first state-field chip, when the block carries one — the row's own state. */
  stateChip?: Extract<RowChip, { kind: "state" }>;
  chips: RowChip[];
}

export interface RowOptions {
  /** Local `YYYY-MM-DD`, for the overdue test. */
  today: string;
  /** Tags not shown as chips: what the query already filtered by (every row would
   *  repeat it), or the group's own tag. Case-insensitive. */
  hideTags?: string[];
}

/** How many field chips (state chips aside) and tag chips a row shows at most. */
const MAX_FIELD_CHIPS = 3;
const MAX_TAG_CHIPS = 2;

/** `#tag` tokens, as the parser and the editor read them (livepreview's TAG_RE). */
const TAG_TOKEN_RE = /(^|\s)#([\p{L}\p{N}_][\p{L}\p{N}_/-]*)/gu;

/** The words of a block, by kind: a task through the task views' rule, a callout
 *  through the claims view's, the rest with the block marker, the fields, a capture
 *  stamp and the anchor taken off, then folded to plain text. */
export function blockText(hit: BlockHit): string {
  const k = hit.block.kind;
  const content = hit.block.content;
  if (k.type === "task") return taskText(content);
  if (k.type === "claim" || k.type === "evidence") return calloutText(stripFields(content));
  let s = stripFields(content).replace(STAMP_RE, "");
  if (k.type === "heading") s = s.replace(/^\s*#{1,6}\s+/, "");
  else if (k.type === "list_item") s = s.replace(/^\s*(?:[-*+]|\d{1,9}[.)])\s+/, "");
  else if (k.type === "code_block") s = s.split("\n").find((l) => l.trim() && !l.trim().startsWith("```")) ?? "";
  return plainText(stripAnchor(s.split("\n")[0] ?? ""));
}

/** A block's field chips: one per key (the first value, as the board reads it), a state
 *  field's as a state chip with its step, the rest as plain chips capped at
 *  MAX_FIELD_CHIPS; a task's date fields are its date chips and are left out. Agenda's
 *  rows and a query row read this one derivation. */
export function fieldChipsOf(hit: BlockHit, types: BlockType[]): FieldChip[] {
  const chips: FieldChip[] = [];
  const task = hit.block.kind.type === "task";
  let fieldCount = 0;
  const seen = new Set<string>();
  for (const f of hit.block.fields ?? []) {
    const lower = f.key.toLowerCase();
    if (seen.has(lower)) continue;
    seen.add(lower);
    if (task && TASK_ATTR_KEYS.has(lower)) continue;
    const vocab = stateField(types, f.key);
    if (vocab) {
      const step = stateIndex(vocab, f.value);
      if (step >= 0) {
        chips.push({ kind: "state", key: f.key, value: vocab[step], step, of: vocab.length });
        continue;
      }
    }
    if (fieldCount++ < MAX_FIELD_CHIPS) chips.push({ kind: "field", key: f.key, value: f.value });
  }
  return chips;
}

export function blockRowOf(hit: BlockHit, types: BlockType[], opts: RowOptions): BlockRow {
  const k = hit.block.kind;
  let text = blockText(hit);
  let titleAsText = false;
  if (!text.replace(TAG_TOKEN_RE, "").trim()) {
    text = hit.note_title;
    titleAsText = true;
  }
  const chips: RowChip[] = fieldChipsOf(hit, types);
  const stateChip = chips.find((c): c is Extract<RowChip, { kind: "state" }> => c.kind === "state");
  const hidden = new Set((opts.hideTags ?? []).map((t) => t.toLowerCase()));
  let tagCount = 0;
  for (const tag of new Set(hit.block.tags ?? [])) {
    if (hidden.has(tag.toLowerCase())) continue;
    if (tagCount++ < MAX_TAG_CHIPS) chips.push({ kind: "tag", tag });
  }
  const row: BlockRow = { hit, text, titleAsText, overdue: false, stateChip, chips };
  if (k.type === "task") {
    row.state = k.state;
    row.due = k.due;
    row.scheduled = k.scheduled;
    row.overdue = !isClosed(k.state) && !!k.due && k.due < opts.today;
  }
  return row;
}

/** The state a row is at, for the glyph in its margin: a task's checkbox state in the
 *  three-step vocabulary, else its first state field's step. */
export function rowStep(row: BlockRow): StateStep | null {
  if (row.state) return { step: taskStep(row.state), of: TASK_STEPS, cancelled: row.state === "cancelled" };
  return row.stateChip ? { step: row.stateChip.step, of: row.stateChip.of } : null;
}

export interface StateSummary {
  key: string;
  values: string[];
  /** Rows at each value, in the vocabulary's order. */
  counts: number[];
  /** Rows with the key at all (a row without it is in no count). */
  total: number;
}

/** The distribution of the rows over one state field — the head of a query block
 *  whose rows carry one. The field is the state key most rows carry; null when no row
 *  carries any (the head then says only how many rows there are). */
export function stateSummary(rows: BlockRow[], types: BlockType[]): StateSummary | null {
  const perKey = new Map<string, { chips: Extract<RowChip, { kind: "state" }>[]; of: number }>();
  for (const r of rows)
    for (const c of r.chips) {
      if (c.kind !== "state") continue;
      const lower = c.key.toLowerCase();
      let e = perKey.get(lower);
      if (!e) {
        e = { chips: [], of: c.of };
        perKey.set(lower, e);
      }
      e.chips.push(c);
    }
  let best: { chips: Extract<RowChip, { kind: "state" }>[]; of: number } | undefined;
  for (const e of perKey.values()) if (!best || e.chips.length > best.chips.length) best = e;
  if (!best) return null;
  const key = best.chips[0].key;
  const counts = new Array<number>(best.of).fill(0);
  // a step outside the vocabulary the first chip reported would lengthen `counts` past
  // `values` and leave the head drawing a bar with no name
  for (const c of best.chips) if (c.step < counts.length) counts[c.step]++;
  const values = stateField(types, key) ?? best.chips.map((c) => c.value);
  return { key, values, counts, total: best.chips.length };
}
