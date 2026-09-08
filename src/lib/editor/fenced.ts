// Locating a fenced code block and reading its contents, shared by the three
// consumers: copy, run and query.
//
// A fence inside a quote block (the normal shape of a clipped selection) has two
// traps, both collected here:
// 1. The button widget is anchored at the **start** of the fence's first line
//    (anchoring inside the line would have it swallowed by the replace decoration
//    that hides the fence line), and at the start of a line inside a quote block sits
//    a QuoteMark — resolveInner's ancestor chain only reaches Blockquote and does not
//    include FencedCode, so the leading quote marker has to be skipped and the lookup
//    retried;
// 2. @lezer/markdown cuts the code into **several** CodeText pieces line by line,
//    interleaved with QuoteMarks (the tree as observed: FencedCode(CodeMark, CodeInfo,
//    QuoteMark, CodeText, QuoteMark, CodeText, …, QuoteMark, CodeMark)); neither the
//    `> ` marker nor the space after it falls inside a piece's range, so concatenating
//    all of them restores clean code. getChild takes the first piece alone — inside a
//    quote block that is just the first line, and both copy and run would truncate
//    silently.
import { syntaxTree } from "@codemirror/language";
import type { EditorState, Text } from "@codemirror/state";

// @lezer/common is not a direct dependency (pnpm's isolated layout cannot import it),
// so SyntaxNode is inferred from syntaxTree's return value — the same idiom livepreview
// already uses
type SyntaxNode = ReturnType<ReturnType<typeof syntaxTree>["resolveInner"]>;

/** Whether pos falls inside code (inline code, a fence, an indented code block). Every
 *  decorator that must not fold inside code (live preview, the formula widgets,
 *  mdlink's ⌘click, the completions) asks this one, so "what counts as code" has a
 *  single answer. Lives here rather than in livepreview.ts so editor/math.ts can ask
 *  without importing the module that imports it. */
export function inCode(state: EditorState, pos: number): boolean {
  const tree = syntaxTree(state);
  for (let n: SyntaxNode | null = tree.resolveInner(pos, 1); n; n = n.parent)
    if (n.name === "InlineCode" || n.name === "FencedCode" || n.name === "CodeBlock") return true;
  return false;
}

/** The length of the quote marker or indentation at the start of a line — fence
 *  content begins after it. The anchor retry and livepreview's closing-fence-line
 *  check share this one "a line may begin with `> `" rule. */
export function contentStart(lineText: string): number {
  return /^[\s>]*/.exec(lineText)![0].length;
}

/** The FencedCode node at pos (the button's anchor: the start of the fence's first
 *  line); null when pos is not on a fence. When the line start misses, the leading
 *  quote marker or indentation is skipped and it is tried once more. */
export function fencedCodeAt(state: EditorState, pos: number): SyntaxNode | null {
  const walk = (p: number): SyntaxNode | null => {
    for (let n: SyntaxNode | null = syntaxTree(state).resolveInner(p, 1); n; n = n.parent)
      if (n.name === "FencedCode") return n;
    return null;
  };
  const found = walk(pos);
  if (found) return found;
  const line = state.doc.lineAt(pos);
  const skip = contentStart(line.text);
  return skip > 0 ? walk(Math.min(line.from + skip, line.to)) : null;
}

/** The code content of a FencedCode node (without the fence lines or the quote markers); an empty block returns "". */
export function fencedCodeText(doc: Text, fenced: SyntaxNode): string {
  return fenced
    .getChildren("CodeText")
    .map((t) => doc.sliceString(t.from, t.to))
    .join("");
}
