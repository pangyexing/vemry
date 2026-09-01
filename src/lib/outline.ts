// Heading outline extraction (for the "Outline" panel on the right). The heading rule
// mirrors insert.ts's headingLevel and the Rust parser.rs's parse_heading: ATX only
// (1-6 #s followed by a space, with leading indentation allowed), skipping the
// frontmatter and phantom headings inside code fences. Offsets are in UTF-16
// characters and can be used as CodeMirror positions directly — which is one reason
// this does not go through Rust's query_blocks (its spans are byte offsets, and it
// would have to wait for the file to reach disk). Changing the rule means changing it
// in all three places.

export interface OutlineItem {
  level: number;
  text: string;
  /** The character offset of the start of the heading's line (the same units as a CM position). */
  from: number;
}

export function extractOutline(doc: string): OutlineItem[] {
  const lines = doc.split("\n");
  const items: OutlineItem[] = [];
  let pos = 0;
  let i = 0;
  if (lines[0]?.trimEnd() === "---") {
    for (let j = 1; j < lines.length; j++) {
      const t = lines[j].trimEnd();
      if (t === "---" || t === "...") {
        for (let k = 0; k <= j; k++) pos += lines[k].length + 1;
        i = j + 1;
        break;
      }
    }
  }
  let fence: { ch: string; len: number } | null = null;
  for (; i < lines.length; i++) {
    const line = lines[i];
    const t = line.trimStart();
    if (fence) {
      const m = t.match(/^(`{3,}|~{3,})\s*$/);
      if (m && m[1][0] === fence.ch && m[1].length >= fence.len) fence = null;
    } else {
      const open = t.match(/^(`{3,}|~{3,})/);
      if (open) {
        fence = { ch: open[1][0], len: open[1].length };
      } else {
        let n = 0;
        while (n < t.length && t[n] === "#") n++;
        if (n >= 1 && n <= 6 && t[n] === " ") {
          items.push({ level: n, text: t.slice(n).trim(), from: pos });
        }
      }
    }
    pos += line.length + 1;
  }
  return items;
}
