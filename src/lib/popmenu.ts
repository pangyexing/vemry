// The floating-menu shell the small anchored menus share: where the panel lands, and
// how it closes. TaskDateMenu (a task's dates) and FieldValueMenu (an inline field's
// value) are the two — ContextMenu keeps its own copy, being store-driven with a
// different lifetime.
// ⚠ The closing listeners must not touch `click`: after a kanban drop (pointerup)
// raises a menu the browser still dispatches a click, which KanbanView swallows with
// onclickcapture + suppressClick — a click listener here would have that click close
// the menu instantly, and it only reproduces under a real drag.

import { usableRight } from "$lib/viewport";

/** How far the panel stays from the viewport's edges. */
const MARGIN = 4;

/** The panel's landing point pulled inside the viewport, measured against its own rect:
 *  a menu raised from a chip at the bottom of a column would otherwise overflow the
 *  lower edge, the taller ones (a "remove" row) worst. Call it after mounting — the
 *  caller renders at the raw coordinates first, then reads the rect. */
export function clampToViewport(el: HTMLElement, x: number, y: number): { x: number; y: number } {
  const r = el.getBoundingClientRect();
  return {
    x: Math.min(x, Math.max(MARGIN, usableRight() - r.width - MARGIN)),
    y: Math.min(y, Math.max(MARGIN, window.innerHeight - r.height - MARGIN)),
  };
}

/** The two capture-phase window handlers a menu closes on. The outside click closes on
 *  pointerdown, which comes before click, so a click inside the panel is unaffected (a
 *  date input inside it does not affect the contains check either); the component is
 *  only mounted while the menu is open and the interaction that opened it precedes the
 *  mount, so it does not close by mistake. Escape is stopped and prevented, or App's
 *  layered Escape would close the projection view carrying the menu along with it. */
export function dismissOn(panel: () => HTMLElement | null, onclose: () => void) {
  return {
    onpointerdowncapture(e: PointerEvent) {
      const el = panel();
      if (el && e.target instanceof Node && el.contains(e.target)) return;
      onclose();
    },
    onkeydowncapture(e: KeyboardEvent) {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      e.preventDefault();
      onclose();
    },
  };
}
