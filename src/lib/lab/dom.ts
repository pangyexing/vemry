// The two DOM primitives a lab card is built from, in one place so the card, the
// charts and the run button agree — charts.ts may not import editor/lab.ts (a cycle),
// and both need them.

/** An element with a class and, optionally, its text. */
export function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

/** A press on a control inside a block widget must not reach the editor: the default
 *  would place the caret by coordinates inside the replaced range, which destroys the
 *  widget under the pointer. Every button in a card or a chart goes through this, so
 *  none of them has to remember the incantation. */
export function swallowPress(target: EventTarget) {
  target.addEventListener("mousedown", (e) => {
    e.preventDefault();
    e.stopPropagation();
  });
}
