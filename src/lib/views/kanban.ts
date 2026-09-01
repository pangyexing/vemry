// The kanban view's pure logic: every task in the vault laid out along a time axis in
// five columns (overdue / today / scheduled / unscheduled / done).
// The grouping reads tasks.ts's whenOf (due first, and a scheduled-only task's
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

export interface Card extends TaskRow {
  col: ColumnId;
  /** The card's tags: deduplicated and truncated once here rather than recomputed in the template on every render of every card. */
  tags: string[];
}

/** The semantic level of a column header's colour dot, which the view maps onto the existing tokens (danger/accent/fg-muted/fg-faint). */
export type ColumnTone = "danger" | "accent" | "muted" | "faint";

/** The column order is the render order. "Overdue" is not a valid drop target
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

/** The task-line payload of a quick create inside a column: it carries the date
 *  marker of the column it lands in (consistent with dropEdit's column semantics, and
 *  a create always writes due, mirroring activeField's "with neither, set due").
 *  null = the text is empty or that column does not support creating. */
export function quickTaskPayload(col: ColumnId, text: string, today: string, date?: string): string | null {
  const t = text.trim();
  if (!t) return null;
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
  return { ...r, col, tags: [...new Set(hit.block.tags ?? [])].slice(0, MAX_TAGS) };
}

/** The order inside a column: doing on top, then byWhen (date ascending plus the note's name) — so the order is stable after a requery. */
const byCard = (a: Card, b: Card) =>
  (a.state === "doing" ? 0 : 1) - (b.state === "doing" ? 0 : 1) || byWhen(a, b);

export function columnsOf(cards: Card[]): Record<ColumnId, Card[]> {
  const cols: Record<ColumnId, Card[]> = { overdue: [], today: [], scheduled: [], unscheduled: [], done: [] };
  for (const c of cards) cols[c.col].push(c);
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
 *  null = no action (the same column, or invalid). "Scheduled" needs a concrete date,
 *  which the view collects from a date menu first and passes in. */
export function dropEdit(card: Card, col: ColumnId, today: string, date?: string): TaskEdit | null {
  if (col === card.col || col === "overdue") return null;
  // Dragging out of "done" reopens it, merged into the same write-back as the target column's date action
  const reopen = card.state === "done" ? ("todo" as const) : undefined;
  switch (col) {
    case "today":
      return { state: reopen, date: { field: activeField(card), value: today } };
    case "scheduled":
      return date ? { state: reopen, date: { field: activeField(card), value: date } } : null;
    case "unscheduled":
      return { state: reopen, clearDates: true };
    case "done":
      return { state: "done" };
  }
}
