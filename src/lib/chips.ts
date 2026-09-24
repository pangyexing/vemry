// The one drawing of a state, shared by every surface that shows one: a task's
// checkbox in the editor, in the reading projection, in Agenda, on a board card and in
// a query row, and the value of a state field (a `type: state` key of a block type —
// fields.ts's stateField) as a chip in the editor, an embed, a card or a row. One
// vocabulary of steps, so the reader learns the shape once: step 0 is an empty box, the
// last step a solid box with a tick, and a step in between fills the box from the bottom
// by step / (of − 1). With three steps it is exactly the todo / doing / done checkbox,
// which is the point: a task is a state field whose vocabulary is fixed at three and
// whose syntax is a checkbox.
// The glyph carries its own colour (the vstate-* classes it wears, coloured once in
// app.css): the faint ink for an empty or cancelled box, the accent for one in progress
// or done. A host sizes it — 15px, the editor's checkbox, wherever it stands for one —
// and adds a hover if it is clickable there; it never colours it, or the five surfaces
// drift apart again (they had, before this drawing became the only one).
// The drawings are strings on the 24 viewBox convention of tasks.ts's dateIconSvg (the
// Svelte views inject them with {@html}; the editor widgets set innerHTML), and the DOM
// builders below are for the imperative surfaces (a CodeMirror widget has only DOM).
// Pure: no i18n, no stores — the glyph has no words, and the words on a chip are the
// key and the value as written.

export type StateTone = "idle" | "active" | "done";

/** The tone of a step: the first is idle, the last is done, the rest are active. A
 *  single-step vocabulary (of ≤ 1) is done at once. The glyph colours itself by it;
 *  the one other reader is a query row, whose words step back at the last step. */
export function stateTone(step: number, of: number): StateTone {
  if (of <= 1 || step >= of - 1) return "done";
  return step <= 0 ? "idle" : "active";
}

/** The glyph of step `step` of `of`: a box filling the 24 viewBox with a small radius
 *  (drawn at 15px its stroke is 1.5px and its corners 2.5px — a square with the corners
 *  taken off, the shape of a checkbox, not a pill), empty at the first step, filled from
 *  the bottom by the share of the way through in between, solid with a tick at the last
 *  (in the ground's colour, --bg, as every glyph on an accent fill: white would fall to
 *  2.6:1 on the dark theme's lighter accent; a style rather than the stroke attribute,
 *  which does not read var()); a cancelled task is the empty box crossed out — closed, but not by being
 *  done. The partial fill is a path with square top corners and rounded bottom ones (a
 *  liquid level) rather than a clipped rect, so that no clipPath id is minted per glyph
 *  (ids are document-global). */
export function stateGlyphSvg(step: number, of: number, cancelled = false): string {
  const last = of <= 1 || step >= of - 1;
  const kind: StateTone | "cancel" = cancelled ? "cancel" : last ? "done" : step > 0 ? "active" : "idle";
  let inner = `<rect x="1.5" y="1.5" width="21" height="21" rx="4"${kind === "done" ? ' fill="currentColor"' : ""}/>`;
  if (kind === "cancel") inner += '<path d="m7.5 7.5 9 9M16.5 7.5l-9 9"/>';
  else if (kind === "done") inner += '<path d="m6.3 12.3 4 4 7.4-8.6" style="stroke:var(--bg)" stroke-width="2.8"/>';
  else if (kind === "active") {
    // The box's inner edge sits at 2.7 (the stroke's centre plus half its width) on every
    // side; the fill is that share of the inner height, its bottom corners following the
    // box's inner radius, shrunk when the level is lower than the radius
    const h = Math.round(((18.6 * step) / (of - 1)) * 100) / 100;
    const r = Math.min(2.8, h);
    const top = Math.round((21.3 - h) * 100) / 100;
    inner += `<path d="M2.7 ${top}H21.3V${21.3 - r}A${r} ${r} 0 0 1 ${21.3 - r} 21.3H${2.7 + r}A${r} ${r} 0 0 1 2.7 ${21.3 - r}Z" fill="currentColor" stroke="none"/>`;
  }
  return `<svg class="vstate vstate-${kind}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">${inner}</svg>`;
}

/** The step of a task's state in the three-step vocabulary; cancelled is closed, the
 *  last step, drawn with the cross rather than the tick. */
export const TASK_STEPS = 3;
export function taskStep(state: "todo" | "doing" | "done" | "cancelled"): number {
  return state === "done" || state === "cancelled" ? 2 : state === "doing" ? 1 : 0;
}

/** A state's place: `step` of `of`; `cancelled` is a task closed by being cancelled
 *  (the last step, drawn with the cross). */
export interface StateStep {
  step: number;
  of: number;
  cancelled?: boolean;
}

/* ---- DOM builders: the chip family of the imperative surfaces (query rows) ---- */

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  e.className = className;
  if (text !== undefined) e.textContent = text;
  return e;
}

/** A state glyph as an element; `title` is the tooltip. */
export function stateGlyph(step: number, of: number, title?: string, cancelled = false): HTMLSpanElement {
  const g = el("span", "vchip-glyph");
  g.innerHTML = stateGlyphSvg(step, of, cancelled);
  if (title) g.title = title;
  return g;
}

/** A `key: value` chip; with `state` it carries the glyph and reads as a state rather
 *  than a plain field. `keyShown` false drops the key (a row whose head already says
 *  which field this is). */
export function fieldChip(key: string, value: string, state?: StateStep | null, keyShown = true): HTMLSpanElement {
  const chip = el("span", state ? "vchip vchip-field vchip-state" : "vchip vchip-field");
  if (state) chip.appendChild(stateGlyph(state.step, state.of));
  if (keyShown) chip.appendChild(el("span", "vchip-key", key));
  chip.appendChild(el("span", "vchip-val", value));
  chip.title = `${key}:: ${value}`;
  return chip;
}

/** A `#tag` chip: the hash kept (it is what makes the word a tag on disk, and what the
 *  writer typed) but faded, so the name carries the chip. */
export function tagChip(tag: string): HTMLSpanElement {
  const chip = el("span", "vchip vchip-tag");
  chip.appendChild(el("span", "vchip-hash", "#"));
  chip.appendChild(el("span", "vchip-val", tag));
  chip.title = `#${tag}`;
  return chip;
}

/** A date chip: the icon (dateIconSvg's string) then the date; `overdue` colours it danger. */
export function dateChip(iconSvg: string, text: string, overdue: boolean, title?: string): HTMLSpanElement {
  const chip = el("span", overdue ? "vchip vchip-date vchip-overdue" : "vchip vchip-date");
  chip.innerHTML = iconSvg;
  chip.appendChild(document.createTextNode(text));
  if (title) chip.title = title;
  return chip;
}
