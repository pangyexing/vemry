// The kanban view's pure logic: every task in the vault laid out in columns. Two
// grouping modes: by state — the time axis in five columns (overdue / today /
// scheduled / unscheduled / done) — or by an inline field, one column per value the
// tasks carry plus "none". Dragging a card writes back what the column means: a date
// edit in state mode, `[key:: value]` in field mode.
// The state grouping reads tasks.ts's whenOf (due first, and a scheduled-only task's
// scheduled date counts as its date; a task starting today with ⏳ belongs to today
// even when its due date is later) — the same rule as Agenda, so the two views cannot
// disagree about a task.
// doing is not a column (it is orthogonal to the date dimension): it lands in its
// column by date as usual, and the card shows a marker and sorts to the top of that
// column.
import { t } from "$lib/i18n/i18n.svelte";
import type { BlockHit } from "$lib/api";
import { byWhen, DATE_FIELDS, type DateField, rowOf, type TaskEdit, type TaskRow, whenOf } from "$lib/views/tasks";

export type ColumnId = "overdue" | "today" | "scheduled" | "unscheduled" | "done";

/** What the columns mean: the time axis, or one inline field's values. */
export type GroupMode = { by: "state" } | { by: "field"; key: string };

/** The column of a task that carries no value of the grouping field (a spelling no real value takes). */
export const NONE_COL = "__none__";

export interface Card extends TaskRow {
  /** The time-axis column (the state grouping's column; the field grouping reads `fields`). */
  col: ColumnId;
  /** The card's tags: deduplicated and truncated once here rather than recomputed in the template on every render of every card. */
  tags: string[];
  /** The task's inline fields, lowercase key → first value (the date fields excluded: they are the chips). */
  fields: Record<string, string>;
  /** The field chips a card shows (at most two). */
  fieldChips: [string, string][];
}

/** The semantic level of a column header's colour dot, which the view maps onto the existing tokens (danger/accent/fg-muted/fg-faint). */
export type ColumnTone = "danger" | "accent" | "muted" | "faint";

/** One rendered column: the id is the stable key of a drop target and of the landing
 *  logic (a state column's name, or a field value), the title is copy. */
export interface ColumnDef {
  id: string;
  title: string;
  drop: boolean;
  add: boolean;
  tone: ColumnTone;
}

/** The state columns in render order. "Overdue" is not a valid drop target
 *  (dragging a task into being overdue means nothing), so KanbanView gives it no
 *  data-drop-col from drop:false; add works the same way: creating something in the
 *  overdue column means nothing, and the done column does not create finished
 *  items. */
export const COLUMNS: { id: ColumnId; drop: boolean; add: boolean; tone: ColumnTone }[] = [
  { id: "overdue", drop: false, add: false, tone: "danger" },
  { id: "today", drop: true, add: true, tone: "accent" },
  { id: "scheduled", drop: true, add: true, tone: "muted" },
  { id: "unscheduled", drop: true, add: true, tone: "muted" },
  { id: "done", drop: true, add: false, tone: "faint" },
];

/** The column's name. Kept apart from the id: the id is the stable key of a drop target and of the landing logic, and the name is copy. */
export function columnTitle(id: ColumnId): string {
  return t(`kanban.col.${id}`);
}

/** A field value's sort: dates and numbers in their own order, everything else alphabetical. */
function byValue(a: string, b: string): number {
  const na = Number(a);
  const nb = Number(b);
  if (a !== "" && b !== "" && !Number.isNaN(na) && !Number.isNaN(nb)) return na - nb;
  return a.localeCompare(b);
}

/** The columns of a grouping: the five state columns, or the distinct values of the
 *  field among the cards (sorted) followed by "none". */
export function columnDefsFor(mode: GroupMode, cards: Card[]): ColumnDef[] {
  if (mode.by === "state") return COLUMNS.map((c) => ({ ...c, title: columnTitle(c.id) }));
  const key = mode.key.toLowerCase();
  const values = [...new Set(cards.map((c) => c.fields[key]).filter((v): v is string => v !== undefined))].sort(byValue);
  const defs: ColumnDef[] = values.map((v) => ({ id: v, title: v, drop: true, add: true, tone: "muted" }));
  defs.push({ id: NONE_COL, title: t("kanban.col.none"), drop: true, add: true, tone: "faint" });
  return defs;
}

/** The column a card sits in under a grouping. */
export function cardCol(card: Card, mode: GroupMode): string {
  if (mode.by === "state") return card.col;
  return card.fields[mode.key.toLowerCase()] ?? NONE_COL;
}

/** The field keys the cards carry (for the group-by menu), most used first. */
export function fieldKeysOf(cards: Card[]): string[] {
  const counts = new Map<string, number>();
  for (const c of cards) for (const k of Object.keys(c.fields)) counts.set(k, (counts.get(k) ?? 0) + 1);
  return [...counts.keys()].sort((a, b) => counts.get(b)! - counts.get(a)! || a.localeCompare(b));
}

/** The task-line payload of a quick create inside a column: it carries what the column
 *  means — the date marker in state mode (a create always writes due, mirroring
 *  activeField's "with neither, set due"), the field in field mode.
 *  null = the text is empty or that column does not support creating. */
export function quickTaskPayload(col: string, text: string, today: string, date?: string, mode: GroupMode = { by: "state" }): string | null {
  const t = text.trim();
  if (!t) return null;
  if (mode.by === "field") return col === NONE_COL ? `- [ ] ${t}` : `- [ ] ${t} [${mode.key}:: ${col}]`;
  const due = (d: string) => `- [ ] ${t} ${DATE_FIELDS.due.emoji} ${d}`;
  switch (col) {
    case "today":
      return due(today);
    case "scheduled":
      return date ? due(date) : null;
    case "unscheduled":
      return `- [ ] ${t}`;
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
 *  rowOf's chain of regexes on every keystroke. */
export function cardsOf(hits: BlockHit[], today: string): Card[] {
  return hits.map((hit) => cardOf(hit, today));
}

function cardOf(hit: BlockHit, today: string): Card {
  const r = rowOf(hit);
  const when = whenOf(r);
  const col: ColumnId =
    r.state === "done"
      ? "done"
      : when && when < today
        ? "overdue"
        : when === today || r.scheduled === today
          ? "today"
          : when
            ? "scheduled"
            : "unscheduled";
  const fields: Record<string, string> = {};
  for (const f of hit.block.fields ?? []) {
    const k = f.key.toLowerCase();
    if (k in DATE_FIELDS || k in fields) continue;
    fields[k] = f.value;
  }
  return {
    ...r,
    col,
    tags: [...new Set(hit.block.tags ?? [])].slice(0, MAX_TAGS),
    fields,
    fieldChips: Object.entries(fields).slice(0, MAX_FIELD_CHIPS),
  };
}

/** The order inside a column: doing on top, then byWhen (date ascending plus the note's name) — so the order is stable after a requery. */
const byCard = (a: Card, b: Card) =>
  (a.state === "doing" ? 0 : 1) - (b.state === "doing" ? 0 : 1) || byWhen(a, b);

export function columnsOf(cards: Card[], defs: ColumnDef[], mode: GroupMode): Record<string, Card[]> {
  const cols: Record<string, Card[]> = Object.fromEntries(defs.map((d) => [d.id, [] as Card[]]));
  for (const c of cards) cols[cardCol(c, mode)]?.push(c);
  for (const list of Object.values(cols)) list.sort(byCard);
  return cols;
}

/** The date field the card's current grouping reads: due first (consistent with
 *  whenOf), and due when it has neither. Rescheduling only touches the field the
 *  parser actually reads and never appends a second marker. */
export function activeField(card: Card): DateField {
  return card.due ? "due" : card.scheduled ? "scheduled" : "due";
}

/** The landing semantics table: target column → one merged TaskEdit written back.
 *  null = no action (the same column, or invalid). In state mode "scheduled" needs a
 *  concrete date, which the view collects from a date menu first and passes in; in
 *  field mode the column is the value to write ("none" removes the field). */
export function dropEdit(card: Card, col: string, today: string, date?: string, mode: GroupMode = { by: "state" }): TaskEdit | null {
  if (col === cardCol(card, mode)) return null;
  if (mode.by === "field") return { field: { key: mode.key, value: col === NONE_COL ? null : col } };
  if (col === "overdue") return null;
  // Dragging out of "done" reopens it, merged into the same write-back as the target column's date action
  const reopen = card.state === "done" ? ("todo" as const) : undefined;
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
