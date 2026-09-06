/** Where a composer overlay sits — the `@` candidates and the `/` skills share it: fixed,
 *  above the composer, as wide as it, capped so it never pushes out of the viewport. The
 *  candidate layer measures the viewport rect of the .composer its anchor (the textarea)
 *  sits in and positions itself fixed — absolute would be clipped away entirely by the
 *  overlay container's overflow:hidden (while idle the overlay is only as tall as the
 *  composer, with no room above inside the container), and fixed is clipped by nothing.
 *  The precondition: no transform or filter on the ancestor chain (either would make that
 *  ancestor the containing block of a fixed element, shifting the coordinates and putting it
 *  back inside the overflow clipping) — so no host container of a conversation surface may
 *  position with translate or the like. */
export interface OverlayRect {
  left: number;
  width: number;
  bottom: number;
  maxH: number;
}

export function composerOverlayRect(anchor: HTMLElement | null): OverlayRect | null {
  const box = anchor?.closest(".composer");
  if (!(box instanceof HTMLElement)) return null;
  const r = box.getBoundingClientRect();
  return {
    left: r.left + 12, // aligned with the composer's padding (12)
    width: Math.max(220, r.width - 24),
    bottom: window.innerHeight - r.top + 4,
    // With less than 40vh of room above, it caps at what is really left so the candidate layer does not push out of the viewport
    maxH: Math.max(120, Math.min(window.innerHeight * 0.4, r.top - 12)),
  };
}
