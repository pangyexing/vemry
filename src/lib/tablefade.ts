// The edge fade of a wide table in rendered Markdown. A table wider than its column keeps
// scrolling sideways (collapsing it would set CJK cells one character to a line), and
// what shows that more of it lies to one side is a fade at that edge: `fade-start` /
// `fade-end` on the .md-tablewrap, which app.css turns into a mask. The classes follow
// the wrapper's scroll position, and are re-read whenever the rendered content changes
// (a streaming answer rebuilds its last block every flush) or the surface is resized.

/** Mark one wrapper: which of its two edges hides more of the table. */
function mark(w: HTMLElement) {
  const max = w.scrollWidth - w.clientWidth;
  // Right-to-left content scrolls with a negative scrollLeft; the distance from the start is the same
  const x = Math.abs(w.scrollLeft);
  w.classList.toggle("fade-start", max > 1 && x > 1);
  w.classList.toggle("fade-end", max > 1 && x < max - 1);
}

/** The Svelte action for a surface holding rendered Markdown: `use:tableFades`. */
export function tableFades(node: HTMLElement) {
  let frame = 0;
  const all = () => {
    frame = 0;
    for (const w of node.querySelectorAll<HTMLElement>(".md-tablewrap")) mark(w);
  };
  // One pass per frame however many changes arrive in it
  const soon = () => {
    if (!frame) frame = requestAnimationFrame(all);
  };
  // Scroll events do not bubble: one capturing listener sees every wrapper's
  const onScroll = (e: Event) => {
    const t = e.target;
    if (t instanceof HTMLElement && t.classList.contains("md-tablewrap")) mark(t);
  };
  node.addEventListener("scroll", onScroll, true);
  const mo = new MutationObserver(soon);
  mo.observe(node, { childList: true, subtree: true });
  const ro = new ResizeObserver(soon);
  ro.observe(node);
  soon();
  return {
    destroy() {
      node.removeEventListener("scroll", onScroll, true);
      mo.disconnect();
      ro.disconnect();
      if (frame) cancelAnimationFrame(frame);
    },
  };
}
