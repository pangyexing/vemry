// A view whose rows open something on a click but whose text can also be selected: a drag
// across the text ends in a click on the same row (mousedown and mouseup both landed in
// it), and that click must not open the note and throw the selection away. A plain click
// collapses any selection at mousedown, so by the time the click arrives a live selection
// inside the row can only mean the reader was selecting.

/** Whether the reader has just selected text inside `el` (so its click is not an "open"). */
export function selectingIn(el: Element | null): boolean {
  const sel = window.getSelection();
  if (!el || !sel || sel.isCollapsed || !sel.toString().trim()) return false;
  return (sel.anchorNode != null && el.contains(sel.anchorNode)) || (sel.focusNode != null && el.contains(sel.focusNode));
}
