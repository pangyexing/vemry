// The editor's block model: what "this block" means while editing, derived live
// from CodeMirror's Lezer markdown tree (the Rust parser derives the same idea
// from the file on disk — two projections of one text; see docs/ARCHITECTURE.md).
// A block is a line-aligned range of the document: a paragraph, a heading, a
// list item (its nested items are child blocks), a quote or callout, a fence, a
// table, a rule. Nothing here touches the DOM: pure functions over EditorState,
// so the transforms (blockops.ts), the selection mode (blocksel.ts), folding,
// the hover handle and drag all agree on the boundaries.
//
// Where the two segmentations differ (a lazy continuation line, a multi-line
// task) the editor never predicts where a ` ^id` goes — it asks Rust
// (`block_id_plan`) with the block's content offset. Own range vs subtree: a
// list item's own range is its marker line and continuation lines before its
// first nested list; its subtree is everything down to its last descendant.
import { ensureSyntaxTree, foldedRanges, syntaxTree } from "@codemirror/language";
import type { EditorState, Text } from "@codemirror/state";

// @lezer/common is not a direct dependency (pnpm's isolated layout cannot import it);
// SyntaxNode is inferred from syntaxTree's return value, the idiom fenced.ts uses
export type SyntaxNode = ReturnType<ReturnType<typeof syntaxTree>["resolveInner"]>;

export type EditorBlockKind =
  | "paragraph"
  | "heading"
  | "list_item"
  | "task"
  | "quote"
  | "callout"
  | "code"
  | "table"
  | "hr"
  | "html"
  | "other";

export interface EditorBlock {
  /** Own range, line-aligned: the first line's start to the last own line's end. */
  from: number;
  to: number;
  kind: EditorBlockKind;
  /** Heading level 1-6. */
  level?: number;
  /** List nesting: 0 for a top-level item and for every non-list block. */
  depth: number;
  /** Exact marker text as written: `- `, `1. `, `- [ ] `. */
  marker?: string;
  /** Column of the marker (a tab counts as one, as in the Rust parser). */
  indentCols?: number;
  /** indentCols + marker length: where continuation lines must start. */
  contentCol?: number;
  ordered?: boolean;
  checked?: boolean | "doing";
  calloutType?: string;
  foldMarker?: "+" | "-";
  parent: EditorBlock | null;
  /** Nested list items (a BulletList/OrderedList child of the item, on later lines). */
  children: EditorBlock[];
  node: SyntaxNode;
}

/** `> [!type]±` on a quote's first line. Groups: 1 = the quote prefix, 2 = the whole
 *  marker, 3 = the type, 4 = the fold marker, 5 = trailing whitespace. Shared with
 *  livepreview's callout rendering and fold.ts (which reads group 4). */
export const CALLOUT_RE = /^(\s{0,3}>\s*)(\[!([\w-]+)\]([+-]?))(\s*)/;

const TASK_BOX_RE = /^\[([ xX/])\](?=\s|$)/;
const MARKER_RE = /^(\s*)([-*+]|\d{1,9}[.)])(\s+)/;

/** Position after the YAML frontmatter (0 when none): the same rule as
 *  livepreview's frontmatter fold — `---` on line 1, closed by `---` or `...`
 *  within 100 lines. Lezer parses the block as a rule plus a setext heading,
 *  which the model must never report as blocks. */
export function frontmatterEnd(doc: Text): number {
  if (doc.lines < 2 || doc.line(1).text.trim() !== "---") return 0;
  const limit = Math.min(doc.lines, 100);
  for (let n = 2; n <= limit; n++) {
    const t = doc.line(n).text.trim();
    if (t === "---" || t === "...") return Math.min(doc.line(n).to + 1, doc.length);
  }
  return 0;
}

const cache = new WeakMap<EditorState, EditorBlock[]>();

/** Every document-level block in order (lists expanded into their depth-0 items). */
export function topBlocks(state: EditorState): EditorBlock[] {
  const hit = cache.get(state);
  if (hit) return hit;
  const doc = state.doc;
  const fmEnd = frontmatterEnd(doc);
  const out: EditorBlock[] = [];
  const tree = syntaxTree(state);
  for (let n = tree.topNode.firstChild; n; n = n.nextSibling) {
    if (n.to <= fmEnd) continue;
    if (n.name === "BulletList" || n.name === "OrderedList") {
      out.push(...listItems(state, n, 0, null));
    } else {
      const b = plainBlock(state, n, 0, null);
      if (b) out.push(b);
    }
  }
  cache.set(state, out);
  return out;
}

function plainBlock(state: EditorState, n: SyntaxNode, depth: number, parent: EditorBlock | null): EditorBlock | null {
  const doc = state.doc;
  const from = doc.lineAt(n.from).from;
  const to = doc.lineAt(Math.max(n.from, n.to - 1)).to;
  const base = { from, to, depth, parent, children: [] as EditorBlock[], node: n };
  switch (n.name) {
    case "ATXHeading1":
    case "ATXHeading2":
    case "ATXHeading3":
    case "ATXHeading4":
    case "ATXHeading5":
    case "ATXHeading6":
      return { ...base, kind: "heading", level: Number(n.name.slice(-1)) };
    case "SetextHeading1":
      return { ...base, kind: "heading", level: 1 };
    case "SetextHeading2":
      return { ...base, kind: "heading", level: 2 };
    case "Paragraph":
      return { ...base, kind: "paragraph" };
    case "FencedCode":
    case "CodeBlock":
      return { ...base, kind: "code" };
    case "Blockquote": {
      const co = CALLOUT_RE.exec(doc.lineAt(n.from).text);
      if (co)
        return {
          ...base,
          kind: "callout",
          calloutType: co[3].toLowerCase(),
          foldMarker: co[4] === "+" || co[4] === "-" ? co[4] : undefined,
        };
      return { ...base, kind: "quote" };
    }
    case "Table":
      return { ...base, kind: "table" };
    case "HorizontalRule":
      return { ...base, kind: "hr" };
    case "HTMLBlock":
      return { ...base, kind: "html" };
    default:
      return { ...base, kind: "other" };
  }
}

function listItems(state: EditorState, list: SyntaxNode, depth: number, parent: EditorBlock | null): EditorBlock[] {
  const doc = state.doc;
  const out: EditorBlock[] = [];
  for (let item = list.firstChild; item; item = item.nextSibling) {
    if (item.name !== "ListItem") continue;
    const line = doc.lineAt(item.from);
    const m = MARKER_RE.exec(doc.sliceString(line.from, Math.min(line.to, item.from + 24)));
    const indentCols = item.from - line.from;
    let marker = m ? m[2] + m[3] : "";
    const ordered = /^\d/.test(marker);
    let kind: EditorBlockKind = "list_item";
    let checked: boolean | "doing" | undefined;
    const afterMarker = line.text.slice(indentCols + marker.length);
    const box = TASK_BOX_RE.exec(afterMarker);
    if (box && !ordered) {
      kind = "task";
      checked = box[1] === "/" ? "doing" : box[1] !== " ";
      const boxLen = 3 + (afterMarker.slice(3).match(/^\s*/)?.[0].length ?? 0);
      marker += afterMarker.slice(0, boxLen);
    }
    // own range: up to the last direct child before the first nested list
    let ownTo = line.to;
    let nested: SyntaxNode[] = [];
    for (let c = item.firstChild; c; c = c.nextSibling) {
      if (c.name === "BulletList" || c.name === "OrderedList") {
        nested.push(c);
      } else if (nested.length === 0 && c.name !== "ListMark") {
        ownTo = Math.max(ownTo, doc.lineAt(Math.max(c.from, c.to - 1)).to);
      }
    }
    const b: EditorBlock = {
      from: line.from,
      to: ownTo,
      kind,
      depth,
      marker,
      indentCols,
      contentCol: indentCols + marker.length,
      ordered,
      checked,
      parent,
      children: [],
      node: item,
    };
    for (const l of nested) b.children.push(...listItems(state, l, depth + 1, b));
    out.push(b);
  }
  return out;
}

/** End of the block's subtree: its own end, or its last descendant's. */
export function subtreeTo(b: EditorBlock): number {
  let to = b.to;
  for (const c of b.children) to = Math.max(to, subtreeTo(c));
  return to;
}

export function subtreeRange(b: EditorBlock): { from: number; to: number } {
  return { from: b.from, to: subtreeTo(b) };
}

/** The block's extent as a unit: its subtree — or, for a folded heading, the
 *  whole folded section, so that what is shown as one thing moves, copies and
 *  deletes as one thing. */
export function extentTo(state: EditorState, b: EditorBlock): number {
  let to = subtreeTo(b);
  if (b.kind === "heading") {
    foldedRanges(state).between(b.to, b.to, (from, end) => {
      if (from === b.to) to = Math.max(to, end);
    });
  }
  return to;
}

/** The innermost block containing pos (a nested item beats its parent); with
 *  `top`, the document-level block. Null on a blank line between blocks or
 *  inside the frontmatter. */
export function blockAt(state: EditorState, pos: number, opts?: { top?: boolean }): EditorBlock | null {
  let found: EditorBlock | null = null;
  let level: EditorBlock[] = topBlocks(state);
  while (level.length) {
    const b = level.find((x) => x.from <= pos && pos <= subtreeTo(x));
    if (!b) break;
    found = b;
    if (opts?.top) break;
    if (pos <= b.to) break;
    level = b.children;
  }
  return found;
}

/** Document-level blocks whose subtree intersects [from, to]. */
export function blocksIn(state: EditorState, from: number, to: number): EditorBlock[] {
  return topBlocks(state).filter((b) => b.from <= to && subtreeTo(b) >= from);
}

export function siblings(state: EditorState, b: EditorBlock): EditorBlock[] {
  return b.parent ? b.parent.children : topBlocks(state);
}

export function nextSibling(state: EditorState, b: EditorBlock): EditorBlock | null {
  const s = siblings(state, b);
  const i = s.indexOf(b);
  return i >= 0 && i + 1 < s.length ? s[i + 1] : null;
}

export function prevSibling(state: EditorState, b: EditorBlock): EditorBlock | null {
  const s = siblings(state, b);
  const i = s.indexOf(b);
  return i > 0 ? s[i - 1] : null;
}

/** The next block in document order (children first, then the next sibling, climbing). */
export function nextBlock(state: EditorState, b: EditorBlock): EditorBlock | null {
  if (b.children.length) return b.children[0];
  for (let cur: EditorBlock | null = b; cur; cur = cur.parent) {
    const n = nextSibling(state, cur);
    if (n) return n;
  }
  return null;
}

/** The previous block in document order (the deepest last descendant of the previous sibling, else the parent). */
export function prevBlock(state: EditorState, b: EditorBlock): EditorBlock | null {
  const p = prevSibling(state, b);
  if (!p) return b.parent;
  let cur = p;
  while (cur.children.length) cur = cur.children[cur.children.length - 1];
  return cur;
}

export function ancestorAtDepth(b: EditorBlock, depth: number): EditorBlock {
  let cur = b;
  while (cur.depth > depth && cur.parent) cur = cur.parent;
  return cur;
}

/** The block's chunk: its subtree plus the blank lines that follow it (the
 *  separator it travels with); the last block of the document takes the blank
 *  run before it instead. Line-aligned; `to` is a line end. */
export function chunkOf(state: EditorState, b: EditorBlock): { from: number; to: number } {
  const doc = state.doc;
  let from = b.from;
  let to = extentTo(state, b);
  let line = doc.lineAt(to);
  // the empty last line after a terminating newline is the file's end, not a separator
  const lastContent = doc.lines - (doc.lines > 1 && doc.line(doc.lines).text === "" ? 1 : 0);
  let trailing = 0;
  while (line.number < lastContent) {
    const next = doc.line(line.number + 1);
    if (next.text.trim() !== "") break;
    line = next;
    trailing++;
  }
  if (trailing > 0) {
    to = line.to;
  } else if (line.number >= lastContent) {
    let first = doc.lineAt(from);
    while (first.number > 1) {
      const prev = doc.line(first.number - 1);
      if (prev.text.trim() !== "") break;
      first = prev;
    }
    from = first.from;
  }
  return { from, to };
}

/** Where the block's own text starts: after indentation, the list marker and
 *  checkbox, a heading's `#`s, a quote's `> ` and a callout's `[!type] `. The
 *  offset handed to Rust when an id is minted by cursor position. */
export function blockContentStart(state: EditorState, b: EditorBlock): number {
  const line = state.doc.lineAt(b.from);
  const text = line.text;
  switch (b.kind) {
    case "list_item":
    case "task":
      return line.from + (b.contentCol ?? 0);
    case "heading": {
      const m = /^\s{0,3}#{1,6}\s+/.exec(text);
      return line.from + (m ? m[0].length : 0);
    }
    case "quote": {
      const m = /^\s{0,3}>\s?/.exec(text);
      return line.from + (m ? m[0].length : 0);
    }
    case "callout": {
      const m = CALLOUT_RE.exec(text);
      return line.from + (m ? m[0].length : 0);
    }
    default:
      return line.from + (text.length - text.trimStart().length);
  }
}

export function blockText(state: EditorState, b: EditorBlock, opts?: { subtree?: boolean }): string {
  return state.doc.sliceString(b.from, opts?.subtree ? subtreeTo(b) : b.to);
}

/** End of a heading's section: the last document-level block before the next
 *  heading of the same or a higher level (the heading's own end when the
 *  section is empty). */
export function sectionEnd(state: EditorState, heading: EditorBlock): number {
  const top = topBlocks(state);
  const start = top.indexOf(heading);
  if (start < 0 || heading.kind !== "heading") return heading.to;
  let end = heading.to;
  for (let i = start + 1; i < top.length; i++) {
    const b = top[i];
    if (b.kind === "heading" && (b.level ?? 1) <= (heading.level ?? 1)) break;
    end = subtreeTo(b);
  }
  return end;
}

/** Make sure the tree covers `upto`; true when it already did or now does. A
 *  caller that gets false may dispatch an empty transaction to pick the new
 *  tree up (the state's own tree is the one blockAt reads). */
export function ensureParsed(state: EditorState, upto: number): boolean {
  const tree = ensureSyntaxTree(state, Math.min(upto, state.doc.length), 50);
  return !!tree && tree.length >= Math.min(upto, state.doc.length);
}
