// Formulas in the live preview. Two shapes, one grammar ($lib/math, shared with the
// hand-written renderer so an embed of the note typesets exactly what the editor does):
// - inline `$…$` and the one-line `$$…$$` fold into a typeset widget on the line,
//   revealed as source when a bare cursor lands inside them (the element-level rule
//   every inline element follows, reveal.ts);
// - a display block whose `$$` fences sit on their own lines is replaced whole by a
//   block widget. A block-level replace spanning lines can only come from a StateField
//   (a ViewPlugin is not allowed), so it is this module's field rather than part of
//   livepreview's plugin; a selection touching the block expands the source
//   (selectionTouches, the block-level rule tables and query blocks use).
// KaTeX is called with trust:false and never throws (renderMathHtml); the widgets
// carry no handlers beyond "a click puts the cursor at the formula", which is what
// reveals the source for editing.
import { mathBlockTex, opensMathBlock, closesMathBlock, renderMathHtml } from "$lib/math";
import { syntaxTree } from "@codemirror/language";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { type EditorState, StateField, type Text } from "@codemirror/state";
import { blockRoot } from "./blockwidget";
import { focusEditor } from "./clickguard";
import { inCode } from "./fenced";
import { selectionTouches } from "./reveal";
import { foldRangesOf, insideFold } from "./table";

/** A click on a typeset formula lands the cursor at its start: the range is then
 *  revealed as source, which is how it gets edited. mousedown rather than click, and
 *  swallowed: the default handling would drop the cursor inside the replaced range
 *  by coordinates and destroy the widget under the pointer before a click arrived
 *  (the same reason the checkbox and image widgets act on mousedown). */
function landCursorOn(el: HTMLElement, view: EditorView) {
  el.onmousedown = (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    let pos: number;
    try {
      pos = view.posAtDOM(el);
    } catch {
      return; // the widget has left the document
    }
    view.dispatch({ selection: { anchor: pos } });
    focusEditor(view);
  };
}

/** An inline formula (`$…$`) or a one-line display one (`$$…$$`) as typeset KaTeX. */
export class MathWidget extends WidgetType {
  constructor(
    readonly tex: string,
    readonly display: boolean,
  ) {
    super();
  }
  eq(other: MathWidget) {
    return other.tex === this.tex && other.display === this.display;
  }
  toDOM(view: EditorView) {
    const el = document.createElement("span");
    el.className = this.display ? "cm-lp-math cm-lp-math-display" : "cm-lp-math";
    el.innerHTML = renderMathHtml(this.tex, this.display);
    landCursorOn(el, view);
    return el;
  }
}

/** The multi-line display block, replaced whole. */
class MathBlockWidget extends WidgetType {
  constructor(readonly tex: string) {
    super();
  }
  eq(other: MathBlockWidget) {
    return other.tex === this.tex;
  }
  get estimatedHeight() {
    return 52;
  }
  toDOM(view: EditorView) {
    const card = document.createElement("div");
    card.className = "cm-lp-mathblock";
    card.innerHTML = renderMathHtml(this.tex, true);
    landCursorOn(card, view);
    return blockRoot(card);
  }
}

export interface MathBlock {
  /** The block's extent: from the start of the opening `$$` line to the end of the closing one. */
  from: number;
  to: number;
  tex: string;
}

/** A block may not run on for ever: an opening `$$` with no closer within this many
 *  lines (or before the next blank line) is left as text, so a stray `$$` does not
 *  swallow the note below it. */
const MAX_BLOCK_LINES = 80;

/** Whether the document holds a `$$` at all — the common note does not, and this cheap
 *  answer spares it the line walk. Chunk by chunk, so no copy of the whole document is
 *  made; a `$$` never straddles two chunks, which are lines and line breaks. */
function hasMathFence(doc: Text): boolean {
  const iter = doc.iter();
  while (!iter.next().done) if (iter.value.includes("$$")) return true;
  return false;
}

/** The multi-line display blocks of the document, in order. A `$$` inside code is
 *  code, and an unclosed opener is not a block. */
function mathBlocks(state: EditorState): MathBlock[] {
  const doc = state.doc;
  const out: MathBlock[] = [];
  for (let n = 1; n <= doc.lines; n++) {
    const line = doc.line(n);
    if (!opensMathBlock(line.text) || inCode(state, line.from)) continue;
    const lines = [line.text];
    let close = -1;
    for (let k = n + 1; k <= Math.min(doc.lines, n + MAX_BLOCK_LINES); k++) {
      const l = doc.line(k);
      if (l.text.trim() === "") break; // the block ends with its paragraph: an unclosed opener stays text
      lines.push(l.text);
      if (closesMathBlock(l.text)) {
        close = k;
        break;
      }
    }
    if (close < 0) continue;
    out.push({ from: line.from, to: doc.line(close).to, tex: mathBlockTex(lines) });
    n = close;
  }
  return out;
}

interface Built {
  decos: DecorationSet;
  /** The blocks the document holds, drawn or not: a revealed or folded one keeps its
   *  place here, because the inline scan must skip its lines either way (mathBlocksOf). */
  blocks: MathBlock[];
}

/** Which of the blocks are drawn as widgets — the selection's and the folds' business,
 *  and the only part of the answer a cursor move can change. */
function decorate(state: EditorState, blocks: MathBlock[]): DecorationSet {
  const decos: ReturnType<Decoration["range"]>[] = [];
  for (const b of blocks) {
    if (selectionTouches(state, b.from, b.to) || insideFold(state, b.from, b.to)) continue;
    decos.push(Decoration.replace({ widget: new MathBlockWidget(b.tex), block: true }).range(b.from, b.to));
  }
  return Decoration.set(decos, true);
}

function build(state: EditorState): Built {
  const blocks = hasMathFence(state.doc) ? mathBlocks(state) : [];
  return { decos: blocks.length ? decorate(state, blocks) : Decoration.none, blocks };
}

/** Registered by livePreview() alongside the table field. */
export const mathBlockField = StateField.define<Built>({
  create: build,
  update(value, tr) {
    const treeChanged = syntaxTree(tr.state) !== syntaxTree(tr.startState);
    const foldChanged = foldRangesOf(tr).length > 0;
    if (!tr.docChanged && !tr.selection && !treeChanged && !foldChanged) return value;
    // Where the blocks are is the document's and the tree's answer (a `$$` inside code is
    // code); a cursor move or a fold only changes which of them are drawn, so the walk is
    // not repeated for one
    if (!tr.docChanged && !treeChanged) return value.blocks.length ? { decos: decorate(tr.state, value.blocks), blocks: value.blocks } : value;
    return build(tr.state);
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
});

/** The display blocks of the document as the field found them: the one answer to "is
 *  this line inside a display block", which the inline scan needs for every block,
 *  including the ones left as source because the selection touches them. */
export function mathBlocksOf(state: EditorState): MathBlock[] {
  return state.field(mathBlockField, false)?.blocks ?? [];
}
