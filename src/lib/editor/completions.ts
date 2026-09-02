// Completion sources for the special markers: #tags and > [!callout types.
// The wikilink completion source lives in wikilink.ts (it needs the host); the
// sources sit side by side in setup.ts's autocompletion({override}), and each returns
// null to make way when its matchBefore does not hit.
import { type Completion, type CompletionContext, type CompletionResult, type CompletionSource, pickedCompletion } from "@codemirror/autocomplete";
import type { EditorView } from "@codemirror/view";
import { t } from "$lib/i18n/i18n.svelte";
import { localDate } from "$lib/views/tasks";
import type { FieldStat } from "$lib/api";
import { blockAt, frontmatterEnd } from "./blocks";
import { inCode } from "./livepreview";
import { CALLOUT_GROUP } from "./livepreview";

/** `@` at the start of a line or after whitespace / an opening bracket / `:` (the
 *  `supports:@` of an evidence block); mirrors the agent composer's AT_RE. */
export const AT_RE = /(^|[\s(（【[:])@([^\s@]*)$/;

function nextMonday(): string {
  const d = new Date();
  const delta = ((8 - d.getDay()) % 7) || 7;
  return localDate(delta);
}

/** The `@` reference picker: notes (picking one turns `@name` into `[[name#]]` and
 *  reopens the completion on the heading/block stage), and dates (today, tomorrow,
 *  the day after, next Monday, or a `YYYY-MM-DD` typed out) — a date on a task line
 *  is inserted as its due date `📅 …`, elsewhere as plain text. */
export function atCompletions(targets: () => string[]): CompletionSource {
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
    return { from: atPos + 1, options, validFor: /^[^\s@]*$/ };
  };
}

/** Inline-field completion, two sources. Values: after `key:: ` the key's values seen
 *  across the vault (most frequent first) plus today/tomorrow for a date-valued key.
 *  Keys: a word followed by one `:` offers the known keys it prefixes, and accepting
 *  writes `key:: ` — so a field is typed as `author:` + Enter. A `::` not followed by
 *  a space or a digit is not a field (the parser's rule), so `std::vector` gets
 *  nothing; code, frontmatter and an unclosed `[[` are left alone. */
export function fieldCompletions(stats: () => FieldStat[]): CompletionSource {
  const DATE_KEYS = new Set(["due", "scheduled", "date", "start", "end", "created", "deadline"]);
  return (context: CompletionContext): CompletionResult | null => {
    const line = context.state.doc.lineAt(context.pos);
    if (line.from < frontmatterEnd(context.state.doc)) return null;
    if (inCode(context.state, context.pos)) return null;
    if (/\[\[[^\]\n]*$/.test(context.state.sliceDoc(line.from, context.pos))) return null;
    const value = context.matchBefore(/(^|[\s[(])([\p{L}\p{N}_-]+)::( ?)([^\s\])]*)$/u);
    if (value) {
      const m = /(^|[\s[(])([\p{L}\p{N}_-]+)::( ?)([^\s\])]*)$/u.exec(value.text)!;
      const key = m[2];
      const spaced = m[3] === " ";
      const typed = m[4];
      if (/^\d+$/.test(key)) return null;
      // `std::vector`: no space and a non-digit value is not a field
      if (!spaced && typed !== "" && !/^\d/.test(typed)) return null;
      const stat = stats().find((s) => s.key.toLowerCase() === key.toLowerCase());
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
      const dateKey = DATE_KEYS.has(key.toLowerCase()) || (stat?.values.some((v) => /^\d{4}-\d{2}-\d{2}$/.test(v)) ?? false);
      if (dateKey) {
        push(localDate(0), t("field.today"), 2);
        push(localDate(1), t("field.tomorrow"), 1);
      }
      for (const v of stat?.values ?? []) push(v);
      if (!options.length) return null;
      return { from: value.from + m[1].length + key.length + 2 + (spaced ? 1 : 0), options, validFor: /^[^\s\])]*$/ };
    }
    const keyMatch = context.matchBefore(/(^|[\s[(])([\p{L}\p{N}_-]+):$/u);
    if (!keyMatch) return null;
    const km = /(^|[\s[(])([\p{L}\p{N}_-]+):$/u.exec(keyMatch.text)!;
    const typed = km[2].toLowerCase();
    const keys = stats().filter((s) => s.key.toLowerCase().startsWith(typed) && !/^\d+$/.test(s.key));
    if (!keys.length) return null;
    const from = keyMatch.from + km[1].length;
    return {
      from,
      options: keys.map((s) => ({
        label: `${s.key}:`,
        detail: t("field.notesWithKey", { n: s.count }),
        type: "text",
        section: { name: t("field.sectionKeys"), rank: 0 },
        apply: (view: EditorView, _c, applyFrom: number, applyTo: number) => {
          view.dispatch({ changes: { from: applyFrom, to: applyTo, insert: `${s.key}:: ` }, selection: { anchor: applyFrom + s.key.length + 3 } });
        },
      })),
      validFor: /^[\p{L}\p{N}_-]*:?$/u,
    };
  };
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
      detail: CALLOUT_GROUP[name],
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
