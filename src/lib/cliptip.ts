// The full text of a clipped line, shown on hover. A native title would do the same, but
// its delay is the platform's (about a second) and cannot be shortened, it also fires on
// text that was never clipped, and it pops below the pointer — over the next row, the one
// the eye goes to. This one shows after a quarter second, only when the element's text
// really overflows it (an ellipsis or a line clamp), as one shared box appended to body:
// the views' own layouts are untouched and their scrolling containers cannot clip it.
//
// The box lies over the clipped text itself, in the text's own font, its first line on the
// element's first line: the words run on to the right where the ellipsis was, and the eye
// stays where it was reading. Only the row being hovered is covered, never the one below
// or above — a single line grows to the right, over the rest of its own row; a clamped
// title is laid out wide enough to fit in the lines it already takes, and only a text
// longer than that grows down.
//
// Given `always`, the box shows whether or not anything was clipped: the full date behind
// a short date word, the name of what an icon stands for.
import { clampToViewport } from "$lib/popmenu";

const DELAY = 250;
/** The box's distance from the viewport's edges. */
const GAP = 4;
/** The box's padding: 2px above and below keeps a single line inside its row's own padding,
 *  6px on the sides sets the words off the box's edge. */
const PAD_Y = 2;
const PAD_X = 6;
const BORDER = 1;
/** The widest a box that has to wrap lays itself out, in the text's em. */
const WRAP_EM = 34;

let tip: HTMLDivElement | null = null;
let shownFor: HTMLElement | null = null;
/** The innermost element the pointer entered: a tip inside another tip's element (an icon
 *  in a meta line) wins over the line's own, whichever timer fires first. */
let armed: HTMLElement | null = null;

function hide() {
  if (tip) tip.hidden = true;
  shownFor = null;
}

function box(): HTMLDivElement {
  if (!tip) {
    tip = document.createElement("div");
    tip.className = "clip-tip";
    document.body.appendChild(tip);
    // Any scroll or wheel takes the box with it (it is fixed): one pair of listeners for
    // the one box, registered with it rather than per element — a board renders hundreds
    window.addEventListener("scroll", hide, true);
    window.addEventListener("wheel", hide, { passive: true });
  }
  return tip;
}

/** Whether the element's own text overflows it: a single-line ellipsis grows scrollWidth,
 *  a line clamp grows scrollHeight; a pixel of slack covers subpixel rounding. */
const clipped = (el: HTMLElement) => el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1;

function show(el: HTMLElement, text: string) {
  const b = box();
  const cs = getComputedStyle(el);
  const r = el.getBoundingClientRect();
  const rtl = cs.direction === "rtl";
  b.textContent = text;
  // The text's own face and measure, so the words land where the clipped ones were, and
  // its language and direction (a note's Japanese, an Arabic task) with them
  b.style.fontFamily = cs.fontFamily;
  b.style.fontSize = cs.fontSize;
  b.style.fontWeight = cs.fontWeight;
  b.style.lineHeight = cs.lineHeight;
  b.style.letterSpacing = cs.letterSpacing;
  b.style.padding = `${PAD_Y}px ${PAD_X}px`;
  const lang = el.closest("[lang]")?.getAttribute("lang");
  if (lang) b.lang = lang;
  else b.removeAttribute("lang");
  b.dir = rtl ? "rtl" : "ltr";
  b.hidden = false;
  // The content box's edges, the text's start and first line
  const padL = parseFloat(cs.paddingLeft) || 0;
  const padR = parseFloat(cs.paddingRight) || 0;
  const padT = parseFloat(cs.paddingTop) || 0;
  const inset = PAD_X + BORDER;
  const own = r.width - padL - padR + 2 * inset;
  // Measured at the origin first: left over from the previous hover, a `left` near the
  // right edge would squeeze the shrink-to-fit box and the measurement with it
  b.style.left = "0px";
  b.style.top = "0px";
  b.style.width = "";
  b.style.minWidth = `${Math.min(own, window.innerWidth - 2 * GAP)}px`;
  b.style.minHeight = "0px";
  b.style.maxWidth = `${window.innerWidth - 2 * GAP}px`;
  const natural = b.getBoundingClientRect().width;
  // One line when the viewport has the room from the text's start; otherwise wide enough
  // to wrap into few lines, never narrower than the element it covers
  const room = rtl ? r.right - padR + inset - GAP : window.innerWidth - GAP - (r.left + padL - inset);
  const fontPx = parseFloat(cs.fontSize) || 13;
  const cap = Math.max(own, Math.min(room, WRAP_EM * fontPx));
  // A clamped title wraps at the cap even when one line would fit the viewport: a single
  // line across the board would lie over the next columns' cards
  const multiLine = r.height > (parseFloat(cs.lineHeight) || fontPx * 1.5) * 1.5;
  const width = natural <= (multiLine ? cap : room) ? natural : cap;
  b.style.width = `${Math.ceil(width)}px`;
  // Never shorter than the element: the clipped lines under it must not show through
  b.style.minHeight = `${r.height - padT - (parseFloat(cs.paddingBottom) || 0) + 2 * (PAD_Y + BORDER)}px`;
  const x0 = rtl ? r.right - padR + inset - width : r.left + padL - inset;
  const { x, y } = clampToViewport(b, x0, r.top + padT - PAD_Y - BORDER);
  b.style.left = `${Math.max(GAP, x)}px`;
  b.style.top = `${Math.max(GAP, y)}px`;
  shownFor = el;
}

/** What the action is given: the element's full text (shown only when it is clipped), or
 *  `{ text, always }` — `text` omitted reads the element's own text at show time (a meta
 *  line of several parts), `always` shows it clipped or not (a hint, a full date). */
export type TipArg = string | { text?: string; always?: boolean };

/** The Svelte action: `use:cliptip={text}` or `use:cliptip={{ text, always: true }}`. */
export function cliptip(node: HTMLElement, arg: TipArg) {
  let opts = typeof arg === "string" ? { text: arg } : arg;
  let timer: ReturnType<typeof setTimeout> | null = null;
  const cancel = () => {
    if (timer) clearTimeout(timer);
    timer = null;
    if (armed === node) armed = null;
    if (shownFor === node) hide();
  };
  const enter = () => {
    cancel();
    armed = node;
    timer = setTimeout(() => {
      timer = null;
      if (armed !== node || !node.isConnected) return;
      const text = opts.text ?? node.textContent ?? "";
      if (text && (opts.always || clipped(node))) show(node, text);
    }, DELAY);
  };
  node.addEventListener("pointerenter", enter);
  node.addEventListener("pointerleave", cancel);
  // A press on the line (opening the task, a drag, a right-click) takes the box with it
  node.addEventListener("pointerdown", cancel);
  return {
    update(next: TipArg) {
      opts = typeof next === "string" ? { text: next } : next;
      if (shownFor === node) box().textContent = opts.text ?? node.textContent ?? "";
    },
    destroy() {
      cancel();
      node.removeEventListener("pointerenter", enter);
      node.removeEventListener("pointerleave", cancel);
      node.removeEventListener("pointerdown", cancel);
    },
  };
}
