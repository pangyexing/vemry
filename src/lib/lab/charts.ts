// The views of a lab block (docs/SYNTAX.md, "Runnable code blocks"), drawn in plain
// DOM and SVG from the canonical spec (api.ts LabView; the source of the shape is
// crates/vemry-script/src/lab.rs). Pure builders: they take the view and a width and
// return an element; the editor session (editor/lab.ts) owns the tooltip, the resize
// observer and the re-runs.
//
// The drawing rules are fixed and deliberately quiet — the data is the only thing
// allowed to be loud: 2px lines with round joins, markers r=4 with a 2px surface ring,
// bars no thicker than 24px with a rounded data-end and a square baseline, hairline
// solid gridlines one step off the surface, text always in the text tokens (a mark
// beside the text carries identity, never the text's colour), a legend whenever there
// are two series or more and none for one, a tooltip on every mark, and a table twin
// of every chart so no value is reachable through colour alone. Series colours are
// the eight validated categorical slots (app.css --lab-s1…s8, assigned in fixed order,
// never cycled: the cap is eight); magnitude is one hue light→dark, and a signed
// heatmap is two poles around a neutral midpoint.
import type { LabView, LineSeries } from "$lib/api";
import { renderMathHtml, scanInlineMath } from "$lib/math";
import { el, swallowPress } from "./dom";

const SVG_NS = "http://www.w3.org/2000/svg";

/** One row of the tooltip: the series' colour slot (1-based; 0 = none), its name and the value. */
export interface TipRow {
  slot: number;
  label: string;
  value: string;
}

/** The tooltip the session owns: one element per lab, moved to the pointer. */
export interface TipHost {
  show(title: string, rows: TipRow[], clientX: number, clientY: number): void;
  /** The same tip following the pointer: nothing rebuilt, nothing measured again. */
  move(clientX: number, clientY: number): void;
  hide(): void;
}

// ---- formatting ----

/** A value as the reader should see it: the requested decimals, else an integer as
 *  is and anything else to four significant digits with the noise trimmed. */
export function fmt(v: number | null | undefined, digits?: number | null): string {
  if (v == null || !Number.isFinite(v)) return "–";
  if (digits != null) return v.toFixed(digits);
  if (Number.isInteger(v)) return String(v);
  return String(parseFloat(v.toPrecision(4)));
}

/** An axis tick: three significant digits, with thousands compacted (1.2k, 3.4M). */
function fmtTick(v: number): string {
  const a = Math.abs(v);
  if (a >= 1e6) return `${parseFloat((v / 1e6).toPrecision(3))}M`;
  if (a >= 1e4) return `${parseFloat((v / 1e3).toPrecision(3))}k`;
  return String(parseFloat(v.toPrecision(3)));
}

// ---- scales ----

/** Round tick positions covering [min, max] (about `count` of them), the way an axis
 *  should read: 0 / 0.5 / 1 rather than 0.13 / 0.26. */
function niceTicks(min: number, max: number, count = 5): number[] {
  if (!(max > min)) {
    const v = Number.isFinite(min) ? min : 0;
    return [v];
  }
  const raw = (max - min) / Math.max(1, count);
  const mag = Math.pow(10, Math.floor(Math.log10(raw)));
  const norm = raw / mag;
  const step = (norm >= 5 ? 10 : norm >= 2 ? 5 : norm >= 1 ? 2 : 1) * mag;
  const start = Math.ceil(min / step) * step;
  const out: number[] = [];
  for (let v = start; v <= max + step * 1e-9 && out.length < 50; v += step) out.push(parseFloat(v.toPrecision(12)));
  return out;
}

interface Scale {
  (v: number): number;
  domain: [number, number];
}

function linear(domain: [number, number], range: [number, number]): Scale {
  const [d0, d1] = domain;
  const [r0, r1] = range;
  const span = d1 - d0 || 1;
  const f = ((v: number) => r0 + ((v - d0) / span) * (r1 - r0)) as Scale;
  f.domain = domain;
  return f;
}

/** A domain padded to its nice ticks (so the extreme point is not glued to the frame). */
function niceDomain(min: number, max: number, includeZero = false): [number, number] {
  if (!Number.isFinite(min) || !Number.isFinite(max)) return [0, 1];
  if (includeZero) {
    min = Math.min(0, min);
    max = Math.max(0, max);
  }
  if (min === max) {
    const pad = Math.abs(min) || 1;
    return [min - pad, max + pad];
  }
  const ticks = niceTicks(min, max);
  const step = ticks.length > 1 ? ticks[1] - ticks[0] : (max - min) / 5;
  return [Math.floor(min / step) * step, Math.ceil(max / step) * step];
}

// ---- svg helpers ----

function svgEl<K extends keyof SVGElementTagNameMap>(tag: K, attrs: Record<string, string | number> = {}): SVGElementTagNameMap[K] {
  const el = document.createElementNS(SVG_NS, tag);
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, String(v));
  return el;
}

function svgText(x: number, y: number, text: string, cls: string, anchor: "start" | "middle" | "end" = "start"): SVGTextElement {
  const t = svgEl("text", { x, y, class: cls, "text-anchor": anchor });
  t.textContent = text;
  return t;
}

/** The colour slot of the i-th series (1-based, capped at eight: a ninth series would
 *  be indistinguishable, and the sandbox cap already refuses more). */
const slotOf = (i: number) => Math.min(8, i + 1);

/** The width of one character of axis text (11px, a monospace-ish estimate): the one
 *  number behind both the room the labels are given and the trimming when they do not fit. */
const CHAR_W = 6.6;

/** The width the y tick labels need. */
const labelWidth = (labels: string[]) => Math.max(0, ...labels.map((l) => l.length)) * CHAR_W + 10;

/** A category label as a tick, trimmed to the room it has with the full text as a title. */
function tickLabel(x: number, y: number, label: string, px: number, anchor: "middle" | "end"): SVGTextElement {
  const maxChars = Math.max(2, Math.floor(px / CHAR_W));
  const text = label.length > maxChars ? `${label.slice(0, Math.max(1, maxChars - 1))}…` : label;
  const t = svgText(x, y, text, "cm-lab-tick", anchor);
  if (text !== label) t.appendChild(svgEl("title")).textContent = label;
  return t;
}

/** How thick one bar of a group may be: the group takes 70% of its slot, the bars share
 *  what the gaps between them leave, and none is thicker than 24px or thinner than 3. */
const barThickness = (slot: number, count: number, gap: number) => Math.min(24, Math.max(3, (slot * 0.7 - gap * (count - 1)) / count));

/** Hovering a category lifts its bars and reads out every series at that index. The
 *  pointer stays on one category for many moves, so the rows are built once per
 *  category rather than per event. */
function barHover(hit: SVGRectElement, bars: SVGPathElement[], label: string, rows: () => TipRow[], tip: TipHost) {
  let shown = false;
  hit.addEventListener("pointermove", (e) => {
    if (!shown) {
      shown = true;
      bars.forEach((b) => b.classList.add("lift"));
      tip.show(label, rows(), e.clientX, e.clientY);
      return;
    }
    tip.move(e.clientX, e.clientY);
  });
  hit.addEventListener("pointerleave", () => {
    shown = false;
    bars.forEach((b) => b.classList.remove("lift"));
    tip.hide();
  });
}

interface Frame {
  svg: SVGSVGElement;
  plot: SVGGElement;
  left: number;
  top: number;
  w: number;
  h: number;
  x: Scale;
  y: Scale;
}

/** The frame every axis chart shares: the y gridlines and ticks on the left, the x
 *  baseline, the axis names. `xTicks` are drawn by the caller (a numeric axis and a
 *  category axis place them differently). */
function frame(width: number, plotH: number, xDom: [number, number], yDom: [number, number], yTicks: number[], xLabel: string | null, yLabel: string | null): Frame {
  const tickLabels = yTicks.map(fmtTick);
  const left = labelWidth(tickLabels) + (yLabel ? 14 : 0);
  const top = 10;
  const bottom = 22 + (xLabel ? 14 : 0);
  const right = 12;
  const w = Math.max(60, width - left - right);
  const h = plotH;
  const svg = svgEl("svg", { width, height: top + h + bottom, viewBox: `0 0 ${width} ${top + h + bottom}`, class: "cm-lab-svg" });
  const plot = svgEl("g", { transform: `translate(${left},${top})` });
  const x = linear(xDom, [0, w]);
  const y = linear(yDom, [h, 0]);
  const grid = svgEl("g", { class: "cm-lab-grid" });
  for (const t of yTicks) {
    const yy = y(t);
    grid.appendChild(svgEl("line", { x1: 0, x2: w, y1: yy, y2: yy }));
    grid.appendChild(svgText(-6, yy + 3.5, fmtTick(t), "cm-lab-tick", "end"));
  }
  plot.appendChild(grid);
  // the baseline: at zero when the domain crosses it, else along the bottom edge
  const base = yDom[0] <= 0 && yDom[1] >= 0 ? y(0) : h;
  plot.appendChild(svgEl("line", { x1: 0, x2: w, y1: base, y2: base, class: "cm-lab-axis" }));
  if (yLabel) {
    const t = svgText(0, 0, yLabel, "cm-lab-axislabel", "middle");
    t.setAttribute("transform", `translate(${-left + 10},${h / 2}) rotate(-90)`);
    plot.appendChild(t);
  }
  if (xLabel) plot.appendChild(svgText(w / 2, h + bottom - 4, xLabel, "cm-lab-axislabel", "middle"));
  svg.appendChild(plot);
  return { svg, plot, left, top, w, h, x, y };
}

/** The mark that carries a series' identity: a short line, a square, or a dot, in the
 *  series' colour slot. The legend and the tooltip draw the same one. */
export const keySwatch = (kind: "line" | "rect" | "dot", slot: number) => el("i", `cm-lab-key cm-lab-key-${kind} s${slot}`);

/** A legend row: a key (a short line for a line chart, a square for bars and dots) and the name, per series. */
function legend(names: (string | null)[], key: "line" | "rect" | "dot"): HTMLElement | null {
  if (names.length < 2) return null;
  const box = el("div", "cm-lab-legend");
  names.forEach((n, i) => {
    const item = el("span", "cm-lab-legend-item");
    item.append(keySwatch(key, slotOf(i)), document.createTextNode(n || `#${i + 1}`));
    box.appendChild(item);
  });
  return box;
}

const seriesName = (name: string | null, i: number, count: number) => name ?? (count > 1 ? `#${i + 1}` : "");

// ---- line ----

type LineView = Extract<LabView, { kind: "line" }>;
type Pt = { x: number; y: number | null };

/** The last point of a series that has a value: where the end marker and the direct
 *  end-label both go. Found once — the search walks backwards rather than copying and
 *  reversing the series. */
function lastPoint(series: Pt[]): Pt | undefined {
  for (let i = series.length - 1; i >= 0; i--) if (series[i].y != null) return series[i];
  return undefined;
}

/** The x of the i-th point of a series: its own x, the view's shared x, else the index. */
function xOf(s: LineSeries, view: LineView, i: number): number | null {
  const v = s.x ? s.x[i] : view.x ? view.x[i] : i;
  return v == null || !Number.isFinite(v) ? null : v;
}

export function drawLine(view: LineView, width: number, tip: TipHost): HTMLElement {
  const wrap = el("div", "cm-lab-chart");
  const pts = view.series.map((s) =>
    s.y.map((y, i) => ({ x: xOf(s, view, i), y: y != null && Number.isFinite(y) ? y : null })).filter((p) => p.x != null) as Pt[],
  );
  const xs = pts.flat().map((p) => p.x);
  const ys = pts.flat().map((p) => p.y).filter((y): y is number => y != null);
  if (!xs.length) {
    wrap.appendChild(el("div", "cm-lab-empty", "–"));
    return wrap;
  }
  const xDom: [number, number] = [Math.min(...xs), Math.max(...xs)];
  if (xDom[0] === xDom[1]) xDom[1] = xDom[0] + 1;
  let yDom = niceDomain(ys.length ? Math.min(...ys) : 0, ys.length ? Math.max(...ys) : 1);
  if (view.y_min != null) yDom[0] = view.y_min;
  if (view.y_max != null) yDom[1] = view.y_max;
  if (yDom[0] >= yDom[1]) yDom = [yDom[0], yDom[0] + 1];
  const yTicks = niceTicks(yDom[0], yDom[1]);
  const f = frame(width, 180, xDom, yDom, yTicks, view.x_label, view.y_label);
  // x ticks: integers when every x is one (an index axis), else nice numbers
  const allInt = xs.every(Number.isInteger);
  const xTicks = niceTicks(xDom[0], xDom[1], Math.max(2, Math.min(8, Math.floor(f.w / 60)))).filter((t) => !allInt || Number.isInteger(t));
  for (const t of xTicks) f.plot.appendChild(svgText(f.x(t), f.h + 14, fmtTick(t), "cm-lab-tick", "middle"));
  const marks = svgEl("g");
  pts.forEach((series, si) => {
    let d = "";
    let pen = false;
    for (const p of series) {
      if (p.y == null) {
        pen = false;
        continue;
      }
      d += `${pen ? "L" : "M"}${f.x(p.x).toFixed(1)},${f.y(p.y).toFixed(1)}`;
      pen = true;
    }
    marks.appendChild(svgEl("path", { d, class: `cm-lab-line s${slotOf(si)}` }));
    const last = lastPoint(series);
    if (last) marks.appendChild(svgEl("circle", { cx: f.x(last.x), cy: f.y(last.y!), r: 4, class: `cm-lab-dot s${slotOf(si)}` }));
  });
  f.plot.appendChild(marks);
  // direct end-labels ride the marks only while they do not collide (else the legend carries identity)
  const ends = pts
    .map((series, si) => ({ si, last: lastPoint(series) }))
    .filter((e) => e.last && view.series[e.si].name)
    .map((e) => ({ si: e.si, y: f.y(e.last!.y!), x: f.x(e.last!.x) }))
    .sort((a, b) => a.y - b.y);
  const collide = ends.some((e, i) => i > 0 && e.y - ends[i - 1].y < 12);
  if (view.series.length <= 4 && !collide) {
    for (const e of ends) {
      const t = svgText(Math.min(e.x + 7, f.w + 6), e.y + 3.5, view.series[e.si].name!, "cm-lab-endlabel", "start");
      f.plot.appendChild(t);
    }
  }
  // hover: a crosshair snapped to the nearest x, one readout for every series there
  const union = [...new Set(xs)].sort((a, b) => a - b);
  const byX = pts.map((series) => new Map(series.filter((p) => p.y != null).map((p) => [p.x, p.y as number])));
  const cross = svgEl("line", { y1: 0, y2: f.h, class: "cm-lab-cross" });
  cross.style.display = "none";
  const hoverDots = svgEl("g");
  f.plot.appendChild(cross);
  f.plot.appendChild(hoverDots);
  const hit = svgEl("rect", { x: 0, y: 0, width: f.w, height: f.h, class: "cm-lab-hit" });
  hit.addEventListener("pointermove", (e) => {
    const r = f.svg.getBoundingClientRect();
    const px = e.clientX - r.left - f.left;
    let best = union[0];
    let bestD = Infinity;
    for (const u of union) {
      const d = Math.abs(f.x(u) - px);
      if (d < bestD) {
        bestD = d;
        best = u;
      }
    }
    const cx = f.x(best);
    cross.setAttribute("x1", String(cx));
    cross.setAttribute("x2", String(cx));
    cross.style.display = "";
    hoverDots.replaceChildren();
    const rows: TipRow[] = [];
    byX.forEach((m, si) => {
      const y = m.get(best);
      if (y == null) return;
      hoverDots.appendChild(svgEl("circle", { cx, cy: f.y(y), r: 4, class: `cm-lab-dot s${slotOf(si)}` }));
      rows.push({ slot: slotOf(si), label: seriesName(view.series[si].name, si, view.series.length), value: fmt(y) });
    });
    tip.show(`${view.x_label ?? "x"} = ${fmt(best)}`, rows, e.clientX, e.clientY);
  });
  hit.addEventListener("pointerleave", () => {
    cross.style.display = "none";
    hoverDots.replaceChildren();
    tip.hide();
  });
  f.plot.appendChild(hit);
  wrap.appendChild(f.svg);
  const lg = legend(view.series.map((s, i) => seriesName(s.name, i, view.series.length)), "line");
  if (lg) wrap.appendChild(lg);
  return wrap;
}

// ---- bars ----

type BarsView = Extract<LabView, { kind: "bars" }>;

/** A bar path with a 4px rounded data-end and a square baseline. Vertical: the bar
 *  grows from `base` (y) to `end`; horizontal: from `base` (x) to `end`. */
function barPath(a0: number, a1: number, base: number, end: number, horizontal: boolean): string {
  const r = Math.min(4, Math.abs(a1 - a0) / 2, Math.abs(end - base));
  const up = end < base; // a positive vertical bar grows upward (smaller y)
  if (!horizontal) {
    const s = up ? -1 : 1; // direction from base toward end
    return `M${a0},${base}V${end - s * r}Q${a0},${end} ${a0 + r},${end}H${a1 - r}Q${a1},${end} ${a1},${end - s * r}V${base}Z`;
  }
  const s = end > base ? 1 : -1;
  return `M${base},${a0}H${end - s * r}Q${end},${a0} ${end},${a0 + r}V${a1 - r}Q${end},${a1} ${end - s * r},${a1}H${base}Z`;
}

export function drawBars(view: BarsView, width: number, tip: TipHost): HTMLElement {
  const wrap = el("div", "cm-lab-chart");
  const n = view.labels.length;
  const s = view.series.length;
  if (!n || !s) {
    wrap.appendChild(el("div", "cm-lab-empty", "–"));
    return wrap;
  }
  const vals = view.series.flatMap((sr) => sr.values).filter((v): v is number => v != null && Number.isFinite(v));
  const vDom = niceDomain(vals.length ? Math.min(...vals) : 0, vals.length ? Math.max(...vals) : 1, true);
  const vTicks = niceTicks(vDom[0], vDom[1]);
  const single = s === 1;
  if (!view.horizontal) {
    const f = frame(width, 180, [0, n], vDom, vTicks, null, view.y_label);
    const slot = f.w / n;
    const gap = 2;
    const bw = barThickness(slot, s, gap);
    const groupW = bw * s + gap * (s - 1);
    const base = f.y(0);
    const marks = svgEl("g");
    view.labels.forEach((label, gi) => {
      const g0 = gi * slot + (slot - groupW) / 2;
      const hit = svgEl("rect", { x: gi * slot, y: 0, width: slot, height: f.h, class: "cm-lab-hit" });
      const bars: SVGPathElement[] = [];
      view.series.forEach((sr, si) => {
        const v = sr.values[gi];
        if (v == null || !Number.isFinite(v)) return;
        const a0 = g0 + si * (bw + gap);
        const p = svgEl("path", { d: barPath(a0, a0 + bw, base, f.y(v), false), class: `cm-lab-bar s${slotOf(si)}` });
        marks.appendChild(p);
        bars.push(p);
        // the value on the cap, only when the chart is sparse enough to read it
        if (single && n <= 8) marks.appendChild(svgText(a0 + bw / 2, (v >= 0 ? f.y(v) - 5 : f.y(v) + 12), fmt(v), "cm-lab-value", "middle"));
      });
      barHover(hit, bars, label, () => view.series.map((sr, si) => ({ slot: slotOf(si), label: seriesName(sr.name, si, s), value: fmt(sr.values[gi]) })), tip);
      marks.appendChild(hit);
      // the category label under the slot, trimmed when the slot is narrow
      f.plot.appendChild(tickLabel(gi * slot + slot / 2, f.h + 14, label, slot, "middle"));
    });
    f.plot.appendChild(marks);
    wrap.appendChild(f.svg);
  } else {
    // horizontal: categories down the left, the value axis along the bottom
    const rowH = Math.min(30, Math.max(14, 8 + s * 10));
    const plotH = rowH * n;
    const catW = labelWidth(view.labels) - 10;
    const left = Math.min(160, catW + 8);
    const top = 6;
    const bottom = 22 + (view.y_label ? 14 : 0);
    const w = Math.max(60, width - left - 12);
    const svg = svgEl("svg", { width, height: top + plotH + bottom, viewBox: `0 0 ${width} ${top + plotH + bottom}`, class: "cm-lab-svg" });
    const plot = svgEl("g", { transform: `translate(${left},${top})` });
    const x = linear(vDom, [0, w]);
    const grid = svgEl("g", { class: "cm-lab-grid" });
    for (const t of vTicks) {
      grid.appendChild(svgEl("line", { x1: x(t), x2: x(t), y1: 0, y2: plotH }));
      grid.appendChild(svgText(x(t), plotH + 14, fmtTick(t), "cm-lab-tick", "middle"));
    }
    plot.appendChild(grid);
    const base = x(0);
    plot.appendChild(svgEl("line", { x1: base, x2: base, y1: 0, y2: plotH, class: "cm-lab-axis" }));
    if (view.y_label) plot.appendChild(svgText(w / 2, plotH + bottom - 4, view.y_label, "cm-lab-axislabel", "middle"));
    const gap = 2;
    const bh = barThickness(rowH, s, gap);
    const groupH = bh * s + gap * (s - 1);
    const marks = svgEl("g");
    view.labels.forEach((label, gi) => {
      const g0 = gi * rowH + (rowH - groupH) / 2;
      const hit = svgEl("rect", { x: 0, y: gi * rowH, width: w, height: rowH, class: "cm-lab-hit" });
      const bars: SVGPathElement[] = [];
      view.series.forEach((sr, si) => {
        const v = sr.values[gi];
        if (v == null || !Number.isFinite(v)) return;
        const a0 = g0 + si * (bh + gap);
        const p = svgEl("path", { d: barPath(a0, a0 + bh, base, x(v), true), class: `cm-lab-bar s${slotOf(si)}` });
        marks.appendChild(p);
        bars.push(p);
        if (single && n <= 12) marks.appendChild(svgText(v >= 0 ? x(v) + 5 : x(v) - 5, a0 + bh / 2 + 3.5, fmt(v), "cm-lab-value", v >= 0 ? "start" : "end"));
      });
      barHover(hit, bars, label, () => view.series.map((sr, si) => ({ slot: slotOf(si), label: seriesName(sr.name, si, s), value: fmt(sr.values[gi]) })), tip);
      marks.appendChild(hit);
      plot.appendChild(tickLabel(-6, gi * rowH + rowH / 2 + 3.5, label, left - 8, "end"));
    });
    plot.appendChild(marks);
    svg.appendChild(plot);
    wrap.appendChild(svg);
  }
  const lg = legend(view.series.map((sr, i) => seriesName(sr.name, i, s)), "rect");
  if (lg) wrap.appendChild(lg);
  return wrap;
}

// ---- scatter ----

type ScatterView = Extract<LabView, { kind: "scatter" }>;

export function drawScatter(view: ScatterView, width: number, tip: TipHost): HTMLElement {
  const wrap = el("div", "cm-lab-chart");
  const all = view.series.flatMap((s, si) => s.points.map((p) => ({ si, x: p[0], y: p[1] })));
  if (!all.length) {
    wrap.appendChild(el("div", "cm-lab-empty", "–"));
    return wrap;
  }
  const xDom = niceDomain(Math.min(...all.map((p) => p.x)), Math.max(...all.map((p) => p.x)));
  const yDom = niceDomain(Math.min(...all.map((p) => p.y)), Math.max(...all.map((p) => p.y)));
  const yTicks = niceTicks(yDom[0], yDom[1]);
  const f = frame(width, 200, xDom, yDom, yTicks, view.x_label, view.y_label);
  const xTicks = niceTicks(xDom[0], xDom[1], Math.max(2, Math.min(8, Math.floor(f.w / 60))));
  for (const t of xTicks) {
    f.plot.insertBefore(svgEl("line", { x1: f.x(t), x2: f.x(t), y1: 0, y2: f.h, class: "cm-lab-gridline" }), f.plot.firstChild);
    f.plot.appendChild(svgText(f.x(t), f.h + 14, fmtTick(t), "cm-lab-tick", "middle"));
  }
  const marks = svgEl("g");
  const dots = all.map((p) => {
    const c = svgEl("circle", { cx: f.x(p.x), cy: f.y(p.y), r: 4, class: `cm-lab-dot s${slotOf(p.si)}` });
    marks.appendChild(c);
    return c;
  });
  f.plot.appendChild(marks);
  // nearest-point hover: the pointer only has to be closest, never dead-centre
  const hit = svgEl("rect", { x: 0, y: 0, width: f.w, height: f.h, class: "cm-lab-hit" });
  let lit: SVGCircleElement | null = null;
  hit.addEventListener("pointermove", (e) => {
    const r = f.svg.getBoundingClientRect();
    const px = e.clientX - r.left - f.left;
    const py = e.clientY - r.top - f.top;
    let best = -1;
    let bestD = 24 * 24;
    all.forEach((p, i) => {
      const d = (f.x(p.x) - px) ** 2 + (f.y(p.y) - py) ** 2;
      if (d < bestD) {
        bestD = d;
        best = i;
      }
    });
    lit?.classList.remove("lift");
    lit = null;
    if (best < 0) {
      tip.hide();
      return;
    }
    lit = dots[best];
    lit.classList.add("lift");
    const p = all[best];
    tip.show(seriesName(view.series[p.si].name, p.si, view.series.length) || view.title, [
      { slot: 0, label: view.x_label ?? "x", value: fmt(p.x) },
      { slot: 0, label: view.y_label ?? "y", value: fmt(p.y) },
    ], e.clientX, e.clientY);
  });
  hit.addEventListener("pointerleave", () => {
    lit?.classList.remove("lift");
    lit = null;
    tip.hide();
  });
  f.plot.appendChild(hit);
  wrap.appendChild(f.svg);
  const lg = legend(view.series.map((s, i) => seriesName(s.name, i, view.series.length)), "dot");
  if (lg) wrap.appendChild(lg);
  return wrap;
}

// ---- heatmap ----

type HeatmapView = Extract<LabView, { kind: "heatmap" }>;

export function buildHeatmap(view: HeatmapView, tip: TipHost): HTMLElement {
  const wrap = el("div", "cm-lab-chart");
  const rows = view.data.length;
  const cols = Math.max(0, ...view.data.map((r) => r.length));
  if (!rows || !cols) {
    wrap.appendChild(el("div", "cm-lab-empty", "–"));
    return wrap;
  }
  const vals = view.data.flat().filter((v): v is number => v != null && Number.isFinite(v));
  let min = view.min ?? (vals.length ? Math.min(...vals) : 0);
  let max = view.max ?? (vals.length ? Math.max(...vals) : 1);
  const diverging = view.diverging || (view.min == null && view.max == null && min < 0 && max > 0);
  if (diverging) {
    const m = Math.max(Math.abs(min), Math.abs(max)) || 1;
    min = -m;
    max = m;
  }
  if (min === max) max = min + 1;
  const grid = el("div", "cm-lab-heat");
  // cell tracks between 22px and 56px: a 3×3 stays a compact square block rather than
  // stretching to the figure's width, a 64-wide map scrolls inside its own container
  grid.style.gridTemplateColumns = `auto repeat(${cols}, minmax(22px, 56px))`;
  const showNumbers = cols <= 12 && rows <= 16;
  const rowLabel = (i: number) => view.rows?.[i] ?? String(i);
  const colLabel = (j: number) => view.cols?.[j] ?? String(j);
  grid.appendChild(el("div", "cm-lab-heat-corner"));
  for (let j = 0; j < cols; j++) {
    const h = el("div", "cm-lab-heat-col", colLabel(j));
    h.title = colLabel(j);
    grid.appendChild(h);
  }
  for (let i = 0; i < rows; i++) {
    const h = el("div", "cm-lab-heat-row", rowLabel(i));
    h.title = rowLabel(i);
    grid.appendChild(h);
    for (let j = 0; j < cols; j++) {
      const v = view.data[i]?.[j];
      const cell = el("div", "cm-lab-heat-cell");
      if (v == null || !Number.isFinite(v)) {
        cell.classList.add("empty");
      } else {
        // magnitude is one hue light→dark; a signed map mixes each pole from a neutral midpoint
        if (diverging) {
          const p = Math.min(1, Math.abs(v) / max);
          cell.style.background = `color-mix(in oklab, var(${v >= 0 ? "--lab-div-pos" : "--lab-div-neg"}) ${Math.round(p * 100)}%, var(--lab-div-mid))`;
          if (p > 0.55) cell.classList.add("ink-hi");
        } else {
          const p = Math.min(1, Math.max(0, (v - min) / (max - min)));
          cell.style.background = `color-mix(in oklab, var(--lab-seq-hi) ${Math.round(p * 100)}%, var(--lab-seq-lo))`;
          if (p > 0.55) cell.classList.add("ink-hi");
        }
        if (showNumbers) cell.textContent = fmt(v, view.digits ?? (Number.isInteger(v) ? 0 : 2));
      }
      cell.addEventListener("pointermove", (e) => {
        tip.show(`${rowLabel(i)} · ${colLabel(j)}`, [{ slot: 0, label: view.title || "value", value: fmt(v, view.digits) }], e.clientX, e.clientY);
      });
      cell.addEventListener("pointerleave", () => tip.hide());
      grid.appendChild(cell);
    }
  }
  wrap.appendChild(grid);
  // the scale legend: what light and dark mean here
  const scale = el("div", "cm-lab-scale");
  const bar = el("i", diverging ? "cm-lab-scale-bar diverging" : "cm-lab-scale-bar");
  scale.append(el("span", "cm-lab-scale-min", fmt(min, view.digits)), bar, el("span", "cm-lab-scale-max", fmt(max, view.digits)));
  wrap.appendChild(scale);
  return wrap;
}

// ---- table ----

type TableView = Extract<LabView, { kind: "table" }>;

export function buildTable(view: TableView): HTMLElement {
  const wrap = el("div", "cm-lab-tablewrap");
  const table = el("table", "cm-lab-table");
  const thead = el("thead");
  const hr = el("tr");
  view.columns.forEach((c, j) => {
    const th = el("th", undefined, c);
    if (view.highlight?.col === j) th.classList.add("hl");
    hr.appendChild(th);
  });
  thead.appendChild(hr);
  table.appendChild(thead);
  const tbody = el("tbody");
  view.rows.forEach((r, i) => {
    const tr = el("tr");
    if (view.highlight?.row === i) tr.classList.add("hl");
    r.forEach((c, j) => {
      const td = el("td");
      if (typeof c === "number") {
        td.textContent = fmt(c, view.digits);
        td.classList.add("num");
      } else td.textContent = c ?? "–";
      if (view.highlight?.col === j) td.classList.add("hl");
      tr.appendChild(td);
    });
    tbody.appendChild(tr);
  });
  table.appendChild(tbody);
  wrap.appendChild(table);
  return wrap;
}

// ---- step ----

type StepView = Extract<LabView, { kind: "step" }>;

/** A step's text with its `$…$` typeset: the prose as text nodes, the formulas as KaTeX (trust:false). */
function richText(text: string): HTMLElement {
  const p = el("span", "cm-lab-step-text");
  let at = 0;
  for (const m of scanInlineMath(text)) {
    if (m.start > at) p.appendChild(document.createTextNode(text.slice(at, m.start)));
    const span = el("span", "cm-lab-step-math");
    span.innerHTML = renderMathHtml(m.tex, m.display);
    p.appendChild(span);
    at = m.end;
  }
  if (at < text.length) p.appendChild(document.createTextNode(text.slice(at)));
  return p;
}

/** A step's value: a scalar, a row of chips, or a small grid. */
function stepValue(v: StepItem["value"]): HTMLElement | null {
  if (v == null) return null;
  if (typeof v === "number") return el("span", "cm-lab-step-num", fmt(v));
  if (!v.length) return null;
  if (Array.isArray(v[0])) {
    const g = el("div", "cm-lab-mini");
    const rows = v as (number | null)[][];
    const cols = Math.max(...rows.map((r) => r.length));
    g.style.gridTemplateColumns = `repeat(${cols}, auto)`;
    for (const r of rows) for (let j = 0; j < cols; j++) g.appendChild(el("span", undefined, fmt(r[j] ?? null)));
    return g;
  }
  const row = el("div", "cm-lab-mini cm-lab-mini-row");
  for (const x of v as (number | null)[]) row.appendChild(el("span", undefined, fmt(x)));
  return row;
}

type StepItem = StepView["steps"][number];

/** The derivation, revealed one step at a time; `labels` are the button captions the caller localises. */
export function buildStep(view: StepView, labels: { next: string; all: string; reset: string }): HTMLElement {
  const wrap = el("div", "cm-lab-steps");
  const list = el("ol", "cm-lab-step-list");
  const items = view.steps.map((s, i) => {
    const li = el("li", "cm-lab-step");
    li.appendChild(el("span", "cm-lab-step-n", String(i + 1)));
    const body = el("div", "cm-lab-step-body");
    body.appendChild(el("span", "cm-lab-step-label", s.label));
    if (s.text) body.appendChild(richText(s.text));
    const v = stepValue(s.value);
    if (v) body.appendChild(v);
    li.appendChild(body);
    list.appendChild(li);
    return li;
  });
  let shown = Math.min(1, items.length);
  const bar = el("div", "cm-lab-step-bar");
  const next = el("button", "cm-lab-btn", labels.next);
  const all = el("button", "cm-lab-btn", labels.all);
  const reset = el("button", "cm-lab-btn", labels.reset);
  const counter = el("span", "cm-lab-step-count");
  const apply = () => {
    items.forEach((li, i) => {
      li.classList.toggle("hidden", i >= shown);
      li.classList.toggle("current", i === shown - 1);
    });
    counter.textContent = `${shown} / ${items.length}`;
    next.disabled = shown >= items.length;
    all.disabled = shown >= items.length;
    reset.disabled = shown <= 1;
  };
  for (const b of [next, all, reset]) swallowPress(b);
  next.addEventListener("click", () => {
    shown = Math.min(items.length, shown + 1);
    apply();
  });
  all.addEventListener("click", () => {
    shown = items.length;
    apply();
  });
  reset.addEventListener("click", () => {
    shown = Math.min(1, items.length);
    apply();
  });
  bar.append(next, all, reset, counter);
  apply();
  wrap.append(list, bar);
  return wrap;
}

// ---- the table twin of a chart ----

/** The kinds that have a table twin — the toggle asks this rather than building the
 *  twin to find out, so a figure nobody switches never pays for one. */
export const hasTableTwin = (kind: LabView["kind"]) => kind === "line" || kind === "bars" || kind === "scatter" || kind === "heatmap";

/** Every chart has a table twin (the WCAG-clean equivalent, and the way to read exact
 *  values without hovering); null for a view that is already a table or a list. */
export function tableTwin(view: LabView): HTMLElement | null {
  switch (view.kind) {
    case "line": {
      const n = view.series.length;
      const xs = [...new Set(view.series.flatMap((s) => s.y.map((_, i) => xOf(s, view, i)).filter((x): x is number => x != null)))].sort((a, b) => a - b);
      const byX = view.series.map((s) => new Map(s.y.map((y, i) => [xOf(s, view, i), y] as const).filter(([x]) => x != null)));
      return buildTable({
        kind: "table",
        title: view.title,
        columns: [view.x_label ?? "x", ...view.series.map((s, i) => seriesName(s.name, i, n) || "y")],
        rows: xs.map((x) => [x, ...byX.map((m) => m.get(x) ?? null)]),
        digits: null,
        highlight: null,
      });
    }
    case "bars":
      return buildTable({
        kind: "table",
        title: view.title,
        columns: ["", ...view.series.map((s, i) => seriesName(s.name, i, view.series.length) || (view.y_label ?? "value"))],
        rows: view.labels.map((l, i) => [l, ...view.series.map((s) => s.values[i] ?? null)]),
        digits: null,
        highlight: null,
      });
    case "scatter":
      return buildTable({
        kind: "table",
        title: view.title,
        columns: [...(view.series.length > 1 ? [""] : []), view.x_label ?? "x", view.y_label ?? "y"],
        rows: view.series.flatMap((s, si) => s.points.map((p) => [...(view.series.length > 1 ? [seriesName(s.name, si, view.series.length)] : []), p[0], p[1]])),
        digits: null,
        highlight: null,
      });
    case "heatmap": {
      const cols = Math.max(0, ...view.data.map((r) => r.length));
      return buildTable({
        kind: "table",
        title: view.title,
        columns: ["", ...Array.from({ length: cols }, (_, j) => view.cols?.[j] ?? String(j))],
        rows: view.data.map((r, i) => [view.rows?.[i] ?? String(i), ...Array.from({ length: cols }, (_, j) => r[j] ?? null)]),
        digits: view.digits,
        highlight: null,
      });
    }
    default:
      return null;
  }
}
