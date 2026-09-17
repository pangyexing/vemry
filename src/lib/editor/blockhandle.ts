// The hover block handle: one grip (⋮⋮) that follows the block under the
// pointer, sitting in the editor's left padding, left of the fold chevron's
// slot (fold.ts) so the two never overlap on a heading. It lives in scrollDOM (like
// CodeMirror's own selection layers), so it scrolls with the content and never
// overlays the dialogs, menus or the lightbox above the editor. Resolution
// costs no layout: `lineBlockAtHeight` is a height-map lookup, and
// `coordsAtPos` runs once per hovered-block change. Clicking the grip selects
// the block and opens the block menu; the drag batch hooks the grip's
// pointerdown. Touch pointers are ignored.
//
// While the grip is shown, the fold chevron on the block's first line (fold.ts)
// carries a `cm-fold-hover` class so it stays visible when the pointer is on
// the grip and the line itself is no longer hovered. A DOM class rather than a
// line decoration: a transaction per hovered block would wake every plugin that
// watches effects. It goes on the chevron, not the line: CodeMirror's mutation
// observer ignores changes inside a widget's DOM, whereas a foreign attribute
// on a line marks it dirty and the next sync strips it. A re-created widget
// leaves the old element detached, and the next pointer move re-applies it.
import { EditorView, ViewPlugin, type ViewUpdate } from "@codemirror/view";
import { type EditorBlock, blockAt, frontmatterEnd } from "./blocks";
import { selectBlock } from "./blocksel";

export interface BlockHost {
  /** The grip was clicked (left or right): open the block menu for `block`. */
  openMenu(view: EditorView, block: EditorBlock, e: MouseEvent): void;
  /** The grip was pressed: a drag may follow; `done(dropped)` runs when it ends. */
  onGripPointerDown?(view: EditorView, block: EditorBlock, e: PointerEvent, grip: HTMLElement, done: (dropped: boolean) => void): void;
  /** Tooltip text for the grip. */
  gripTip(): string;
}

const HANDLE_W = 18;
const HANDLE_H = 22;
/** .cm-content's horizontal padding (app.css); the drop bar in blockdrag.ts uses it too. */
export const CONTENT_PAD = 34;
/** How far the fold chevron (fold.ts) hangs into the padding from the line's text: its
 *  -20px margin less CodeMirror's 6px line padding. The grip stops where it begins. */
const FOLD_TOGGLE_HANG = 20 - 6;

/** The fold chevron on the line at pos; null when the line is not foldable or not in the DOM. */
function foldToggleAt(view: EditorView, pos: number): HTMLElement | null {
  const { node } = view.domAtPos(pos);
  const el = node instanceof HTMLElement ? node : node.parentElement;
  return el?.closest(".cm-line")?.querySelector<HTMLElement>(".cm-fold-toggle") ?? null;
}

/** The innermost block at a viewport y coordinate, via the height map (no layout read). */
export function hoveredBlockAt(view: EditorView, clientY: number): EditorBlock | null {
  const y = Math.max(0, Math.min(view.contentHeight, clientY - view.documentTop));
  const lb = view.lineBlockAtHeight(y);
  const line = view.state.doc.lineAt(lb.from);
  if (line.text.trim() === "") return null;
  return blockAt(view.state, Math.min(line.to, lb.to));
}

class HandlePlugin {
  handle: HTMLElement;
  grip: HTMLElement;
  current: EditorBlock | null = null;
  shown = false;
  muted = false;
  /** The chevron carrying cm-fold-hover (null: none on that line), and the block start it was looked up for. */
  marked: HTMLElement | null = null;
  markedFrom = -1;
  showTimer: ReturnType<typeof setTimeout> | null = null;
  raf = 0;
  lastX = -1;
  lastY = -1;
  suppressClick = false;
  private readonly onMove = (e: MouseEvent) => this.mouseMove(e);
  private readonly onLeave = () => this.hide();
  private readonly onKey = () => {
    this.muted = true;
    this.hide();
  };
  private readonly onClick = (e: MouseEvent) => this.gripClick(e);
  private readonly onContext = (e: MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    this.gripClick(e);
  };
  private readonly onDown = (e: PointerEvent) => {
    if (e.button !== 0 || e.pointerType === "touch") return;
    e.preventDefault();
    if (this.current)
      this.host.onGripPointerDown?.(this.view, this.current, e, this.grip, (dropped) => {
        // the click that follows a drag must not open the menu
        this.suppressClick = dropped;
      });
  };

  constructor(
    readonly view: EditorView,
    readonly host: BlockHost,
  ) {
    this.handle = document.createElement("div");
    this.handle.className = "cm-block-handle cm-hidden";
    this.grip = document.createElement("button");
    this.grip.className = "cm-block-grip";
    this.grip.tabIndex = -1;
    this.grip.setAttribute("aria-label", host.gripTip());
    this.grip.dataset.tip = host.gripTip();
    this.grip.innerHTML =
      '<svg viewBox="0 0 20 20" fill="currentColor" aria-hidden="true"><circle cx="7" cy="5" r="1.6"/><circle cx="13" cy="5" r="1.6"/><circle cx="7" cy="10" r="1.6"/><circle cx="13" cy="10" r="1.6"/><circle cx="7" cy="15" r="1.6"/><circle cx="13" cy="15" r="1.6"/></svg>';
    this.handle.appendChild(this.grip);
    view.scrollDOM.appendChild(this.handle);
    view.scrollDOM.addEventListener("mousemove", this.onMove);
    view.scrollDOM.addEventListener("mouseleave", this.onLeave);
    view.contentDOM.addEventListener("keydown", this.onKey);
    this.grip.addEventListener("click", this.onClick);
    this.grip.addEventListener("contextmenu", this.onContext);
    this.grip.addEventListener("pointerdown", this.onDown);
  }

  mouseMove(e: MouseEvent) {
    if (this.handle.contains(e.target as Node)) return;
    if (Math.abs(e.clientX - this.lastX) < 2 && Math.abs(e.clientY - this.lastY) < 2 && !this.muted) return;
    this.lastX = e.clientX;
    this.lastY = e.clientY;
    this.muted = false;
    if (this.raf) return;
    this.raf = requestAnimationFrame(() => {
      this.raf = 0;
      this.resolve(this.lastY);
    });
  }

  resolve(clientY: number) {
    const b = hoveredBlockAt(this.view, clientY);
    if (!b || b.from < frontmatterEnd(this.view.state.doc)) {
      this.hide();
      return;
    }
    if (this.current && this.current.from === b.from && this.shown) {
      this.mark();
      return;
    }
    this.current = b;
    this.place(b);
    if (this.shown) {
      this.mark();
    } else {
      if (this.showTimer) clearTimeout(this.showTimer);
      this.showTimer = setTimeout(() => {
        this.showTimer = null;
        if (this.current === b) {
          this.handle.classList.remove("cm-hidden");
          this.shown = true;
          this.mark();
        }
      }, 60);
    }
  }

  /** Put cm-fold-hover on the current block's chevron, or take it off when the grip is
   *  hidden. Nothing to do while the answer for this block is still current (a chevron
   *  that is still attached, or a block that has none). */
  mark() {
    const want = this.shown && this.current ? this.current.from : -1;
    if (want === this.markedFrom && (this.marked?.isConnected ?? true)) return;
    this.marked?.classList.remove("cm-fold-hover");
    this.marked = want >= 0 ? foldToggleAt(this.view, want) : null;
    this.marked?.classList.add("cm-fold-hover");
    this.markedFrom = want;
  }

  place(b: EditorBlock) {
    const view = this.view;
    const base = view.scrollDOM.getBoundingClientRect();
    const content = view.contentDOM.getBoundingClientRect();
    // The grip is centred on whatever stands at the block's first position: a character's
    // rect (the font's content area, centred in its line) or, on a task line, the checkbox
    // widget's box (15px, sitting on the text's centre). Centred, not top-aligned with a
    // clamp: both rects are shorter than the grip, and top-aligning put the grip's centre
    // 1.5px under a paragraph's and 3.5px under a task's — the task's visibly so beside
    // its checkbox
    const coords = view.coordsAtPos(b.from, 1);
    const lb = view.lineBlockAt(b.from);
    const top = coords ? coords.top - base.top + view.scrollDOM.scrollTop : lb.top;
    const lineH = coords ? coords.bottom - coords.top : lb.height;
    const left = content.left - base.left + view.scrollDOM.scrollLeft + CONTENT_PAD - FOLD_TOGGLE_HANG - HANDLE_W;
    this.handle.style.left = `${Math.max(0, left)}px`;
    this.handle.style.top = `${top + (lineH - HANDLE_H) / 2}px`;
  }

  hide() {
    if (this.showTimer) {
      clearTimeout(this.showTimer);
      this.showTimer = null;
    }
    if (this.shown) {
      this.handle.classList.add("cm-hidden");
      this.shown = false;
    }
    this.current = null;
    this.mark();
  }

  gripClick(e: MouseEvent) {
    if (this.suppressClick) {
      this.suppressClick = false;
      return;
    }
    if (!this.current) return;
    // re-resolve: the document may have changed since the handle was placed
    const b = blockAt(this.view.state, this.current.from);
    if (!b) return;
    selectBlock(this.view, b);
    this.host.openMenu(this.view, b, e);
  }

  update(u: ViewUpdate) {
    if (u.docChanged || u.geometryChanged || u.viewportChanged) this.hide();
  }

  destroy() {
    if (this.raf) cancelAnimationFrame(this.raf);
    if (this.showTimer) clearTimeout(this.showTimer);
    this.view.scrollDOM.removeEventListener("mousemove", this.onMove);
    this.view.scrollDOM.removeEventListener("mouseleave", this.onLeave);
    this.view.contentDOM.removeEventListener("keydown", this.onKey);
    this.marked?.classList.remove("cm-fold-hover");
    this.handle.remove();
  }
}

export const blockHandle = (host: BlockHost) => ViewPlugin.define((view) => new HandlePlugin(view, host));
