// Drag a block by its grip. The same pointer contract as the file tree, the
// board and the tab bar: pointer capture on the pressed element, a 5px
// threshold, a ghost that intercepts no events, Escape cancels, and the click
// that follows a drag is swallowed. The drop target is computed per frame from
// the height map: the boundary before or after the block under the pointer
// (by that block's own vertical middle), and a depth from the horizontal
// travel — one level per 24px, only for list items, at most one deeper than
// the block above the boundary. The drop is one `moveBlockTo` transaction.
import { StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView } from "@codemirror/view";
import { type EditorBlock, blockAt, extentTo, frontmatterEnd } from "./blocks";
import { CONTENT_PAD } from "./blockhandle";
import { moveBlockTo } from "./blockops";
import { selectBlock } from "./blocksel";
import { DRAG_THRESHOLD, focusEditor } from "./clickguard";

const DEPTH_PX = 24;
const EDGE = 40;
const SCROLL_STEP = 8;

/** The lines of the block being dragged are dimmed while the drag lasts. */
export const setDragSource = StateEffect.define<{ from: number; to: number } | null>();
const dragLine = Decoration.line({ class: "cm-block-dragging" });

export const dragSourceField = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(value, tr) {
    let v = tr.docChanged ? value.map(tr.changes) : value;
    for (const e of tr.effects) {
      if (!e.is(setDragSource)) continue;
      if (!e.value) {
        v = Decoration.none;
        continue;
      }
      const doc = tr.state.doc;
      const ranges = [];
      for (let n = doc.lineAt(e.value.from).number; n <= doc.lineAt(e.value.to).number; n++) ranges.push(dragLine.range(doc.line(n).from));
      v = Decoration.set(ranges, true);
    }
    return v;
  },
  provide: (f) => EditorView.decorations.from(f),
});

interface Drop {
  /** Insert before the block starting here; null = the end of the document. */
  targetPos: number | null;
  /** Indentation columns for a list item at the drop. */
  indentCols: number;
  depth: number;
  /** y of the indicator bar in scroller coordinates. */
  barTop: number;
}

const LIST_KINDS = ["list_item", "task"];

/** The block whose subtree ends on the non-blank line right above `pos`. */
function blockBefore(view: EditorView, pos: number): EditorBlock | null {
  const doc = view.state.doc;
  let line = doc.lineAt(pos);
  while (line.number > 1) {
    line = doc.line(line.number - 1);
    if (line.text.trim() !== "") return blockAt(view.state, line.to);
  }
  return null;
}

function computeDrop(view: EditorView, source: EditorBlock, x: number, y: number, pressX: number): Drop | null {
  const state = view.state;
  const doc = state.doc;
  const yDoc = Math.max(0, Math.min(view.contentHeight, y - view.documentTop));
  const lb = view.lineBlockAtHeight(yDoc);
  const line = doc.lineAt(lb.from);
  const fmEnd = frontmatterEnd(doc);
  let targetPos: number | null;
  let prev: EditorBlock | null;
  let barTop: number;
  const hovered = line.text.trim() === "" ? null : blockAt(state, Math.min(line.to, lb.to));
  if (!hovered) {
    targetPos = line.from;
    prev = blockBefore(view, targetPos);
    barTop = lb.top;
  } else {
    const ownTop = view.lineBlockAt(hovered.from).top;
    const ownBottom = view.lineBlockAt(hovered.to).bottom;
    if (yDoc < (ownTop + ownBottom) / 2) {
      targetPos = doc.lineAt(hovered.from).from;
      prev = blockBefore(view, targetPos);
      barTop = ownTop;
    } else {
      const after = doc.lineAt(hovered.to).to + 1;
      targetPos = after > doc.length ? null : after;
      prev = hovered;
      barTop = ownBottom;
    }
  }
  if (targetPos != null && targetPos < fmEnd) targetPos = fmEnd;
  const srcFrom = source.from;
  const srcTo = extentTo(state, source);
  if (targetPos != null && targetPos > srcFrom && targetPos <= srcTo + 1) return null;
  if (targetPos != null && targetPos === srcFrom) return null;
  // depth: list items only, relative to the block above the boundary
  let depth = 0;
  let indentCols = 0;
  if (LIST_KINDS.includes(source.kind)) {
    const maxDepth = prev && LIST_KINDS.includes(prev.kind) ? prev.depth + 1 : 0;
    depth = Math.max(0, Math.min(maxDepth, source.depth + Math.round((x - pressX) / DEPTH_PX)));
    if (depth > 0 && prev) {
      // the content column of the item at depth-1 above the boundary
      let anchor: EditorBlock | null = prev;
      while (anchor && anchor.depth > depth - 1) anchor = anchor.parent;
      indentCols = anchor?.contentCol ?? depth * 2;
    }
  }
  return { targetPos, indentCols, depth, barTop };
}

/** Start a drag from the grip's pointerdown. `done` runs after a drop or a
 *  cancel, with whether a drop happened (the handle swallows the click that
 *  follows a drag). */
export function startBlockDrag(
  view: EditorView,
  block: EditorBlock,
  e: PointerEvent,
  grip: HTMLElement,
  done: (dropped: boolean) => void,
): void {
  const pressX = e.clientX;
  const pressY = e.clientY;
  const startDoc = view.state.doc;
  let dragging = false;
  let ghost: HTMLElement | null = null;
  let bar: HTMLElement | null = null;
  let x = pressX;
  let y = pressY;
  let raf = 0;
  let drop: Drop | null = null;
  grip.setPointerCapture(e.pointerId);

  const begin = () => {
    dragging = true;
    view.dispatch({ effects: setDragSource.of({ from: block.from, to: extentTo(view.state, block) }) });
    view.dom.classList.add("cm-block-drag");
    ghost = document.createElement("div");
    ghost.className = "cm-block-ghost";
    const first = view.state.doc.lineAt(block.from).text.replace(/^\s*(?:[-*+]|\d+[.)])\s+(?:\[[ xX/]\]\s+)?|^#{1,6}\s+|^\s*>\s*/, "").trim();
    ghost.textContent = first.length > 40 ? first.slice(0, 40) + "…" : first || "…";
    document.body.appendChild(ghost);
    bar = document.createElement("div");
    bar.className = "cm-block-drop";
    bar.hidden = true;
    view.scrollDOM.appendChild(bar);
    tick();
  };

  const tick = () => {
    raf = 0;
    if (!dragging) return;
    if (ghost) {
      ghost.style.left = `${x + 12}px`;
      ghost.style.top = `${y + 10}px`;
    }
    const base = view.scrollDOM.getBoundingClientRect();
    // autoscroll near the edges, then recompute for the moved content
    if (y < base.top + EDGE) view.scrollDOM.scrollTop -= SCROLL_STEP;
    else if (y > base.bottom - EDGE) view.scrollDOM.scrollTop += SCROLL_STEP;
    drop = view.state.doc === startDoc ? computeDrop(view, block, x, y, pressX) : null;
    if (bar) {
      if (drop) {
        const content = view.contentDOM.getBoundingClientRect();
        const textLeft = content.left - base.left + view.scrollDOM.scrollLeft + CONTENT_PAD;
        const width = Math.max(40, content.width - CONTENT_PAD * 2 - drop.depth * DEPTH_PX);
        bar.hidden = false;
        bar.style.top = `${drop.barTop - 1}px`;
        bar.style.left = `${textLeft + drop.depth * DEPTH_PX}px`;
        bar.style.width = `${width}px`;
      } else {
        bar.hidden = true;
      }
    }
    if (y < base.top + EDGE || y > base.bottom - EDGE) raf = requestAnimationFrame(tick);
  };

  const onMove = (ev: PointerEvent) => {
    x = ev.clientX;
    y = ev.clientY;
    if (!dragging) {
      if (Math.hypot(x - pressX, y - pressY) < DRAG_THRESHOLD) return;
      begin();
    }
    ev.preventDefault();
    if (!raf) raf = requestAnimationFrame(tick);
  };

  const cleanup = () => {
    grip.removeEventListener("pointermove", onMove);
    grip.removeEventListener("pointerup", onUp);
    grip.removeEventListener("pointercancel", onCancel);
    window.removeEventListener("keydown", onKey, true);
    if (raf) cancelAnimationFrame(raf);
    ghost?.remove();
    bar?.remove();
    view.dom.classList.remove("cm-block-drag");
    if (dragging) view.dispatch({ effects: setDragSource.of(null) });
    try {
      grip.releasePointerCapture(e.pointerId);
    } catch {
      // already released
    }
  };

  const onUp = () => {
    const wasDragging = dragging;
    const target = drop;
    cleanup();
    if (!wasDragging) {
      done(false);
      return;
    }
    if (target && view.state.doc === startDoc) {
      const spec = moveBlockTo(view.state, block, target.targetPos, target.indentCols);
      if (spec) {
        view.dispatch(spec);
        const moved = blockAt(view.state, view.state.selection.main.head);
        if (moved) selectBlock(view, moved);
      }
    }
    focusEditor(view);
    done(true);
  };

  const onCancel = () => {
    const wasDragging = dragging;
    cleanup();
    done(wasDragging);
  };

  const onKey = (ev: KeyboardEvent) => {
    if (ev.key !== "Escape") return;
    ev.stopPropagation();
    ev.preventDefault();
    onCancel();
  };

  grip.addEventListener("pointermove", onMove);
  grip.addEventListener("pointerup", onUp);
  grip.addEventListener("pointercancel", onCancel);
  window.addEventListener("keydown", onKey, true);
}
