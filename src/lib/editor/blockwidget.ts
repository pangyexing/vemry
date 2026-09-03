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

/** Wrap a block widget's card in the element CodeMirror measures. */
export function blockRoot(card: HTMLElement): HTMLElement {
  const root = document.createElement("div");
  root.className = "cm-block-root";
  root.appendChild(card);
  return root;
}
