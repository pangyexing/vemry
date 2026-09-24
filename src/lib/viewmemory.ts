// A projection view keeps its place when the reader leaves it and comes back. The views are
// unmounted when another tab takes the main area, and kept that way on purpose: a mounted
// view re-queries on every vault event (an autosave a second while typing), the graph's
// force simulation reheats on each one, and a view registers itself as the AI's current
// view context while mounted. So instead of keeping them alive, the place is written down
// as the view goes and put back as it returns: every scrolled container (by its position
// in the view's tree, with the first keyed row in sight when the view marks rows with
// data-anchor) and the open state of every <details>.
//
// A returning view loads its rows again, asynchronously, so the place is put back as the
// content arrives — each container as soon as it is tall enough — and never after the
// reader has scrolled, clicked or typed in it themselves, nor later than a moment after.
import { captureAnchor, restoreAnchor, type ScrollAnchor } from "$lib/scrollanchor";

interface ScrollMark {
  /** The child indices from the view's host down to the container. */
  path: number[];
  /** The container's tag and first class: a check that the path still names the same kind of element. */
  sig: string;
  top: number;
  left: number;
  anchor: ScrollAnchor | null;
}

export interface ViewPlace {
  scrolls: ScrollMark[];
  /** The open state of every <details>, in document order. */
  details: boolean[];
}

/** How long a returning view may take to load before its place is given up. */
const SETTLE_MS = 2000;

const sigOf = (el: Element) => `${el.tagName}.${el.classList[0] ?? ""}`;

function pathOf(host: Element, el: Element): number[] {
  const path: number[] = [];
  for (let n: Element | null = el; n && n !== host; n = n.parentElement) {
    path.unshift(Array.prototype.indexOf.call(n.parentElement?.children ?? [], n));
  }
  return path;
}

function resolve(host: Element, path: number[]): HTMLElement | null {
  let n: Element | undefined = host;
  for (const i of path) n = n?.children[i];
  return n instanceof HTMLElement ? n : null;
}

/** Write down where the reader is in the view under `host` (called while it is still on screen). */
export function capturePlace(host: HTMLElement): ViewPlace {
  const scrolls: ScrollMark[] = [];
  for (const el of host.querySelectorAll<HTMLElement>("*")) {
    if (el.scrollTop <= 0 && el.scrollLeft <= 0) continue;
    if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) continue;
    scrolls.push({ path: pathOf(host, el), sig: sigOf(el), top: el.scrollTop, left: el.scrollLeft, anchor: captureAnchor(el) });
  }
  const details = [...host.querySelectorAll("details")].map((d) => d.open);
  return { scrolls, details };
}

/** Put a place back into the view mounting under `host` as its content arrives. Returns
 *  the function that stops trying (the caller runs it when the view goes again). */
export function restorePlace(host: HTMLElement, place: ViewPlace): () => void {
  const pending = new Set(place.scrolls);
  let detailsDone = place.details.length === 0;
  let frame = 0;
  let stopped = false;

  const attempt = () => {
    frame = 0;
    if (!detailsDone) {
      // Only once the view has drawn the same set of sections: a different count means the
      // content changed shape while away, and the view's own defaults are then the better guess
      const ds = host.querySelectorAll("details");
      if (ds.length === place.details.length) {
        ds.forEach((d, i) => (d.open = place.details[i]));
        detailsDone = true;
      }
    }
    for (const s of pending) {
      const el = resolve(host, s.path);
      if (!el || sigOf(el) !== s.sig) continue;
      // Not tall (or wide) enough yet: the rows are still arriving
      if (el.scrollHeight - el.clientHeight < s.top - 1 || el.scrollWidth - el.clientWidth < s.left - 1) continue;
      el.scrollTop = s.top;
      el.scrollLeft = s.left;
      // The rows may have changed while away: the row the reader was on wins over the pixel offset
      restoreAnchor(el, s.anchor);
      pending.delete(s);
    }
    if (detailsDone && pending.size === 0) stop();
  };
  const schedule = () => {
    if (!stopped && !frame) frame = requestAnimationFrame(attempt);
  };

  const observer = new MutationObserver(schedule);
  observer.observe(host, { childList: true, subtree: true });
  const timer = setTimeout(stop, SETTLE_MS);
  // The reader's own move ends it: from then on the place is theirs
  const takeover = ["wheel", "pointerdown", "keydown"] as const;
  for (const type of takeover) host.addEventListener(type, stop, { capture: true, passive: true });

  function stop() {
    if (stopped) return;
    stopped = true;
    observer.disconnect();
    clearTimeout(timer);
    if (frame) cancelAnimationFrame(frame);
    for (const type of takeover) host.removeEventListener(type, stop, { capture: true });
  }

  schedule();
  return stop;
}
