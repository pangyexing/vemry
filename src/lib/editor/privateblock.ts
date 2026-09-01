// Where the private blocks of a document are, in the editor buffer. Mirrors the
// Rust parser's rule (parser.rs: a `> [!private]` callout opens on a line that
// starts, after indentation, with `>` and the marker; it runs while the following
// lines start with `>`; fenced code is skipped first) so the status bar, the
// callout tint and the selection guard judge exactly what the index will. Rust
// stays the real gate — this is the editor's own knowledge of the buffer, which
// the index cannot see until autosave.
import type { Text } from "@codemirror/state";

export const PRIVATE_HEAD_RE = /^\s*>\s*\[!private\][+-]?(?=\s|$)/i;
const FENCE_RE = /^\s*(`{3,}|~{3,})/;

export interface PrivateRange {
  /** Character range of the block's lines (from the head line's start to the last `>` line's end). */
  from: number;
  to: number;
}

const cache = new WeakMap<Text, PrivateRange[]>();

/** Every private block of the document, in order (cached per Text instance). */
export function privateRanges(doc: Text): PrivateRange[] {
  const hit = cache.get(doc);
  if (hit) return hit;
  const out: PrivateRange[] = [];
  let fence: string | null = null;
  for (let n = 1; n <= doc.lines; n++) {
    const line = doc.line(n);
    const text = line.text;
    if (fence) {
      const t = text.trimStart();
      if (t.startsWith(fence) && t.slice(fence.length).trim() === "") fence = null;
      continue;
    }
    const f = FENCE_RE.exec(text);
    if (f) {
      fence = f[1];
      continue;
    }
    if (!PRIVATE_HEAD_RE.test(text)) continue;
    let last = line;
    while (last.number < doc.lines) {
      const next = doc.line(last.number + 1);
      if (!next.text.trimStart().startsWith(">")) break;
      last = next;
    }
    out.push({ from: line.from, to: last.to });
    n = last.number;
  }
  cache.set(doc, out);
  return out;
}

/** The private block containing pos (inclusive at both ends), if any. */
export function privateRangeAt(doc: Text, pos: number): PrivateRange | null {
  return privateRanges(doc).find((r) => r.from <= pos && pos <= r.to) ?? null;
}

/** Whether the selection [from,to] touches any private block. */
export function selectionTouchesPrivate(doc: Text, from: number, to: number): boolean {
  return privateRanges(doc).some((r) => r.from <= to && r.to >= from);
}
