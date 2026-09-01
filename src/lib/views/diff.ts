// Unified-diff line classification, shared by the permission dialog and the
// session replay view.

export interface DiffLine {
  cls: "meta" | "hunk" | "add" | "del" | "ctx";
  text: string;
}

export function classifyDiff(diff: string): DiffLine[] {
  // `---`/`+++` are a file header only before the first @@: deleting a line
  // `--verbose` inside a hunk renders as `---verbose`, and judging by the prefix alone
  // would grey it out as meta, where a reviewer easily misses the deletion
  let inHunks = false;
  return diff.split("\n").map((line): DiffLine => {
    if (line.startsWith("@@")) {
      inHunks = true;
      return { cls: "hunk", text: line };
    }
    if (!inHunks && (line.startsWith("+++") || line.startsWith("---"))) return { cls: "meta", text: line };
    if (line.startsWith("+")) return { cls: "add", text: line };
    if (line.startsWith("-")) return { cls: "del", text: line };
    return { cls: "ctx", text: line };
  });
}
