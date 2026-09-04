// Agenda-view task actions: flip a checkbox / reschedule a date on disk, span-accurate.
import { t, tn } from "$lib/i18n/i18n.svelte";
import { setFieldValue, stripAnchor, stripFields } from "$lib/fields";
import { api, type BlockHit } from "$lib/api";
import { byNote } from "$lib/views/queryrows";
import { plainText } from "$lib/markdown";

/** A task line as the field layer sees it: a bullet or ordered marker and a checkbox.
 *  Wider than CHECKBOX_RE, which does not take ordered items. The renderer and the
 *  chip's writer read this one rule — otherwise a chip could be drawn through one and
 *  written through the other. */
export const TASK_LINE_RE = /^\s*(?:[-*+]|\d{1,9}[.)])\s+\[[ xX/]\]/;

/** Checkbox prefix at the start of a task block. The state char is `(.)`
 *  because the Rust parser accepts any single char (unknowns count as todo) —
 *  a `[ /xX]`-only regex leaves `- [-] x` style tasks with a dead checkbox. */
const CHECKBOX_RE = /^(\s*[-*+]\s+)\[(.)\](?=\s|$)/;

/** Read and validate one task block's span slice (the stale-span guard shared by
 *  editing and deleting).
 *  Spans are the Rust parser's UTF-8 byte offsets, so slicing has to go through
 *  TextEncoder/Decoder and never through UTF-16 string indices. null = stale (the note
 *  changed after the query) and the caller requeries. */
async function readTaskSpan(hit: BlockHit): Promise<{ bytes: Uint8Array; slice: string } | null> {
  const { content } = await api.noteRead(hit.note);
  const bytes = new TextEncoder().encode(content);
  const { start, end } = hit.block.span;
  if (end > bytes.length) return null;
  const slice = new TextDecoder().decode(bytes.slice(start, end));
  const m = slice.match(CHECKBOX_RE);
  if (!m) return null;
  // A stale span may have shifted wholesale onto another task line (text of the same
  // length was inserted before it): the shape alone is not enough, and the body has to
  // match the block's content at query time before a rewrite is allowed
  if (slice.slice(m[0].length).trim() !== hit.block.content) return null;
  return { bytes, slice };
}

/**
 * Rewrite exactly a task's block span with `edit(slice)`.
 * Returns false when the span is stale (the note changed since the query);
 * the caller should requery and let the user retry on fresh data.
 */
async function editTaskSpan(hit: BlockHit, edit: (slice: string) => string): Promise<boolean> {
  const r = await readTaskSpan(hit);
  if (!r) return false;
  const { start, end } = hit.block.span;
  const dec = new TextDecoder();
  const updated = dec.decode(r.bytes.slice(0, start)) + edit(r.slice) + dec.decode(r.bytes.slice(end));
  await api.noteWrite(hit.note, updated);
  return true;
}

/** Delete one task block from disk (the whole span: a multi-line task's sub-content
 *  goes with it).
 *  A newline is taken with it — deleting the span alone would leave a blank line: the
 *  \n after the block (a \r\n included) is preferred, and when the block ends the file
 *  with no trailing newline, the one before it is taken instead. false = a stale span,
 *  and the caller requeries. */
export async function deleteTask(hit: BlockHit): Promise<boolean> {
  const r = await readTaskSpan(hit);
  if (!r) return false;
  const { bytes } = r;
  let from = hit.block.span.start;
  let to = hit.block.span.end;
  if (bytes[to] === 0x0d && bytes[to + 1] === 0x0a) to += 2;
  else if (bytes[to] === 0x0a) to += 1;
  else if (from > 0 && bytes[from - 1] === 0x0a) from -= bytes[from - 2] === 0x0d ? 2 : 1;
  const dec = new TextDecoder();
  await api.noteWrite(hit.note, dec.decode(bytes.slice(0, from)) + dec.decode(bytes.slice(to)));
  return true;
}

/** Toggle a task's checkbox on disk. false = stale span, requery. */
export async function toggleTask(hit: BlockHit): Promise<boolean> {
  return editTaskSpan(hit, (slice) => {
    const m = slice.match(CHECKBOX_RE)!;
    const next = m[2] === "x" || m[2] === "X" ? " " : "x";
    return slice.replace(CHECKBOX_RE, `$1[${next}]`);
  });
}

export type DateField = "due" | "scheduled";

/** Everything known about each date field, collected into one table: the two marker
 *  spellings, emoji and attr:: (mirroring Rust's parse_date_attr; emoji comes first =
 *  the parse priority, and with both present emoji wins), the chip's title, and the
 *  lucide-style stroke icon path (due = a calendar, scheduled = an hourglass, a 24
 *  viewBox).
 *  The editor's date chip, Agenda's chip and the reschedule and strip logic all derive
 *  from it — adding a field or changing a marker touches this one place. The marker
 *  tokens are all regex-safe literals. */
export const DATE_FIELDS: Record<DateField, { emoji: string; attr: string; icon: string }> = {
  due: {
    emoji: "📅",
    attr: "due::",
    icon: '<rect x="3" y="4" width="18" height="18" rx="2"/><path d="M16 2v4M8 2v4M3 10h18"/>',
  },
  scheduled: {
    emoji: "⏳",
    attr: "scheduled::",
    icon: '<path d="M5 22h14M5 2h14M17 22v-4.172a2 2 0 0 0-.586-1.414L12 12l-4.414 4.414A2 2 0 0 0 7 18.828V22M7 2v4.172a2 2 0 0 0 .586 1.414L12 12l4.414-4.414A2 2 0 0 0 17 6.172V2"/>',
  },
};

/** The chip icon's complete `<svg>`: the editor widget and the Agenda view use the
 *  same one — sharing only the path would let the stroke width and line caps of the
 *  two quietly drift apart. */
export function dateIconSvg(field: DateField): string {
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${DATE_FIELDS[field].icon}</svg>`;
}

/** The three-state stroke icon of a task checkbox (todo an empty box / doing a
 *  half-filled one / done a box with a tick), on the same 24 viewBox convention as
 *  dateIconSvg — the ☑◐☐ emoji are the system's colour glyphs and clash with the
 *  accent palette in dark mode. Agenda and the kanban share this one. */
export function taskStateIconSvg(state: "todo" | "doing" | "done"): string {
  const box = '<rect x="4" y="4" width="16" height="16" rx="3"/>';
  const inner =
    state === "done"
      ? box + '<path d="m8.2 12.4 2.6 2.6 5-5.8"/>'
      : state === "doing"
        ? box + '<path d="M12 4H7a3 3 0 0 0-3 3v10a3 3 0 0 0 3 3h5Z" fill="currentColor" stroke="none"/>'
        : box;
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${inner}</svg>`;
}

/** The shape of a date token. `\d{1,2}` is deliberately tolerant: chrono's `%m`/`%d`
 *  accept unpadded numbers, so `📅 2026-8-5` is a live date rather than junk. */
const DATE_SRC = String.raw`\d{4}-\d{1,2}-\d{1,2}`;
const DATE_ONLY_RE = new RegExp(`^${DATE_SRC}$`);

const FIELDS = Object.keys(DATE_FIELDS) as DateField[];
const markerTokens = (f: DateField) => [DATE_FIELDS[f].emoji, DATE_FIELDS[f].attr];

/** Per-field markers carrying a date, in parse priority order. Capture group 1 = the
 *  marker prefix, 2 = the date — a replacement has to edit the very token the parser
 *  reads. */
const markerRes = (shape: string): Record<DateField, RegExp[]> =>
  Object.fromEntries(
    FIELDS.map((f) => [f, markerTokens(f).map((t) => new RegExp(`(${t}[ \\t]*)(${shape})`, "u"))]),
  ) as Record<DateField, RegExp[]>;
const MARKER_RES = markerRes(DATE_SRC);
/** The tolerant version: a marker plus the first non-whitespace token after it (which
 *  may be empty) — mirroring how Rust's parse_date_attr "finds the marker and takes
 *  the next token", so it matches however bad the token is. */
const LOOSE_MARKER_RES = markerRes(String.raw`\S*`);

/** The tolerant scanner: any marker plus the first non-whitespace token after it
 *  (valid or not). Capture group 1 = the marker, 2 = the token; marker → field goes
 *  through FIELD_OF_MARKER. */
const DATE_MARKER_SCAN_RE = new RegExp(`(${FIELDS.flatMap(markerTokens).join("|")})[ \\t]*(\\S*)`, "gu");

const FIELD_OF_MARKER: Record<string, DateField> = {};
for (const f of FIELDS) for (const t of markerTokens(f)) FIELD_OF_MARKER[t] = f;

/** One occurrence of a date marker on a task's first line: the offsets are relative to the text scanned. */
export interface TaskDateMark {
  start: number;
  end: number;
  field: DateField;
  /** A zero-padded `YYYY-MM-DD` when the token is valid, otherwise null */
  date: string | null;
  /** This occurrence is the one Rust's parse_date_attr actually reads (that field's
   *  winner): only the **first** occurrence of each marker counts, emoji comes first,
   *  and a failed parse falls through to the attr:: spelling.
   *  A non-winner is ordinary text to the parser even when its token is valid — so a
   *  chip is only drawn for the winner, or the editor would display a date that does
   *  not exist in the index. */
  active: boolean;
  /** The parse failed and it visibly was an attempt at a date (starting with a digit,
   *  or explicit attr:: syntax) — only that draws a bad-date warning; a decorative use
   *  such as `⏳ waiting for a reply` is silent to the parser, and a warning would only
   *  be a false alarm. */
  attempted: boolean;
}

/** Scan a task's **first line** the way Rust's parse_date_attr
 *  (crates/vemry-core/src/parser.rs) reads it: return every marker occurrence and mark
 *  each field's winner. The editor's chips and bad-date warnings all derive from this
 *  one scan — so there can be only this one point of divergence from the parser. */
export function scanTaskDates(text: string): TaskDateMark[] {
  const marks: (TaskDateMark & { marker: string })[] = [];
  DATE_MARKER_SCAN_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = DATE_MARKER_SCAN_RE.exec(text))) {
    const date = parseYmd(m[2]);
    marks.push({
      start: m.index,
      end: m.index + m[0].length,
      field: FIELD_OF_MARKER[m[1]],
      date,
      active: false,
      attempted: date === null && (m[1].endsWith("::") || /^\d/.test(m[2])),
      marker: m[1],
    });
  }
  for (const f of FIELDS) {
    for (const t of markerTokens(f)) {
      const first = marks.find((k) => k.field === f && k.marker === t);
      if (first?.date) {
        first.active = true;
        break; // the parser stops as soon as it has a date and never looks at the fallback marker
      }
    }
  }
  return marks;
}

/** For stripping display text: every marker of every field (carrying a date) caught in one net. */
const MARKER_STRIP_RE = new RegExp(`(?:${FIELDS.flatMap(markerTokens).join("|")})[ \\t]*${DATE_SRC}`, "gu");

/** The reading-side regex of a memo or clip signature `*(YYYY-MM-DD HH:MM)*` — the
 *  format's only producer is vault.capture (vault.svelte.ts), and changing it there
 *  means changing this too. Agenda's stripping and the editor's fading decoration
 *  share this one. */
export const STAMP_RE = /\*\(\d{4}-\d{2}-\d{2} \d{2}:\d{2}\)\*/g;

/** Date parsing as tolerant as chrono's `%Y-%m-%d`: the month and day may be unpadded,
 *  but it has to be a real calendar date (chrono rejects 2026-02-30, so this has to as
 *  well, or the editor's warning would fall out of step with the parser). Returns a
 *  zero-padded `YYYY-MM-DD` when valid (the chip's display and every string comparison
 *  need it — comparing `2026-8-5` directly sorts wrongly), otherwise null. */
export function parseYmd(token: string): string | null {
  if (!DATE_ONLY_RE.test(token)) return null;
  const [y, mo, d] = token.split("-").map(Number);
  const dt = new Date(y, mo - 1, d);
  return dt.getFullYear() === y && dt.getMonth() === mo - 1 && dt.getDate() === d ? ymd(dt) : null;
}

/**
 * Set / replace / remove (`date === null`) a task line's date marker.
 * Keeps the marker style the user wrote (📅 vs due::); a fresh marker is
 * inserted emoji-style before any trailing `^anchor` (appending after it
 * would silently kill the block anchor).
 */
export function rescheduleLine(line: string, field: DateField, date: string | null): string {
  if (date === null) {
    // The two spellings (📅 and due::) may both be present: the parser falls back from
    // emoji to attr::, so removing only one would not make the date disappear but
    // quietly move it onto the other marker — every one of them is removed
    let out = line;
    for (const re of MARKER_RES[field]) {
      let m: RegExpExecArray | null;
      while ((m = re.exec(out))) {
        let before = out.slice(0, m.index);
        const after = out.slice(m.index + m[0].length);
        // Do not leave a double space behind once the token is removed
        if (/\s$/.test(before) && (after === "" || /^\s/.test(after))) before = before.replace(/\s+$/, "");
        out = before + after;
      }
    }
    return out;
  }
  // The rewrite has to land on the token the parser reads (each marker's first
  // occurrence, emoji first — the same winner rule as scanTaskDates). A token shaped
  // like a date is replaced in place; a bad token (📅 next Friday) has the new date
  // inserted after the marker and the bad token demoted to ordinary text — appending a
  // second marker would be ignored forever by the parser's "first marker" reading, and
  // rescheduling would silently do nothing
  for (const re of LOOSE_MARKER_RES[field]) {
    const m = re.exec(line);
    if (!m) continue;
    const tokenAt = m.index + m[1].length;
    const token = m[2];
    if (DATE_ONLY_RE.test(token)) return line.slice(0, tokenAt) + date + line.slice(tokenAt + token.length);
    const sep = /[ \t]$/.test(m[1]) ? "" : " ";
    const trail = token !== "" ? " " : "";
    return line.slice(0, tokenAt) + sep + date + trail + line.slice(tokenAt);
  }
  const marker = DATE_FIELDS[field].emoji;
  const anchor = /[ \t]\^[\w-]+[ \t]*$/.exec(line);
  if (anchor)
    return line.slice(0, anchor.index).replace(/[ \t]+$/, "") + ` ${marker} ${date}` + line.slice(anchor.index);
  return line.replace(/[ \t]+$/, "") + ` ${marker} ${date}`;
}

/** Reschedule a task on disk (null clears the date). false = stale span, requery. */
export async function rescheduleTask(hit: BlockHit, field: DateField, date: string | null): Promise<boolean> {
  return editTaskSpan(hit, (slice) => {
    // By convention a date marker is written on the task's first line; a multi-line block only has its first line touched and its sub-content is kept verbatim
    const nl = slice.indexOf("\n");
    const head = nl < 0 ? slice : slice.slice(0, nl);
    return rescheduleLine(head, field, date) + (nl < 0 ? "" : slice.slice(nl));
  });
}

export interface TaskEdit {
  state?: "todo" | "doing" | "done";
  /** Set (or with null remove) one inline field on the task's first line — a kanban grouped by a field moves cards this way */
  field?: { key: string; value: string | null };
  /** Set or change one date field (rescheduleLine semantics: the marker style is kept and it is inserted before the ^anchor) */
  date?: { field: DateField; value: string };
  /** Clear both due and scheduled (dragging to "unscheduled" on the kanban — clearing only one would land it in the other field's group) */
  clearDates?: boolean;
}

/** The merged write-back of a kanban drag: the state and the date are done in **one**
 *  span rewrite — with two editTaskSpan calls the second span would necessarily be
 *  stale (the first write already invalidated the offsets).
 *  false = a stale span, and the caller requeries. */
export async function updateTask(hit: BlockHit, edit: TaskEdit): Promise<boolean> {
  return editTaskSpan(hit, (slice) => {
    let out = slice;
    if (edit.state) {
      const ch = edit.state === "done" ? "x" : edit.state === "doing" ? "/" : " ";
      out = out.replace(CHECKBOX_RE, `$1[${ch}]`);
    }
    const nl = out.indexOf("\n");
    let head = nl < 0 ? out : out.slice(0, nl);
    if (edit.clearDates) for (const f of FIELDS) head = rescheduleLine(head, f, null);
    if (edit.date) head = rescheduleLine(head, edit.date.field, edit.date.value);
    if (edit.field) head = setFieldValue(head, edit.field.key, edit.field.value);
    return head + (nl < 0 ? "" : out.slice(nl));
  });
}


/** Task text for display: checkbox prefix, date markers, capture stamps and
 *  markdown ornaments stripped — Agenda is a projection, and cramming the source's
 *  markers into a single line would only be noise.
 *  Folding emphasis, links, wikilinks and bare URLs is collected in plainText (shared
 *  with the claim and evidence cards), so "the plain-text form" has one rule only. */
export function taskText(content: string): string {
  return plainText(
    stripAnchor(
      // a block id minted on the line is a badge in the editor, not a word
      stripFields(content) // `[key:: value]` metadata is a chip on the card, not part of the words
        .replace(/^\s*[-*+]\s+\[[ /xX]\]\s*/, "")
        .replace(MARKER_STRIP_RE, "") // the date markers
        .replace(STAMP_RE, ""), // the memo or clip signature
    ),
  );
}

/** A task block's view row: the kind unpacked, and the display text stripped once (not
 *  on every render inside the template).
 *  Agenda's rows and the kanban's cards come from the same place — the two views must
 *  read the same state and dates for the same task, so there can be only this one
 *  point of divergence. */
export interface TaskRow {
  hit: BlockHit;
  text: string;
  state: "todo" | "doing" | "done";
  due?: string;
  scheduled?: string;
  /** When the task's current text appeared (ms; the block's modified_ms, 0 when unknown). */
  modified: number;
}

export function rowOf(hit: BlockHit): TaskRow {
  const k = hit.block.kind;
  const text = taskText(hit.block.content);
  const modified = hit.block.modified_ms ?? 0;
  if (k.type !== "task") return { hit, text, state: "todo", modified };
  return { hit, text, state: k.state, due: k.due, scheduled: k.scheduled, modified };
}

/** The filter key of Agenda and the kanban — one definition, so the two views give the
 *  same answer to the same keystroke.
 *  The body is the display text rowOf already stripped (markers from the source would
 *  only match spuriously), plus the raw due and scheduled dates: both views bucket
 *  dates coarsely — everything past 7 days becomes "later" and every future date
 *  becomes "scheduled" — so "which ones are due in September" has nowhere to land on
 *  screen, and the filter box is the only place it can be expressed. fuzzyKeep is
 *  binary and order-preserving, so a longer key does not disturb the order. */
export const taskFilterKey = (r: TaskRow) =>
  `${r.text} ${r.hit.note_title} ${r.hit.note} ${r.due ?? ""} ${r.scheduled ?? ""}`;

/** A task's identity across requeries: the note plus the block's byte span start,
 *  physically unique within a note (an explicit ^id may be copied into duplicates and
 *  cannot be a key). Both views key their lists by it, so the DOM of an unchanged row
 *  survives a requery, and the board finds a card again in whatever hits are current. */
export const taskKey = (r: TaskRow) => r.hit.note + ":" + r.hit.block.span.start;

/** The date a task groups by: due first, and without a due the ⏳scheduled date counts
 *  too — otherwise a task "⏳ starting tomorrow" would carry a date chip while lying
 *  under "no date". Agenda's grouping and the kanban's column assignment read this one
 *  rule. Asked for one `field`, it reads that field alone: Agenda with a date kind pressed
 *  places every task by the date the user asked to see, not by the due-first default. */
export const whenOf = (r: TaskRow, field?: DateField) => (field ? r[field] : r.due ?? r.scheduled);

/** Date ascending (whenOf's date, or the one `field` when given), and within a day the
 *  source order (the note's path, then the block's position in it) — so the order is
 *  stable after a requery whatever order the hits arrived in. */
export const byDate = (field?: DateField) => (a: TaskRow, b: TaskRow) =>
  (whenOf(a, field) ?? "9999").localeCompare(whenOf(b, field) ?? "9999") || bySource(a, b);

/** The last-touched order, for the tasks that have no date to read (the unscheduled and
 *  the done groups): the most recently changed task first, then the source order. The
 *  time is the block's own (`modified`: when its current text appeared — a toggled
 *  checkbox, an edited word, a changed date all count; a minted `^id` does not), so
 *  checking one task moves that task alone. Agenda's no-date and done groups and the
 *  board's unscheduled and done columns read this one rule. */
export const byModified = (a: TaskRow, b: TaskRow) => b.modified - a.modified || bySource(a, b);

/** The source order: the note's path, then the block's byte position in it (queryrows'
 *  byNote, read off the row's hit). The tie-break of every task ordering — two cards that
 *  compare equal on any key keep the order they have in the notes. */
export const bySource = (a: TaskRow, b: TaskRow) => byNote(a.hit, b.hit);

/** Local-timezone `YYYY-MM-DD` for a Date — the one day-key both Agenda and
 *  Timeline group by (two copies would let a UTC-parse fix drift them apart). */
export function ymd(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** Local date as `YYYY-MM-DD`, offset by whole days. */
export function localDate(offsetDays = 0): string {
  const d = new Date();
  d.setDate(d.getDate() + offsetDays);
  return ymd(d);
}

/** Whole days from `YYYY-MM-DD` to today (in the local time zone, by calendar day):
 *  negative = in the past.
 *  Both ends are zeroed to local midnight before subtracting, and `Math.round` absorbs
 *  the 23/25-hour day of a daylight-saving change; parsing has to go through
 *  `new Date(y, m-1, d)` — `new Date("YYYY-MM-DD")` is read as UTC and would be a whole
 *  day out east of Greenwich (the same trap is already noted in the timeline's
 *  parseCreated). */
function daysUntil(date: string): number {
  const [y, mo, d] = date.split("-").map(Number);
  const then = new Date(y, mo - 1, d);
  const now = new Date();
  return Math.round((then.getTime() - new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()) / 86400_000);
}

/** The date string shown for an overdue task: the amount of lateness is appended. What
 *  matters most at a glance for an overdue task is "how late", which an absolute date
 *  does not show (the reader would have to count from today). Only overdue gets the
 *  relative amount — giving a future date "in 3 days" as well would mix two units in
 *  one column and make it harder to scan.
 *  The gate is the task's own lateness — an unfinished task whose due date has passed —
 *  rather than "the date is in the past": a finished task is not late however long ago
 *  its due date was, and it belongs to "done". On the board the column's overdue is the
 *  same test wherever a due chip is painted; Agenda spells it out, since with a date kind
 *  pressed its groups follow that date and not the due one.
 *
 *  Two densities, because the two views' horizontal budgets differ by an order of
 *  magnitude — but the wording is the same sentence, and "N days late" reads as the
 *  same thing everywhere:
 *   - full: Agenda's 13em fixed-width date track, where the absolute date and the
 *     lateness both fit
 *   - short: a kanban card's meta line has only about 110px, where the full form
 *     overflows the card (a measured screenshot had it cut to "2026-08-08 · ov"), and
 *     the absolute date falls back to the title */
export function dueLabel(due: string, overdue: boolean, density: "full" | "short" = "full"): string {
  // The gate is checked before the days are computed: the non-overdue rows (all of
  // "today / later / done") are the majority, and they should not pay for three Date
  // constructions to produce a value that is dropped straight away
  if (!overdue) return due;
  const late = -daysUntil(due);
  if (late <= 0) return due;
  const phrase = tn("task.overdue", late);
  return density === "short" ? phrase : `${due} · ${phrase}`;
}

/** A date field's display name ("Due date" / "Start date"). Shared by the menu items and the chip's tooltip. */
export function dateFieldLabel(field: DateField): string {
  return t(`task.field.${field}`);
}
