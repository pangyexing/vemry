// A single click never becomes a range selection, and never moves the document.
//
// CodeMirror computes a pointer selection's anchor at mousedown and its head from whatever
// lies under the pointer at each later mousemove, with no movement threshold once the button
// is down. Between the two the layout can move under a stationary pointer, and WebKit then
// delivers a mousemove without any motion. Two ways it moved, both traced live:
//
// - the line just clicked reveals its markup (livepreview), and the text shifts sideways by
//   the width of the markers — the click ended as a one-character range, backwards;
// - the editor was not focused (a click on a button or a menu blurs it and focuses nothing,
//   leaving the native selection inside), a document switch then made that selection stale,
//   and CodeMirror's focus() on the next mousedown had WebKit reveal it — a jump of hundreds
//   of pixels inside the mousedown, and a click that ended as a fourteen-line range from the
//   pressed position to what the jump put under the pointer.
//
// Two guards, both narrow: the native caret is parked at the pressed point before CodeMirror
// focuses (parkNativeCaret; focusEditor does the same for a focus that comes from a menu or a
// command rather than a click), and while the pointer has not travelled the drag threshold since
// a plain single click, a pointer selection that is not empty is collapsed back to its anchor
// (the pressed position). A double or triple click (a word, a line), a shift-click and a real
// drag are untouched.
import { EditorSelection, EditorState, Prec, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

/** The pointer travel that turns a press into a drag: the one threshold every press-to-drag
 *  gesture in the app reads (the block grip, the file tree, the board, the tab bar). */
export const DRAG_THRESHOLD = 5;

/** Before the editor is focused, put the native caret at document position `pos`. WebKit
 *  reveals an editable element's own selection when it is focused, whatever preventScroll
 *  says, and after a document switch that selection is stale: the text node it sat in is
 *  gone, so it has collapsed to a child index of the content element, which in the new
 *  document is some line far down. Revealing that scrolled the document by hundreds of
 *  pixels. A caret at a position already in view gives the reveal nothing to do; CodeMirror
 *  sets its own position again right after. */
function parkNativeCaret(view: EditorView, pos: number) {
  const sel = view.contentDOM.ownerDocument.getSelection();
  if (!sel) return;
  const { node, offset } = view.domAtPos(pos);
  try {
    sel.collapse(node, offset);
  } catch {
    /* a detached node: leave it to CodeMirror */
  }
}

/** Focus the editor from outside a pointer event (a menu entry, a palette command, a jump
 *  from another view). The same stale-selection reveal as the click case, traced live on the
 *  block menu's "copy link": the first copy after opening a note scrolled the document down,
 *  since the entry's focus() had WebKit show a caret left far below from the previous
 *  document. The native caret is parked first at `pos` — the place the user is working, which
 *  is in view — or, by default, at the selection head: for a menu entry the head is inside
 *  the block the menu was opened on. A caller whose head may be far from that place (a field
 *  chip's menu, on a note opened and scrolled but never clicked into: the head is still at
 *  the top) passes the position. An editor that already has focus needs nothing. */
export function focusEditor(view: EditorView, pos = view.state.selection.main.head) {
  if (!view.hasFocus) parkNativeCaret(view, pos);
  view.focus();
}

export function clickGuard(): Extension {
  /** The current plain single click: where it pressed, and whether the pointer has since travelled the threshold. */
  let press: { x: number; y: number; moved: boolean } | null = null;
  // The travel is watched on the document for the length of the press, the way CodeMirror's
  // own mouse selection does: a drag leaves the content area at once when it starts near an
  // edge, and a listener on the editor alone would miss it and collapse a real drag
  const onMove = (e: MouseEvent) => {
    if (press && !press.moved && Math.hypot(e.clientX - press.x, e.clientY - press.y) >= DRAG_THRESHOLD) press.moved = true;
  };
  const onUp = () => {
    press = null;
    document.removeEventListener("mousemove", onMove);
    document.removeEventListener("mouseup", onUp);
  };
  return [
    Prec.highest(
      EditorView.domEventHandlers({
        mousedown(e, view) {
          // A shift-click extends from the selection CodeMirror holds, which parking would replace
          if (!view.hasFocus && !e.shiftKey) {
            const pos = view.posAtCoords({ x: e.clientX, y: e.clientY });
            if (pos != null) parkNativeCaret(view, pos);
          }
          onUp();
          if (e.button !== 0 || e.shiftKey || e.detail !== 1) return false;
          press = { x: e.clientX, y: e.clientY, moved: false };
          document.addEventListener("mousemove", onMove);
          document.addEventListener("mouseup", onUp);
          return false;
        },
      }),
    ),
    EditorState.transactionFilter.of((tr) => {
      if (!press || press.moved || !tr.selection || !tr.isUserEvent("select.pointer")) return tr;
      const main = tr.newSelection.main;
      if (main.empty) return tr;
      return [tr, { selection: EditorSelection.cursor(main.anchor), userEvent: "select.pointer" }];
    }),
  ];
}
