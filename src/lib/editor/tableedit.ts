// Editing a GFM table through its source. With the cursor in a table, Tab and
// Shift-Tab step from cell to cell (Tab past the last cell opens a new row) and Enter
// opens a new row below the current one — Enter on an empty last row ends the table
// instead, the way Enter on an empty list item ends a list. The row, column and
// alignment operations (the block menu, the context menu, the palette, the rendered
// table's + bars) are rewrites of the same grid.
// A table is read line by line, split on unescaped | like table.ts's rendering, into
// rows of raw cells: each cell's text between its pipes, spaces and escapes as
// written. An operation edits that grid and writes back only the span that changed,
// so a row it does not touch keeps its bytes and an inserted column leaves the other
// columns' spacing alone. A row missing an outer pipe gains it when its line is
// rewritten — without one, a first or last cell left empty would not count as a cell.
import { syntaxTree } from "@codemirror/language";
import { EditorSelection, type EditorState, type TransactionSpec } from "@codemirror/state";
import type { Command, EditorView } from "@codemirror/view";
import { completionStatus } from "@codemirror/autocomplete";

interface Row {
  /** The line's start in the document. */
  from: number;
  to: number;
  /** Indentation and quote markers before the row, kept on a rewrite and on a new row. */
  prefix: string;
  /** The raw text of each cell, between its pipes. */
  cells: string[];
  /** The document position where each raw cell begins (just after its opening pipe). */
  starts: number[];
  /** Whether the row is written with its leading and its trailing pipe. */
  open: boolean;
  close: boolean;
}

export interface TableModel {
  from: number;
  to: number;
  /** rows[0] is the header, rows[1] the delimiter row, the rest the body. */
  rows: Row[];
}

export interface CellRef {
  row: number;
  col: number;
}

export type TableOp =
  | "row-above"
  | "row-below"
  | "col-left"
  | "col-right"
  | "delete-row"
  | "delete-col"
  | "align-left"
  | "align-center"
  | "align-right";

/** The quote markers and indentation a row line starts with. */
const PREFIX_RE = /^(?:[ \t]*>)*[ \t]*/;

/** The raw cell a new column gets in the delimiter row, and in every other row. */
const NEW_DELIM = " --- ";
const NEW_CELL = "  ";

type AlignOp = "align-left" | "align-center" | "align-right";

const DELIM_FOR: Record<AlignOp, string> = {
  "align-left": " :--- ",
  "align-center": " :---: ",
  "align-right": " ---: ",
};

/** The alignment a delimiter cell spells, as the operation that would set it. */
function alignOf(raw: string): AlignOp | null {
  const t = raw.trim();
  if (t.startsWith(":") && t.endsWith(":")) return "align-center";
  if (t.endsWith(":")) return "align-right";
  if (t.startsWith(":")) return "align-left";
  return null;
}

function parseRow(text: string, from: number): Row {
  const prefix = PREFIX_RE.exec(text)![0];
  let i = prefix.length;
  const open = text[i] === "|";
  if (open) i++;
  const cells: string[] = [];
  const starts: number[] = [];
  let start = i;
  for (; i < text.length; i++) {
    if (text[i] === "\\") i++;
    else if (text[i] === "|") {
      cells.push(text.slice(start, i));
      starts.push(from + start);
      start = i + 1;
    }
  }
  // What follows the last pipe is a cell of its own, unless only spaces are left: then
  // that pipe closed the row
  const tail = text.slice(start);
  const close = tail.trim() === "" && cells.length > 0;
  if (!close) {
    cells.push(tail);
    starts.push(from + start);
  }
  return { from, to: from + text.length, prefix, cells, starts, open, close };
}

/** The table the line at `pos` belongs to, or null. */
export function tableAt(state: EditorState, pos: number): TableModel | null {
  const doc = state.doc;
  const line = doc.lineAt(pos);
  // collected through an array: an assignment inside the callback would not be seen by the narrowing after it
  const found: { from: number; to: number }[] = [];
  syntaxTree(state).iterate({
    from: line.from,
    to: line.to,
    enter(node) {
      if (found.length) return false;
      if (node.name !== "Table") return;
      found.push({ from: node.from, to: node.to });
      return false;
    },
  });
  if (!found.length) return null;
  const first = doc.lineAt(found[0].from);
  const last = doc.lineAt(Math.min(found[0].to, doc.length));
  if (line.number < first.number || line.number > last.number) return null;
  const rows: Row[] = [];
  for (let n = first.number; n <= last.number; n++) {
    const l = doc.line(n);
    rows.push(parseRow(l.text, l.from));
  }
  if (rows.length < 2) return null;
  return { from: first.from, to: last.to, rows };
}

/** The cell holding `pos`; a position before the first cell (a quote marker, the
 *  opening pipe) counts as the first, one past the last as the last. */
export function cellAt(model: TableModel, pos: number): CellRef {
  let row = model.rows.findIndex((r) => pos >= r.from && pos <= r.to);
  if (row < 0) row = pos < model.from ? 0 : model.rows.length - 1;
  const starts = model.rows[row].starts;
  let col = 0;
  for (let i = 0; i < starts.length; i++) if (starts[i] <= pos) col = i;
  return { row, col };
}

/** The table's column count: the header's (the delimiter row always agrees). */
function width(model: TableModel): number {
  return model.rows[0].cells.length;
}

/** Where the cursor goes in a cell: its text selected (`select`) or the caret after it;
 *  in an empty cell, one space in from its opening pipe. */
function cellSelection(start: number, raw: string, select: boolean): EditorSelection {
  const body = raw.trim();
  if (!body) return EditorSelection.single(start + Math.min(1, raw.length));
  const from = start + raw.length - raw.trimStart().length;
  return select ? EditorSelection.single(from, from + body.length) : EditorSelection.single(from + body.length);
}

/** A copy of a row's cells to edit and write back with writeRow. An outer pipe the row
 *  was written without is coming, so the edge cell gets a space to stand apart from it. */
function editCells(row: Row): string[] {
  const cells = [...row.cells];
  const last = cells.length - 1;
  if (!row.open && !/^\s/.test(cells[0])) cells[0] = " " + cells[0];
  if (!row.close && !/\s$/.test(cells[last])) cells[last] += " ";
  return cells;
}

function writeRow(row: Row, cells: string[]): string {
  return row.prefix + "|" + cells.join("|") + "|";
}

function emptyRow(prefix: string, n: number): string {
  return prefix + "|" + `${NEW_CELL}|`.repeat(n);
}

/** Replace the table's lines with `lines`, as the one change between the two texts, and
 *  put the cursor in `target` (a cell of the new lines); a null target leaves the
 *  selection to be mapped through the change. */
function commit(state: EditorState, model: TableModel, lines: string[], target: CellRef | null, select = false): TransactionSpec {
  const old = state.sliceDoc(model.from, model.to);
  const text = lines.join("\n");
  let a = 0;
  while (a < old.length && a < text.length && old[a] === text[a]) a++;
  let b = 0;
  while (b < old.length - a && b < text.length - a && old[old.length - 1 - b] === text[text.length - 1 - b]) b++;
  const spec: TransactionSpec = {
    changes: { from: model.from + a, to: model.to - b, insert: text.slice(a, text.length - b) },
    scrollIntoView: true,
    userEvent: "input.table",
  };
  if (!target) return spec;
  let off = model.from;
  for (let i = 0; i < target.row; i++) off += lines[i].length + 1;
  const row = parseRow(lines[target.row], off);
  const col = Math.min(target.col, row.cells.length - 1);
  return { ...spec, selection: cellSelection(row.starts[col], row.cells[col], select) };
}

function lineTexts(state: EditorState, model: TableModel): string[] {
  return model.rows.map((r) => state.sliceDoc(r.from, r.to));
}

/** Whether `op` can apply at `at`: the header has no row above it and is never deleted,
 *  the delimiter row stands for the header, and the last column is not deleted. */
export function canTableOp(model: TableModel, op: TableOp, at: CellRef): boolean {
  if (op === "row-above" || op === "delete-row") return at.row >= 2;
  if (op === "delete-col") return width(model) > 1 && at.col < width(model);
  if (op.startsWith("align-") || op === "col-left" || op === "col-right") return at.col < width(model);
  return true;
}

/** The transaction for one operation on the cell at `at`, or null when it cannot apply. */
export function tableEdit(state: EditorState, model: TableModel, op: TableOp, at: CellRef): TransactionSpec | null {
  if (!canTableOp(model, op, at)) return null;
  const lines = lineTexts(state, model);
  const n = width(model);
  const { row, col } = at;
  switch (op) {
    case "row-above":
    case "row-below": {
      // below the header or the delimiter row is the first body row
      const index = op === "row-above" ? row : Math.max(row, 1) + 1;
      lines.splice(index, 0, emptyRow(model.rows[row].prefix, n));
      return commit(state, model, lines, { row: index, col });
    }
    case "delete-row": {
      lines.splice(row, 1);
      return commit(state, model, lines, { row: lines.length > 2 ? Math.min(row, lines.length - 1) : 0, col });
    }
    case "col-left":
    case "col-right": {
      const index = op === "col-left" ? col : col + 1;
      model.rows.forEach((r, i) => {
        // a short row (GFM fills its missing cells) has nothing to shift past
        if (index > r.cells.length) return;
        const cells = editCells(r);
        cells.splice(index, 0, i === 1 ? NEW_DELIM : NEW_CELL);
        lines[i] = writeRow(r, cells);
      });
      return commit(state, model, lines, { row, col: index });
    }
    case "delete-col": {
      model.rows.forEach((r, i) => {
        if (col >= r.cells.length) return;
        const cells = editCells(r);
        cells.splice(col, 1);
        lines[i] = writeRow(r, cells);
      });
      return commit(state, model, lines, { row, col: Math.min(col, n - 2) });
    }
    default: {
      // a second press of the same alignment takes it off again
      const delim = model.rows[1];
      const cells = editCells(delim);
      cells[col] = alignOf(cells[col]) === op ? NEW_DELIM : DELIM_FOR[op];
      lines[1] = writeRow(delim, cells);
      return commit(state, model, lines, null);
    }
  }
}

/** The operation at the cell holding `pos`, or null outside a table. */
export function tableEditAt(state: EditorState, pos: number, op: TableOp): TransactionSpec | null {
  const model = tableAt(state, pos);
  return model ? tableEdit(state, model, op, cellAt(model, pos)) : null;
}

/** A new row after the table's last, or a new column after its last (the rendered
 *  table's + bars, and the block menu when no cell is pointed at). */
export function tableAppend(state: EditorState, pos: number, what: "row" | "col"): TransactionSpec | null {
  const model = tableAt(state, pos);
  if (!model) return null;
  const at = what === "row" ? { row: model.rows.length - 1, col: 0 } : { row: 0, col: width(model) - 1 };
  return tableEdit(state, model, what === "row" ? "row-below" : "col-right", at);
}

/** The cursor's table, when a key may act on it: one plain range, no completion menu
 *  open, no composition under way. A completion still being computed ("pending", the
 *  moment after every typed character) does not count: Tab right after typing a cell
 *  would otherwise fall through to indenting the row. */
function keyTable(view: EditorView): { model: TableModel; at: CellRef } | null {
  const state = view.state;
  if (state.selection.ranges.length !== 1 || completionStatus(state) === "active" || view.composing) return null;
  const head = state.selection.main.head;
  const model = tableAt(state, head);
  return model ? { model, at: cellAt(model, head) } : null;
}

function step(dir: 1 | -1): Command {
  return (view) => {
    const found = keyTable(view);
    if (!found) return false;
    const { model } = found;
    let { row, col } = found.at;
    const cells = (r: number) => model.rows[r].cells.length;
    // the delimiter row is passed over, as the end of the header
    if (dir > 0) {
      if (row === 1) [row, col] = [2, 0];
      else if (col + 1 < cells(row)) col++;
      else [row, col] = [row === 0 ? 2 : row + 1, 0];
      if (row >= model.rows.length) {
        const lines = lineTexts(view.state, model);
        lines.push(emptyRow(model.rows[model.rows.length - 1].prefix, width(model)));
        view.dispatch(commit(view.state, model, lines, { row: lines.length - 1, col: 0 }));
        return true;
      }
    } else {
      if (row === 1) [row, col] = [0, cells(0) - 1];
      else if (col > 0) col--;
      // the first cell: the key stays in the table rather than outdenting its line
      else if (row === 0) return true;
      else {
        row = row === 2 ? 0 : row - 1;
        col = cells(row) - 1;
      }
    }
    const r = model.rows[row];
    view.dispatch({ selection: cellSelection(r.starts[col], r.cells[col], true), scrollIntoView: true });
    return true;
  };
}

export const tableNextCell = step(1);
export const tablePrevCell = step(-1);

export const tableEnter: Command = (view) => {
  const found = keyTable(view);
  if (!found) return false;
  const { model, at } = found;
  const row = model.rows[at.row];
  if (at.row >= 2 && at.row === model.rows.length - 1 && row.cells.every((c) => c.trim() === "")) {
    // The empty row gives way to the blank line a table needs below it — a line of text
    // right under a row would be read as one more row — and the cursor goes to the line
    // after it, inside the same quote or list item as the table
    const blank = row.prefix.trimEnd();
    view.dispatch({
      changes: { from: row.from, to: row.to, insert: blank + "\n" + row.prefix },
      selection: { anchor: row.from + blank.length + 1 + row.prefix.length },
      scrollIntoView: true,
      userEvent: "input.table",
    });
    return true;
  }
  const spec = tableEdit(view.state, model, "row-below", { row: at.row, col: 0 });
  if (!spec) return false;
  view.dispatch(spec);
  return true;
};

/** A table of `rows` rows (the header row among them) and `cols` columns, every cell empty. */
export function tableTemplate(rows: number, cols: number): string {
  const lines = [emptyRow("", cols), "|" + `${NEW_DELIM}|`.repeat(cols)];
  for (let i = 1; i < rows; i++) lines.push(emptyRow("", cols));
  return lines.join("\n");
}
