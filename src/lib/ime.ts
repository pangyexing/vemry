// The input-method guard. On WKWebView (WebKit), on the Enter that confirms a
// composition, compositionend is dispatched before keydown, so isComposing is already
// false by the time keydown arrives (Blink is the other way round, dispatching
// keydown during the composition) — which is why checking e.isComposing alone
// mistakes confirming a candidate for submitting on macOS. The remedy: record the
// moment of every compositionend globally, and treat a keydown that came from the
// same physical key press (a timestamp difference of ~0ms) as an input-method event;
// two separate presses by hand are at least tens of milliseconds apart, so a 50ms
// window never swallows a real submit.
let endedAt = -1e9;
window.addEventListener("compositionend", (e) => (endedAt = e.timeStamp), true);

/** Whether this keydown belongs to the input method (mid-composition, or the press that just confirmed one). */
export function isImeKeydown(e: KeyboardEvent): boolean {
  return e.isComposing || e.timeStamp - endedAt < 50;
}
