// Block transforms: turn into another kind, move, indent, duplicate, delete,
// copy. Every function returns a TransactionSpec (or null when the operation
// does not apply) and never dispatches — the caller (a keymap, the block menu,
// a drag) owns the dispatch and the focus. All of them work on whole lines and
// rewrite one contiguous region with a single change, so one ⌘Z undoes one
// operation; the `block.*` user event keeps history from merging it with
// typing.
//
// The one seam rule (`seamSeparator`): a list item packs against a list item,
// anything packs directly under a heading line, everything else wants a blank
// line between — blank lines the user already had are never removed, only
// added where a seam would otherwise be missing one.
import { Transaction, type EditorState, type TransactionSpec } from "@codemirror/state";
import { suppressReveal } from "./reveal";
import {
  CALLOUT_RE,
  type EditorBlock,
  type EditorBlockKind,
  blockAt as blockAtLazy,
  blockContentStart,
  chunkOf,
  extentTo,
  nextSibling,
  prevSibling,
  subtreeTo,
} from "./blocks";

export type TurnTarget =
  | "paragraph"
  | "heading1"
  | "heading2"
  | "heading3"
  | "bullet"
  | "numbered"
  | "task"
  | "quote"
  | "callout"
  | "code";

const LIST_KINDS: EditorBlockKind[] = ["list_item", "task"];

/** What a seam looks at: a block's kind, except that a list item is its list's
 *  type — a bullet item (tasks included) or an ordered item. Two items of the
 *  same type may touch; a bullet item and an ordered item are two lists and
 *  need a blank line between them, because an ordered marker other than `1.`
 *  cannot interrupt a paragraph in CommonMark — placed right under a bullet
 *  item's text it becomes a lazy continuation of that text and rides along with
 *  the item from then on. */
export type SeamKind = EditorBlockKind | "ulist" | "olist";

export function seamKindOf(b: EditorBlock): SeamKind {
  if (LIST_KINDS.includes(b.kind)) return b.ordered ? "olist" : "ulist";
  return b.kind;
}

function annotations(op: string, keepReveal = false) {
  return keepReveal ? [Transaction.userEvent.of(op)] : [Transaction.userEvent.of(op), suppressReveal.of(true)];
}

/** "" when `next` may follow `prev` directly, "\n" when a blank line is wanted. */
export function seamSeparator(prev: SeamKind | null, next: SeamKind | null): "" | "\n" {
  if (prev == null || next == null) return "";
  if (prev === "heading") return "";
  if ((prev === "ulist" || prev === "olist") && prev === next) return "";
  return "\n";
}

function leadingWs(s: string): number {
  return s.length - s.trimStart().length;
}

/** Shift every non-blank line's indentation by `delta` columns (never below 0). */
function reindent(text: string, delta: number): string {
  if (delta === 0) return text;
  return text
    .split("\n")
    .map((l) => {
      if (l.trim() === "") return l;
      if (delta > 0) return " ".repeat(delta) + l;
      const ws = leadingWs(l);
      return l.slice(Math.min(ws, -delta));
    })
    .join("\n");
}

// ---------------------------------------------------------------- turn into

export function canTurnInto(b: EditorBlock, target: TurnTarget): boolean {
  if (["table", "hr", "html", "other"].includes(b.kind)) return false;
  switch (target) {
    case "paragraph":
      return b.kind !== "paragraph";
    case "heading1":
    case "heading2":
    case "heading3":
      return !(b.kind === "heading" && b.level === Number(target.slice(-1)));
    case "bullet":
      return !(b.kind === "list_item" && !b.ordered);
    case "numbered":
      return !(b.kind === "list_item" && b.ordered);
    case "task":
      return b.kind !== "task";
    case "quote":
      return b.kind !== "quote";
    case "callout":
      return b.kind !== "callout";
    case "code":
      return b.kind !== "code";
  }
}

/** The block's own lines with its current prefix removed. */
export function blockPlainLines(state: EditorState, b: EditorBlock): string[] {
  return stripPrefix(state.doc.sliceString(b.from, b.to).split("\n"), b);
}

function stripPrefix(lines: string[], b: EditorBlock): string[] {
  switch (b.kind) {
    case "heading": {
      if (b.node.name.startsWith("Setext")) return [lines[0].trim()];
      const first = lines[0].replace(/^\s{0,3}#{1,6}\s+/, "").replace(/\s+#+\s*$/, "");
      return [first, ...lines.slice(1)];
    }
    case "list_item":
    case "task": {
      const col = b.contentCol ?? 0;
      return lines.map((l, i) => (i === 0 ? l.slice(col) : l.slice(Math.min(leadingWs(l), col))));
    }
    case "quote":
      return lines.map((l) => l.replace(/^\s{0,3}>\s?/, ""));
    case "callout": {
      const out = lines.map((l) => l.replace(/^\s{0,3}>\s?/, ""));
      out[0] = out[0].replace(/^\[![\w-]+\][+-]?\s*/, "");
      if (out[0].trim() === "" && out.length > 1) out.shift();
      return out;
    }
    case "code": {
      if (/^\s{0,3}(`{3,}|~{3,})/.test(lines[0])) return lines.slice(1, lines.length > 1 ? -1 : undefined);
      return lines.map((l) => l.replace(/^ {0,4}/, ""));
    }
    default:
      return lines;
  }
}

function nextNumber(state: EditorState, b: EditorBlock): number {
  const p = prevSibling(state, b);
  if (p && p.kind === "list_item" && p.ordered && p.marker) {
    const n = parseInt(p.marker, 10);
    if (!Number.isNaN(n)) return n + 1;
  }
  return 1;
}

/** Turn a block into another kind; `omit` removes a span (the `/query` token of
 *  the slash menu) from the block's text in the same change. Children of a
 *  list item follow: re-indented for a list target, outdented to the item's
 *  own indentation for paragraph/heading, quoted or fenced along with it for
 *  quote/callout/code. */
export function turnInto(
  state: EditorState,
  b: EditorBlock,
  target: TurnTarget,
  opts?: { omit?: { from: number; to: number } },
): TransactionSpec | null {
  if (!canTurnInto(b, target)) return null;
  const doc = state.doc;
  const subtreeEnd = subtreeTo(b);
  let own = doc.sliceString(b.from, b.to);
  let ownTo = b.to;
  if (opts?.omit && opts.omit.from >= b.from && opts.omit.to <= b.to) {
    own = own.slice(0, opts.omit.from - b.from) + own.slice(opts.omit.to - b.from);
  }
  const plain = stripPrefix(own.split("\n"), b);
  const childrenText = subtreeEnd > b.to ? doc.sliceString(b.to + 1, subtreeEnd) : "";
  const indent = b.indentCols ?? leadingWs(doc.lineAt(b.from).text);
  const pad = " ".repeat(indent);
  const isList = LIST_KINDS.includes(b.kind);
  const oldContentCol = b.contentCol ?? indent;
  let lines: string[] = [];
  let children = childrenText;
  const listTarget = (marker: string) => {
    const newContentCol = indent + marker.length;
    lines = plain.map((l, i) => (i === 0 ? pad + marker + l : l.trim() === "" ? "" : " ".repeat(newContentCol) + l.trimStart()));
    if (children) children = reindent(children, newContentCol - oldContentCol);
  };
  switch (target) {
    case "paragraph":
      lines = plain.map((l, i) => (i === 0 ? pad + l.trimStart() : pad + l.trimStart()));
      if (children && isList) children = reindent(children, indent - oldContentCol);
      break;
    case "heading1":
    case "heading2":
    case "heading3":
      lines = [pad + "#".repeat(Number(target.slice(-1))) + " " + plain.map((l) => l.trim()).filter(Boolean).join(" ")];
      if (children && isList) children = reindent(children, indent - oldContentCol);
      break;
    case "bullet":
      listTarget("- ");
      break;
    case "numbered":
      listTarget(`${isList && b.ordered ? parseInt(b.marker ?? "1", 10) || 1 : nextNumber(state, b)}. `);
      break;
    case "task": {
      const box = b.kind === "task" ? (b.marker?.match(/\[(.)\]/)?.[0] ?? "[ ]") : "[ ]";
      listTarget(`- ${box} `);
      break;
    }
    case "quote":
    case "callout": {
      const head = target === "callout" ? "[!note] " : "";
      const body = [...plain];
      if (children) body.push(...children.split("\n"));
      lines = body.map((l, i) => pad + "> " + (i === 0 ? head : "") + l.trimStart());
      children = "";
      break;
    }
    case "code": {
      const body = [...plain];
      if (children) body.push(...children.split("\n"));
      lines = [pad + "```", ...body, pad + "```"];
      children = "";
      break;
    }
  }
  let text = lines.join("\n");
  if (children) text += "\n" + children;
  // content-offset preserving cursor: the same distance into the block's text,
  // clamped to the new first line
  const head = state.selection.main.head;
  const oldStart = blockContentStart(state, b);
  const newFirstLen = lines[0].length;
  const newContentOffset = Math.min(newFirstLen, leadingWs(lines[0]) + (lines[0].trimStart().length - plain[0].trimStart().length));
  let offset = Math.max(0, head - oldStart);
  if (opts?.omit && head >= opts.omit.to) offset -= opts.omit.to - opts.omit.from;
  const anchor = b.from + Math.min(newFirstLen, newContentOffset + Math.max(0, offset));
  return {
    changes: { from: b.from, to: subtreeEnd, insert: text },
    selection: { anchor },
    annotations: annotations("block.turn", true),
  };
}

// ---------------------------------------------------------------- moving

function kindOfLastDescendant(b: EditorBlock): SeamKind {
  let cur = b;
  while (cur.children.length) cur = cur.children[cur.children.length - 1];
  return seamKindOf(cur);
}

/** Blank lines (their texts) between the end of one block's subtree and the
 *  start of the next, or up to the next non-blank line / the document's end. */
function blankRunAfter(state: EditorState, pos: number): string[] {
  const doc = state.doc;
  let line = doc.lineAt(pos);
  const out: string[] = [];
  while (line.number < doc.lines) {
    const next = doc.line(line.number + 1);
    if (next.text.trim() !== "") break;
    out.push(next.text);
    line = next;
  }
  return out;
}

function blankRunBefore(state: EditorState, pos: number): string[] {
  const doc = state.doc;
  let line = doc.lineAt(pos);
  const out: string[] = [];
  while (line.number > 1) {
    const prev = doc.line(line.number - 1);
    if (prev.text.trim() !== "") break;
    out.unshift(prev.text);
    line = prev;
  }
  return out;
}

/** The separator to write between two pieces: the line break ending the first,
 *  then the blank lines that were there — plus one when the seam wants a blank
 *  line and there was none. */
function separator(prev: SeamKind, next: SeamKind, gap: string[]): string {
  const lines = [...gap];
  if (seamSeparator(prev, next) === "\n" && lines.length === 0) lines.push("");
  return "\n" + lines.map((l) => l + "\n").join("");
}

/** Renumber ordered items at exactly `indent` columns, sequentially from
 *  `start` (deeper lists are untouched). */
function renumber(text: string, indent: number, start: number): string {
  let n = start;
  return text
    .split("\n")
    .map((l) => {
      const m = /^(\s*)(\d{1,9})([.)])(\s)/.exec(l);
      if (!m || m[1].length !== indent) return l;
      const out = `${m[1]}${n}${m[3]}${l.slice(m[0].length - 1)}`;
      n += 1;
      return out;
    })
    .join("\n");
}

function orderedNumber(b: EditorBlock): number | null {
  if (b.kind !== "list_item" || !b.ordered || !b.marker) return null;
  const n = parseInt(b.marker, 10);
  return Number.isNaN(n) ? null : n;
}

/** Move a contiguous run of siblings one step up or down among its siblings
 *  (at depth 0 the siblings are the document's blocks, so a top-level item can
 *  cross a paragraph; a nested item stays inside its parent). Null at the edge. */
export function moveBlocks(state: EditorState, run: EditorBlock[], dir: -1 | 1): TransactionSpec | null {
  if (!run.length) return null;
  const sorted = [...run].sort((a, b) => a.from - b.from);
  const first = sorted[0];
  const last = sorted[sorted.length - 1];
  const neighbor = dir < 0 ? prevSibling(state, first) : nextSibling(state, last);
  if (!neighbor) return null;
  const doc = state.doc;
  // E = the earlier piece, L = the later one; after the move L comes first
  const eStart = dir < 0 ? neighbor.from : first.from;
  const eEnd = dir < 0 ? extentTo(state, neighbor) : extentTo(state, last);
  const lStart = dir < 0 ? first.from : neighbor.from;
  const lEnd = dir < 0 ? extentTo(state, last) : extentTo(state, neighbor);
  const eFirst = seamKindOf(dir < 0 ? neighbor : first);
  const eLast = kindOfLastDescendant(dir < 0 ? neighbor : last);
  const lFirst = seamKindOf(dir < 0 ? first : neighbor);
  const lLast = kindOfLastDescendant(dir < 0 ? last : neighbor);
  const eText = doc.sliceString(eStart, eEnd);
  const lText = doc.sliceString(lStart, lEnd);
  const gapEL = blankRunAfter(state, eEnd);
  const beforeGap = blankRunBefore(state, eStart);
  const afterGap = blankRunAfter(state, lEnd);
  const beforeLine = doc.lineAt(eStart).number - beforeGap.length - 1;
  const beforeKind = beforeLine >= 1 ? (blockAtLazy(state, doc.line(beforeLine).to)?.kind ?? null) : null;
  const afterLine = doc.lineAt(lEnd).number + afterGap.length + 1;
  const afterBlock = afterLine <= doc.lines ? blockAtLazy(state, doc.line(afterLine).from) : null;
  const afterKind = afterBlock ? seamKindOf(afterBlock) : null;
  const beforeKindLast = beforeLine >= 1 ? (() => { const b = blockAtLazy(state, doc.line(beforeLine).to, { top: true }); return b ? kindOfLastDescendant(b) : null; })() : null;
  const needBefore = beforeKindLast != null && beforeGap.length === 0 && seamSeparator(beforeKindLast, lFirst) === "\n";
  const needAfter = afterKind != null && afterGap.length === 0 && seamSeparator(eLast, afterKind) === "\n";
  const lead = needBefore ? "\n" : "";
  const mid = separator(lLast, eFirst, gapEL);
  let text = lead + lText + mid + eText + (needAfter ? "\n" : "");
  const indent = first.indentCols ?? 0;
  const eNumber = orderedNumber(dir < 0 ? neighbor : first);
  const lNumber = orderedNumber(dir < 0 ? first : neighbor);
  if (eNumber != null || lNumber != null) text = renumber(text, indent, eNumber ?? lNumber ?? 1);
  // keep the cursor on the moved run
  const head = state.selection.main.head;
  const runStart = first.from;
  const runEnd = extentTo(state, last);
  const newRunStart = dir < 0 ? eStart + lead.length : eStart + lead.length + lText.length + mid.length;
  const anchor = head >= runStart && head <= runEnd ? newRunStart + (head - runStart) : head;
  return {
    changes: { from: eStart, to: lEnd, insert: text },
    selection: { anchor: Math.min(anchor, eStart + text.length) },
    annotations: annotations("block.move"),
  };
}

/** Move a block's subtree before the block starting at `targetPos` (null =
 *  the end of the document), re-indented to `indentCols` (list items only).
 *  A target inside the block's own subtree is refused. */
export function moveBlockTo(
  state: EditorState,
  b: EditorBlock,
  targetPos: number | null,
  indentCols: number,
): TransactionSpec | null {
  const doc = state.doc;
  const chunk = chunkOf(state, b);
  const sub = subtreeTo(b);
  if (targetPos != null && targetPos > b.from && targetPos <= sub) return null;
  let text = doc.sliceString(b.from, sub);
  if (LIST_KINDS.includes(b.kind)) text = reindent(text, indentCols - (b.indentCols ?? 0));
  const del =
    chunk.to + 1 <= doc.length
      ? { from: chunk.from, to: Math.min(doc.length, chunk.to + 1) }
      : { from: Math.max(0, chunk.from - 1), to: chunk.to };
  const lastKind = kindOfLastDescendant(b);
  let insert: string;
  let at: number;
  if (targetPos == null) {
    // after the last content line, past the blank run that closes the file
    const lastContent = doc.lines - (doc.lines > 1 && doc.line(doc.lines).text === "" ? 1 : 0);
    let line = doc.line(lastContent);
    while (line.number > 1 && line.text.trim() === "") line = doc.line(line.number - 1);
    at = line.to;
    const prevBlock = blockAtLazy(state, line.from);
    const prevKind = prevBlock ? seamKindOf(prevBlock) : null;
    if (at >= del.from && at <= del.to) {
      // the block is already the last one
      return null;
    }
    insert = "\n" + (prevKind && seamSeparator(prevKind, seamKindOf(b)) === "\n" ? "\n" : "") + text;
  } else {
    const targetLine = doc.lineAt(targetPos);
    const target = blockAtLazy(state, targetLine.from);
    const blanks = blankRunBefore(state, targetLine.from);
    at = blanks.length ? doc.line(targetLine.number - blanks.length).from : targetLine.from;
    let keptBlanks = blanks.length;
    if (at >= del.from && at <= del.to) {
      // the blank run before the target is the moved block's own separator
      at = del.from;
      keptBlanks = 0;
    }
    const atLine = doc.lineAt(at);
    const prevLine = atLine.number > 1 ? doc.line(atLine.number - 1) : null;
    const prevKind = prevLine && prevLine.text.trim() !== "" ? (blockAtLazy(state, prevLine.from, { top: true }) ? kindOfLastDescendant(blockAtLazy(state, prevLine.from, { top: true })!) : null) : null;
    const sepBefore = prevKind && seamSeparator(prevKind, seamKindOf(b)) === "\n" ? "\n" : "";
    const sepAfter = keptBlanks === 0 && seamSeparator(lastKind, target ? seamKindOf(target) : null) === "\n" ? "\n" : "";
    insert = sepBefore + text + "\n" + sepAfter;
  }
  const changes = state.changes([del, { from: at, insert }]);
  const lead = insert.length - insert.replace(/^\n+/, "").length;
  const anchor = changes.mapPos(at, -1) + lead;
  return { changes, selection: { anchor }, annotations: annotations("block.move") };
}

/** Indent a list item under its previous sibling (its children follow). */
export function indentBlock(state: EditorState, b: EditorBlock): TransactionSpec | null {
  if (!LIST_KINDS.includes(b.kind)) return null;
  const p = prevSibling(state, b);
  if (!p || !LIST_KINDS.includes(p.kind)) return null;
  const delta = (p.contentCol ?? 0) - (b.indentCols ?? 0);
  if (delta <= 0) return null;
  const sub = subtreeTo(b);
  const text = reindent(state.doc.sliceString(b.from, sub), delta);
  const head = state.selection.main.head;
  return {
    changes: { from: b.from, to: sub, insert: text },
    selection: { anchor: head >= b.from && head <= sub ? head + delta : head },
    annotations: annotations("block.indent", true),
  };
}

/** Outdent a nested list item to its parent's level (its children follow; the
 *  siblings after it become its children, as CommonMark reads it). */
export function outdentBlock(state: EditorState, b: EditorBlock): TransactionSpec | null {
  if (!LIST_KINDS.includes(b.kind) || !b.parent) return null;
  const delta = (b.parent.indentCols ?? 0) - (b.indentCols ?? 0);
  if (delta >= 0) return null;
  const sub = subtreeTo(b);
  const text = reindent(state.doc.sliceString(b.from, sub), delta);
  const head = state.selection.main.head;
  return {
    changes: { from: b.from, to: sub, insert: text },
    selection: { anchor: head >= b.from && head <= sub ? Math.max(b.from, head + delta) : head },
    annotations: annotations("block.indent", true),
  };
}

/** Insert a copy of the run right after it (a list packs tight, anything else
 *  gets a blank line); the cursor lands at the copy's start. */
export function duplicateBlocks(state: EditorState, run: EditorBlock[]): TransactionSpec | null {
  if (!run.length) return null;
  const sorted = [...run].sort((a, b) => a.from - b.from);
  const first = sorted[0];
  const last = sorted[sorted.length - 1];
  const sub = extentTo(state, last);
  const text = state.doc.sliceString(first.from, sub);
  const sep = seamSeparator(kindOfLastDescendant(last), seamKindOf(first));
  const insert = "\n" + sep + text;
  return {
    changes: { from: sub, insert },
    selection: { anchor: sub + 1 + sep.length + (blockContentStart(state, first) - first.from) },
    annotations: annotations("block.duplicate"),
  };
}

/** Delete the run with its separator; the cursor lands on the block that
 *  follows (else the one before). */
export function deleteBlocks(state: EditorState, run: EditorBlock[]): TransactionSpec | null {
  if (!run.length) return null;
  const sorted = [...run].sort((a, b) => a.from - b.from);
  const doc = state.doc;
  const first = chunkOf(state, sorted[0]);
  const last = chunkOf(state, sorted[sorted.length - 1]);
  const from = Math.min(first.from, last.from);
  const to = Math.max(first.to, last.to);
  const del = to + 1 <= doc.length ? { from, to: Math.min(doc.length, to + 1) } : { from: Math.max(0, from - 1), to };
  const anchor = Math.min(del.from, Math.max(0, doc.length - (del.to - del.from)));
  return { changes: del, selection: { anchor }, annotations: annotations("block.delete") };
}

/** The run as Markdown for the clipboard: de-indented by the first block's
 *  indentation, exactly one trailing newline. */
export function copyBlocksText(state: EditorState, run: EditorBlock[]): string {
  if (!run.length) return "";
  const sorted = [...run].sort((a, b) => a.from - b.from);
  const text = state.doc.sliceString(sorted[0].from, extentTo(state, sorted[sorted.length - 1]));
  return reindent(text, -(sorted[0].indentCols ?? 0)).replace(/\n+$/, "") + "\n";
}

/** A callout's type as written on its first line, for menus. */
export function calloutTypeOf(text: string): string | null {
  const m = CALLOUT_RE.exec(text);
  return m ? m[3].toLowerCase() : null;
}
