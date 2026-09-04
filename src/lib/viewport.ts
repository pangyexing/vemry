// The part of the window our DOM can use. A docked browser tab is a native view laid
// over the right edge of the main window, and it covers whatever DOM is under it — a
// context menu, a hover card or a toast placed there would be cut off. Every popup
// that clamps itself to the viewport clamps to this right edge instead of
// window.innerWidth.
import { browser } from "$lib/stores/browser.svelte";

/** The x coordinate of the rightmost usable pixel column (window.innerWidth when no native panel is docked). */
export function usableRight(): number {
  return window.innerWidth - browser.panelInset;
}
