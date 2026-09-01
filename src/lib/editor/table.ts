// Live preview of a GFM table: with the selection outside the table, the whole thing
// renders as a real <table>, and clicking a cell sends the cursor back to the
// corresponding document position to edit.
// A block-level replace spanning lines can only come from a StateField (a ViewPlugin
// is not allowed), the same reason as livepreview.ts's frontmatter.
// Rows are split from the source on unescaped | (rather than the syntax tree's
// TableCell — an empty cell has no node, which would throw the columns out of line);
// the inline Markdown inside a cell goes through a miniature renderer that only
// displays.
// Incremental updates: a cell records its offset relative to the table's start (a
// click resolves the absolute position with posAtDOM), so a widget does not depend on
// an absolute position and can be mapped across document changes; each transaction
// only recomputes the dirty regions (the edited range, the old and new selections,
// newly parsed areas) rather than rescanning the document.
import { syntaxTree } from "@codemirror/language";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { type EditorState, StateField, type Transaction } from "@codemirror/state";
import { selectionTouches } from "./reveal";

interface Cell {
  text: string;
  pos: number; // the offset relative to the table's start, where the cursor lands on a click
}

type Align = "left" | "center" | "right" | null;

/** Split one row into cells on unescaped |, recording each cell's document position */
function splitRow(text: string, lineFrom: number): Cell[] {
  const cells: Cell[] = [];
  // A table row inside a quote block carries a "> " prefix (the shape a clipped
  // selection produces), so splitting starts after the prefix, or the prefix would
  // become a superfluous empty first column
  const prefix = /^[\s>]*(?=\|)/.exec(text)?.[0].length ?? 0;
  let start = prefix;
  const push = (end: number) => {
    const raw = text.slice(start, end);
    const lead = raw.length - raw.trimStart().length;
    cells.push({ text: raw.trim().replace(/\\\|/g, "|"), pos: lineFrom + start + lead });
    start = end + 1;
  };
  for (let i = prefix; i < text.length; i++) {
    if (text[i] === "\\") i++;
    else if (text[i] === "|") push(i);
  }
  push(text.length);
  // The leading and trailing | produce empty edge cells, which are dropped
  if (cells.length && cells[0].text === "" && text.slice(prefix).startsWith("|")) cells.shift();
  if (cells.length && cells[cells.length - 1].text === "" && text.trimEnd().endsWith("|")) cells.pop();
  return cells;
}

// An inline link is only matched up to "](", and the target is left to closingParen's
// bracket balancing: [^)]* would stop at the first ) inside the target and leak the
// rest of the URL into the body text
const INLINE_RE =
  /`([^`\n]+)`|\*\*([^*\n]+)\*\*|~~([^~\n]+)~~|==([^=\n]+)==|\[\[([^\[\]\n]+)\]\]|\[([^\]\n]+)\]\(|\*([^*\s][^*\n]*)\*|_([^_\s][^_\n]*)_/g;

/** From the open position after "](", find the balanced ")" and return its index; -1
 *  when it does not balance.
 *  CommonMark allows paired brackets inside a link target (an observed sample:
 *  .../sc/(Z(@@x'))/stocks/…) */
function closingParen(text: string, open: number): number {
  let depth = 1;
  for (let i = open; i < text.length; i++) {
    if (text[i] === "(") depth++;
    else if (text[i] === ")" && --depth === 0) return i;
  }
  return -1;
}

/** Cell text → DOM, recognising only the frequent inline syntax and never nesting; written through textContent, so there is no injection surface */
function renderInline(text: string, into: HTMLElement) {
  INLINE_RE.lastIndex = 0;
  let last = 0;
  let m: RegExpExecArray | null;
  const flush = (to: number) => {
    if (to > last) into.appendChild(document.createTextNode(text.slice(last, to)));
  };
  const span = (cls: string, content: string) => {
    const el = document.createElement("span");
    el.className = cls;
    el.textContent = content;
    into.appendChild(el);
  };
  while ((m = INLINE_RE.exec(text))) {
    flush(m.index);
    // The end of the matched marker itself. An inline link still has to eat its target
    // afterwards (see below), while every other branch stops here — the two meanings
    // are named apart, and `last` only ever means "how far has been emitted"
    const end = m.index + m[0].length;
    last = end;
    if (m[1] != null) span("cm-md-mono cm-lp-cell-code", m[1]);
    else if (m[2] != null) span("cm-lp-cell-strong", m[2]);
    else if (m[3] != null) span("cm-lp-strike", m[3]);
    else if (m[4] != null) span("cm-md-mark", m[4]);
    else if (m[5] != null) span("cm-wikilink", m[5].split("|").pop() ?? m[5]);
    else if (m[6] != null) {
      const close = closingParen(text, end);
      if (close < 0) {
        // The brackets do not balance: the whole run falls back to its source rather
        // than swallowing "[label](". flush does not advance last, so it is rewound to
        // the marker's start and the next flush emits the run verbatim
        last = m.index;
        continue;
      }
      span("cm-wikilink", m[6]);
      last = close + 1;
      INLINE_RE.lastIndex = last;
    } else span("cm-lp-cell-em", m[7] ?? m[8]);
  }
  flush(text.length);
}

class TableWidget extends WidgetType {
  constructor(
    readonly src: string,
    readonly header: Cell[],
    readonly aligns: Align[],
    readonly rows: Cell[][],
  ) {
    super();
  }
  eq(other: TableWidget) {
    return other.src === this.src;
  }
  get estimatedHeight() {
    return (this.rows.length + 1) * 34 + 10;
  }
  toDOM(view: EditorView) {
    const wrap = document.createElement("div");
    wrap.className = "cm-lp-table";
    const table = document.createElement("table");
    const addRow = (cells: Cell[], tag: "th" | "td") => {
      const tr = document.createElement("tr");
      cells.forEach((cell, i) => {
        const el = document.createElement(tag);
        el.dataset.pos = String(cell.pos);
        const align = this.aligns[i];
        if (align) el.style.textAlign = align;
        renderInline(cell.text, el);
        tr.appendChild(el);
      });
      table.appendChild(tr);
      return tr;
    };
    addRow(this.header, "th");
    for (const row of this.rows) addRow(row, "td");
    wrap.appendChild(table);
    wrap.onmousedown = (e) => {
      const cell = (e.target as HTMLElement).closest("[data-pos]") as HTMLElement | null;
      if (!cell) return;
      e.preventDefault();
      let base: number;
      try {
        base = view.posAtDOM(wrap); // the widget's current position in the document
      } catch {
        return;
      }
      if (base < 0) return;
      view.dispatch({ selection: { anchor: base + Number(cell.dataset.pos) } });
      view.focus();
    };
    return wrap;
  }
}

function parseAligns(delim: string): Align[] {
  return splitRow(delim, 0).map((c) => {
    const t = c.text;
    if (t.startsWith(":") && t.endsWith(":")) return "center";
    if (t.endsWith(":")) return "right";
    if (t.startsWith(":")) return "left";
    return null;
  });
}

interface Region {
  from: number;
  to: number;
}

/** A Table node's range widened to whole lines */
function tableExtent(state: EditorState, nodeFrom: number, nodeTo: number): Region {
  const doc = state.doc;
  return {
    from: doc.lineAt(nodeFrom).from,
    to: doc.lineAt(Math.min(nodeTo, doc.length)).to,
  };
}

/** One Table node → its decoration; a selection touching the table returns null and the whole source expands for editing */
function buildTable(state: EditorState, nodeFrom: number, nodeTo: number) {
  const doc = state.doc;
  const { from, to } = tableExtent(state, nodeFrom, nodeTo);
  if (selectionTouches(state, from, to)) return null;
  const lines: { text: string; from: number }[] = [];
  for (let pos = from; pos <= to; ) {
    const line = doc.lineAt(pos);
    lines.push({ text: line.text, from: line.from });
    if (line.to >= to) break;
    pos = line.to + 1;
  }
  if (lines.length < 2) return null;
  const header = splitRow(lines[0].text, lines[0].from - from);
  const aligns = parseAligns(lines[1].text);
  const rows = lines.slice(2).map((l) => splitRow(l.text, l.from - from));
  const src = doc.sliceString(from, to);
  return Decoration.replace({
    widget: new TableWidget(src, header, aligns, rows),
    block: true,
  }).range(from, to);
}

/** The decorations of every table in region (iterate enters nodes that intersect the region) */
function computeRegion(state: EditorState, region: Region): ReturnType<Decoration["range"]>[] {
  const decos: ReturnType<Decoration["range"]>[] = [];
  syntaxTree(state).iterate({
    from: region.from,
    to: region.to,
    enter(node) {
      if (node.name !== "Table") return;
      const deco = buildTable(state, node.from, node.to);
      if (deco) decos.push(deco);
      return false;
    },
  });
  return decos;
}

function tableDecos(state: EditorState): DecorationSet {
  return Decoration.set(computeRegion(state, { from: 0, to: state.doc.length }), true);
}

/** The dirty regions this transaction has to recompute (in new document coordinates, already widened to whole lines, sorted and merged) */
function dirtyRegions(tr: Transaction, treeChanged: boolean): Region[] {
  const doc = tr.state.doc;
  const raw: Region[] = [];
  if (tr.docChanged) {
    tr.changes.iterChangedRanges((_fromA, _toA, fromB, toB) => raw.push({ from: fromB, to: toB }));
  }
  // Both the old and the new selection count as dirty: a table expands and collapses as the selection moves in and out
  for (const r of tr.startState.selection.ranges) {
    raw.push({ from: tr.changes.mapPos(r.from, -1), to: tr.changes.mapPos(r.to, 1) });
  }
  for (const r of tr.state.selection.ranges) raw.push({ from: r.from, to: r.to });
  if (treeChanged) {
    // Incremental parsing advanced: what lies after the old tree's end is newly parsed
    const oldLen = Math.min(syntaxTree(tr.startState).length, tr.startState.doc.length);
    const parsedFrom = tr.changes.mapPos(oldLen, -1);
    const parsedTo = syntaxTree(tr.state).length;
    if (parsedTo > parsedFrom) raw.push({ from: parsedFrom, to: parsedTo });
  }
  const clamp = (n: number) => Math.max(0, Math.min(n, doc.length));
  const regions = raw
    .map((r) => ({ from: doc.lineAt(clamp(r.from)).from, to: doc.lineAt(clamp(r.to)).to }))
    .sort((a, b) => a.from - b.from);
  const merged: Region[] = [];
  for (const r of regions) {
    const last = merged[merged.length - 1];
    if (last && r.from <= last.to + 1) last.to = Math.max(last.to, r.to);
    else merged.push({ ...r });
  }
  return merged;
}

export const tableField = StateField.define<DecorationSet>({
  create: tableDecos,
  update(value, tr) {
    // Comparing syntax trees: as incremental parsing advances, language dispatches
    // transactions that do not change the document, and this is what renders a Table
    // that only appears once the first parse finishes
    const treeChanged = syntaxTree(tr.state) !== syntaxTree(tr.startState);
    if (!tr.docChanged && !tr.selection && !treeChanged) return value;

    let decos = tr.docChanged ? value.map(tr.changes) : value;
    for (const region of dirtyRegions(tr, treeChanged)) {
      // Widen to the full range of the old decorations and Table nodes intersecting the
      // dirty region, until it converges: a table straddling the dirty region's edge is
      // either recomputed whole or left alone, never half
      // (otherwise, as an edit on a neighbouring line makes the table grow or shrink,
      // the old and new decorations would overlap, which a block-level replace forbids)
      for (let pass = 0; pass < 8; pass++) {
        const { from, to } = region;
        decos.between(region.from, region.to, (dFrom, dTo) => {
          region.from = Math.min(region.from, dFrom);
          region.to = Math.max(region.to, dTo);
        });
        syntaxTree(tr.state).iterate({
          from: region.from,
          to: region.to,
          enter(node) {
            if (node.name !== "Table") return;
            const ext = tableExtent(tr.state, node.from, node.to);
            region.from = Math.min(region.from, ext.from);
            region.to = Math.max(region.to, ext.to);
            return false;
          },
        });
        if (region.from === from && region.to === to) break;
      }
      const add = computeRegion(tr.state, region);
      let hasOld = false;
      decos.between(region.from, region.to, () => {
        hasOld = true;
        return false;
      });
      // The dirty region holds neither an old decoration nor a new table: skip it, keeping the RangeSet reference unchanged
      if (!add.length && !hasOld) continue;
      decos = decos.update({
        filterFrom: region.from,
        filterTo: region.to,
        filter: () => false,
        add,
        sort: true,
      });
    }
    return decos;
  },
  provide: (f) => EditorView.decorations.from(f),
});
