// The lezer inline extension for ==highlight==. Neither CommonMark nor GFM has it
// (see docs/SYNTAX.md), and it follows how @lezer/markdown's own Strikethrough is
// written: paired == delimiters, a Highlight content node and a HighlightMark marker
// node.
import type { MarkdownConfig } from "@lezer/markdown";
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
