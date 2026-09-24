// Block provenance in the editor: who wrote each block (the user, the AI, a clip, a
// capture) and which AI blocks still await review. The rows come from
// `provenance_get` (byte spans mapped to character positions by the editor) and
// are held in a StateField that maps them through later edits, so the marks stay
// on their lines while the user types; the field is refilled from disk truth
// whenever the index changes and the buffer is clean. Two independent layers:
// the amber pending bar (on by default, a setting) and the four-colour origin
// overlay (a command toggles it). Labels come from the catalog.
//
// Reviewing works in runs. The ledger keeps each write as hunks; the blocks one
// hunk produced (a heading and the paragraphs under it, say) are shown as one
// run with one pair of controls, and accepting or reverting the run takes all of
// its blocks. The block stays the unit of the ledger: a block inside a run of
// several gets its own pair on hover. A hunk that replaced lines carries what it
// replaced, unfolded above the run on request.
import { t } from "$lib/i18n/i18n.svelte";
import type { Origin } from "$lib/api";
import { formatSessionId } from "$lib/views/session";
import { blockRoot } from "./blockwidget";
import { type EditorState, type Extension, Prec, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, keymap, WidgetType } from "@codemirror/view";

export interface ProvRow {
  /** Character positions (the editor's), covering the block's lines. */
  from: number;
  to: number;
  /** The lines the pending write added, when known: the bar and the controls sit there. */
  changed?: { from: number; to: number };
  /** Where the pending write's hunk sits (the run key: blocks sharing it arrived together). */
  hunk?: { from: number; to: number };
  /** The lines that hunk replaced, when the write replaced rather than added. */
  replaced?: string;
  local_id: string;
  origin: Origin;
}

/** The pending blocks one write's hunk produced, reviewed as one. */
export interface Run {
  /** The shown lines (character positions): the union of the blocks' changed spans. */
  from: number;
  to: number;
  ids: string[];
  rows: ProvRow[];
}

/** A run's key — the first block's local id, what the unfolded-old-text set is keyed
 *  by — and the lines its write replaced, when it replaced rather than added. Both are
 *  the first row's, read from it rather than copied beside it. */
const runKey = (run: Run) => run.rows[0].local_id;
const runReplaced = (run: Run) => run.rows[0].replaced;

/** Replace every row (a fresh read of disk truth). */
export const provenanceSet = StateEffect.define<ProvRow[]>();
/** The origin overlay on or off. */
export const provenanceOverlay = StateEffect.define<boolean>();
/** The pending bar on or off (the `provenance_marks` setting). */
export const provenanceMarks = StateEffect.define<boolean>();
/** Unfold or fold what a run's write replaced (by the run's key). */
export const provenanceToggleOld = StateEffect.define<string>();

/** What the editor needs from its host to act on blocks: marking them reviewed, or
 *  putting the write behind them back. `null` means every pending block of the note. */
export interface ProvenanceHost {
  acceptProvenance(ids: string[] | null): void;
  rejectProvenance(ids: string[] | null): void;
}

/** The one-click controls on a run's first line (and, on hover, on a block inside a
 *  run of several) — reviewing is one gesture either way, the way an inline diff is
 *  accepted or rejected, instead of grip → menu → item. */
class ReviewWidget extends WidgetType {
  constructor(
    readonly ids: string[],
    readonly host: ProvenanceHost | null,
    /** The run's key and whether its old text is unfolded — null when there is none to
     *  unfold, which is the only state in which "unfolded" would mean anything. */
    readonly old: { key: string; open: boolean } | null,
    /** A block's own pair inside a run: shown on hover only. */
    readonly hover: boolean,
  ) {
    super();
  }
  eq(other: ReviewWidget) {
    return (
      other.ids.join("\0") === this.ids.join("\0") &&
      other.old?.key === this.old?.key &&
      other.old?.open === this.old?.open &&
      other.hover === this.hover
    );
  }
  toDOM(view: EditorView) {
    const wrap = document.createElement("span");
    wrap.className = this.hover ? "cm-prov-controls cm-prov-block" : "cm-prov-controls";
    const button = (glyph: string, title: string, cls: string, act: () => void) => {
      const el = document.createElement("button");
      el.className = cls;
      el.type = "button";
      el.textContent = glyph;
      el.title = title;
      el.addEventListener("mousedown", (e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.stopPropagation();
        act();
      });
      wrap.appendChild(el);
    };
    // bare glyphs: the amber bar already says what the block is, the tooltips say what a click does
    const one = this.ids.length === 1;
    button("✓", t(one ? "prov.acceptBlock" : "prov.acceptRun", { n: this.ids.length }), "cm-prov-review", () =>
      this.host?.acceptProvenance(this.ids),
    );
    button("✕", t(one ? "prov.rejectBlock" : "prov.rejectRun", { n: this.ids.length }), "cm-prov-review cm-prov-reject", () =>
      this.host?.rejectProvenance(this.ids),
    );
    if (this.old) {
      const { key, open } = this.old;
      button("⇄", t(open ? "prov.hideReplaced" : "prov.showReplaced"), `cm-prov-review cm-prov-old-toggle${open ? " active" : ""}`, () =>
        view.dispatch({ effects: provenanceToggleOld.of(key) }),
      );
    }
    return wrap;
  }
}

/** What a run's write replaced, unfolded above the run: read-only, struck through. */
class ReplacedWidget extends WidgetType {
  constructor(
    readonly key: string,
    readonly text: string,
  ) {
    super();
  }
  eq(other: ReplacedWidget) {
    return other.key === this.key && other.text === this.text;
  }
  toDOM() {
    const card = document.createElement("div");
    card.className = "cm-prov-old";
    const label = document.createElement("div");
    label.className = "cm-prov-old-label";
    label.textContent = t("prov.replacedLabel");
    const body = document.createElement("pre");
    body.className = "cm-prov-old-body";
    body.textContent = this.text.replace(/\n$/, "");
    card.append(label, body);
    return blockRoot(card);
  }
}

export function isPending(o: Origin): boolean {
  return o.origin === "agent" && o.pending;
}

/** The ledger's RFC3339 (UTC) stamp as local `YYYY-MM-DD HH:MM`. */
function when(ts?: string): string | null {
  if (!ts) return null;
  const d = new Date(ts);
  if (Number.isNaN(d.getTime())) return ts.slice(0, 16).replace("T", " ");
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** One line of text for a tooltip, the status bar and the block menu. Times are
 *  local; the session is named by its local start time (its id is a UTC stamp that
 *  reads like a time and is not one the user recognises). `withState` adds the
 *  unreviewed word — off where a count badge already says it (the status bar). */
export function originLabel(o: Origin, opts: { withState?: boolean } = {}): string {
  const withState = opts.withState ?? true;
  switch (o.origin) {
    case "you":
      return t("prov.origin.you");
    case "agent": {
      const parts = [t("prov.origin.agent")];
      const w = when(o.ts);
      if (w) parts.push(w);
      else if (o.session) parts.push(t("prov.session", { id: formatSessionId(o.session) }));
      else parts.push(t("prov.sessionUnknown"));
      if (o.pending && withState) parts.push(t("prov.pending"));
      return parts.join(" · ");
    }
    case "clip": {
      const parts = [t("prov.origin.clip")];
      if (o.url) parts.push(o.url);
      const w = when(o.ts);
      if (w) parts.push(w);
      return parts.join(" · ");
    }
    case "capture": {
      const parts = [t("prov.origin.capture")];
      const w = when(o.ts);
      if (w) parts.push(w);
      return parts.join(" · ");
    }
  }
}

/** The lines a pending row shows: the ones its write added when the ledger knows
 *  them (a block the model extended keeps its reviewed lines bare), else the block. */
function shown(r: ProvRow): { from: number; to: number } {
  return r.changed ?? { from: r.from, to: r.to };
}

/** Group the pending rows into runs: rows sharing a located hunk arrived in one
 *  write and go together; a row whose hunk the ledger cannot place stands alone. */
export function runsOf(rows: ProvRow[]): Run[] {
  const runs: Run[] = [];
  let open: Run | null = null;
  let openHunk: string | null = null;
  for (const r of rows) {
    if (!isPending(r.origin)) continue;
    const s = shown(r);
    const hunk = r.hunk ? `${r.hunk.from}:${r.hunk.to}` : null;
    if (open && hunk && hunk === openHunk) {
      open.from = Math.min(open.from, s.from);
      open.to = Math.max(open.to, s.to);
      open.ids.push(r.local_id);
      open.rows.push(r);
      continue;
    }
    open = { from: s.from, to: s.to, ids: [r.local_id], rows: [r] };
    openHunk = hunk;
    runs.push(open);
  }
  return runs;
}

interface ProvState {
  rows: ProvRow[];
  overlay: boolean;
  marks: boolean;
  /** The runs whose old text is unfolded (by run key). */
  oldOpen: Set<string>;
  decos: DecorationSet;
}

function build(state: EditorState, rows: ProvRow[], overlay: boolean, marks: boolean, oldOpen: Set<string>, host: ProvenanceHost | null): DecorationSet {
  const doc = state.doc;
  const decos: ReturnType<Decoration["range"]>[] = [];
  const lines = (from: number, to: number) => {
    const f = Math.min(from, doc.length);
    const t = Math.min(Math.max(to, f), doc.length);
    return { first: doc.lineAt(f).number, last: doc.lineAt(t > f ? t - 1 : t).number };
  };
  for (const r of rows) {
    const pending = marks && isPending(r.origin);
    if (!overlay && !pending) continue;
    const block = lines(r.from, r.to);
    // The pending bar sits on the lines the write changed when the ledger knows them (a
    // block the model extended keeps its reviewed lines bare), else on the whole block;
    // the block stays the unit of accepting, the line is the unit of showing
    const pend = pending ? lines(shown(r).from, shown(r).to) : null;
    // No title on the lines: a native tooltip over every hovered line of an AI block
    // covered the text being read, and the status bar already names the origin of the
    // block under the cursor
    for (let n = block.first; n <= block.last; n++) {
      const cls: string[] = [];
      if (overlay) cls.push(`cm-prov-${r.origin.origin}`);
      if (pend && n >= pend.first && n <= pend.last) cls.push("cm-prov-pending");
      if (cls.length) decos.push(Decoration.line({ class: cls.join(" ") }).range(doc.line(n).from));
    }
  }
  if (marks) {
    for (const run of runsOf(rows)) {
      const first = doc.lineAt(Math.min(run.from, doc.length));
      const key = runKey(run);
      const replaced = runReplaced(run);
      const open = oldOpen.has(key);
      // the run's controls after its first line's text; what the write replaced above it when unfolded
      decos.push(Decoration.widget({ widget: new ReviewWidget(run.ids, host, replaced ? { key, open } : null, false), side: 1 }).range(first.to));
      if (open && replaced) {
        decos.push(Decoration.widget({ widget: new ReplacedWidget(key, replaced), block: true, side: -1 }).range(first.from));
      }
      // a block inside a run of several keeps its own pair, shown on hover
      if (run.rows.length > 1) {
        for (const r of run.rows) {
          const line = doc.lineAt(Math.min(shown(r).from, doc.length));
          if (line.number === first.number) continue;
          decos.push(Decoration.widget({ widget: new ReviewWidget([r.local_id], host, null, true), side: 1 }).range(line.to));
        }
      }
    }
  }
  return Decoration.set(decos, true);
}

let currentHost: ProvenanceHost | null = null;

const field = StateField.define<ProvState>({
  create: () => ({ rows: [], overlay: false, marks: true, oldOpen: new Set(), decos: Decoration.none }),
  update(v, tr) {
    let { rows, overlay, marks, oldOpen } = v;
    let changed = false;
    if (tr.docChanged) {
      // A block that was deleted outright maps to an empty range and is dropped; a
      // changed span that collapses falls back to the block; the hunk key maps along
      const span = (s: { from: number; to: number }) => ({ from: tr.changes.mapPos(s.from, 1), to: tr.changes.mapPos(s.to, -1) });
      rows = rows
        .map((r) => {
          const c = r.changed && span(r.changed);
          const h = r.hunk && span(r.hunk);
          return { ...r, ...span(r), changed: c && c.to > c.from ? c : undefined, hunk: h && h.to >= h.from ? h : undefined };
        })
        .filter((r) => r.to > r.from);
      changed = true;
    }
    for (const e of tr.effects) {
      if (e.is(provenanceSet)) {
        rows = e.value;
        // a fresh read names fresh runs: what was unfolded for a run that is gone is dropped
        const keys = new Set(runsOf(rows).map(runKey));
        oldOpen = new Set([...oldOpen].filter((k) => keys.has(k)));
        changed = true;
      } else if (e.is(provenanceOverlay)) {
        overlay = e.value;
        changed = true;
      } else if (e.is(provenanceMarks)) {
        marks = e.value;
        changed = true;
      } else if (e.is(provenanceToggleOld)) {
        oldOpen = new Set(oldOpen);
        if (oldOpen.has(e.value)) oldOpen.delete(e.value);
        else oldOpen.add(e.value);
        changed = true;
      }
    }
    if (!changed) return v;
    return { rows, overlay, marks, oldOpen, decos: build(tr.state, rows, overlay, marks, oldOpen, currentHost) };
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
});

/** The host is module-level: one editor instance at a time, and a StateField cannot carry a closure. */
export function provenanceExtension(host: ProvenanceHost | null = null): Extension {
  currentHost = host;
  return [field, reviewKeymap(host)];
}

/** ⌘Y accepts the run under the cursor and ⌘⇧Y puts it back — the host advances to
 *  the next run afterwards, so a review is the same key pressed again and again. Off a
 *  run the keys fall through (Ctrl+Y is redo where the platform binds it so). */
function reviewKeymap(host: ProvenanceHost | null): Extension {
  const on = (act: (ids: string[]) => void) => (v: EditorView) => {
    const run = runAt(v.state, v.state.selection.main.head);
    if (!run || !host) return false;
    act(run.ids);
    return true;
  };
  return Prec.high(
    keymap.of([
      { key: "Mod-y", run: on((ids) => host?.acceptProvenance(ids)) },
      { key: "Mod-Shift-y", run: on((ids) => host?.rejectProvenance(ids)) },
    ]),
  );
}

/** The row whose lines hold pos (the block under the cursor), if any. */
export function originAt(state: EditorState, pos: number): ProvRow | null {
  const rows = state.field(field, false)?.rows;
  if (!rows?.length) return null;
  const line = state.doc.lineAt(pos);
  for (const r of rows) {
    if (r.from > line.to) break;
    if (line.from <= Math.max(r.to - 1, r.from) && line.to >= r.from) return r;
  }
  return null;
}

export function pendingRows(state: EditorState): ProvRow[] {
  return state.field(field, false)?.rows.filter((r) => isPending(r.origin)) ?? [];
}

export function pendingRuns(state: EditorState): Run[] {
  return runsOf(state.field(field, false)?.rows ?? []);
}

/** The run whose shown lines hold pos's line, if any. */
export function runAt(state: EditorState, pos: number): Run | null {
  const line = state.doc.lineAt(pos);
  for (const run of pendingRuns(state)) {
    if (run.from > line.to) break;
    if (line.from <= Math.max(run.to - 1, run.from) && line.to >= run.from) return run;
  }
  return null;
}

export type ReviewStep = "next" | "prev" | "first" | "last";

/** The start of the run a review step lands on, from pos: the next run after pos's
 *  line, the last one starting before it, or the note's first or last run. Null when
 *  there is none that way — the note's end, where the host steps to the next note. */
export function stepRun(state: EditorState, pos: number, which: ReviewStep): number | null {
  const runs = pendingRuns(state);
  if (!runs.length) return null;
  const line = state.doc.lineAt(pos);
  let run: Run | undefined;
  switch (which) {
    case "first":
      run = runs[0];
      break;
    case "last":
      run = runs[runs.length - 1];
      break;
    case "next":
      run = runs.find((r) => r.from > line.to);
      break;
    case "prev":
      run = [...runs].reverse().find((r) => r.from < line.from);
      break;
  }
  return run ? run.from : null;
}
