// The task line as the views and the editor read and rewrite it: every attribute after
// the text (the mirror of vemry-core's taskattrs.rs), the rewrites of a state change
// and a reschedule (the mirror of taskedit.rs and repeat.rs), and the rows, orders and
// menus Agenda and the board share. The Rust side is the agent's copy of the same rules;
// change the two together.
import { t, tn } from "$lib/i18n/i18n.svelte";
import { scanFields, setFieldValue, stripAnchor, stripFields } from "$lib/fields";
import { api, type BlockHit, type Priority, type TaskState } from "$lib/api";
import { byNote } from "$lib/views/queryrows";
import { readBlockSpan } from "$lib/views/blockspan";
import { plainText } from "$lib/markdown";
import { stateGlyphSvg, TASK_STEPS, taskStep } from "$lib/chips";
import type { MenuEntry } from "$lib/stores/contextMenu.svelte";

export type { Priority, TaskState };

/** A task line as the field layer sees it: a bullet or ordered marker and a checkbox.
 *  Wider than CHECKBOX_RE, which does not take ordered items. The renderer and the
 *  chip's writer read this one rule — otherwise a chip could be drawn through one and
 *  written through the other. */
export const TASK_LINE_RE = /^\s*(?:[-*+]|\d{1,9}[.)])\s+\[[ xX/-]\]/;

/** Checkbox prefix at the start of a task block. The state char is `(.)`
 *  because the Rust parser accepts any single char (unknowns count as todo) —
 *  a `[ /xX-]`-only regex leaves `- [?] x` style tasks with a dead checkbox. */
const CHECKBOX_RE = /^(\s*[-*+]\s+)\[(.)\](?=\s|$)/;
/** The checkbox of a task's first line, an ordered item's included (Rust's set_checkbox). */
const ANY_CHECKBOX_RE = /^(\s*(?:[-*+]|\d{1,9}[.)])\s+)\[([ xX/-])\](?=\s|$)/;

/** Done or cancelled: closed, and out of every list of open work. */
export const isClosed = (state: TaskState) => state === "done" || state === "cancelled";

/** The checkbox character of a state. */
export function stateChar(state: TaskState): string {
  return state === "done" ? "x" : state === "doing" ? "/" : state === "cancelled" ? "-" : " ";
}
/** The state a checkbox character means (Rust's parse_task_line: an unknown one is to-do). */
export function stateOfChar(ch: string): TaskState {
  return ch === "x" || ch === "X" ? "done" : ch === "/" ? "doing" : ch === "-" ? "cancelled" : "todo";
}

/* ---------------------------------------------------------------- the attribute table */

export type DateField = "due" | "scheduled" | "start" | "created" | "done" | "cancelled";
export type AttrKind = DateField | "priority" | "repeat" | "delete" | "id" | "depends" | "estimate";

/** The date attributes: the emoji spellings in read order (the first is the written
 *  one, the rest read for vaults written by other tools), the `key::` spelling, and the
 *  chip's stroke icon (a 24 viewBox, the convention of chips.ts). */
export const DATE_FIELDS: Record<DateField, { emoji: string[]; attr: string; icon: string }> = {
  due: {
    emoji: ["📅"],
    attr: "due::",
    // a flag: the deadline
    icon: '<path d="M6 3v18"/><path d="M6 4h10.5l-2.5 3.5 2.5 3.5H6z" fill="currentColor" stroke="none"/>',
  },
  scheduled: {
    emoji: ["⏳", "⌛"],
    attr: "scheduled::",
    // a calendar with the day pinned
    icon: '<rect x="3" y="5" width="18" height="16" rx="2"/><path d="M3 10h18M8 3v4M16 3v4"/><circle cx="12" cy="15.5" r="2" fill="currentColor" stroke="none"/>',
  },
  start: {
    emoji: ["⏩", "🛫"],
    attr: "start::",
    // play from the bar
    icon: '<path d="M5 5v14"/><path d="M9 6v12l9-6z" fill="currentColor" stroke="none"/>',
  },
  created: {
    emoji: ["➕"],
    attr: "created::",
    icon: '<circle cx="12" cy="12" r="8.5"/><path d="M12 8v8M8 12h8"/>',
  },
  done: {
    emoji: ["✅"],
    attr: "completion::",
    icon: '<circle cx="12" cy="12" r="9" fill="currentColor" stroke="none"/><path d="m8 12.5 2.7 2.7 5.3-6" stroke="var(--bg)"/>',
  },
  cancelled: {
    emoji: ["❌"],
    attr: "cancelled::",
    icon: '<circle cx="12" cy="12" r="8.5"/><path d="m9 9 6 6M15 9l-6 6"/>',
  },
};

/** The date fields a menu offers and the legend lists: the ones a person sets. The
 *  done and cancelled dates are stamps a state change leaves; created is written on
 *  creation. */
export type EditField = "due" | "scheduled" | "start";
export const EDIT_FIELDS: EditField[] = ["due", "scheduled", "start"];

/** The icons of the other attributes (priority by its value). */
const ATTR_ICONS: Record<string, string> = {
  "priority-highest": '<path d="M8.5 5v8.5M15.5 5v8.5" stroke-width="3"/><circle cx="8.5" cy="18.5" r="1.6" fill="currentColor" stroke="none"/><circle cx="15.5" cy="18.5" r="1.6" fill="currentColor" stroke="none"/>',
  "priority-high": '<path d="M12 5v8.5" stroke-width="3"/><circle cx="12" cy="18.5" r="1.6" fill="currentColor" stroke="none"/>',
  "priority-low": '<path d="m7 10 5 5 5-5"/>',
  "priority-lowest": '<path d="m7 7 5 5 5-5M7 13l5 5 5-5"/>',
  repeat: '<path d="M5 11v-1a3 3 0 0 1 3-3h10"/><path d="m15 4 3 3-3 3"/><path d="M19 13v1a3 3 0 0 1-3 3H6"/><path d="m9 20-3-3 3-3"/>',
  delete: '<path d="M5 7h14M9.5 7V4.5h5V7"/><path d="m7 7 1 13h8l1-13"/><path d="M10.5 11v5M13.5 11v5"/>',
  id: '<circle cx="8" cy="15.5" r="3.75"/><path d="M10.7 12.8 19.5 4M16 7.5l2 2M13.5 10l2 2"/>',
  depends: '<path d="M8 11V8a4 4 0 0 1 8 0v3"/><rect x="5" y="11" width="14" height="9" rx="2" fill="currentColor" stroke="none"/><circle cx="12" cy="15.5" r="1.3" fill="var(--bg)" stroke="none"/>',
  estimate: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5v5l3.25 1.75"/>',
};

const svgOf = (inner: string) =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${inner}</svg>`;

/** The chip icon's complete `<svg>`: the editor widget and the Agenda view use the
 *  same one — sharing only the path would let the stroke width and line caps of the
 *  two quietly drift apart. */
export function dateIconSvg(field: DateField): string {
  return svgOf(DATE_FIELDS[field].icon);
}

/** The icon of any attribute; a priority's follows its value (normal has none). */
export function attrIconSvg(kind: AttrKind, priority?: Priority): string {
  if (kind in DATE_FIELDS) return dateIconSvg(kind as DateField);
  const key = kind === "priority" ? `priority-${priority ?? "high"}` : kind;
  return svgOf(ATTR_ICONS[key] ?? ATTR_ICONS.repeat);
}

/** The stroke icon of a task checkbox (todo an empty box / doing a half-filled one /
 *  done a box with a tick / cancelled a box with a cross): the shared state glyph
 *  (chips.ts) at the task's three steps, so a task and a state field's value read the
 *  same shape. Agenda, the kanban and a query row share this one. */
export function taskStateIconSvg(state: TaskState): string {
  return stateGlyphSvg(taskStep(state), TASK_STEPS, state === "cancelled");
}

/** The `key::` names that are task attributes (lowercase): no field chip is drawn for
 *  them on a task line, they are attribute chips. */
export const TASK_ATTR_KEYS = new Set([
  "due", "scheduled", "start", "created", "completion", "cancelled", "priority", "repeat", "oncompletion", "id", "dependson", "estimate",
]);

/** Every emoji that opens an attribute: where a repeat rule's text stops. */
const EMOJI_MARKERS = ["📅", "⏳", "⌛", "⏩", "🛫", "➕", "✅", "❌", "❗", "❕", "🔺", "⏫", "🔼", "🔽", "⏬", "🔁", "🚮", "🏁", "🆔", "⛔", "🕐"];

/** Minutes in an estimate's `d`: a working day (taskattrs.rs DAY_MINUTES). */
const DAY_MINUTES = 8 * 60;

/** Without U+FE0F: an emoji typed with the variation selector reads as the bare one. */
const stripVs = (s: string) => s.replace(/\uFE0F/g, "");

/** The shape of a date token. `\d{1,2}` is deliberately tolerant: chrono's `%m`/`%d`
 *  accept unpadded numbers, so `📅 2026-8-5` is a live date rather than junk. */
const DATE_SRC = String.raw`\d{4}-\d{1,2}-\d{1,2}`;
const DATE_ONLY_RE = new RegExp(`^${DATE_SRC}$`);

const FIELDS = Object.keys(DATE_FIELDS) as DateField[];
const markerTokens = (f: DateField) => [...DATE_FIELDS[f].emoji, DATE_FIELDS[f].attr];
/** A marker with an optional variation selector and the space after it, then its token
 *  (up to whitespace or a closing bracket — a `[key:: value]` closes at its bracket). */
const markerRe = (marker: string) => new RegExp(`(${marker}\\uFE0F?[ \\t]*)([^\\s\\])]*)`, "gu");

/** A `priority::` value or a priority token (Rust's priority_word / priority_token). */
function priorityWord(s: string): Priority | null {
  switch (s.trim().toLowerCase()) {
    case "highest":
    case "urgent":
      return "highest";
    case "high":
    case "medium":
      return "high";
    case "normal":
    case "none":
      return "normal";
    case "low":
      return "low";
    case "lowest":
      return "lowest";
    default:
      return null;
  }
}
function priorityToken(tok: string): Priority | null {
  if (/^❗+$/u.test(tok)) return tok.length > 1 ? "highest" : "high";
  if (/^❕+$/u.test(tok)) return tok.length > 1 ? "lowest" : "low";
  switch (tok) {
    case "🔺":
      return "highest";
    case "⏫":
    case "🔼":
      return "high";
    case "🔽":
      return "low";
    case "⏬":
      return "lowest";
    default:
      return null;
  }
}

/** `30m`, `2h`, `1.5h`, `2h30m`, `1d`, `1d2h` → minutes; null when not a duration. */
export function parseEstimate(s: string): number | null {
  const m = s.trim().match(/^(?:(\d+(?:\.\d+)?)d)?(?:(\d+(?:\.\d+)?)h)?(?:(\d+(?:\.\d+)?)m)?$/);
  if (!m || (m[1] === undefined && m[2] === undefined && m[3] === undefined)) return null;
  const total = Number(m[1] ?? 0) * DAY_MINUTES + Number(m[2] ?? 0) * 60 + Number(m[3] ?? 0);
  return total > 0 ? Math.round(total) : null;
}

/** Minutes back to the shortest spelling: `45m`, `2h`, `2h30m`, `1d`, `1d2h`. */
export function formatEstimate(minutes: number): string {
  const days = Math.floor(minutes / DAY_MINUTES);
  const hours = Math.floor((minutes % DAY_MINUTES) / 60);
  const mins = minutes % 60;
  let out = "";
  if (days > 0) out += `${days}d`;
  if (hours > 0) out += `${hours}h`;
  if (mins > 0 || out === "") out += `${mins}m`;
  return out;
}

const splitIds = (s: string) => s.split(/[,\s]+/).map((x) => x.trim()).filter(Boolean);

/** A token that ends a repeat rule's text: the next attribute, a field, a tag or an anchor. */
const isStop = (tok: string) => {
  const bare = stripVs(tok);
  return EMOJI_MARKERS.some((m) => bare.startsWith(m)) || /^[[(^#]/.test(tok) || tok.includes("::");
};

/** One attribute occurrence on a task's first line: offsets relative to the text scanned. */
export interface TaskAttrMark {
  start: number;
  end: number;
  kind: AttrKind;
  /** The value as read: a normalised priority, a repeat rule, an id, an id list, an
   *  estimate as written, "" for a flag. */
  value: string;
  /** A date kind: a zero-padded `YYYY-MM-DD` when the token is valid, otherwise null. */
  date: string | null;
  /** This occurrence is the one the parser actually reads (a field's winner): only the
   *  **first** occurrence of each marker counts, the emoji spellings come first, and a
   *  failed date falls through to the next spelling. A non-winner is ordinary text to the
   *  parser even when its token is valid — so a chip is only drawn for the winner, or the
   *  editor would display something that does not exist in the index. */
  active: boolean;
  /** The parse failed and it visibly was an attempt at a date (starting with a digit,
   *  or explicit attr:: syntax) — only that draws a bad-date warning; a decorative use
   *  such as `⏳ waiting for a reply` is silent to the parser, and a warning would only
   *  be a false alarm. */
  attempted: boolean;
}

/** Scan a task's **first line** the way Rust's parse_task_attrs reads it: every
 *  attribute occurrence, each field's winner marked. The editor's chips and warnings, the
 *  display text and the rewrites all derive from this one scan — so there can be only
 *  this one point of divergence from the parser. */
export function scanTaskAttrs(text: string): TaskAttrMark[] {
  const marks: TaskAttrMark[] = [];
  // The dates: every occurrence of every spelling, the winner per field
  for (const f of FIELDS) {
    let won = false;
    for (const marker of markerTokens(f)) {
      const re = markerRe(marker);
      let first = true;
      let m: RegExpExecArray | null;
      while ((m = re.exec(text))) {
        let start = m.index;
        let end = m.index + m[0].length;
        // a `[key:: value]` is one construct with its brackets
        if (marker.endsWith("::")) {
          const open = text[start - 1];
          const close = text[end];
          if ((open === "[" && close === "]") || (open === "(" && close === ")")) {
            start -= 1;
            end += 1;
          }
        }
        const date = parseYmd(m[2]);
        const active = !won && first && date !== null;
        if (active) won = true;
        marks.push({ start, end, kind: f, value: m[2], date, active, attempted: date === null && (marker.endsWith("::") || /^\d/.test(m[2])) });
        first = false;
        if (m[0].length === 0) re.lastIndex++;
      }
      if (won) break;
    }
  }
  // The emoji spellings of the rest, token by token (Rust: the same walk)
  const tokens: { tok: string; at: number }[] = [];
  const TOKEN_RE = /\S+/gu;
  let tm: RegExpExecArray | null;
  while ((tm = TOKEN_RE.exec(text))) tokens.push({ tok: tm[0], at: tm.index });
  const have = { priority: false, repeat: false, delete: false, id: false, depends: false, estimate: false };
  const push = (kind: AttrKind, from: number, to: number, value: string) => marks.push({ start: from, end: to, kind, value, date: null, active: true, attempted: false });
  /** The value of a marker: the rest of its token, or the next token when it is not a stop. */
  const valueOf = (i: number, marker: string): { value: string; next: number; end: number } | null => {
    const bare = stripVs(tokens[i].tok);
    if (!bare.startsWith(marker)) return null;
    const rem = bare.slice(marker.length);
    const tokEnd = tokens[i].at + tokens[i].tok.length;
    if (rem) return { value: rem, next: i + 1, end: tokEnd };
    const n = tokens[i + 1];
    if (n && !isStop(n.tok)) return { value: n.tok, next: i + 2, end: n.at + n.tok.length };
    return { value: "", next: i + 1, end: tokEnd };
  };
  for (let i = 0; i < tokens.length; ) {
    const { tok, at } = tokens[i];
    const bare = stripVs(tok);
    const tokEnd = at + tok.length;
    if (!have.priority) {
      const p = priorityToken(bare);
      if (p) {
        have.priority = true;
        push("priority", at, tokEnd, p);
        i++;
        continue;
      }
    }
    if (!have.repeat && bare.startsWith("🔁")) {
      const rem = bare.slice("🔁".length);
      const parts: string[] = rem ? [rem] : [];
      let j = i + 1;
      let end = tokEnd;
      while (j < tokens.length && !isStop(tokens[j].tok)) {
        parts.push(tokens[j].tok);
        end = tokens[j].at + tokens[j].tok.length;
        j++;
      }
      if (parts.length) {
        have.repeat = true;
        push("repeat", at, end, parts.join(" "));
      }
      i = j;
      continue;
    }
    if (bare === "🚮") {
      have.delete = true;
      push("delete", at, tokEnd, "");
      i++;
      continue;
    }
    if (bare.startsWith("🏁")) {
      const v = valueOf(i, "🏁")!;
      const del = v.value.toLowerCase() !== "keep";
      if (del) have.delete = true;
      marks.push({ start: at, end: v.end, kind: "delete", value: v.value, date: null, active: del, attempted: false });
      i = Math.max(v.next, i + 1);
      continue;
    }
    if (!have.id) {
      const v = valueOf(i, "🆔");
      if (v) {
        if (v.value) {
          have.id = true;
          push("id", at, v.end, v.value);
        }
        i = v.next;
        continue;
      }
    }
    if (!have.depends) {
      const v = valueOf(i, "⛔");
      if (v) {
        let ids = splitIds(v.value);
        let last = v.value;
        let next = v.next;
        let end = v.end;
        // `⛔ a, b`: a trailing comma carries on to the next token
        while (last.endsWith(",") && next < tokens.length && !isStop(tokens[next].tok)) {
          last = tokens[next].tok;
          ids = ids.concat(splitIds(last));
          end = tokens[next].at + last.length;
          next++;
        }
        if (ids.length) {
          have.depends = true;
          push("depends", at, end, ids.join(","));
        }
        i = next;
        continue;
      }
    }
    if (!have.estimate) {
      const v = valueOf(i, "🕐");
      if (v) {
        const min = parseEstimate(v.value);
        if (min !== null) have.estimate = true;
        marks.push({ start: at, end: v.end, kind: "estimate", value: v.value, date: null, active: min !== null, attempted: false });
        i = v.next;
        continue;
      }
    }
    i++;
  }
  // The `key::` spellings fill what no emoji set (the dates were read both ways above)
  for (const f of scanFields(text, 0)) {
    const key = f.key.toLowerCase();
    const value = f.value.trim();
    const mark = (kind: AttrKind, active: boolean, v = value) => marks.push({ start: f.from, end: f.to, kind, value: v, date: null, active, attempted: false });
    switch (key) {
      case "priority": {
        const p = priorityWord(value);
        mark("priority", !have.priority && p !== null && p !== "normal", p ?? value);
        if (p && !have.priority) have.priority = true;
        break;
      }
      case "repeat":
        mark("repeat", !have.repeat && value !== "");
        if (value) have.repeat = true;
        break;
      case "oncompletion":
        mark("delete", !have.delete && value.toLowerCase() === "delete");
        break;
      case "id":
        mark("id", !have.id && value !== "");
        if (value) have.id = true;
        break;
      case "dependson":
        mark("depends", !have.depends && value !== "", splitIds(value).join(","));
        if (value) have.depends = true;
        break;
      case "estimate": {
        const min = parseEstimate(value);
        mark("estimate", !have.estimate && min !== null);
        if (min !== null) have.estimate = true;
        break;
      }
    }
  }
  return marks.sort((a, b) => a.start - b.start);
}

/** What a task's first line says, read off its marks (the winners only). */
export interface LineAttrs {
  due?: string;
  scheduled?: string;
  start?: string;
  created?: string;
  done?: string;
  cancelled?: string;
  priority: Priority;
  repeat?: string;
  deleteOnDone: boolean;
  taskId?: string;
  dependsOn: string[];
  estimate?: number;
}

export function lineAttrs(line: string): LineAttrs {
  const a: LineAttrs = { priority: "normal", deleteOnDone: false, dependsOn: [] };
  for (const m of scanTaskAttrs(line)) {
    if (!m.active) continue;
    if (m.kind in DATE_FIELDS) a[m.kind as DateField] = m.date!;
    else if (m.kind === "priority") a.priority = m.value as Priority;
    else if (m.kind === "repeat") a.repeat = m.value;
    else if (m.kind === "delete") a.deleteOnDone = true;
    else if (m.kind === "id") a.taskId = m.value;
    else if (m.kind === "depends") a.dependsOn = m.value.split(",");
    else if (m.kind === "estimate") a.estimate = parseEstimate(m.value) ?? undefined;
  }
  return a;
}

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

/* ---------------------------------------------------------------- the rewrites */

/** Remove `[from, to)` from a line without leaving a double space behind. */
function cut(line: string, from: number, to: number): string {
  let before = line.slice(0, from);
  const after = line.slice(to);
  if (/[ \t]$/.test(before) && (after === "" || /^[ \t]/.test(after))) before = before.replace(/[ \t]+$/, "");
  return before + after;
}

/**
 * Set / replace / remove (`date === null`) a task line's date marker.
 * Keeps the marker style the user wrote (📅 vs due::, a bracketed field with its
 * brackets); a fresh marker is inserted emoji-style before any trailing `^anchor`
 * (appending after it would silently kill the block anchor). The mirror of Rust's
 * reschedule_line.
 */
export function rescheduleLine(line: string, field: DateField, date: string | null): string {
  if (date === null) {
    // The spellings may all be present (the parser falls back from emoji to attr::), so
    // removing only one would not make the date disappear but quietly move it onto the
    // other marker — every one carrying a date is removed
    let out = line;
    for (const marker of markerTokens(field)) {
      const re = markerRe(marker);
      let m: RegExpExecArray | null;
      while ((m = re.exec(out))) {
        if (!DATE_ONLY_RE.test(m[2])) continue;
        let from = m.index;
        let to = m.index + m[0].length;
        if (marker.endsWith("::")) {
          const open = out[from - 1];
          const close = out[to];
          if ((open === "[" && close === "]") || (open === "(" && close === ")")) {
            from -= 1;
            to += 1;
          }
        }
        out = cut(out, from, to);
        re.lastIndex = Math.min(from, out.length);
      }
    }
    return out;
  }
  // The rewrite has to land on the token the parser reads (each marker's first
  // occurrence, emoji first — the same winner rule as scanTaskAttrs). A token shaped
  // like a date is replaced in place; a bad token (📅 next Friday) has the new date
  // inserted after the marker and the bad token demoted to ordinary text — appending a
  // second marker would be ignored forever by the parser's "first marker" reading, and
  // rescheduling would silently do nothing
  for (const marker of markerTokens(field)) {
    const re = markerRe(marker);
    re.lastIndex = 0;
    const m = re.exec(line);
    if (!m) continue;
    const tokenAt = m.index + m[1].length;
    const token = m[2];
    if (DATE_ONLY_RE.test(token)) return line.slice(0, tokenAt) + date + line.slice(tokenAt + token.length);
    const sep = /[ \t]$/.test(m[1]) ? "" : " ";
    const trail = token !== "" ? " " : "";
    return line.slice(0, tokenAt) + sep + date + trail + line.slice(tokenAt);
  }
  const marker = DATE_FIELDS[field].emoji[0];
  const anchor = /[ \t]\^[\w-]+[ \t]*$/.exec(line);
  if (anchor)
    return line.slice(0, anchor.index).replace(/[ \t]+$/, "") + ` ${marker} ${date}` + line.slice(anchor.index);
  return line.replace(/[ \t]+$/, "") + ` ${marker} ${date}`;
}

/** The checkbox of a task's first line set to a state; a line with no checkbox is unchanged. */
export function setCheckbox(line: string, state: TaskState): string {
  return line.replace(ANY_CHECKBOX_RE, `$1[${stateChar(state)}]`);
}

/** A state change with the stamp it leaves: done writes ✅ today (and drops any ❌),
 *  cancelled writes ❌ today (and drops any ✅), reopening drops both. The mirror of
 *  Rust's set_state. */
export function setStateLine(line: string, state: TaskState, today: string): string {
  let out = setCheckbox(line, state);
  if (state === "done") {
    out = rescheduleLine(out, "cancelled", null);
    out = rescheduleLine(out, "done", today);
  } else if (state === "cancelled") {
    out = rescheduleLine(out, "done", null);
    out = rescheduleLine(out, "cancelled", today);
  } else {
    out = rescheduleLine(out, "done", null);
    out = rescheduleLine(out, "cancelled", null);
  }
  return out;
}

/** A repeat rule: `every [N] day|week|month|year [when done]` (repeat.rs). */
interface Repeat {
  every: number;
  unit: "day" | "week" | "month" | "year";
  whenDone: boolean;
}
export function parseRepeat(rule: string): Repeat | null {
  const words = rule.trim().toLowerCase().split(/\s+/);
  const whenDone = words.length >= 2 && words[words.length - 2] === "when" && words[words.length - 1] === "done";
  if (whenDone) words.splice(words.length - 2, 2);
  if (words[0] !== "every") return null;
  let every = 1;
  let unitWord: string | undefined;
  if (words.length === 2) unitWord = words[1];
  else if (words.length === 3) {
    every = Number(words[1]);
    if (!Number.isInteger(every) || every <= 0) return null;
    unitWord = words[2];
  } else return null;
  const unit = unitWord.replace(/s$/, "");
  if (unit !== "day" && unit !== "week" && unit !== "month" && unit !== "year") return null;
  return { every, unit, whenDone };
}

const toDate = (s: string) => {
  const [y, m, d] = s.split("-").map(Number);
  return new Date(y, m - 1, d);
};
const addDays = (s: string, n: number) => {
  const d = toDate(s);
  d.setDate(d.getDate() + n);
  return ymd(d);
};
/** The same day of the month `n` months on, clamped to the month's length (chrono's checked_add_months). */
const addMonths = (s: string, n: number) => {
  const [y, m, day] = s.split("-").map(Number);
  const total = m - 1 + n;
  const ny = y + Math.floor(total / 12);
  const nm = (total % 12) + 1;
  const last = new Date(ny, nm, 0).getDate();
  return ymd(new Date(ny, nm - 1, Math.min(day, last)));
};
const diffDays = (a: string, b: string) => Math.round((toDate(a).getTime() - toDate(b).getTime()) / 86400_000);

function nextOf(rule: Repeat, from: string): string {
  switch (rule.unit) {
    case "day":
      return addDays(from, rule.every);
    case "week":
      return addDays(from, 7 * rule.every);
    case "month":
      return addMonths(from, rule.every);
    case "year":
      return addMonths(from, 12 * rule.every);
  }
}

/** The line of the next occurrence of a task being ticked on `today`, from the line as
 *  it is before the tick; null when the line has no readable rule. Every date moves by
 *  the same amount; the checkbox reopens, the stamps go, a ➕ is refreshed, and the
 *  `^anchor` is dropped (an id must not be duplicated). The mirror of repeat.rs. */
export function nextOccurrenceLine(line: string, today: string): string | null {
  const a = lineAttrs(line.replace(ANY_CHECKBOX_RE, "$1[ ]"));
  if (!a.repeat) return null;
  const rule = parseRepeat(a.repeat);
  if (!rule) return null;
  const anchor = a.due ?? a.scheduled ?? a.start;
  const from = rule.whenDone ? today : (anchor ?? today);
  const target = nextOf(rule, from);
  const shift = diffDays(target, anchor ?? from);
  let out = setCheckbox(line, "todo");
  out = rescheduleLine(out, "done", null);
  out = rescheduleLine(out, "cancelled", null);
  for (const f of ["due", "scheduled", "start"] as const) if (a[f]) out = rescheduleLine(out, f, addDays(a[f]!, shift));
  if (!anchor) out = rescheduleLine(out, "scheduled", target);
  if (a.created) out = rescheduleLine(out, "created", today);
  const tail = /[ \t]\^[\w-]+[ \t]*$/.exec(out);
  if (tail) out = out.slice(0, tail.index).replace(/[ \t]+$/, "");
  return out;
}

/** The whole rewrite of a task block's slice for a state change: the first line gets
 *  setStateLine; ticking a repeating task puts its next occurrence above it; ticking a
 *  🚮 task removes it — with a repeat the next occurrence stands in its place, without
 *  one the result is "" and the caller deletes the block (its line break included).
 *  The mirror of Rust's complete_task_slice. */
export function completeTaskSlice(slice: string, state: TaskState, today: string): string {
  const nl = slice.indexOf("\n");
  const cr = nl > 0 && slice[nl - 1] === "\r" ? 1 : 0;
  const head = nl < 0 ? slice : slice.slice(0, nl - cr);
  const tail = nl < 0 ? "" : slice.slice(nl - cr);
  const newHead = setStateLine(head, state, today);
  if (state === "done") {
    const a = lineAttrs(head);
    const next = nextOccurrenceLine(head, today);
    if (next !== null) return a.deleteOnDone ? next + tail : `${next}\n${newHead}${tail}`;
    if (a.deleteOnDone) return "";
  }
  return newHead + tail;
}

/* ---------------------------------------------------------------- writing to disk */

/** The stale-span guard for a task slice: it still starts with a checkbox, and its body
 *  still is the content the query indexed. A stale span may have shifted wholesale onto
 *  another task line (text of the same length was inserted before it), so the shape
 *  alone is not enough. */
const isTaskSlice = (hit: BlockHit) => (slice: string) => {
  const m = slice.match(CHECKBOX_RE);
  return !!m && slice.slice(m[0].length).trim() === hit.block.content;
};

/** Rewrite exactly a task's block span with `edit(slice)`; an empty result deletes the
 *  block, a newline with it — deleting the span alone would leave a blank line: the \n
 *  after the block (a \r\n included) is preferred, and when the block ends the file
 *  with no trailing newline, the one before it is taken instead.
 *  Returns false when the span is stale (the note changed since the query); the caller
 *  should requery and let the user retry on fresh data. */
async function writeTaskSpan(hit: BlockHit, edit: (slice: string) => string): Promise<boolean> {
  const r = await readBlockSpan(hit, isTaskSlice(hit));
  if (!r) return false;
  const { bytes } = r;
  const next = edit(r.slice);
  let from = hit.block.span.start;
  let to = hit.block.span.end;
  if (next === "") {
    if (bytes[to] === 0x0d && bytes[to + 1] === 0x0a) to += 2;
    else if (bytes[to] === 0x0a) to += 1;
    else if (from > 0 && bytes[from - 1] === 0x0a) from -= bytes[from - 2] === 0x0d ? 2 : 1;
  }
  const dec = new TextDecoder();
  await api.noteWrite(hit.note, dec.decode(bytes.slice(0, from)) + next + dec.decode(bytes.slice(to)));
  return true;
}

/** Delete one task block from disk (the whole span: a multi-line task's sub-content
 *  goes with it). false = a stale span, and the caller requeries. */
export async function deleteTask(hit: BlockHit): Promise<boolean> {
  return writeTaskSpan(hit, () => "");
}

/** Tick a task on disk, or reopen a closed one (the stamp, a repeat's next occurrence
 *  and a 🚮 removal follow). false = stale span, requery. */
export async function toggleTask(hit: BlockHit): Promise<boolean> {
  return writeTaskSpan(hit, (slice) => {
    const m = slice.match(CHECKBOX_RE)!;
    const next: TaskState = isClosed(stateOfChar(m[2])) ? "todo" : "done";
    return completeTaskSlice(slice, next, localDate());
  });
}

/** Reschedule a task on disk (null clears the date). false = stale span, requery. */
export async function rescheduleTask(hit: BlockHit, field: DateField, date: string | null): Promise<boolean> {
  return writeTaskSpan(hit, (slice) => {
    // By convention a date marker is written on the task's first line; a multi-line block only has its first line touched and its sub-content is kept verbatim
    const nl = slice.indexOf("\n");
    const head = nl < 0 ? slice : slice.slice(0, nl);
    return rescheduleLine(head, field, date) + (nl < 0 ? "" : slice.slice(nl));
  });
}

export interface TaskEdit {
  state?: TaskState;
  /** Set (or with null remove) one inline field on the task's first line — a kanban grouped by a field moves cards this way */
  field?: { key: string; value: string | null };
  /** Set or change one date field (rescheduleLine semantics: the marker style is kept and it is inserted before the ^anchor) */
  date?: { field: DateField; value: string };
  /** Clear due, scheduled and start (dragging to "unscheduled" on the kanban — clearing only one would land it in another field's group) */
  clearDates?: boolean;
}

/** The merged write-back of a kanban drag: the state and the date are done in **one**
 *  span rewrite — with two writes the second span would necessarily be stale (the
 *  first write already invalidated the offsets). The dates and the field go first, so
 *  a repeat's next occurrence is counted from the rescheduled line; then the state.
 *  false = a stale span, and the caller requeries. */
export async function updateTask(hit: BlockHit, edit: TaskEdit): Promise<boolean> {
  return writeTaskSpan(hit, (slice) => {
    const nl = slice.indexOf("\n");
    let head = nl < 0 ? slice : slice.slice(0, nl);
    if (edit.clearDates) for (const f of EDIT_FIELDS) head = rescheduleLine(head, f, null);
    if (edit.date) head = rescheduleLine(head, edit.date.field, edit.date.value);
    if (edit.field) head = setFieldValue(head, edit.field.key, edit.field.value);
    const out = head + (nl < 0 ? "" : slice.slice(nl));
    return edit.state ? completeTaskSlice(out, edit.state, localDate()) : out;
  });
}

/* ---------------------------------------------------------------- rows */

/** Task text for display: checkbox prefix, every attribute, capture stamps and
 *  markdown ornaments stripped — Agenda is a projection, and cramming the source's
 *  markers into a single line would only be noise.
 *  Folding emphasis, links, wikilinks and bare URLs is collected in plainText (shared
 *  with the claim and evidence cards), so "the plain-text form" has one rule only. */
export function taskText(content: string): string {
  let s = content.replace(/^\s*[-*+]\s+\[.\]\s*/, "");
  // every attribute occurrence, a decorative `⏳ waiting` (no date, no attempt) left as words
  const marks = scanTaskAttrs(s).filter((m) => (m.kind in DATE_FIELDS ? m.date !== null || m.attempted : m.active));
  for (let i = marks.length - 1; i >= 0; i--) s = cut(s, marks[i].start, marks[i].end);
  return plainText(
    stripAnchor(
      // a block id minted on the line is a badge in the editor, not a word
      stripFields(s) // `[key:: value]` metadata is a chip on the card, not part of the words
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
  state: TaskState;
  due?: string;
  scheduled?: string;
  start?: string;
  created?: string;
  done?: string;
  cancelled?: string;
  priority: Priority;
  repeat?: string;
  deleteOnDone: boolean;
  taskId?: string;
  dependsOn: string[];
  /** Minutes. */
  estimate?: number;
  /** When the task's current text appeared (ms; the block's modified_ms, 0 when unknown). */
  modified: number;
}

export function rowOf(hit: BlockHit): TaskRow {
  const k = hit.block.kind;
  const text = taskText(hit.block.content);
  const modified = hit.block.modified_ms ?? 0;
  if (k.type !== "task") return { hit, text, state: "todo", priority: "normal", deleteOnDone: false, dependsOn: [], modified };
  return {
    hit,
    text,
    state: k.state,
    due: k.due,
    scheduled: k.scheduled,
    start: k.start,
    created: k.created,
    done: k.done,
    cancelled: k.cancelled,
    priority: k.priority ?? "normal",
    repeat: k.repeat,
    deleteOnDone: !!k.delete_on_done,
    taskId: k.task_id,
    dependsOn: k.depends_on ?? [],
    estimate: k.estimate,
    modified,
  };
}

/** The filter key of Agenda and the kanban — one definition, so the two views give the
 *  same answer to the same keystroke.
 *  The body is the display text rowOf already stripped (markers from the source would
 *  only match spuriously), plus the raw dates: both views bucket dates coarsely —
 *  everything past 7 days becomes "later" and every future date becomes "scheduled" —
 *  so "which ones are due in September" has nowhere to land on screen, and the filter
 *  box is the only place it can be expressed. fuzzyKeep is binary and order-preserving,
 *  so a longer key does not disturb the order. */
export const taskFilterKey = (r: TaskRow) =>
  `${r.text} ${r.hit.note_title} ${r.hit.note} ${r.due ?? ""} ${r.scheduled ?? ""} ${r.start ?? ""} ${r.taskId ?? ""}`;

/** A task's identity across requeries: the note plus the block's byte span start,
 *  physically unique within a note (an explicit ^id may be copied into duplicates and
 *  cannot be a key). Both views key their lists by it, so the DOM of an unchanged row
 *  survives a requery, and the board finds a card again in whatever hits are current. */
export const taskKey = (r: TaskRow) => r.hit.note + ":" + r.hit.block.span.start;

/** The date a task groups by: due first, then the ⏳ scheduled day, then the ⏩ start —
 *  otherwise a task "starting next week" would carry a date chip while lying under "no
 *  date". Agenda's grouping and the kanban's column assignment read this one rule.
 *  Asked for one `field`, it reads that field alone: Agenda with a date kind pressed
 *  places every task by the date the user asked to see, not by the due-first default. */
export const whenOf = (r: TaskRow, field?: DateField) => (field ? r[field] : (r.due ?? r.scheduled ?? r.start));

/** Date ascending (whenOf's date, or the one `field` when given), and within a day the
 *  source order (the note's path, then the block's position in it) — so the order is
 *  stable after a requery whatever order the hits arrived in. */
export const byDate = (field?: DateField) => (a: TaskRow, b: TaskRow) =>
  (whenOf(a, field) ?? "9999").localeCompare(whenOf(b, field) ?? "9999") || bySource(a, b);

/** The rank of a priority: the higher the more urgent. */
export const priorityRank = (p: Priority) => ({ highest: 4, high: 3, normal: 2, low: 1, lowest: 0 })[p];

/** The most urgent first, then `then` (a date order, a change-time order). Agenda's
 *  groups and the board's columns put what is marked urgent at the top of whatever
 *  order they otherwise keep. */
export const byPriority = (then: (a: TaskRow, b: TaskRow) => number) => (a: TaskRow, b: TaskRow) =>
  priorityRank(b.priority) - priorityRank(a.priority) || then(a, b);

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

/** The ids of the open tasks among `rows`: what a ⛔ can still wait on. A dependency on
 *  an id nobody carries, or one whose task is closed, blocks nothing. */
export function openIds(rows: TaskRow[]): Set<string> {
  const ids = new Set<string>();
  for (const r of rows) if (r.taskId && !isClosed(r.state)) ids.add(r.taskId);
  return ids;
}

/** Waiting on another task that is still open. */
export const isBlocked = (r: TaskRow, open: Set<string>) => !isClosed(r.state) && r.dependsOn.some((id) => open.has(id));

/** The estimates of `rows` added up (minutes; the rows without one count nothing). */
export const estimateSum = (rows: TaskRow[]) => rows.reduce((n, r) => n + (r.estimate ?? 0), 0);

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
  const now = new Date();
  return diffDays(date, ymd(now));
}

/** The date string shown for an overdue task: the amount of lateness is appended. What
 *  matters most at a glance for an overdue task is "how late", which an absolute date
 *  does not show (the reader would have to count from today). Only overdue gets the
 *  relative amount — giving a future date "in 3 days" as well would mix two units in
 *  one column and make it harder to scan.
 *  The gate is the task's own lateness — an open task whose due date has passed —
 *  rather than "the date is in the past": a closed task is not late however long ago
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
 *     the absolute date falls back to the title; a date that is not late is written
 *     without the current year (chipDate) */
export function dueLabel(due: string, overdue: boolean, density: "full" | "short" = "full"): string {
  // The gate is checked before the days are computed: the non-overdue rows (all of
  // "today / later / done") are the majority, and they should not pay for three Date
  // constructions to produce a value that is dropped straight away
  if (!overdue) return density === "short" ? chipDate(due) : due;
  const late = -daysUntil(due);
  if (late <= 0) return due;
  const phrase = tn("task.overdue", late);
  return density === "short" ? phrase : `${due} · ${phrase}`;
}

/** A date at a kanban chip's density: the year is dropped while it is the current one
 *  ("09-10"), so the chip fits beside the note chip on the ~130px meta row of a 176px
 *  card instead of pushing the note chip onto a second line. The full date stays in the
 *  chip's title. A date in another year keeps the year: that is the one case where it
 *  carries information. */
export function chipDate(ymd: string): string {
  return ymd.startsWith(localDate().slice(0, 4) + "-") ? ymd.slice(5) : ymd;
}

/** What a task's context menu does, as the view that raised it does them. An Agenda row
 *  and a kanban card offer the same entries in the same words (the menu is the one place
 *  a task's state can be set to "in progress" or "cancelled" from a view), and each view
 *  brings its own write funnel — its dirty guard, its requery on a stale span — and its
 *  own date menu. */
export interface TaskMenuActions {
  /** Jump to the task's line in its note. */
  open: () => void;
  /** Raise the date menu for one field; current is the date it has, absent when the date is being set for the first time. */
  editDate: (field: DateField, current?: string) => void;
  /** Set the task's state; `act` is the wording of the action for the dirty guard's message. */
  setState: (state: TaskState, act: string) => void;
  remove: () => void;
}

/** The entries of a task's context menu, computed for the task at the moment of open.
 *  A date the task has can be changed; one it does not have can be set (not on a closed
 *  task — a date on a finished task is a record, not a plan); the state entries read
 *  "mark / reopen" by what the task is now; and removing deletes the line from the
 *  note's source, so it takes ContextMenu's second confirmation. */
export function taskMenuEntries(task: Pick<TaskRow, "state" | "due" | "scheduled" | "start">, act: TaskMenuActions): MenuEntry[] {
  const entries: MenuEntry[] = [{ label: t("task.menu.jumpToSource"), action: act.open }, { separator: true }];
  const closed = isClosed(task.state);
  for (const field of EDIT_FIELDS) {
    const current = task[field];
    if (current) entries.push({ label: `${t("task.editDate", { field: dateFieldLabel(field) })}…`, action: () => act.editDate(field, current) });
    else if (!closed) entries.push({ label: `${t("task.setDate", { field: dateFieldLabel(field) })}…`, action: () => act.editDate(field) });
  }
  entries.push({ separator: true });
  // "changing the state" when "in progress" or "cancelled" is involved on either side, "ticking" otherwise
  const setState = (to: TaskState) =>
    act.setState(to, t(to === "done" && task.state !== "doing" ? "task.act.toggle" : to === "todo" && task.state === "done" ? "task.act.toggle" : "task.act.state"));
  if (!closed)
    entries.push({
      label: t(task.state === "doing" ? "task.menu.markBackTodo" : "task.menu.markDoing"),
      action: () => setState(task.state === "doing" ? "todo" : "doing"),
    });
  entries.push({
    label: t(closed ? "task.menu.reopen" : "task.menu.markDone"),
    action: () => setState(closed ? "todo" : "done"),
  });
  if (!closed) entries.push({ label: t("task.menu.markCancelled"), action: () => setState("cancelled") });
  entries.push({ separator: true }, { label: t("task.menu.remove"), danger: true, confirm: t("task.menu.removeConfirm"), action: act.remove });
  return entries;
}

/** A date field's display name ("Due date" / "Scheduled date" / "Start date"). Shared by the menu items and the chip's tooltip. */
export function dateFieldLabel(field: DateField): string {
  return t(`task.field.${field}`);
}

/** A priority's display name ("urgent" / "important" / "later" / "someday"); normal has none. */
export function priorityLabel(p: Priority): string {
  return p === "normal" ? "" : t(`task.priority.${p}`);
}
