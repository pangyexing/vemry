// The `/` menu: at the start of a line's content (after indentation, a list
// marker, a checkbox or a quote marker) `/` offers block templates in three
// groups — basic, blocks, references. On an empty line an item inserts its
// template; on a line that already has text an item that names a block kind
// turns the current block into it (the same `turnInto` the block menu uses,
// with the `/query` token removed in the same change); a multi-line template
// on a non-empty line goes below the block. Chaining items (callout, evidence,
// link, block reference, embed) insert their opener and hand over to the next
// completion source through the picked-completion annotation.
import { t } from "$lib/i18n/i18n.svelte";
import { type Completion, type CompletionContext, type CompletionResult, type CompletionSource, pickedCompletion } from "@codemirror/autocomplete";
import type { EditorView } from "@codemirror/view";
import { localDate } from "$lib/views/tasks";
import { blockAt, extentTo } from "./blocks";
import { type TurnTarget, canTurnInto, turnInto } from "./blockops";
import { inCode } from "./livepreview";

/** `/query` at the start of the line's content. Groups: 1 = indentation, 2 = list /
 *  checkbox / quote markers, 3 = the query. Anchored at the line start, so `a/b` and
 *  `https://` never trigger. */
export const SLASH_RE = /^([ \t]*)((?:>[ \t]*|[-*+][ \t]+(?:\[[ xX/]\][ \t]+)?|\d+[.)][ \t]+)*)\/([^\s/]*)$/;

type Section = "basic" | "blocks" | "refs";

/** A finite id set: `slash.${SlashId}.title` is then a finite union that matches the catalog's keys. */
type SlashId =
  | "heading1"
  | "heading2"
  | "heading3"
  | "bullet"
  | "numbered"
  | "task"
  | "quote"
  | "callout"
  | "claim"
  | "evidence"
  | "code"
  | "js"
  | "query"
  | "table"
  | "hr"
  | "date"
  | "link"
  | "blockref"
  | "embed";

interface SlashItem {
  id: SlashId;
  /** prefix: a line prefix (replaces existing markers); inline: at the token; block: a multi-line template. */
  kind: "prefix" | "inline" | "block";
  section: Section;
  text: string | (() => string);
  /** Caret offset inside `text` (default: its end). */
  caret?: number;
  /** Hand over to the next completion source after inserting. */
  chain?: boolean;
  /** On a non-empty line, turn the block into this kind instead of inserting. */
  turn?: TurnTarget;
}

const ITEMS: SlashItem[] = [
  { id: "heading1", kind: "prefix", section: "basic", text: "# ", turn: "heading1" },
  { id: "heading2", kind: "prefix", section: "basic", text: "## ", turn: "heading2" },
  { id: "heading3", kind: "prefix", section: "basic", text: "### ", turn: "heading3" },
  { id: "bullet", kind: "prefix", section: "basic", text: "- ", turn: "bullet" },
  { id: "numbered", kind: "prefix", section: "basic", text: "1. ", turn: "numbered" },
  { id: "task", kind: "prefix", section: "basic", text: "- [ ] ", turn: "task" },
  { id: "quote", kind: "prefix", section: "basic", text: "> ", turn: "quote" },
  { id: "callout", kind: "prefix", section: "blocks", text: "> [!", chain: true, turn: "callout" },
  { id: "claim", kind: "prefix", section: "blocks", text: "> [!claim] " },
  { id: "evidence", kind: "prefix", section: "blocks", text: "> [!evidence] \n> supports:@", chain: true },
  { id: "code", kind: "block", section: "blocks", text: "```\n\n```", caret: 4, turn: "code" },
  { id: "js", kind: "block", section: "blocks", text: "```js\n\n```", caret: 6 },
  { id: "query", kind: "block", section: "blocks", text: "```vemry-query\nkind: task\n```", caret: 25 },
  { id: "table", kind: "block", section: "blocks", text: "|  |  |\n| --- | --- |\n|  |  |", caret: 2 },
  { id: "hr", kind: "block", section: "blocks", text: "---" },
  { id: "date", kind: "inline", section: "refs", text: () => localDate() },
  { id: "link", kind: "inline", section: "refs", text: "[[", chain: true },
  { id: "blockref", kind: "inline", section: "refs", text: "@", chain: true },
  { id: "embed", kind: "inline", section: "refs", text: "![[", chain: true },
];

const RANK: Record<Section, number> = { basic: 0, blocks: 1, refs: 2 };

function apply(view: EditorView, item: SlashItem, completion: Completion, applyTo: number) {
  const state = view.state;
  const line = state.doc.lineAt(applyTo);
  const m = SLASH_RE.exec(state.sliceDoc(line.from, applyTo));
  if (!m) return;
  const indentLen = m[1].length;
  const markersLen = m[2].length;
  const slashPos = line.from + indentLen + markersLen;
  const text = typeof item.text === "function" ? item.text() : item.text;
  const rest = line.text.slice(applyTo - line.from);
  const lineHasText = rest.trim() !== "" || (markersLen > 0 && item.kind !== "prefix");
  const block = blockAt(state, applyTo);
  const chainTx = item.chain ? { userEvent: "input.complete", annotations: pickedCompletion.of(completion) } : {};
  const caretIn = item.caret ?? text.length;

  // a block that already has text (or a marker to keep): turn it into the kind
  if (item.turn && block && (rest.trim() !== "" || markersLen > 0)) {
    if (!canTurnInto(block, item.turn)) {
      view.dispatch({ changes: { from: slashPos, to: applyTo, insert: "" }, selection: { anchor: slashPos } });
      return;
    }
    const spec = turnInto(state, block, item.turn, { omit: { from: slashPos, to: applyTo } });
    if (spec) view.dispatch(spec);
    return;
  }
  if (item.kind === "inline") {
    view.dispatch({
      changes: { from: slashPos, to: applyTo, insert: text },
      selection: { anchor: slashPos + caretIn },
      ...chainTx,
    });
    return;
  }
  if (item.kind === "prefix") {
    // replaces the existing markers too: `- /task` → `- [ ] `
    const from = line.from + indentLen;
    const pad = " ".repeat(indentLen);
    const body = text.replace(/\n/g, "\n" + pad);
    view.dispatch({
      changes: { from, to: applyTo, insert: body },
      selection: { anchor: from + (item.caret ?? body.length) },
      ...chainTx,
    });
    return;
  }
  // block template
  if (lineHasText && block) {
    // below the current block, on its own lines
    const at = extentTo(state, block);
    const prevBlank = false;
    const insert = "\n\n" + text + (prevBlank ? "" : "");
    view.dispatch({
      changes: [
        { from: slashPos, to: applyTo, insert: "" },
        { from: at, insert },
      ],
      selection: { anchor: at - (applyTo - slashPos) + 2 + caretIn },
    });
    return;
  }
  const prev = line.number > 1 ? state.doc.line(line.number - 1).text : "";
  // a rule right under a paragraph would read as a setext underline
  const lead = item.id === "hr" && prev.trim() !== "" ? "\n" : "";
  const tail = applyTo < line.to ? "\n" : "";
  const insert = lead + text + tail;
  view.dispatch({
    changes: { from: line.from, to: applyTo, insert },
    selection: { anchor: line.from + lead.length + caretIn },
  });
}

export function slashCompletions(): CompletionSource {
  return (context: CompletionContext): CompletionResult | null => {
    const before = context.matchBefore(SLASH_RE);
    if (!before) return null;
    const m = SLASH_RE.exec(before.text)!;
    const slashPos = before.from + m[1].length + m[2].length;
    if (inCode(context.state, slashPos)) return null;
    const options: Completion[] = ITEMS.map((item, i) => ({
      label: t(`slash.${item.id}.title`),
      detail: t(`slash.${item.id}.desc`),
      type: "text",
      sortText: String(i).padStart(3, "0"),
      section: { name: t(`slash.section.${item.section}`), rank: RANK[item.section] },
      apply: (view: EditorView, completion: Completion, _from: number, applyTo: number) => apply(view, item, completion, applyTo),
    }));
    return { from: slashPos + 1, options, validFor: /^[^\s/]*$/ };
  };
}
