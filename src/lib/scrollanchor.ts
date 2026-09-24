// Keeping the text in a scrolling list still while the list changes under it. The reader
// did not ask for a background event (an autosave, an index update, an AI write, an edit
// elsewhere) to move what they are reading, so a view that re-renders on one pins the
// first row in view to where it was, the way a browser's scroll anchoring does for a page.

/** The rows an anchor may be taken from: any element carrying a stable key. A view marks
 *  its rows with `data-anchor="<stable id>"` (a task id, a note path), never an index or a
 *  byte offset, since those change when the list does. */
const ROW = "[data-anchor]";

export interface ScrollAnchor {
  key: string;
  /** The row's top edge relative to the container's top edge, in px. */
  offset: number;
}

/** The first keyed row whose bottom edge is below the container's top: the row the reader's
 *  eye is on. Null when the container is scrolled to the top — there the list is allowed to
 *  grow from the top edge (new entries appear where the reader is looking for them). */
export function captureAnchor(container: HTMLElement): ScrollAnchor | null {
  if (container.scrollTop <= 0) return null;
  const top = container.getBoundingClientRect().top;
  for (const el of container.querySelectorAll<HTMLElement>(ROW)) {
    const r = el.getBoundingClientRect();
    if (r.bottom > top) return { key: el.dataset.anchor!, offset: r.top - top };
  }
  return null;
}

/** Scroll so the anchored row sits at the offset it had; a row that no longer exists
 *  leaves the scroll position alone. */
export function restoreAnchor(container: HTMLElement, anchor: ScrollAnchor | null): void {
  if (!anchor) return;
  const el = container.querySelector<HTMLElement>(`${ROW}[data-anchor="${CSS.escape(anchor.key)}"]`);
  if (!el) return;
  const top = container.getBoundingClientRect().top;
  const drift = el.getBoundingClientRect().top - top - anchor.offset;
  if (Math.abs(drift) >= 0.5) container.scrollTop += drift;
}

/** Run a change to the list with the reader's row held in place: capture before, restore
 *  after the DOM has been updated (`settle` resolves once it has — Svelte's `tick`). */
export async function holdAnchor(container: HTMLElement | null | undefined, change: () => void | Promise<void>, settle: () => Promise<void>): Promise<void> {
  const anchor = container ? captureAnchor(container) : null;
  await change();
  await settle();
  if (container) restoreAnchor(container, anchor);
}
