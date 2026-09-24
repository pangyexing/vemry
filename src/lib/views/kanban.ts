// The kanban view's pure logic: every task in the vault laid out in columns. Two
// grouping modes: by state — the time axis in five columns (overdue / today /
// scheduled / unscheduled / done) — or by an inline field, one column per value the
// tasks carry plus "none". Dragging a card writes back what the column means: a date
// edit in state mode, `[key:: value]` in field mode.
// The state grouping reads tasks.ts's placeTask, the function Agenda's groups are: a
// state column is a coarser cut of the same phases (stateColumn), so the two views
// cannot disagree about a task, and a card shows the date that put it in its column.
// doing is not a column (it is orthogonal to the date dimension): it lands in its
// column by date as usual, and the card shows a marker and sorts to the top of that
// column.
// Inside a column the cards follow one order the user picks (CardOrder). The default,
// "auto", is each column's own natural order — the date where there is one, the task's
// own change time for the unscheduled and the done columns; every other key (the date,
// the change time, the source order, the text, a field) applies to every column alike,
// ascending or descending, so the label never hides a special case. doing stays on top
// under every one of them.
import { compareNames, t } from "$lib/i18n/i18n.svelte";
import type { BlockHit } from "$lib/api";
import { cmpMissingLast, cmpValue } from "$lib/views/queryrows";
import type { BlockType } from "$lib/api";
import { stateField, stateIndex } from "$lib/fields";
import { stateGlyphSvg } from "$lib/chips";
import { isClosed, TASK_ATTR_KEYS, bySource, DATE_FIELDS, type DateField, placeTask, rowOf, type TaskEdit, type TaskPhase, type TaskRow, whenOf } from "$lib/views/tasks";

type ColumnId = "overdue" | "today" | "scheduled" | "unscheduled" | "done";

/** The state column a phase falls in: every future date is "scheduled" (the next
 *  seven days, later, and a start day still to come), a task with no date to place it
 *  "unscheduled" whether or not it is in progress. */
const PHASE_COLUMN: Record<TaskPhase, ColumnId> = {
  overdue: "overdue",
  today: "today",
  week: "scheduled",
  later: "scheduled",
  future: "scheduled",
  doing: "unscheduled",
  nodate: "unscheduled",
  done: "done",
};

/** A task's state column and the date that decided it (placeTask). */
export function stateColumn(r: TaskRow, today: string): { col: ColumnId; field?: DateField } {
  const p = placeTask(r, today);
  return { col: PHASE_COLUMN[p.phase], field: p.field };
}

/** What the columns mean: the time axis, or one inline field's values. */
export type GroupMode = { by: "state" } | { by: "field"; key: string };

/** The column of a task that carries no value of the grouping field (a spelling no real value takes). */
const NONE_COL = "__none__";

export interface Card extends TaskRow {
  /** The time-axis column (the state grouping's column; the field grouping reads `fields`). */
  col: ColumnId;
  /** The date that put the card in its column — the one its main date slot shows. */
  colField?: DateField;
  /** The task's inline fields, lowercase key → first value (the date fields excluded: they are the chips). */
  fields: Record<string, string>;
  /** The field chips a card shows (at most two): the key, the value, and — when the key
   *  is a state field — the value's step, for the glyph. */
  fieldChips: FieldChipSpec[];
}

export interface FieldChipSpec {
  key: string;
  value: string;
  step?: number;
  of?: number;
}

/** The semantic level of a column header's colour dot, which the view maps onto the existing tokens (danger/accent/fg-muted/fg-faint). */
type ColumnTone = "danger" | "accent" | "muted" | "faint";

/** One rendered column: the id is the stable key of a drop target and of the landing
 *  logic (a state column's name, or a field value), the title is copy. */
interface ColumnDef {
  id: string;
  title: string;
  drop: boolean;
  add: boolean;
  tone: ColumnTone;
  /** A state field's column: the step's glyph (chips.ts, which colours itself) stands in for the tone dot. */
  glyph?: string;
}

/** The state columns in render order. "Overdue" is not a valid drop target
 *  (dragging a task into being overdue means nothing), so KanbanView gives it no
 *  data-drop-col from drop:false; add works the same way: creating something in the
 *  overdue column means nothing, and the done column does not create finished
 *  items. */
const COLUMNS: { id: ColumnId; drop: boolean; add: boolean; tone: ColumnTone }[] = [
  { id: "overdue", drop: false, add: false, tone: "danger" },
  { id: "today", drop: true, add: true, tone: "accent" },
  { id: "scheduled", drop: true, add: true, tone: "muted" },
  { id: "unscheduled", drop: true, add: true, tone: "muted" },
  { id: "done", drop: true, add: false, tone: "faint" },
];

/** The column's name. Kept apart from the id: the id is the stable key of a drop target and of the landing logic, and the name is copy. */
function columnTitle(id: ColumnId): string {
  return t(`kanban.col.${id}`);
}

/** The columns of a grouping: the five state columns, or the distinct values of the
 *  field among the cards (dates and numbers in their own order, the rest alphabetical
 *  — a query block's cmpValue) followed by "none". Grouped by a state field (`types`
 *  declare it), the columns are its vocabulary in the declared order, every value a
 *  column whether or not a card sits in it, with each step's glyph; a value outside
 *  the vocabulary follows them. */
export function columnDefsFor(mode: GroupMode, cards: Card[], types: BlockType[], kept: Iterable<string> = []): ColumnDef[] {
  if (mode.by === "state") return COLUMNS.map((c) => ({ ...c, title: columnTitle(c.id) }));
  const key = mode.key.toLowerCase();
  const declared = stateField(types, mode.key);
  // `kept`: values that had a column earlier in this visit — a column whose last card
  // went elsewhere stays, empty, rather than taking the board's width with it
  const seen = [...new Set([...cards.map((c) => c.fields[key]).filter((v): v is string => v !== undefined), ...[...kept].filter((v) => v !== NONE_COL)])];
  let defs: ColumnDef[];
  if (declared) {
    const extra = seen.filter((v) => stateIndex(declared, v) < 0).sort(cmpValue);
    defs = declared.map((v, i) => ({ id: v, title: v, drop: true, add: true, tone: "muted", glyph: stateGlyphSvg(i, declared.length) }));
    defs.push(...extra.map((v) => ({ id: v, title: v, drop: true, add: true, tone: "muted" as ColumnTone })));
  } else {
    defs = seen.sort(cmpValue).map((v) => ({ id: v, title: v, drop: true, add: true, tone: "muted" }));
  }
  defs.push({ id: NONE_COL, title: t("kanban.col.none"), drop: true, add: true, tone: "faint" });
  return defs;
}

/** The column a card sits in under a grouping. Under a state field the column is the
 *  declared spelling of the card's value (a card that wrote `Done` sits in `done`). */
export function cardCol(card: Card, mode: GroupMode, types: BlockType[], declared?: string[] | null): string {
  if (mode.by === "state") return card.col;
  const v = card.fields[mode.key.toLowerCase()];
  if (v === undefined) return NONE_COL;
  // the grouping key's vocabulary is a loop invariant: columnsOf reads it once for the
  // whole board rather than once per card
  const vocab = declared === undefined ? stateField(types, mode.key) : declared;
  if (!vocab) return v;
  const i = stateIndex(vocab, v);
  return i < 0 ? v : vocab[i];
}

/** The field keys the cards carry (for the group-by menu), most used first. */
export function fieldKeysOf(cards: Card[]): string[] {
  const counts = new Map<string, number>();
  for (const c of cards) for (const k of Object.keys(c.fields)) counts.set(k, (counts.get(k) ?? 0) + 1);
  return [...counts.keys()].sort((a, b) => counts.get(b)! - counts.get(a)! || compareNames(a, b));
}

/** The task-line payload of a quick create inside a column: it carries what the column
 *  means — the date marker in state mode (a create always writes due, mirroring
 *  activeField's "with neither, set due"), the field in field mode — and the `➕`
 *  created stamp a task made from a view gets (a hand-written line may leave it out),
 *  the attributes in their written order.
 *  null = the text is empty or that column does not support creating. */
export function quickTaskPayload(col: string, text: string, today: string, date?: string, mode: GroupMode = { by: "state" }): string | null {
  const t = text.trim();
  if (!t) return null;
  const created = `${DATE_FIELDS.created.emoji[0]} ${today}`;
  if (mode.by === "field") return col === NONE_COL ? `- [ ] ${t} ${created}` : `- [ ] ${t} [${mode.key}:: ${col}] ${created}`;
  const due = (d: string) => `- [ ] ${t} ${created} ${DATE_FIELDS.due.emoji[0]} ${d}`;
  switch (col) {
    case "today":
      return due(today);
    case "scheduled":
      return date ? due(date) : null;
    case "unscheduled":
      return `- [ ] ${t} ${created}`;
    default:
      return null;
  }
}

/** How many tags a card shows at most. */
const MAX_TAGS = 3;
/** How many field chips a card shows at most. */
const MAX_FIELD_CHIPS = 2;

/** Block → card, once per block. It is kept apart from the column assignment because
 *  the kanban's filter box filters at the card level: filtering reads tasks.ts's
 *  taskFilterKey (the same key as Agenda's), and building the cards should not rerun
 *  rowOf's chain of regexes on every keystroke. `keys` are the hits' identities
 *  (TaskKeys), in their order. */
export function cardsOf(hits: BlockHit[], keys: string[], today: string, types: BlockType[]): Card[] {
  return hits.map((hit, i) => cardOf(hit, keys[i], today, types));
}

function cardOf(hit: BlockHit, key: string | undefined, today: string, types: BlockType[]): Card {
  const r = rowOf(hit, key);
  const { col, field: colField } = stateColumn(r, today);
  const fields: Record<string, string> = {};
  for (const f of hit.block.fields ?? []) {
    const k = f.key.toLowerCase();
    if (TASK_ATTR_KEYS.has(k) || k in fields) continue;
    fields[k] = f.value;
  }
  return {
    ...r,
    col,
    colField,
    tags: r.tags.slice(0, MAX_TAGS),
    fields,
    fieldChips: Object.entries(fields)
      .slice(0, MAX_FIELD_CHIPS)
      .map(([key, value]) => {
        const declared = stateField(types, key);
        const step = declared ? stateIndex(declared, value) : -1;
        return step >= 0 ? { key, value: declared![step], step, of: declared!.length } : { key, value };
      }),
  };
}

/** What the cards inside a column are ordered by. `auto` is each column's own natural
 *  order (columnComparator) and has no direction; the rest apply one key to every
 *  column alike: the task's date (whenOf — the same date Agenda sorts by), the task's
 *  own change time (TaskRow.modified), the source order (the note's path, then the
 *  position in it), the display text, or one inline field's value. */
export type SortKey =
  | { by: "auto" }
  | { by: "date" }
  | { by: "modified" }
  | { by: "note" }
  | { by: "text" }
  | { by: "field"; key: string };

/** A column's order: the key and its direction (meaningless under `auto`). */
export interface CardOrder {
  key: SortKey;
  desc: boolean;
}

export const DEFAULT_ORDER: CardOrder = { key: { by: "auto" }, desc: false };
const DATE_ASC: CardOrder = { key: { by: "date" }, desc: false };
/** The natural order of a column with no date to read: most recently changed first. */
const MODIFIED_DESC: CardOrder = { key: { by: "modified" }, desc: true };

/** The direction a key starts in when it is picked: newest first for the change time
 *  (nobody picks it to see what they touched longest ago), ascending for the rest. The
 *  arrow shows it and flips it. */
export function defaultDesc(key: SortKey): boolean {
  return key.by === "modified";
}

/** The one spelling of a sort key the select's options and the persisted UI state
 *  share: `auto` / `date` / `modified` / `note` / `text` / `field:<key>`. */
export function sortKeyId(key: SortKey): string {
  return key.by === "field" ? `field:${key.key}` : key.by;
}

/** The inverse of sortKeyId; null for anything else (a stored value from an older build, an empty field key). */
export function parseSortKey(id: string): SortKey | null {
  if (id === "auto" || id === "date" || id === "modified" || id === "note" || id === "text") return { by: id };
  const key = id.startsWith("field:") ? id.slice("field:".length) : "";
  return key ? { by: "field", key } : null;
}

const doingFirst = (a: Card, b: Card) => (a.state === "doing" ? 0 : 1) - (b.state === "doing" ? 0 : 1);

/** The comparator of one resolved key, the same for every column (columnComparator is
 *  what resolves `auto`, and where it differs per column). doing stays on top whatever
 *  the key — it is the task being worked on, and it is orthogonal to every key. The
 *  missing-value and tie-break rules are cmpMissingLast's, the same implementation a
 *  query block's `sort` reads, so the two cannot drift. */
function cardComparator(order: CardOrder, types: BlockType[]): (a: Card, b: Card) => number {
  const k = order.key;
  const dir = order.desc ? -1 : 1;
  // The field key is a loop invariant: lowercasing it inside the comparator would run
  // twice per comparison
  const fieldKey = k.by === "field" ? k.key.toLowerCase() : "";
  const valueOf = (c: Card): string | undefined => (k.by === "date" ? whenOf(c) : k.by === "field" ? c.fields[fieldKey] : undefined);
  // a sort by a state field reads its declared order (cmpMissingLast's `declared`), any
  // other field cmpValue — the vocabulary is the sort key's own, never the grouping key's
  const order_ = k.by === "field" ? stateField(types, k.key) : null;
  return (a, b) => {
    const doing = doingFirst(a, b);
    if (doing) return doing;
    let c: number;
    if (k.by === "note") c = bySource(a, b);
    else if (k.by === "text") c = compareNames(a.text, b.text);
    else if (k.by === "modified") c = a.modified - b.modified;
    else return cmpMissingLast(a, b, valueOf(a), valueOf(b), dir, bySource, order_);
    return c * dir || bySource(a, b);
  };
}

/** The comparator one column reads. An explicit key is the same for every column. Under
 *  `auto` each column reads its own natural order: the date, soonest first, where there
 *  is one; and for the two columns that have no date to read — an unscheduled task has
 *  none, and a finished task's due date says nothing about when it was finished — the
 *  task's own change time, most recently changed first, the way a task manager's inbox
 *  and completed list read (Agenda's no-date and done groups read the same rule). Only
 *  the state grouping has these two columns. */
function columnComparator(col: string, mode: GroupMode, order: CardOrder, types: BlockType[]): (a: Card, b: Card) => number {
  if (order.key.by !== "auto") return cardComparator(order, types);
  const noDate = mode.by === "state" && (col === "unscheduled" || col === "done");
  return cardComparator(noDate ? MODIFIED_DESC : DATE_ASC, types);
}

/** A card held where it stood: the column it was in and the card as it was then, which
 *  it is sorted as — so neither its column nor its place in it moves (the view's `stay`). */
export interface HeldCard {
  col: string;
  as: Card;
}

/** The cards of each column, sorted. A card in `held` sits in the column it was held
 *  in (when that column is still on the board) and sorts as it was then. */
export function columnsOf(
  cards: Card[],
  defs: ColumnDef[],
  mode: GroupMode,
  order: CardOrder,
  types: BlockType[],
  held?: ReadonlyMap<string, HeldCard>,
): Record<string, Card[]> {
  const cols: Record<string, { card: Card; as: Card }[]> = Object.fromEntries(defs.map((d) => [d.id, []]));
  const declared = mode.by === "field" ? stateField(types, mode.key) : null;
  for (const c of cards) {
    const h = held?.get(c.key);
    const col = h && h.col in cols ? h.col : cardCol(c, mode, types, declared);
    cols[col]?.push({ card: c, as: h && h.col in cols ? h.as : c });
  }
  const out: Record<string, Card[]> = {};
  for (const [id, list] of Object.entries(cols)) {
    const cmp = columnComparator(id, mode, order, types);
    out[id] = list.sort((a, b) => cmp(a.as, b.as)).map((e) => e.card);
  }
  return out;
}

/** The date field the card's current grouping reads: due first (consistent with
 *  whenOf), and due when it has neither. Rescheduling only touches the field the
 *  parser actually reads and never appends a second marker. */
export function activeField(card: Card): DateField {
  return card.due ? "due" : card.scheduled ? "scheduled" : "due";
}

/** The card as a drop into "scheduled" leaves it once `date` is written (dropEdit's
 *  write, read back): the date in activeField, a closed card reopened, and the column
 *  and the date that places it. The view shows this card while the write lands, at the
 *  place the reload will put it — so the card does not first sit somewhere else. */
export function droppedCard(card: Card, date: string, today: string): Card {
  const r: Card = { ...card, [activeField(card)]: date, state: isClosed(card.state) ? "todo" : card.state };
  const { col, field } = stateColumn(r, today);
  return { ...r, col, colField: field };
}

/** The landing semantics table: target column → one merged TaskEdit written back.
 *  null = no action (the same column, or invalid). In state mode "scheduled" needs a
 *  concrete date, which the view collects from a date menu first and passes in; in
 *  field mode the column is the value to write ("none" removes the field). */
export function dropEdit(card: Card, col: string, today: string, date: string | undefined, mode: GroupMode, types: BlockType[]): TaskEdit | null {
  if (col === cardCol(card, mode, types)) return null;
  if (mode.by === "field") return { field: { key: mode.key, value: col === NONE_COL ? null : col } };
  if (col === "overdue") return null;
  // Dragging out of "done" reopens it, merged into the same write-back as the target column's date action
  const reopen = isClosed(card.state) ? ("todo" as const) : undefined;
  switch (col as ColumnId) {
    case "today":
      return { state: reopen, date: { field: activeField(card), value: today } };
    case "scheduled":
      return date ? { state: reopen, date: { field: activeField(card), value: date } } : null;
    case "unscheduled":
      return { state: reopen, clearDates: true };
    case "done":
      return { state: "done" };
    default:
      return null;
  }
}
