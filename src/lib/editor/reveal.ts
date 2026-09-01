// The one definition of "reveal" (whether the syntax folding is expanded to its
// source), shared by live preview's decoration rebuild, the whole-block expansion of
// tables and query blocks, and the "rendered = open / raw = edit" boundary of a link
// click.
// It is its own file rather than part of livepreview.ts: that imports table.ts, and
// importing back would make a cycle.
import { Annotation, StateField, type EditorState } from "@codemirror/state";

/** Carried by a programmatic jump (the revealPos of the outline, the kanban, Agenda
 *  and the fallback landing point when a window closes): the selection moved, but that
 *  is not a user's intent to reveal, so the landing point stays rendered. */
export const suppressReveal = Annotation.define<boolean>();

/** The armed gate: false = there has been no user selection intent yet (the initial
 *  cursor of resetDoc on a freshly opened document, or just after a programmatic
 *  jump), and nothing is revealed. Any transaction carrying tr.selection without the
 *  annotation (keyboard, mouse, input, IME, undo) arms it again. It replaces the old
 *  r.to > 0 position exemption — that guessed "this is not a user's intent" from a
 *  position value, could not stop a programmatic landing at p>0, and came with the
 *  cost of "the first line is not revealed after ⌘Home"; judging by where the
 *  transaction came from fixes both.
 *  Registered in: livePreview()'s extension array (every consumer of reveal lives in
 *  note mode alone). */
export const revealArmed = StateField.define<boolean>({
  create: () => false,
  update: (value, tr) => (tr.selection ? !tr.annotation(suppressReveal) : value),
});

// fail-open: an editor instance without the field registered keeps revealing (forgetting to register it must not make the source permanently invisible)
const armed = (state: EditorState) => state.field(revealArmed, false) ?? true;

/** Element-level reveal: a bare cursor only — it reveals when the cursor lands inside
 *  the element's range [from,to] (inclusive, so exactly on a boundary counts and it
 *  stays keyboard-reachable); a non-empty range from dragging or ⌘A stays rendered. */
export function selectionReveals(state: EditorState, from: number, to: number): boolean {
  return armed(state) && state.selection.ranges.some((r) => r.empty && r.from >= from && r.from <= to);
}

/** Line level (a heading #, a quote >, an hr, a fence line, a callout icon): a bare
 *  cursor reveals its whole line; inline elements do not use it. */
export function revealRanges(state: EditorState): readonly (readonly [number, number])[] {
  if (!armed(state)) return [];
  return state.selection.ranges
    .filter((r) => r.empty)
    .map((r) => [state.doc.lineAt(r.from).from, state.doc.lineAt(r.from).to] as const);
}

/** Block level (the whole-block replace widget of a table, a query block or the
 *  frontmatter): a range selection that intersects expands the source too — CM draws
 *  no selection highlight over a replaced block widget, so keeping it rendered leaves
 *  "is this actually selected" invisible, while expanding makes it plain (the user's
 *  ruling, deliberately different from the empty-only rule of inline elements). */
export function selectionTouches(state: EditorState, from: number, to: number): boolean {
  return armed(state) && state.selection.ranges.some((r) => r.from <= to && r.to >= from);
}
