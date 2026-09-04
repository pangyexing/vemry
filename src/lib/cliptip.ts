// The full text of a clipped line, shown on hover. A native title would do the same, but
// its delay is the platform's (about a second) and cannot be shortened, and it also fires
// on text that was never clipped. This one shows after a quarter second, only when the
// element's text really overflows it (an ellipsis or a line clamp), as one shared box
// appended to body: the views' own layouts are untouched and their scrolling containers
// cannot clip it.
import { clampToViewport } from "$lib/popmenu";

const DELAY = 250;
/** The gap between the element's bottom edge and the box; above the element it is the same gap upward. */
const GAP = 4;

let tip: HTMLDivElement | null = null;
let shownFor: HTMLElement | null = null;

function box(): HTMLDivElement {
  if (!tip) {
    tip = document.createElement("div");
    tip.className = "clip-tip";
    document.body.appendChild(tip);
  }
  return tip;
}

function hide() {
  if (tip) tip.hidden = true;
  shownFor = null;
}

/** Whether the element's own text overflows it: a single-line ellipsis grows scrollWidth,
 *  a line clamp grows scrollHeight; a pixel of slack covers subpixel rounding. */
const clipped = (el: HTMLElement) => el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1;

function show(el: HTMLElement, text: string) {
  const b = box();
  b.textContent = text;
  b.hidden = false;
  const r = el.getBoundingClientRect();
  // Below the element, then pulled inside the viewport; when below does not fit, above it
  const { x, y } = clampToViewport(b, r.left, r.bottom + GAP);
  const h = b.getBoundingClientRect().height;
  const top = r.bottom + GAP + h > window.innerHeight - GAP ? Math.max(GAP, r.top - GAP - h) : y;
  b.style.left = `${x}px`;
  b.style.top = `${top}px`;
  shownFor = el;
}

/** The Svelte action: `use:cliptip={text}`. The text is the element's full content, so the
 *  box reads what the element would have shown had it had the room. */
export function cliptip(node: HTMLElement, text: string) {
  let full = text;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const cancel = () => {
    if (timer) clearTimeout(timer);
    timer = null;
    if (shownFor === node) hide();
  };
  const enter = () => {
    cancel();
    timer = setTimeout(() => {
      timer = null;
      if (node.isConnected && clipped(node)) show(node, full);
    }, DELAY);
  };
  node.addEventListener("pointerenter", enter);
  node.addEventListener("pointerleave", cancel);
  // A press on the line (opening the task, a drag, a right-click) takes the box with it; so does any scroll or wheel, the box being fixed
  node.addEventListener("pointerdown", cancel);
  window.addEventListener("scroll", cancel, true);
  window.addEventListener("wheel", cancel, { passive: true });
  return {
    update(next: string) {
      full = next;
      if (shownFor === node) box().textContent = next;
    },
    destroy() {
      cancel();
      node.removeEventListener("pointerenter", enter);
      node.removeEventListener("pointerleave", cancel);
      node.removeEventListener("pointerdown", cancel);
      window.removeEventListener("scroll", cancel, true);
      window.removeEventListener("wheel", cancel);
    },
  };
}
