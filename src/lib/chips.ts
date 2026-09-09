// The one drawing of a state, shared by every surface that shows one: a task's
// checkbox state in Agenda, on a board card and in a query row, and the value of a
// state field (a `type: state` key of a block type — fields.ts's stateField) as a chip
// in the editor, an embed, a card or a row. One vocabulary of steps, so the reader
// learns the shape once: step 0 is an empty box, the last step a ticked box, and a step
// in between fills the box from the bottom by step / (of − 1). With three steps it is
// exactly the todo / doing / done icon the task views drew before, which is the point:
// a task is a state field whose vocabulary is fixed at three and whose syntax is a
// checkbox.
// The drawings are strings on the 24 viewBox convention of tasks.ts's dateIconSvg (the
// Svelte views inject them with {@html}; the editor widgets set innerHTML), and the DOM
// builders below are for the imperative surfaces (a CodeMirror widget has only DOM).
// Pure: no i18n, no stores — the glyph has no words, and the words on a chip are the
// key and the value as written.

export type StateTone = "idle" | "active" | "done";

/** The tone of a step: the first is idle, the last is done, the rest are active. A
 *  single-step vocabulary (of ≤ 1) is done at once. */
export function stateTone(step: number, of: number): StateTone {
  if (of <= 1 || step >= of - 1) return "done";
  return step <= 0 ? "idle" : "active";
}

/** The glyph of step `step` of `of`: a rounded box, filled from the bottom by the share
 *  of the way through, a tick on the last step. The fill is an inset rect rather than a
 *  clipped one so that no clipPath id is minted per glyph (ids are document-global). */
export function stateGlyphSvg(step: number, of: number, cancelled = false): string {
  const last = of <= 1 || step >= of - 1;
  const box = '<rect x="4" y="4" width="16" height="16" rx="3"/>';
  let inner = box;
  // A cancelled task: the box with a cross — closed, but not by being done
  if (cancelled) inner += '<path d="m9 9 6 6M15 9l-6 6"/>';
  else if (last) inner += '<path d="m8.2 12.4 2.6 2.6 5-5.8"/>';
  else if (step > 0) {
    const h = Math.round((13 * step) / (of - 1) * 100) / 100;
    inner += `<rect x="5.5" y="${18.5 - h}" width="13" height="${h}" rx="1.5" fill="currentColor" stroke="none"/>`;
  }
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${inner}</svg>`;
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
  const g = el("span", `vchip-glyph vchip-tone-${stateTone(step, of)}`);
  g.innerHTML = stateGlyphSvg(step, of, cancelled);
  if (title) g.title = title;
  return g;
}

/** A `key: value` chip; with `state` it carries the glyph and the step's tone, and reads
 *  as a state rather than a plain field. `keyShown` false drops the key (a row whose
 *  head already says which field this is). */
export function fieldChip(key: string, value: string, state?: StateStep | null, keyShown = true): HTMLSpanElement {
  const chip = el("span", state ? `vchip vchip-field vchip-state vchip-tone-${stateTone(state.step, state.of)}` : "vchip vchip-field");
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
