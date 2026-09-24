// The root element of a block widget (an embed, a query block, a run block's
// output, the frontmatter strip). CodeMirror measures a block widget by its root's
// getBoundingClientRect, which excludes the root's own margins: a card with
// vertical margins leaves the height map that much shorter than the DOM, and the
// error accumulates down the note — measured on the fixture note, the frontmatter
// strip's 12px and an embed's 8px put every line below them 20px lower on screen
// than the height map says, so a click in the middle of a line's text resolved to
// the line after it (posAtCoords picks the line from the height map first). The
// card keeps its margins; this root is a block formatting context, so those
// margins sit inside its box and are measured with it.
//
// The live half: a query block and an embed show what the index says now, and the
// index moves under a note the reader is only reading (an autosave elsewhere, an AI
// write, a clip landing). They used to be rebuilt on every index version — a query
// collapsed to one "querying…" line and grew back, an embed blanked to "loading" —
// and in a note holding both, the reader's view jumped by the whole difference
// (574px, 8 times out of 8). Now the widget is equal to itself across versions, so
// CodeMirror keeps its DOM, and a refresh re-reads the content in the background
// (watchLive): the old DOM and its height stay until the new content is ready, an
// unchanged answer leaves the DOM untouched, and a changed one is swapped in one step
// with the text under the viewport's top edge held where it was (swapInPlace).
import { StateEffect, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

/** Wrap a block widget's card in the element CodeMirror measures. */
export function blockRoot(card: HTMLElement): HTMLElement {
  const root = document.createElement("div");
  root.className = "cm-block-root";
  root.appendChild(card);
  return root;
}

/** An index version change → every live widget re-reads its content in place (the
 *  editor dispatches it on vault.indexVersion; a stale span sends a fresh negative key). */
export const queryRefresh = StateEffect.define<number>();

type Refresh = (version: number) => void;

/** Per view, the mounted live widgets by root; per root, its view's table (so a widget's
 *  destroy, which is handed only the DOM, can take itself off). */
const liveByView = new WeakMap<EditorView, Map<HTMLElement, Refresh>>();
const tableOf = new WeakMap<HTMLElement, Map<HTMLElement, Refresh>>();

/** Refresh `root`'s content on every queryRefresh while it is mounted. */
export function watchLive(view: EditorView, root: HTMLElement, refresh: Refresh): void {
  let table = liveByView.get(view);
  if (!table) liveByView.set(view, (table = new Map()));
  table.set(root, refresh);
  tableOf.set(root, table);
}

/** A widget's destroy: stop refreshing this root. */
export function unwatchLive(root: HTMLElement): void {
  tableOf.get(root)?.delete(root);
  tableOf.delete(root);
}

/** The listener behind watchLive (both live extensions include it; CodeMirror keeps one
 *  copy of an extension value it is handed twice). A root that has left the document
 *  without a destroy — the whole state replaced — is dropped on the way. */
export const liveRefresh: Extension = EditorView.updateListener.of((u) => {
  let version: number | null = null;
  for (const tr of u.transactions) for (const e of tr.effects) if (e.is(queryRefresh)) version = e.value;
  if (version === null) return;
  const table = liveByView.get(u.view);
  if (!table) return;
  for (const [root, refresh] of [...table]) {
    if (!root.isConnected) {
      unwatchLive(root);
      continue;
    }
    refresh(version);
  }
});

/** The line the reader's eye is on, held for one frame: the first line or block of the
 *  document whose top edge is at or below the viewport's top edge (a block cut by that
 *  edge is skipped — the reader is looking at what comes after it), and where it stood. */
interface Held {
  el: Element;
  top: number;
}
const held = new WeakMap<EditorView, Held>();

function readerLine(view: EditorView): Held | null {
  const top = view.scrollDOM.getBoundingClientRect().top;
  for (const el of view.contentDOM.children) {
    const r = el.getBoundingClientRect();
    if (r.height > 0 && r.top >= top - 0.5) return { el, top: r.top };
  }
  return null;
}

/** Swap a mounted block widget's content in one step (`apply`), keeping the line at the
 *  top of the viewport where it is.
 *  The first swap of a frame notes that line and where it stands; once CodeMirror has
 *  measured the new heights and applied its own scroll anchoring (its measure runs first
 *  in the frame: it is scheduled before the check), the line's drift, whatever caused it,
 *  goes back into scrollTop before the frame is painted. Several widgets changing at once
 *  (every embed of one source, a refresh answered together) share the one correction.
 *  Measuring the outcome rather than predicting CodeMirror's anchor is what makes this
 *  hold: its anchor sits 8px below its own idea of the viewport's top and is read from a
 *  height map that an earlier swap in the same frame has already made stale, and a
 *  correction computed from either compensated twice. */
export function swapInPlace(view: EditorView, root: HTMLElement, apply: () => void): void {
  if (root.isConnected && !held.has(view)) {
    const line = readerLine(view);
    if (line) {
      held.set(view, line);
      view.requestMeasure();
      requestAnimationFrame(() => {
        held.delete(view);
        if (!line.el.isConnected) return;
        const drift = line.el.getBoundingClientRect().top - line.top;
        if (Math.abs(drift) >= 0.5) view.scrollDOM.scrollTop += drift;
      });
    }
  }
  apply();
  view.requestMeasure();
}

/** Remember a widget root's measured height (the estimate its next mount starts from),
 *  once the current measure pass has laid it out. */
export function recordHeight(view: EditorView, root: HTMLElement, into: Map<string, number>, key: string): void {
  view.requestMeasure({
    read: () => (root.isConnected ? root.offsetHeight : 0),
    write: (h) => {
      if (!h) return;
      // a bounded memory: one entry per widget the reader has seen, the oldest dropped first
      into.delete(key);
      into.set(key, h);
      if (into.size > 500) into.delete(into.keys().next().value!);
    },
  });
}

/** Make a region of a widget a place where text can be selected: the press focuses the
 *  region itself rather than the editor, so CodeMirror neither reads the browser's
 *  selection back as its cursor (which beside the widget would reveal the source under
 *  the pointer) nor redraws its own selection over the reader's. */
export function selectableIsland(el: HTMLElement): void {
  if (!el.hasAttribute("tabindex")) el.tabIndex = -1;
  el.classList.add("cm-select-island");
}

/** A press that ended as a selection of words inside `el` (a drag, a double click):
 *  not a click that opens something. */
export function selectedWithin(el: HTMLElement): boolean {
  const sel = el.ownerDocument.getSelection();
  return !!sel && !sel.isCollapsed && !!sel.anchorNode && el.contains(sel.anchorNode);
}
