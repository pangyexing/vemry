// The lab of a js block, live in the editor: the controls the block declared, the
// views it drew, and the re-run loop between them. A session begins when the user
// presses Run on a block that called vemry.lab.* (runblock.ts hands the result
// here) and ends when the output is cleared or the document closes — while it is
// on, moving a control re-runs the block with the controls' values as params
// (through the same one-shot read-only sandbox, under the same fuse) and redraws
// the views in place. The code is read again at every re-run, so an edit to the
// block takes effect on the next move. Nothing in this loop writes to the note:
// the values live in this object, and "Pin" is the one explicit write, rewriting
// the declared literals through pin.ts.
import { t } from "$lib/i18n/i18n.svelte";
import type { BlockEvalResult, LabControl, LabParams, LabSpec, LabView, LabPreset } from "$lib/api";
import { buildHeatmap, buildStep, buildTable, drawBars, drawLine, drawScatter, fmt, hasTableTwin, keySwatch, tableTwin, type TipHost, type TipRow } from "$lib/lab/charts";
import { el, swallowPress } from "$lib/lab/dom";
import { pinCode } from "$lib/lab/pin";
import { toast } from "$lib/stores/toast.svelte";
import { Annotation } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";

/** What the session needs from the editor: the block's code as it currently stands
 *  (null once the block has left the document), a way to replace it (pin), and the
 *  sandbox. `range` is the code's span in the document, or null when it cannot be
 *  replaced as one piece (a fence inside a quote is cut into pieces by the parser). */
export interface LabHost {
  locate(): { code: string; range: { from: number; to: number } | null } | null;
  evalBlock(code: string, params: LabParams | null): Promise<BlockEvalResult>;
  replaceCode(range: { from: number; to: number }, code: string): void;
  /** Show the code: the cursor lands on the first code line, and the card gives way to the source. */
  reveal(): void;
  close(): void;
}

const RERUN_DEBOUNCE_MS = 60;

// ---- the interaction window ----
// While a pointer is down on a card, and briefly after, the browser may move its own
// caret to the edge of the widget's non-editable island (a slider, a switch and a
// select keep their default mousedown — they are dragged and ticked — and WebKit
// settles its caret beside the island on such a press, before it or after it as it
// pleases). The editor reads that caret back as a selection on the fence's first or
// last line, which reveals the code under the reader's hand. runblock's transaction
// filter drops a selection-only transaction while this window is open; the card's
// own ✎ reveal carries `cardReveal` and passes.
let interactingUntil = 0;
export function labInteracting(): boolean {
  return Date.now() < interactingUntil;
}
/** Carried by the card's ✎: a selection the reader asked for, never dropped. */
export const cardReveal = Annotation.define<boolean>();

const ICON_PIN =
  '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 17v5"/><path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z"/></svg>';
const ICON_RERUN =
  '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 12a9 9 0 1 1-3-6.7"/><path d="M21 3v6h-6"/></svg>';
const ICON_CLOSE =
  '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>';
export const ICON_EDIT =
  '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"/><path d="m15 5 4 4"/></svg>';
const ICON_TABLE =
  '<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="16" rx="2"/><path d="M3 10h18M3 15h18M10 4v16"/></svg>';
const ICON_CHART =
  '<svg viewBox="0 0 24 24" width="13" height="13" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 3v18h18"/><path d="m7 15 4-5 4 3 5-7"/></svg>';

/** A button inside the widget: mousedown is swallowed so the editor's selection stays
 *  where it is (a click inside a block widget would otherwise land the cursor by
 *  coordinates), and the action runs on click. */
export function iconButton(cls: string, icon: string, tip: string, onClick: () => void): HTMLButtonElement {
  const b = el("button", cls);
  b.innerHTML = icon;
  b.dataset.tip = tip;
  b.title = tip;
  swallowPress(b);
  b.addEventListener("click", (e) => {
    e.preventDefault();
    e.stopPropagation();
    onClick();
  });
  return b;
}

/** What a run leaves to read: its output and its error, in that order. The live card
 *  and the plain output widget (runblock.ts) show the same text. */
export const joinOutput = (output: string, error: string | null) => [output, error ?? ""].filter(Boolean).join("\n");

/** The initial params: what every control ran with (so the first move sends the whole set). */
function paramsOf(controls: LabControl[]): LabParams {
  const p: LabParams = {};
  for (const c of controls) p[c.name] = c.kind === "matrix" ? c.rows.map((r) => r.slice()) : c.kind === "presets" ? (c.value ?? "") : c.value;
  return p;
}

/** A control as it would read with `v` as its value (the params' view of the spec). */
function withValue(c: LabControl, v: LabParams[string] | undefined): LabControl {
  switch (c.kind) {
    case "slider":
      return { ...c, value: typeof v === "number" ? v : c.value };
    case "toggle":
      return { ...c, value: typeof v === "boolean" ? v : c.value };
    case "select":
      return { ...c, value: typeof v === "string" ? v : c.value };
    case "matrix":
      return { ...c, rows: Array.isArray(v) ? (v as number[][]) : c.rows };
    case "presets":
      return { ...c, value: typeof v === "string" && v ? v : null };
  }
}

/** The identity of the control set: a re-run that changes it rebuilds the controls, one
 *  that keeps it updates nothing the user is holding. A slider's range belongs to that
 *  identity — a block whose bounds follow another control would otherwise keep a widget
 *  the reader can no longer move to the values it now accepts. */
const controlKey = (controls: LabControl[]) =>
  controls
    .map(
      (c) =>
        `${c.kind}:${c.name}${
          c.kind === "slider"
            ? `:${c.min}:${c.max}:${c.step}`
            : c.kind === "select"
              ? `:${c.options.join("|")}`
              : c.kind === "matrix"
                ? `:${c.rows.length}x${c.rows[0]?.length ?? 0}`
                : c.kind === "presets"
                  ? `:${c.items.map((i) => i.label).join("|")}`
                  : ""
        }`,
    )
    .join("\n");

export class LabSession implements TipHost {
  readonly root: HTMLElement;
  private readonly controlsEl: HTMLElement;
  private readonly viewsEl: HTMLElement;
  private readonly outEl: HTMLElement;
  private readonly statusEl: HTMLElement;
  private readonly tipEl: HTMLElement;
  /** The tip's measured size, kept so following the pointer forces no layout. */
  private tipW = 0;
  private tipH = 0;
  private params: LabParams;
  private key: string;
  private timer = 0;
  private inflight = false;
  private queued = false;
  private observers: ResizeObserver[] = [];
  private view: EditorView | null = null;
  private closed = false;
  private stale = false;
  /** The code the last run used: the card says so when the fence has been edited since. */
  lastCode: string;

  constructor(
    private readonly host: LabHost,
    private spec: LabSpec,
    output: string,
    code: string,
  ) {
    this.lastCode = code;
    this.params = paramsOf(spec.controls);
    this.key = controlKey(spec.controls);
    this.root = el("div", "cm-lab");
    const head = el("div", "cm-lab-head");
    const live = el("span", "cm-lab-live");
    live.appendChild(el("i", "cm-lab-live-dot"));
    live.appendChild(document.createTextNode(t("runBlock.live")));
    this.statusEl = el("span", "cm-lab-status");
    const tools = el("span", "cm-lab-tools");
    tools.append(
      iconButton("cm-lab-tool", ICON_PIN, t("runBlock.pin"), () => this.pin()),
      iconButton("cm-lab-tool cm-lab-rerun", ICON_RERUN, t("runBlock.rerun"), () => this.schedule(0)),
      iconButton("cm-lab-tool", ICON_EDIT, t("runBlock.editCode"), () => this.host.reveal()),
      iconButton("cm-lab-tool", ICON_CLOSE, t("runBlock.closeLab"), () => this.host.close()),
    );
    head.append(live, this.statusEl, tools);
    this.controlsEl = el("div", "cm-lab-controls");
    this.viewsEl = el("div", "cm-lab-views");
    this.outEl = el("pre", "cm-lab-out");
    this.tipEl = el("div", "cm-lab-tip");
    this.tipEl.style.display = "none";
    this.root.append(head, this.controlsEl, this.viewsEl, this.outEl, this.tipEl);
    // A press anywhere on the card that is not on a form control is swallowed. The
    // browser would otherwise place its caret inside the widget's DOM, and the editor
    // reads that caret back as a selection touching the fence: one press on empty card
    // space (which arms reveal), then one on a button, and the code opened under the
    // reader. A slider, a switch, a select and a number cell keep their default (they
    // are dragged, ticked and typed in); the icon buttons swallow their own.
    this.root.addEventListener("mousedown", (e) => {
      e.stopPropagation();
      if (!(e.target as Element).closest("input, select, textarea, label")) e.preventDefault();
    });
    this.root.addEventListener(
      "pointerdown",
      () => {
        // a select's native popup can swallow the pointerup: the window closes by itself
        interactingUntil = Date.now() + 2000;
        // the caret is placed at the press itself and read back within a few milliseconds:
        // after the release only that tail is covered, so a click elsewhere right after is not
        window.addEventListener("pointerup", () => (interactingUntil = Date.now() + 50), { once: true, capture: true });
      },
      true,
    );
    this.buildControls();
    this.buildViews();
    this.setOutput(output, null);
  }

  /** Called by the widget when its DOM is placed (the view is needed for re-measuring). */
  attach(view: EditorView) {
    this.view = view;
  }

  /** The fence has been edited since the last run: the card says so and the rerun
   *  button is lit; nothing runs until the reader presses it. */
  setStale(stale: boolean) {
    this.stale = stale;
    this.root.classList.toggle("stale", stale);
    if (!this.inflight) this.statusEl.textContent = stale ? t("runBlock.codeChanged") : "";
  }

  destroy() {
    this.closed = true;
    clearTimeout(this.timer);
    for (const o of this.observers) o.disconnect();
    this.observers = [];
  }

  // ---- tooltip (TipHost) ----

  show(title: string, rows: TipRow[], clientX: number, clientY: number) {
    const tip = this.tipEl;
    tip.replaceChildren();
    if (title) tip.appendChild(el("div", "cm-lab-tip-title", title));
    for (const r of rows) {
      const row = el("div", "cm-lab-tip-row");
      if (r.slot) row.appendChild(keySwatch("line", r.slot));
      row.appendChild(el("span", "cm-lab-tip-value", r.value));
      if (r.label) row.appendChild(el("span", "cm-lab-tip-label", r.label));
      tip.appendChild(row);
    }
    tip.style.display = "";
    // the only measurement: while the pointer stays on the same datum the tip's
    // content, and so its size, does not change (move)
    this.tipW = tip.offsetWidth;
    this.tipH = tip.offsetHeight;
    this.place(clientX, clientY);
  }

  move(clientX: number, clientY: number) {
    if (this.tipEl.style.display === "none") return;
    this.place(clientX, clientY);
  }

  private place(clientX: number, clientY: number) {
    const box = this.root.getBoundingClientRect();
    let x = clientX - box.left + 14;
    let y = clientY - box.top + 14;
    if (x + this.tipW > box.width - 4) x = Math.max(4, clientX - box.left - this.tipW - 14);
    if (y + this.tipH > box.height - 4) y = Math.max(4, clientY - box.top - this.tipH - 14);
    this.tipEl.style.left = `${x}px`;
    this.tipEl.style.top = `${y}px`;
  }

  hide() {
    this.tipEl.style.display = "none";
  }

  // ---- controls ----

  private buildControls() {
    this.controlsEl.replaceChildren();
    this.controlsEl.hidden = this.spec.controls.length === 0;
    for (const c of this.spec.controls) this.controlsEl.appendChild(this.buildControl(c));
  }

  private setParam(name: string, value: LabParams[string]) {
    this.params[name] = value;
    this.schedule(RERUN_DEBOUNCE_MS);
  }

  /** A preset sets several controls at once: the params, the widgets (rebuilt from
   *  the new values so the sliders show them), and one re-run. */
  private applyPreset(c: Extract<LabControl, { kind: "presets" }>, item: LabPreset) {
    for (const [k, v] of Object.entries(item.set)) {
      if (!Array.isArray(v)) {
        this.params[k] = v;
        continue;
      }
      // a grid of a shape the matrix was not declared with is one the sandbox drops: the
      // widget must not take it either, or the cells shown stop being the cells that ran
      const target = this.spec.controls.find((ctl) => ctl.name === k);
      if (target?.kind === "matrix" && v.length === target.rows.length && target.rows.every((r, i) => r.length === v[i].length)) {
        this.params[k] = v.map((r) => r.slice());
      }
    }
    this.params[c.name] = item.label;
    this.spec = { ...this.spec, controls: this.spec.controls.map((ctl) => withValue(ctl, this.params[ctl.name])) };
    this.buildControls();
    this.schedule(0);
  }

  private buildControl(c: LabControl): HTMLElement {
    // a label element would hand a click on a preset button to its first input
    const box = el(c.kind === "presets" ? "div" : "label", `cm-lab-ctl cm-lab-ctl-${c.kind}`);
    const stop = (e: Event) => e.stopPropagation();
    box.addEventListener("mousedown", stop);
    box.addEventListener("keydown", stop);
    const name = el("span", "cm-lab-ctl-name", c.label ?? c.name);
    name.title = c.name;
    switch (c.kind) {
      case "slider": {
        const input = el("input");
        input.type = "range";
        input.min = String(c.min);
        input.max = String(c.max);
        input.step = String(c.step);
        input.value = String(c.value);
        const readout = el("span", "cm-lab-ctl-value", fmt(c.value));
        input.addEventListener("input", () => {
          const v = Number(input.value);
          readout.textContent = fmt(v);
          this.setParam(c.name, v);
        });
        box.append(name, input, readout);
        break;
      }
      case "toggle": {
        const input = el("input");
        input.type = "checkbox";
        input.checked = c.value;
        input.className = "cm-lab-switch";
        input.addEventListener("change", () => this.setParam(c.name, input.checked));
        box.append(input, name);
        break;
      }
      case "select": {
        const select = el("select");
        for (const o of c.options) {
          const opt = el("option", undefined, o);
          opt.value = o;
          select.appendChild(opt);
        }
        select.value = c.value;
        select.addEventListener("change", () => this.setParam(c.name, select.value));
        box.append(name, select);
        break;
      }
      case "presets": {
        box.classList.add("cm-lab-ctl-wide");
        const row = el("div", "cm-lab-presets");
        for (const item of c.items) {
          const b = el("button", "cm-lab-preset", item.label);
          b.type = "button";
          if (c.value === item.label) b.setAttribute("aria-pressed", "true");
          // the box above stops the press from reaching the editor; the default (a
          // caret placed in the widget) still has to be refused here, as iconButton does
          swallowPress(b);
          b.addEventListener("click", (e) => {
            e.preventDefault();
            this.applyPreset(c, item);
          });
          row.appendChild(b);
        }
        box.append(name, row);
        break;
      }
      case "matrix": {
        box.classList.add("cm-lab-ctl-wide");
        const grid = el("div", "cm-lab-matrix");
        const cols = c.rows[0]?.length ?? 0;
        grid.style.gridTemplateColumns = `repeat(${cols}, 56px)`;
        const rows = c.rows.map((r) => r.slice());
        c.rows.forEach((r, i) =>
          r.forEach((v, j) => {
            const cell = el("input");
            cell.type = "number";
            cell.value = String(v);
            if (c.min != null) cell.min = String(c.min);
            if (c.max != null) cell.max = String(c.max);
            cell.step = c.step != null ? String(c.step) : "any";
            cell.addEventListener("input", () => {
              const n = Number(cell.value);
              if (!Number.isFinite(n)) return;
              rows[i][j] = n;
              this.setParam(c.name, rows.map((row) => row.slice()));
            });
            grid.appendChild(cell);
          }),
        );
        box.append(name, grid);
        break;
      }
    }
    return box;
  }

  // ---- views ----

  private buildViews() {
    for (const o of this.observers) o.disconnect();
    this.observers = [];
    this.viewsEl.replaceChildren();
    this.viewsEl.hidden = this.spec.views.length === 0;
    for (const v of this.spec.views) this.viewsEl.appendChild(this.buildFigure(v));
    this.view?.requestMeasure();
  }

  private buildFigure(v: LabView): HTMLElement {
    const fig = el("figure", `cm-lab-fig cm-lab-fig-${v.kind}`);
    const head = el("figcaption", "cm-lab-fig-head");
    head.appendChild(el("span", "cm-lab-fig-title", v.title));
    const body = el("div", "cm-lab-fig-body");
    // The twin is built the first time it is asked for: a re-run rebuilds every figure,
    // and a full table of a capped series is thousands of elements nobody has looked at
    let twin: HTMLElement | null = null;
    let showTable = false;
    const draw = () => {
      body.replaceChildren();
      if (showTable) {
        twin ??= tableTwin(v);
        if (twin) {
          body.appendChild(twin);
          return;
        }
      }
      const width = Math.max(200, Math.floor(body.clientWidth || this.viewsEl.clientWidth || 480) - 2);
      switch (v.kind) {
        case "line":
          body.appendChild(drawLine(v, width, this));
          break;
        case "bars":
          body.appendChild(drawBars(v, width, this));
          break;
        case "scatter":
          body.appendChild(drawScatter(v, width, this));
          break;
        case "heatmap":
          body.appendChild(buildHeatmap(v, this));
          break;
        case "table":
          body.appendChild(buildTable(v));
          break;
        case "step":
          body.appendChild(buildStep(v));
          break;
      }
    };
    if (hasTableTwin(v.kind)) {
      const toggle = iconButton("cm-lab-fig-toggle", ICON_TABLE, t("runBlock.tableView"), () => {
        showTable = !showTable;
        toggle.innerHTML = showTable ? ICON_CHART : ICON_TABLE;
        toggle.dataset.tip = toggle.title = t(showTable ? "runBlock.chartView" : "runBlock.tableView");
        draw();
        this.view?.requestMeasure();
      });
      head.appendChild(toggle);
    }
    fig.append(head, body);
    // an axis chart is drawn to the width it has: redrawn as the note's column changes
    if (v.kind === "line" || v.kind === "bars" || v.kind === "scatter") {
      let last = -1;
      let settle: ReturnType<typeof setTimeout> | undefined;
      const redraw = () => {
        if (this.closed) return;
        draw();
        this.view?.requestMeasure();
      };
      const ro = new ResizeObserver(() => {
        const w = body.clientWidth;
        if (w <= 0 || w === last) return;
        const first = last < 0;
        last = w;
        // The first width is drawn at once; after that a drag of the sash or the window
        // changes the width every frame and only the width it settles at is looked at,
        // so the chart is rebuilt once when the drag stops rather than sixty times
        clearTimeout(settle);
        if (first) redraw();
        else settle = setTimeout(redraw, 80);
      });
      ro.observe(body);
      this.observers.push(ro);
    } else {
      draw();
    }
    return fig;
  }

  // ---- output & status ----

  private setOutput(output: string, error: string | null) {
    const text = joinOutput(output, error);
    this.outEl.textContent = text;
    this.outEl.hidden = text === "";
    this.outEl.classList.toggle("cm-lab-out-err", !!error);
  }

  private setStatus(text: string, busy: boolean) {
    this.statusEl.textContent = text || (this.stale ? t("runBlock.codeChanged") : "");
    this.root.classList.toggle("busy", busy);
  }

  // ---- the re-run loop ----

  private schedule(delay: number) {
    if (this.closed) return;
    clearTimeout(this.timer);
    this.timer = window.setTimeout(() => void this.run(), delay);
  }

  private async run() {
    if (this.closed) return;
    if (this.inflight) {
      this.queued = true;
      return;
    }
    const located = this.host.locate();
    if (!located) return; // the block has left the document; the widget goes with it
    this.inflight = true;
    this.setStatus(t("runBlock.running"), true);
    try {
      const r = await this.host.evalBlock(located.code, this.params);
      if (this.closed) return;
      this.lastCode = located.code;
      this.setStale(false);
      this.setOutput(r.output, r.error);
      if (r.lab) {
        this.spec = r.lab;
        const key = controlKey(r.lab.controls);
        if (key !== this.key) {
          // the block now declares a different set: rebuild, starting from what it ran with
          this.key = key;
          this.params = paramsOf(r.lab.controls);
          this.buildControls();
        }
        this.buildViews();
      }
      this.setStatus("", false);
    } catch (e) {
      if (!this.closed) {
        this.setOutput("", String(e));
        this.setStatus("", false);
      }
    } finally {
      this.inflight = false;
      this.view?.requestMeasure();
      if (this.queued) {
        this.queued = false;
        this.schedule(0);
      }
    }
  }

  /** Write the current values into the code's literals (pin.ts) — the one write. */
  private pin() {
    const located = this.host.locate();
    if (!located) return;
    if (!located.range) {
      toast.push(t("runBlock.pinNoRange"));
      return;
    }
    const controls: LabControl[] = this.spec.controls.map((c) => withValue(c, this.params[c.name]));
    const r = pinCode(located.code, controls);
    if (!r.pinned.length) {
      toast.push(t("runBlock.pinNothing"));
      return;
    }
    this.host.replaceCode(located.range, r.code);
    toast.push(r.skipped.length ? t("runBlock.pinPartial", { names: r.skipped.join(", ") }) : t("runBlock.pinDone"));
  }
}
