// Block selection mode and the block keys. Text mode: ⌘⇧↑ / ⌘⇧↓ move the block
// at the cursor, ⌘⇧D duplicates it (keys defaultKeymap leaves free; ⌥↑/⌥↓ stay
// CodeMirror's line moves). Block mode: Esc from a bare cursor selects the
// innermost block (a nested item is a unit); ↑/↓ walk in document order; ⇧↑/⇧↓
// and ⇧click extend over a sibling run (climbing past the last sibling lifts the
// selection to the parent's level); Enter returns to text; ⌫ deletes; ⌘D
// duplicates; ⌘C/⌘X go through the DOM copy/cut events so the native Edit menu
// works too; ⌥↑/⌥↓ and ⌘⇧↑/↓ move the run; Tab/⇧Tab indent; ⌘A selects every
// block; typing lands at the block's end and exits; a click exits.
//
// The selection itself is a StateField over block start positions; CodeMirror's
// own selection is collapsed to the head block's start with suppressReveal, so
// tables, query blocks and embeds stay rendered (they key off selectionTouches).
// Any transaction that sets a selection without our annotation ends the mode.
import { completionStatus } from "@codemirror/autocomplete";
import { foldedRanges } from "@codemirror/language";
import { Annotation, EditorSelection, Prec, RangeSetBuilder, StateEffect, StateField, type EditorState, type Extension } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, keymap, type Command } from "@codemirror/view";
import { suppressReveal } from "./reveal";
import {
  type EditorBlock,
  ancestorAtDepth,
  blockAt,
  blockContentStart,
  ensureParsed,
  extentTo,
  nextBlock,
  nextSibling,
  prevBlock,
  prevSibling,
  siblings,
  topBlocks,
} from "./blocks";
import { copyBlocksText, deleteBlocks, duplicateBlocks, indentBlock, moveBlocks, outdentBlock } from "./blockops";

export interface BlockSel {
  /** Start positions of the anchor and head blocks (mapped through changes). */
  anchor: number;
  head: number;
}

export const setBlockSel = StateEffect.define<BlockSel | null>();
/** Marks the mode's own transactions: a selection change carrying it keeps the mode. */
export const blockSelTx = Annotation.define<boolean>();

const selLine = Decoration.line({ class: "cm-blocksel" });
const selFirst = Decoration.line({ class: "cm-blocksel cm-blocksel-first" });
const selLast = Decoration.line({ class: "cm-blocksel cm-blocksel-last" });
const selOnly = Decoration.line({ class: "cm-blocksel cm-blocksel-first cm-blocksel-last" });

function decorate(state: EditorState, sel: BlockSel | null): DecorationSet {
  const run = sel ? runOf(state, sel) : [];
  if (!run.length) return Decoration.none;
  const doc = state.doc;
  const from = run[0].from;
  const to = extentTo(state, run[run.length - 1]);
  const b = new RangeSetBuilder<Decoration>();
  const firstLine = doc.lineAt(from).number;
  const lastLine = doc.lineAt(to).number;
  for (let n = firstLine; n <= lastLine; n++) {
    const line = doc.line(n);
    const deco = n === firstLine && n === lastLine ? selOnly : n === firstLine ? selFirst : n === lastLine ? selLast : selLine;
    b.add(line.from, line.from, deco);
  }
  return b.finish();
}

export const blockSelField = StateField.define<{ sel: BlockSel | null; decos: DecorationSet }>({
  create: () => ({ sel: null, decos: Decoration.none }),
  update(value, tr) {
    let sel = value.sel;
    let touched = false;
    for (const e of tr.effects) {
      if (e.is(setBlockSel)) {
        sel = e.value;
        touched = true;
      }
    }
    if (!touched && tr.docChanged && sel) {
      sel = { anchor: tr.changes.mapPos(sel.anchor, 1), head: tr.changes.mapPos(sel.head, 1) };
    }
    if (!touched && tr.selection && !tr.annotation(blockSelTx) && sel) sel = null;
    if (sel === value.sel && !tr.docChanged) return value;
    return { sel, decos: decorate(tr.state, sel) };
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
});

export function isBlockMode(state: EditorState): boolean {
  return state.field(blockSelField, false)?.sel != null;
}

/** The sibling run between the anchor and head blocks (both lifted to the same
 *  depth); the head block alone when they are not siblings any more. */
function runOf(state: EditorState, sel: BlockSel): EditorBlock[] {
  const a = blockAt(state, sel.anchor);
  const h = blockAt(state, sel.head);
  if (!h) return [];
  if (!a) return [h];
  const depth = Math.min(a.depth, h.depth);
  const aa = ancestorAtDepth(a, depth);
  const hh = ancestorAtDepth(h, depth);
  const sib = siblings(state, hh);
  const i = sib.indexOf(aa);
  const j = sib.indexOf(hh);
  if (i < 0 || j < 0) return [hh];
  return sib.slice(Math.min(i, j), Math.max(i, j) + 1);
}

export function selectedBlocks(state: EditorState): EditorBlock[] {
  const sel = state.field(blockSelField, false)?.sel;
  return sel ? runOf(state, sel) : [];
}

function modeTx(view: EditorView, sel: BlockSel, extra?: { scroll?: boolean }) {
  view.dispatch({
    selection: EditorSelection.cursor(sel.head),
    effects: setBlockSel.of(sel),
    annotations: [suppressReveal.of(true), blockSelTx.of(true)],
    scrollIntoView: extra?.scroll ?? true,
  });
}

/** Enter the mode on `b` (the handle, the menu and Esc all come through here). */
export function selectBlock(view: EditorView, b: EditorBlock): void {
  modeTx(view, { anchor: b.from, head: b.from });
}

function exitTo(view: EditorView, pos: number) {
  view.dispatch({ selection: EditorSelection.cursor(pos), effects: setBlockSel.of(null), annotations: blockSelTx.of(true) });
}

function headBlock(view: EditorView): EditorBlock | null {
  const sel = view.state.field(blockSelField, false)?.sel;
  return sel ? blockAt(view.state, sel.head) : null;
}

/** Skip blocks hidden inside a fold when walking. */
function hiddenInFold(state: EditorState, b: EditorBlock): boolean {
  let hidden = false;
  foldedRanges(state).between(0, state.doc.length, (from, to) => {
    if (from < b.from && b.from <= to) hidden = true;
  });
  return hidden;
}

function walk(state: EditorState, b: EditorBlock, dir: -1 | 1): EditorBlock | null {
  let cur: EditorBlock | null = b;
  do {
    cur = dir < 0 ? prevBlock(state, cur) : nextBlock(state, cur);
  } while (cur && hiddenInFold(state, cur));
  return cur;
}

const inMode = (f: (view: EditorView, sel: BlockSel, head: EditorBlock) => boolean): Command => (view) => {
  const sel = view.state.field(blockSelField, false)?.sel;
  if (!sel) return false;
  const head = blockAt(view.state, sel.head);
  if (!head) {
    exitTo(view, sel.head);
    return true;
  }
  return f(view, sel, head);
};

const escape: Command = (view) => {
  const sel = view.state.field(blockSelField, false)?.sel;
  if (sel) {
    const head = blockAt(view.state, sel.head);
    exitTo(view, head ? blockContentStart(view.state, head) : sel.head);
    return true;
  }
  // A completion popup (open or still being computed) owns Escape; and an IME
  // composition must not be turned into a block selection under the user's fingers
  if (completionStatus(view.state) != null || view.composing) return false;
  const main = view.state.selection.main;
  if (!main.empty) return false;
  if (!ensureParsed(view.state, Math.min(view.state.doc.length, main.head + 5000))) view.dispatch({});
  const b = blockAt(view.state, main.head);
  if (!b) return false;
  selectBlock(view, b);
  return true;
};

const enter = inMode((view, _sel, head) => {
  exitTo(view, head.to);
  return true;
});

const step = (dir: -1 | 1): Command =>
  inMode((view, _sel, head) => {
    const next = walk(view.state, head, dir);
    if (next) modeTx(view, { anchor: next.from, head: next.from });
    return true;
  });

const extend = (dir: -1 | 1): Command =>
  inMode((view, sel, head) => {
    const anchorBlock = blockAt(view.state, sel.anchor) ?? head;
    let cur: EditorBlock | null = head;
    let next: EditorBlock | null = null;
    // a sibling in that direction, else climb to the nearest ancestor that has one
    while (cur) {
      next = dir < 0 ? prevSibling(view.state, cur) : nextSibling(view.state, cur);
      if (next) break;
      cur = cur.parent;
    }
    if (!next || !cur) return true;
    const anchor = ancestorAtDepth(anchorBlock, next.depth);
    modeTx(view, { anchor: anchor.from, head: next.from });
    return true;
  });

function reselectAt(view: EditorView, pos: number, count = 1) {
  const b = blockAt(view.state, Math.min(pos, view.state.doc.length));
  if (!b) {
    exitTo(view, Math.min(pos, view.state.doc.length));
    return;
  }
  let head = b;
  for (let i = 1; i < count; i++) {
    const n = nextSibling(view.state, head);
    if (!n) break;
    head = n;
  }
  modeTx(view, { anchor: b.from, head: head.from });
}

const remove: Command = inMode((view) => {
  const run = selectedBlocks(view.state);
  const spec = deleteBlocks(view.state, run);
  if (!spec) return true;
  view.dispatch(spec);
  reselectAt(view, view.state.selection.main.head);
  return true;
});

const duplicate: Command = inMode((view) => {
  const run = selectedBlocks(view.state);
  const spec = duplicateBlocks(view.state, run);
  if (!spec) return true;
  view.dispatch(spec);
  reselectAt(view, view.state.selection.main.head, run.length);
  return true;
});

const move = (dir: -1 | 1): Command =>
  inMode((view) => {
    const run = selectedBlocks(view.state);
    const spec = moveBlocks(view.state, run, dir);
    if (!spec) return true;
    view.dispatch(spec);
    reselectAt(view, view.state.selection.main.head, run.length);
    return true;
  });

const shift = (dir: -1 | 1): Command =>
  inMode((view) => {
    const run = selectedBlocks(view.state);
    // last to first: each edit lies after the earlier blocks, whose positions stay valid
    for (const b of [...run].reverse()) {
      const spec = dir > 0 ? indentBlock(view.state, b) : outdentBlock(view.state, b);
      if (spec) view.dispatch(spec);
    }
    const first = blockAt(view.state, run[0]?.from ?? 0);
    if (first) reselectAt(view, first.from, run.length);
    return true;
  });

const selectAll: Command = inMode((view) => {
  const top = topBlocks(view.state);
  if (top.length) modeTx(view, { anchor: top[0].from, head: top[top.length - 1].from }, { scroll: false });
  return true;
});

// ---- text-mode block keys

function withBlock(f: (view: EditorView, b: EditorBlock) => boolean): Command {
  return (view) => {
    if (isBlockMode(view.state)) return false;
    const head = view.state.selection.main.head;
    if (!ensureParsed(view.state, Math.min(view.state.doc.length, head + 5000))) view.dispatch({});
    const b = blockAt(view.state, head);
    if (!b) return false;
    return f(view, b);
  };
}

const textMove = (dir: -1 | 1): Command =>
  withBlock((view, b) => {
    const spec = moveBlocks(view.state, [b], dir);
    if (spec) view.dispatch(spec);
    return true;
  });

const textDuplicate: Command = withBlock((view, b) => {
  const spec = duplicateBlocks(view.state, [b]);
  if (spec) view.dispatch(spec);
  return true;
});

const handlers = Prec.highest(
  EditorView.domEventHandlers({
    mousedown(e, view) {
      const sel = view.state.field(blockSelField, false)?.sel;
      if (!sel) return false;
      if (e.shiftKey && e.button === 0) {
        const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
        const b = pos == null ? null : blockAt(view.state, pos);
        if (b) {
          const anchorBlock = blockAt(view.state, sel.anchor) ?? b;
          const depth = Math.min(anchorBlock.depth, b.depth);
          modeTx(view, { anchor: ancestorAtDepth(anchorBlock, depth).from, head: ancestorAtDepth(b, depth).from }, { scroll: false });
          e.preventDefault();
          return true;
        }
      }
      view.dispatch({ effects: setBlockSel.of(null) });
      return false;
    },
    copy(e, view) {
      if (!isBlockMode(view.state)) return false;
      e.clipboardData?.setData("text/plain", copyBlocksText(view.state, selectedBlocks(view.state)));
      e.preventDefault();
      return true;
    },
    cut(e, view) {
      if (!isBlockMode(view.state)) return false;
      const run = selectedBlocks(view.state);
      e.clipboardData?.setData("text/plain", copyBlocksText(view.state, run));
      e.preventDefault();
      const spec = deleteBlocks(view.state, run);
      if (spec) {
        view.dispatch(spec);
        reselectAt(view, view.state.selection.main.head);
      }
      return true;
    },
    // An IME starts with a keydown whose keyCode is 229, before any composition
    // event: that is the last moment the cursor may be moved safely, so block mode
    // ends here (at the block's end, the same place a typed character lands)
    keydown(e, view) {
      if (e.keyCode !== 229 && !e.isComposing) return false;
      const head = headBlock(view);
      if (head) exitTo(view, head.to);
      return false;
    },
    // Still in block mode when the composition starts (a path without the 229
    // keydown): only drop the mode. Moving the selection now would pull the DOM out
    // from under the composition, after which WebKit stops delivering input
    compositionstart(_e, view) {
      if (isBlockMode(view.state)) view.dispatch({ effects: setBlockSel.of(null), annotations: blockSelTx.of(true) });
      return false;
    },
  }),
);

const input = EditorView.inputHandler.of((view, _from, _to, text) => {
  // Never rewrite a composition's text: the DOM already holds it
  if (view.composing) return false;
  const head = headBlock(view);
  if (!head) return false;
  const at = head.to;
  view.dispatch({
    changes: { from: at, insert: text },
    selection: EditorSelection.cursor(at + text.length),
    effects: setBlockSel.of(null),
    annotations: blockSelTx.of(true),
  });
  return true;
});

const attrs = EditorView.editorAttributes.of((view) => (isBlockMode(view.state) ? { class: "cm-blocksel-active" } : null));

/** The whole block-mode extension plus the text-mode block keys. */
export function blockSelection(): Extension {
  return [
    blockSelField,
    attrs,
    handlers,
    input,
    Prec.highest(
      keymap.of([
        { key: "Escape", run: escape },
        { key: "Enter", run: enter },
        { key: "ArrowUp", run: step(-1) },
        { key: "ArrowDown", run: step(1) },
        { key: "Shift-ArrowUp", run: extend(-1) },
        { key: "Shift-ArrowDown", run: extend(1) },
        { key: "Backspace", run: remove },
        { key: "Delete", run: remove },
        { key: "Mod-d", run: duplicate },
        { key: "Alt-ArrowUp", run: move(-1) },
        { key: "Alt-ArrowDown", run: move(1) },
        { key: "Tab", run: shift(1) },
        { key: "Shift-Tab", run: shift(-1) },
        { key: "Mod-a", run: selectAll },
        { key: "Mod-Shift-ArrowUp", run: move(-1) },
        { key: "Mod-Shift-ArrowDown", run: move(1) },
        { key: "Mod-Shift-d", run: duplicate },
        { key: "Mod-Shift-ArrowUp", run: textMove(-1) },
        { key: "Mod-Shift-ArrowDown", run: textMove(1) },
        { key: "Mod-Shift-d", run: textDuplicate },
      ]),
    ),
  ];
}
