// Completion sources for the special markers: #tags and > [!callout types.
// The wikilink completion source lives in wikilink.ts (it needs the host); the
// sources sit side by side in setup.ts's autocompletion({override}), and each returns
// null to make way when its matchBefore does not hit.
import {
  type Completion,
  type CompletionContext,
  type CompletionResult,
  type CompletionSource,
  acceptCompletion,
  closeCompletion,
  completionStatus,
  pickedCompletion,
  startCompletion,
} from "@codemirror/autocomplete";
import { type Command, EditorView } from "@codemirror/view";
import type { Extension } from "@codemirror/state";
import { t } from "$lib/i18n/i18n.svelte";
import { type AttrKind, ATTR_KEY_NAME, attrIconSvg, DATE_FIELDS, type EditField, isUrgent, localDate, newTaskId, priorityLabel } from "$lib/views/tasks";
import type { Priority } from "$lib/api";
import type { BlockType, FieldStat } from "$lib/api";
import { blockAt, frontmatterEnd } from "./blocks";
import { type FieldChipHandler, inCode } from "./livepreview";
import { CALLOUT_GROUP, calloutDefaultTitle } from "$lib/callout";
import { declaredField, fieldValueKind, scanFields } from "$lib/fields";

/** Tab accepts the selected completion, like Enter; with none open it falls through
 *  to indentation. The difference is what a note name does with it: Enter finishes
 *  the link and lands after `]]`, Tab completes the stem and keeps the caret inside
 *  the brackets, so the target can go on being typed — `#` opens the headings and
 *  blocks of that note, `|` an alias. The same convention as the `@` picker and the
 *  palette, where Tab on a note row drills into it. `acceptCompletion` calls the
 *  option's apply synchronously, which is what lets a flag carry the key to it. */
let tabAccepting = false;
export function acceptedByTab(): boolean {
  return tabAccepting;
}
export const acceptWithTab: Command = (view) => {
  tabAccepting = true;
  try {
    return acceptCompletion(view);
  } finally {
    tabAccepting = false;
  }
};

/** `@` at the start of a line or after whitespace / an opening bracket / `:` (the
 *  `supports:@` of an evidence block); mirrors the agent composer's AT_RE. */
export const AT_RE = /(^|[\s(（【[:])@([^\s@]*)$/;

function nextMonday(): string {
  const d = new Date();
  const delta = ((8 - d.getDay()) % 7) || 7;
  return localDate(delta);
}

/** A completion entry that carries a drawn attribute icon (the `@` palette's task
 *  attributes): the icon's svg (attrIconSvg) and its tone class (app.css's
 *  cm-lp-attr-* — the chip's colours), rendered after the label by attrIconOption. */
interface AttrCompletion extends Completion {
  attrIcon?: string;
  attrTone?: string;
}

/** The option renderer that draws an attribute entry's icon between the label and the
 *  detail (autocompletion's addToOptions; label is 50, detail 80): the same stroke
 *  icon as the chip the pick produces, not the file's emoji. */
export const attrIconOption = {
  render(completion: Completion): Node | null {
    const c = completion as AttrCompletion;
    if (!c.attrIcon) return null;
    const el = document.createElement("span");
    el.className = `cm-completionAttrIcon ${c.attrTone ?? ""}`;
    el.innerHTML = c.attrIcon;
    return el;
  },
  position: 60,
};

/** The `@` reference picker: notes (picking one turns `@name` into `[[name#]]` and
 *  reopens the completion on the heading/block stage), and dates (today, tomorrow,
 *  the day after, next Monday, or a `YYYY-MM-DD` typed out) — a date on a task line
 *  is inserted as its due date `📅 …`, elsewhere as plain text. On a task line the
 *  picker is also the way the attributes are typed: each entry inserts its marker
 *  (`@due` a `📅` and the date menu, `@urgent` a `❗❗`, `@every week` a `🔁` rule,
 *  `@estimate 2h`, `@id` a fresh id, `@depends on` a `⛔` to paste an id after).
 *  The labels are English like the `/` menu's, so that typing after `@` filters them
 *  in any locale; the emoji goes in the detail column. */
/** The vocabularies the `@` palette offers and `[key:: …]` completes: one declaration,
 *  so the two palettes cannot come to know different words. */
const REPEAT_RULES = ["every day", "every week", "every month", "every year"] as const;
const ESTIMATES = ["30m", "1h", "2h", "1d"] as const;
const PRIORITY_PICKS = [
  ["urgent", "highest", "❗❗"],
  ["important", "high", "❗"],
  ["later", "low", "❕"],
  ["someday", "lowest", "❕❕"],
] as const;

export function atCompletions(targets: () => string[], onFieldChip?: FieldChipHandler): CompletionSource {
  return (context: CompletionContext): CompletionResult | null => {
    const before = context.matchBefore(AT_RE);
    if (!before) return null;
    const m = AT_RE.exec(before.text)!;
    const atPos = before.from + m[1].length;
    if (inCode(context.state, atPos)) return null;
    const line = context.state.doc.lineAt(atPos);
    if (/\[\[[^\]\n]*$/.test(context.state.sliceDoc(line.from, atPos))) return null;
    const query = m[2];
    const options: Completion[] = targets().map((name) => ({
      label: name,
      type: "chain",
      section: { name: t("wikilink.sectionNotes"), rank: 0 },
      apply: (view: EditorView, completion, _applyFrom: number, applyTo: number) => {
        view.dispatch({
          changes: { from: atPos, to: applyTo, insert: `[[${name}#]]` },
          selection: { anchor: atPos + 2 + name.length + 1 },
          userEvent: "input.complete",
          annotations: pickedCompletion.of(completion),
        });
      },
    }));
    const dates: [string, string][] = [
      [t("wikilink.today"), localDate(0)],
      [t("wikilink.tomorrow"), localDate(1)],
      [t("wikilink.dayAfter"), localDate(2)],
      [t("wikilink.nextMonday"), nextMonday()],
    ];
    const typed = /^(\d{4})-(\d{1,2})-(\d{1,2})$/.exec(query);
    if (typed) {
      const iso = `${typed[1]}-${typed[2].padStart(2, "0")}-${typed[3].padStart(2, "0")}`;
      if (!Number.isNaN(new Date(iso).getTime())) dates.unshift([iso, iso]);
    }
    for (const [label, date] of dates) {
      options.push({
        label,
        detail: date,
        type: "text",
        section: { name: t("wikilink.sectionDates"), rank: 2 },
        apply: (view: EditorView, _c, _applyFrom: number, applyTo: number) => {
          const onTask = blockAt(view.state, atPos)?.kind === "task";
          const text = onTask ? `📅 ${date}` : date;
          view.dispatch({ changes: { from: atPos, to: applyTo, insert: text }, selection: { anchor: atPos + text.length } });
        },
      });
    }
    if (blockAt(context.state, atPos)?.kind === "task") {
      const section = { name: t("wikilink.sectionTaskAttrs"), rank: 1 };
      // The entry shows the drawn icon of the chip the pick produces (the file gets the emoji)
      const item = (label: string, kind: AttrKind, text: string, detail = "", priority?: Priority, then?: (view: EditorView) => void): AttrCompletion => ({
        label,
        detail,
        type: "text",
        section,
        attrIcon: attrIconSvg(kind, priority),
        attrTone: `cm-lp-attr-${kind}${priority ? (isUrgent(priority) ? " cm-lp-attr-urgent" : " cm-lp-attr-faint") : ""}`,
        apply: (view: EditorView, _c, _applyFrom: number, applyTo: number) => {
          view.dispatch({ changes: { from: atPos, to: applyTo, insert: text }, selection: { anchor: atPos + text.length } });
          then?.(view);
        },
      });
      // a date: the bare marker, then the date menu under it — its pick fills the marker in (rescheduleLine)
      const dateItem = (field: EditField) =>
        item(field, field, `${DATE_FIELDS[field].emoji[0]} `, "…", undefined, (view) => {
          const c = onFieldChip && view.coordsAtPos(atPos);
          if (c) onFieldChip(view, atPos, field, "", { x: c.left, y: c.bottom + 2 }, { keyboard: true });
        });
      options.push(dateItem("due"), dateItem("scheduled"), dateItem("start"));
      for (const [label, p, mark] of PRIORITY_PICKS) options.push(item(label, "priority", `${mark} `, priorityLabel(p), p));
      for (const rule of REPEAT_RULES) options.push(item(rule, "repeat", `🔁 ${rule} `));
      for (const v of ESTIMATES) options.push(item(`estimate ${v}`, "estimate", `🕐 ${v} `));
      options.push(item("removed once done", "delete", "🚮 "));
      options.push(item("id", "id", `🆔 ${newTaskId()} `));
      options.push(item("depends on", "depends", "⛔ ", "id"));
    }
    return { from: atPos + 1, options, validFor: /^[^\s@]*$/ };
  };
}

/** Inline-field completion, two sources. Values: after `key:: ` the values a block
 *  type declares for the key come first, then today/tomorrow and "Pick a date…" for
 *  a date-valued key (the last raises the date menu at the caret — the one step from
 *  `[due::` to the calendar, since the bracket auto-closes and the field is source,
 *  not a clickable chip, while the caret is in it), then the key's values seen across
 *  the vault (most frequent first). Keys: a word followed by one `:` offers the keys
 *  it prefixes — the types' declared keys, then the vault's — and accepting writes
 *  `key:: `, so a field is typed as `author:` + Enter. A `::` not followed by a space
 *  or a digit is not a field (the parser's rule), so `std::vector` gets nothing;
 *  code, frontmatter and an unclosed `[[` are left alone. */
export function fieldCompletions(
  stats: () => FieldStat[] | Promise<FieldStat[]>,
  blockTypes: () => BlockType[] = () => [],
  onFieldChip?: FieldChipHandler,
): CompletionSource {
  const DATE_KEYS = new Set(["due", "scheduled", "date", "start", "end", "created", "deadline"]);
  /** The values a task attribute's `[key:: …]` takes (taskattrs.rs), offered on a task line. */
  const TASK_KEY_VALUES: Record<string, readonly string[]> = {
    [ATTR_KEY_NAME.priority.toLowerCase()]: PRIORITY_PICKS.map(([, p]) => p),
    [ATTR_KEY_NAME.repeat.toLowerCase()]: REPEAT_RULES,
    [ATTR_KEY_NAME.estimate.toLowerCase()]: ESTIMATES,
    [ATTR_KEY_NAME.delete.toLowerCase()]: ["delete"],
  };
  // The vault's stats are fetched on demand (a promise the first time after an index
  // update, the cached list afterwards); the cheap guards run before asking for them,
  // so the plain keystrokes that produce nothing never wait
  return (context: CompletionContext): CompletionResult | null | Promise<CompletionResult | null> => {
    const line = context.state.doc.lineAt(context.pos);
    if (line.from < frontmatterEnd(context.state.doc)) return null;
    if (inCode(context.state, context.pos)) return null;
    if (/\[\[[^\]\n]*$/.test(context.state.sliceDoc(line.from, context.pos))) return null;
    if (!context.matchBefore(/[\p{L}\p{N}_-]+(:|:: ?[^\s\])]*)$/u)) return null;
    const list = stats();
    return list instanceof Promise ? list.then((all) => build(context, all)) : build(context, list);
  };
  function build(context: CompletionContext, all: FieldStat[]): CompletionResult | null {
    const value = context.matchBefore(/(^|[\s[(])([\p{L}\p{N}_-]+)::( ?)([^\s\])]*)$/u);
    if (value) {
      const m = /(^|[\s[(])([\p{L}\p{N}_-]+)::( ?)([^\s\])]*)$/u.exec(value.text)!;
      const key = m[2];
      const spaced = m[3] === " ";
      const typed = m[4];
      if (/^\d+$/.test(key)) return null;
      // `std::vector`: no space and a non-digit value is not a field
      if (!spaced && typed !== "" && !/^\d/.test(typed)) return null;
      const stat = all.find((s) => s.key.toLowerCase() === key.toLowerCase());
      const options: Completion[] = [];
      const seen = new Set<string>();
      const push = (label: string, detail?: string, boost = 0) => {
        if (seen.has(label)) return;
        seen.add(label);
        options.push({
          label,
          detail,
          type: "text",
          boost,
          section: { name: t("field.sectionValues"), rank: 0 },
          apply: spaced ? undefined : ` ${label}`,
        });
      };
      const decl = declaredField(blockTypes(), key);
      for (const d of decl.values) push(d.value, t("field.fromType", { name: d.type }), 3);
      // a task attribute's `key::` on a task line: the values it takes
      const canned = TASK_KEY_VALUES[key.toLowerCase()];
      if (canned && blockAt(context.state, context.pos)?.kind === "task") for (const v of canned) push(v, undefined, 2.5);
      const dateKey = decl.kind === "date" || DATE_KEYS.has(key.toLowerCase()) || (stat?.values.some((v) => fieldValueKind(v) === "date") ?? false);
      if (dateKey) {
        push(localDate(0), t("field.today"), 2);
        push(localDate(1), t("field.tomorrow"), 1);
      }
      const from = value.from + m[1].length + key.length + 2 + (spaced ? 1 : 0);
      if (dateKey && onFieldChip) {
        const open = onFieldChip;
        options.push({
          label: t("field.pickDate"),
          type: "text",
          boost: 0.5,
          section: { name: t("field.sectionValues"), rank: 0 },
          // The menu edits the field in place, so there has to be one: an auto-closed
          // `[due::]` or a whole-line `due::` already is; an unclosed `[due::` (auto-close
          // off) gets its bracket first. The menu is anchored under the field's start.
          apply: (view: EditorView, _c, applyFrom: number, applyTo: number) => {
            const lower = key.toLowerCase();
            const find = () => {
              const line = view.state.doc.lineAt(applyFrom);
              return scanFields(line.text, line.from, { empty: true }).find((f) => f.key.toLowerCase() === lower && f.from <= applyFrom && applyFrom <= f.to);
            };
            let mark = find();
            if (!mark) {
              const line = view.state.doc.lineAt(applyFrom);
              const opener = /([[(])[\p{L}\p{N}_-]+::\s?$/u.exec(view.state.sliceDoc(line.from, applyFrom));
              if (!opener) return;
              view.dispatch({ changes: { from: applyTo, insert: opener[1] === "[" ? "]" : ")" }, selection: { anchor: applyTo } });
              mark = find();
              if (!mark) return;
            }
            const c = view.coordsAtPos(mark.from);
            if (c) open(view, mark.from, mark.key, "", { x: c.left, y: c.bottom + 2 }, { keyboard: true });
          },
        });
      }
      for (const v of stat?.values ?? []) push(v);
      if (!options.length) return null;
      return { from, options, validFor: /^[^\s\])]*$/ };
    }
    const keyMatch = context.matchBefore(/(^|[\s[(])([\p{L}\p{N}_-]+):$/u);
    if (!keyMatch) return null;
    const km = /(^|[\s[(])([\p{L}\p{N}_-]+):$/u.exec(keyMatch.text)!;
    const typed = km[2].toLowerCase();
    const keys: { key: string; detail: string }[] = [];
    const seenKeys = new Set<string>();
    for (const ty of blockTypes())
      for (const f of ty.fields) {
        const lower = f.key.toLowerCase();
        if (!lower.startsWith(typed) || seenKeys.has(lower)) continue;
        seenKeys.add(lower);
        keys.push({ key: f.key, detail: t("field.fromType", { name: ty.name }) });
      }
    for (const s of all) {
      const lower = s.key.toLowerCase();
      if (!lower.startsWith(typed) || /^\d+$/.test(s.key) || seenKeys.has(lower)) continue;
      seenKeys.add(lower);
      keys.push({ key: s.key, detail: t("field.notesWithKey", { n: s.count }) });
    }
    if (!keys.length) return null;
    const from = keyMatch.from + km[1].length;
    return {
      from,
      options: keys.map((s) => ({
        label: `${s.key}:`,
        detail: s.detail,
        type: "text",
        section: { name: t("field.sectionKeys"), rank: 0 },
        apply: (view: EditorView, _c, applyFrom: number, applyTo: number) => {
          view.dispatch({ changes: { from: applyFrom, to: applyTo, insert: `${s.key}:: ` }, selection: { anchor: applyFrom + s.key.length + 3 } });
        },
      })),
      validFor: /^[\p{L}\p{N}_-]*:?$/u,
    };
  }
}

/** `#tag` completion: the candidates are the vault's tags (the caller keeps them fresh and sorted by how often they are used). */
export function tagCompletions(tags: () => string[]): CompletionSource {
  return (context: CompletionContext): CompletionResult | null => {
    const before = context.matchBefore(/#[\p{L}\p{N}_/-]*$/u);
    if (!before) return null;
    // TAG_RE's rule: a # must be preceded by the start of a line or whitespace (which rules out a URL anchor, a run of ##, and so on)
    const prev = context.state.sliceDoc(before.from - 1, before.from);
    if (prev && !/\s/.test(prev)) return null;
    // inside an unclosed `[[`, the `#` starts a heading/block stage, not a tag
    const lineStart = context.state.doc.lineAt(before.from).from;
    if (/\[\[[^\]\n]*$/.test(context.state.sliceDoc(lineStart, before.from))) return null;
    // A bare # at the start of a line is most likely a heading being typed: nothing pops up by itself, and only an explicit trigger (⌃Space) offers it
    const line = context.state.doc.lineAt(before.from);
    if (before.from === line.from && before.text === "#" && !context.explicit) return null;
    const options = tags().map((tag): Completion => ({ label: tag, type: "text" }));
    if (!options.length) return null;
    return { from: before.from + 1, options, validFor: /^[\p{L}\p{N}_/-]*$/u };
  };
}

/** Callout type completion for `> [!type` and for a bare `[!type` at the start of a
 *  line; accepting the latter adds the `> ` prefix (a callout only holds inside a
 *  quote block). detail shows the colour group. */
export function calloutCompletions(context: CompletionContext): CompletionResult | null {
  const before = context.matchBefore(/^(\s{0,3})(>\s*)?\[!([\w-]*)$/);
  if (!before) return null;
  const head = /^(\s{0,3})(>\s*)?\[!/.exec(before.text);
  if (!head) return null;
  const quoted = head[2] !== undefined;
  const bracketPos = before.from + head[1].length; // where `> ` is inserted before the `[` when unquoted
  const typeFrom = before.from + before.text.lastIndexOf("[!") + 2;
  const options = Object.keys(CALLOUT_GROUP).map(
    (name): Completion => ({
      label: name,
      detail: calloutDefaultTitle(name),
      type: "text",
      // closeBrackets has already added the ]: with a ] after the cursor only the type name is inserted
      apply: (view: EditorView, _completion, applyFrom: number, applyTo: number) => {
        const closed = view.state.sliceDoc(applyTo, applyTo + 1) === "]";
        const changes = [{ from: applyFrom, to: applyTo, insert: closed ? name : `${name}]` }];
        if (!quoted) changes.push({ from: bracketPos, to: bracketPos, insert: "> " });
        const shift = quoted ? 0 : 2; // the `> ` inserted in front shifts the cursor target right
        view.dispatch({
          changes,
          selection: { anchor: applyFrom + shift + name.length + 1 }, // land after the ]
        });
      },
    }),
  );
  return { from: typeFrom, options, validFor: /^[\w-]*$/ };
}

/** The triggers of the typed completions, as the text before the cursor shows them:
 *  an unclosed `[[`, an `@` token, a `/` command at the line's start, a `key::` value. */
const TRIGGER_BEFORE_RE = /\[\[[^\]\n]*$|(?:^|[\s(（【[:])@[^\s@]*$|^[ \t]*(?:>[ \t]*|[-*+][ \t]+(?:\[[ xX/]\][ \t]+)?|\d+[.)][ \t]+)*\/[^\s/]*$|[\p{L}\p{N}_-]+:: ?[^\s\])]*$/u;

/** Keep the completion popup out of an IME composition. A popup that is open when a
 *  composition starts keeps re-filtering on every composition update and rewriting
 *  the content element's aria-activedescendant, and WebKit treats attribute changes
 *  on the editing host during marked-text input as a reason to abandon the
 *  composition — the editor then looks wedged. So the popup closes the moment a
 *  composition starts and comes back, if a trigger still sits before the cursor,
 *  once the composition has ended and the browser has settled. */
export function imeCompletionGuard(): Extension {
  return EditorView.domEventHandlers({
    // The IME's first keydown (keyCode 229) precedes compositionstart: closing here
    // keeps the aria attribute change out of the composition altogether
    keydown(e, view) {
      if ((e.keyCode === 229 || e.isComposing) && completionStatus(view.state) != null) closeCompletion(view);
      return false;
    },
    compositionstart(_e, view) {
      if (completionStatus(view.state) != null) closeCompletion(view);
      return false;
    },
    compositionend(_e, view) {
      setTimeout(() => {
        if (view.composing || completionStatus(view.state) != null) return;
        const head = view.state.selection.main.head;
        const line = view.state.doc.lineAt(head);
        if (TRIGGER_BEFORE_RE.test(view.state.sliceDoc(line.from, head))) startCompletion(view);
      }, 30);
      return false;
    },
  });
}
