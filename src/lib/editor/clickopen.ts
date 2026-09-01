// The shared assembly of the link-click gesture, used by both wikilink and mdlink:
// - In the rendered state (the syntax is folded) an ordinary click opens; in the raw
//   state (the selection touches the link element and the syntax is expanded) a click
//   edits as usual — the click behaviour follows what is on screen, on the same
//   (element-level) verdict as livepreview's reveal.
// - On a hit in the folded state the mousedown is swallowed entirely (no cursor is
//   placed — a cursor inside a link would trigger reveal, so the link would burst
//   open into its source before jumping), and the click only really opens when it did
//   not drag far: button semantics, where dragging away after pressing cancels. The
//   price is that a drag-selection cannot start on a folded link (starting from the
//   adjacent text works), which matches a browser, where a link cannot start a
//   selection either.
// - ⌘/Ctrl+mousedown keeps the old behaviour: open at once, swallow the event, leave
//   the cursor alone. Where a plain click works, the modifier is promoted to invert
//   ("the other way of opening it", the same global meaning as in the projection and
//   the AI panel); where the plain click is off, the modifier is the only way to open
//   and follows the setting without inverting.
import { EditorView } from "@codemirror/view";
import type { Line } from "@codemirror/state";
import { inCode } from "./livepreview";
import { selectionReveals } from "./reveal";

export interface LinkHit {
  target: string;
  from: number;
  to: number;
  /** A `![](…)` image embed: in the rendered state it is an image widget and a click
   *  belongs to the widget itself (its copy button and so on), so only a modified
   *  click opens the target. */
  embed?: boolean;
}

/** A displacement between click and mousedown beyond this counts as a drag-selection and does not open */
const DRAG_PX = 4;

/** A global regex scan across a line, returning the match that spans pos (the
 *  boundary includes to). Shared by wikilink's and mdlink's `*LinkAt` — resetting
 *  lastIndex, the `from > pos` early exit and the closed-interval boundary are
 *  details that may exist only once. */
export function matchAt(line: Line, re: RegExp, pos: number): RegExpExecArray | null {
  re.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(line.text))) {
    const from = line.from + m.index;
    if (from > pos) return null;
    if (pos <= from + m[0].length) return m;
  }
  return null;
}

export function linkClickHandlers(
  find: (view: EditorView, pos: number) => LinkHit | null,
  open: (target: string, invert: boolean) => void,
  singleClick: () => boolean,
) {
  let pending: { target: string; x: number; y: number } | null = null;
  return EditorView.domEventHandlers({
    mousedown(event, view) {
      pending = null;
      if (event.button !== 0) return false;
      if (event.metaKey || event.ctrlKey) {
        const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
        if (pos == null) return false;
        const hit = find(view, pos);
        if (!hit) return false;
        open(hit.target, singleClick());
        event.preventDefault();
        return true;
      }
      if (!singleClick() || event.shiftKey || event.altKey) return false; // shift/alt are selection gestures
      // A block-level widget (a table, a frontmatter strip) has already dispatched the
      // cursor into the source text from its own mousedown, so posAtCoords now lands on
      // the source that just expanded — a click outside a .cm-line never counts as a
      // click on a link. The .cm-wikilink decoration spans inside a table cell are
      // excluded by this layer too (clicking a table means entering the cell to edit)
      // and must not be folded into the check below
      if (!(event.target as HTMLElement).closest?.(".cm-line")) return false;
      // A hit goes by what is visible: the click has to land on the DOM the link
      // really renders as (the .cm-wikilink marker span of the highlight or decoration,
      // or the ›/¶ badge of a wikilink anchor). posAtCoords folds a click on trailing
      // whitespace or leading indentation onto the nearest character position, and a
      // purely positional verdict would have "click at the end of the line to place the
      // cursor" open the link that ends it. ⌘click keeps the old purely positional
      // semantics and is not bound by this
      if (!(event.target as HTMLElement).closest(".cm-wikilink, .cm-lp-blockref, .cm-lp-linksep"))
        return false;
      // posAtCoords forces a layout read, so it comes after every cheap guard has
      // passed — this handler hangs off every mousedown (one for wikilink and one for
      // mdlink)
      const pos = view.posAtCoords({ x: event.clientX, y: event.clientY });
      if (pos == null) return false;
      if (inCode(view.state, pos)) return false; // link text inside code is a literal
      const hit = find(view, pos);
      if (!hit || hit.embed) return false;
      if (selectionReveals(view.state, hit.from, hit.to)) return false; // raw state = editing state
      pending = { target: hit.target, x: event.clientX, y: event.clientY };
      event.preventDefault(); // swallowed: no cursor enters the link, reveal does not fire, and the existing selection survives
      return true;
    },
    click(event) {
      const p = pending;
      pending = null;
      if (!p || event.button !== 0) return false;
      if (Math.abs(event.clientX - p.x) > DRAG_PX || Math.abs(event.clientY - p.y) > DRAG_PX) return false;
      open(p.target, false);
      event.preventDefault();
      return true;
    },
  });
}
