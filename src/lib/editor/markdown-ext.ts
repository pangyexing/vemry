// The lezer extensions the note grammar adds on top of CommonMark + GFM:
// - ==highlight==, which neither has (see docs/SYNTAX.md), written the way
//   @lezer/markdown's own Strikethrough is: paired == delimiters, a Highlight content
//   node and a HighlightMark marker node;
// - the four checkbox states, replacing GFM's two-state task list.
import type { BlockContext, LeafBlock, LeafBlockParser, MarkdownConfig } from "@lezer/markdown";
import { Tag, tags } from "@lezer/highlight";

/** setup.ts's HighlightStyle hangs a class on it */
export const highlightTag = Tag.define();

const HighlightDelim = { resolve: "Highlight", mark: "HighlightMark" };

// The punctuation class matching @lezer/markdown's internals (used for the flanking
// check); ‐-‧ is the punctuation range from the hyphen to the interpunct
const Punctuation = /[!"#$%&'()*+,\-./:;<=>?@[\\\]^_`{|}~\xA1\u2010-\u2027]/;

export const Highlight: MarkdownConfig = {
  defineNodes: [
    { name: "Highlight", style: { "Highlight/...": highlightTag } },
    { name: "HighlightMark", style: tags.processingInstruction },
  ],
  parseInline: [
    {
      name: "Highlight",
      parse(cx, next, pos) {
        // Exactly two = (three or more are literal, matching how ~~ is handled)
        if (next !== 61 /* = */ || cx.char(pos + 1) !== 61 || cx.char(pos + 2) === 61) return -1;
        // The flanking check matches Strikethrough's: a == with whitespace on both
        // sides neither opens nor closes, or a comparison like "a == b == c" would be
        // taken for a pair of highlight delimiters
        const before = cx.slice(pos - 1, pos);
        const after = cx.slice(pos + 2, pos + 3);
        const sBefore = /\s|^$/.test(before);
        const sAfter = /\s|^$/.test(after);
        const pBefore = Punctuation.test(before);
        const pAfter = Punctuation.test(after);
        return cx.addDelimiter(
          HighlightDelim,
          pos,
          pos + 2,
          !sAfter && (!pAfter || sBefore || pBefore),
          !sBefore && (!pBefore || sAfter || pAfter),
        );
      },
      after: "Emphasis",
    },
  ],
};

/** GFM's task list recognises `[ ]`, `[x]` and `[X]` alone, so a `[/]` (in progress) or
 *  `[-]` (cancelled) line produced no Task/TaskMarker node: the editor drew no checkbox
 *  for it, and ⌥-clicking a box into `[/]` made the box disappear under the pointer. The
 *  block parser is replaced by name (configure swaps a parser with an existing name in
 *  place), with the four states parse_task_line reads in Rust; the nodes themselves stay
 *  GFM's, so nothing else about a task changes. */
class TaskParser implements LeafBlockParser {
  nextLine() {
    return false;
  }
  finish(cx: BlockContext, leaf: LeafBlock) {
    cx.addLeafElement(
      leaf,
      cx.elt("Task", leaf.start, leaf.start + leaf.content.length, [
        cx.elt("TaskMarker", leaf.start, leaf.start + 3),
        ...cx.parser.parseInline(leaf.content.slice(3), leaf.start + 3),
      ]),
    );
    return true;
  }
}

/** The checkbox characters, mirroring views/tasks.ts's ANY_CHECKBOX_RE and Rust's
 *  parse_task_line (an unknown character is not a checkbox here, and reads as to-do there). */
const TASK_MARKER_RE = /^\[[ xX/-]\][ \t]/;

export const TaskStates: MarkdownConfig = {
  parseBlock: [
    {
      name: "TaskList",
      leaf: (cx, leaf) => (TASK_MARKER_RE.test(leaf.content) && cx.parentType().name === "ListItem" ? new TaskParser() : null),
      after: "SetextHeading",
    },
  ],
};
