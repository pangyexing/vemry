// Block provenance in the editor: who wrote each block (the user, the AI, a clip, a
// capture) and which AI blocks still await review. The rows come from
// `provenance_get` (byte spans mapped to character positions by the editor) and
// are held in a StateField that maps them through later edits, so the marks stay
// on their lines while the user types; the field is refilled from disk truth
// whenever the index changes and the buffer is clean. Two independent layers:
// the amber pending bar (on by default, a setting) and the four-colour origin
// overlay (a command toggles it). Labels come from the catalog.
import { t } from "$lib/i18n/i18n.svelte";
import type { Origin } from "$lib/api";
import { formatSessionId } from "$lib/views/session";
import { type EditorState, type Extension, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";

export interface ProvRow {
  /** Character positions (the editor's), covering the block's lines. */
  from: number;
  to: number;
  local_id: string;
  origin: Origin;
}

/** Replace every row (a fresh read of disk truth). */
export const provenanceSet = StateEffect.define<ProvRow[]>();
/** The origin overlay on or off. */
export const provenanceOverlay = StateEffect.define<boolean>();
/** The pending bar on or off (the `provenance_marks` setting). */
export const provenanceMarks = StateEffect.define<boolean>();

/** What the editor needs from its host to act on a block: marking it reviewed. */
export interface ProvenanceHost {
  acceptProvenance(localId: string | null): void;
}

/** The one-click control on an unreviewed block's first line — reviewing is one
 *  gesture, the way an inline diff is accepted, instead of grip → menu → item. */
class ReviewWidget extends WidgetType {
  constructor(
    readonly localId: string,
    readonly host: ProvenanceHost | null,
  ) {
    super();
  }
  eq(other: ReviewWidget) {
    return other.localId === this.localId;
  }
  toDOM() {
    const el = document.createElement("button");
    el.className = "cm-prov-review";
    el.type = "button";
    // a bare check: the amber bar already says what the block is, the tooltip says what the click does
    el.textContent = "✓";
    el.title = t("prov.acceptBlock");
    el.addEventListener("mousedown", (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      this.host?.acceptProvenance(this.localId);
    });
    return el;
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

interface ProvState {
  rows: ProvRow[];
  overlay: boolean;
  marks: boolean;
  decos: DecorationSet;
}

function build(state: EditorState, rows: ProvRow[], overlay: boolean, marks: boolean, host: ProvenanceHost | null): DecorationSet {
  const doc = state.doc;
  const decos: ReturnType<Decoration["range"]>[] = [];
  for (const r of rows) {
    const cls: string[] = [];
    const pending = marks && isPending(r.origin);
    if (overlay) cls.push(`cm-prov-${r.origin.origin}`);
    if (pending) cls.push("cm-prov-pending");
    if (!cls.length) continue;
    const from = Math.min(r.from, doc.length);
    const to = Math.min(Math.max(r.to, from), doc.length);
    const first = doc.lineAt(from).number;
    const last = doc.lineAt(to > from ? to - 1 : to).number;
    const deco = Decoration.line({ class: cls.join(" "), attributes: { title: originLabel(r.origin) } });
    for (let n = first; n <= last; n++) decos.push(deco.range(doc.line(n).from));
    // the one-click review control, after the first line's text
    if (pending) decos.push(Decoration.widget({ widget: new ReviewWidget(r.local_id, host), side: 1 }).range(doc.line(first).to));
  }
  return Decoration.set(decos, true);
}

let currentHost: ProvenanceHost | null = null;

const field = StateField.define<ProvState>({
  create: () => ({ rows: [], overlay: false, marks: true, decos: Decoration.none }),
  update(v, tr) {
    let { rows, overlay, marks } = v;
    let changed = false;
    if (tr.docChanged) {
      // A block that was deleted outright maps to an empty range and is dropped
      rows = rows
        .map((r) => ({ ...r, from: tr.changes.mapPos(r.from, 1), to: tr.changes.mapPos(r.to, -1) }))
        .filter((r) => r.to > r.from);
      changed = true;
    }
    for (const e of tr.effects) {
      if (e.is(provenanceSet)) {
        rows = e.value;
        changed = true;
      } else if (e.is(provenanceOverlay)) {
        overlay = e.value;
        changed = true;
      } else if (e.is(provenanceMarks)) {
        marks = e.value;
        changed = true;
      }
    }
    if (!changed) return v;
    return { rows, overlay, marks, decos: build(tr.state, rows, overlay, marks, currentHost) };
  },
  provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
});

/** The host is module-level: one editor instance at a time, and a StateField cannot carry a closure. */
export function provenanceExtension(host: ProvenanceHost | null = null): Extension {
  currentHost = host;
  return field;
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

/** The start of the next pending block after pos (wrapping to the first), or null. */
export function nextPending(state: EditorState, pos: number): number | null {
  const pend = pendingRows(state);
  if (!pend.length) return null;
  const line = state.doc.lineAt(pos);
  const next = pend.find((r) => r.from > line.to) ?? pend[0];
  return next.from;
}
