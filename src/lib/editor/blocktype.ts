// Applying a block type (`.vemry/types/*.md`) in the editor: a fresh block from the
// type's template (the `/` menu on an empty line), or the block at hand turned into
// the type's kind with its missing fields and tag appended (the `/` menu on a line
// with text, "turn into ›" in the block menu). One transaction either way.
import { ChangeSet, type EditorState, type TransactionSpec } from "@codemirror/state";
import type { BlockType } from "$lib/api";
import { ANCHOR_TAIL_RE, scanFields } from "$lib/fields";
import { localDate } from "$lib/views/tasks";
import { blockAt, type EditorBlock } from "./blocks";
import { type TurnTarget, canTurnInto, turnInto } from "./blockops";

/** The line prefix a kind starts with (`code` wraps instead). */
const PREFIX: Record<string, string> = {
  paragraph: "",
  heading1: "# ",
  heading2: "## ",
  heading3: "### ",
  bullet: "- ",
  numbered: "1. ",
  task: "- [ ] ",
  quote: "> ",
  callout: "> [!note] ",
  private: "> [!private] ",
  code: "",
};

/** The template with its placeholders filled: `{{key}}` → the field's default,
 *  `{{date}}` → today, `{{cursor}}` → removed, its position returned as the caret. */
export function renderTypeTemplate(type: BlockType): { text: string; caret: number } {
  const defaults = new Map(type.fields.map((f) => [f.key.toLowerCase(), f.default ?? ""]));
  let text = type.template.replace(/\{\{\s*([^}\s]+)\s*\}\}/g, (m, name: string) => {
    if (name === "cursor") return m; // resolved below
    if (name === "date") return localDate();
    return defaults.get(name.toLowerCase()) ?? "";
  });
  let caret = text.indexOf("{{cursor}}");
  if (caret >= 0) text = text.slice(0, caret) + text.slice(caret + "{{cursor}}".length);
  else caret = text.length;
  text = text.replace(/\{\{cursor\}\}/g, "");
  return { text, caret };
}

/** A new block of the type at the start of a line: the kind's prefix (a callout takes
 *  the type's callout name), the rendered template, a fence for `code`. */
export function typeBlockText(type: BlockType): { text: string; caret: number } {
  const { text, caret } = renderTypeTemplate(type);
  if (type.kind === "code") return { text: `\`\`\`\n${text}\n\`\`\``, caret: 4 + caret };
  let prefix = PREFIX[type.kind] ?? "";
  if (type.kind === "callout") prefix = `> [!${type.callout ?? "note"}] `;
  const cont = prefix === "" ? "" : prefix.startsWith(">") ? "> " : "  ";
  const body = text.replace(/\n/g, `\n${cont}`);
  const withTag = type.tag && !body.includes(`#${type.tag}`) ? `${body} #${type.tag}` : body;
  // every line break before the caret gained a continuation prefix
  return { text: prefix + withTag, caret: prefix.length + caret + newlinesBefore(text, caret) * cont.length };
}

/** How many line breaks `text` has before `caret`. */
export function newlinesBefore(text: string, caret: number): number {
  return (text.slice(0, caret).match(/\n/g) ?? []).length;
}

/** Turn the block into the type: its kind first (turnInto, when the kind differs and
 *  the block allows it), then the fields the block does not carry yet and the tag,
 *  appended to the first line before any trailing ` ^id`. `omit` is a range to drop
 *  (the `/` token). Returns null when nothing changes. */
export function applyType(
  state: EditorState,
  b: EditorBlock,
  type: BlockType,
  opts: { omit?: { from: number; to: number } } = {},
): TransactionSpec | null {
  const kind = type.kind as TurnTarget;
  let changes = ChangeSet.empty(state.doc.length);
  let st = state;
  const spec = canTurnInto(b, kind) ? turnInto(state, b, kind, opts.omit ? { omit: opts.omit } : undefined) : null;
  if (spec) {
    const tr = state.update(spec);
    changes = tr.changes;
    st = tr.state;
  } else if (opts.omit) {
    const tr = state.update({ changes: { from: opts.omit.from, to: opts.omit.to, insert: "" } });
    changes = tr.changes;
    st = tr.state;
  }
  const nb = blockAt(st, Math.min(changes.mapPos(b.from, 1), st.doc.length));
  let caret: number | null = null;
  // a code block's first line is its fence: fields there would be its info string
  if (nb && nb.kind !== "code") {
    const line = st.doc.lineAt(nb.from);
    const text = line.text;
    const existing = new Set(scanFields(text).map((f) => f.key.toLowerCase()));
    const pieces: string[] = [];
    let firstEmpty = -1;
    for (const f of type.fields) {
      if (existing.has(f.key.toLowerCase())) continue;
      const value = f.default ?? "";
      if (firstEmpty < 0 && value === "") firstEmpty = pieces.join(" ").length + (pieces.length ? 1 : 0) + `[${f.key}:: `.length;
      pieces.push(`[${f.key}:: ${value}]`);
    }
    if (type.tag && !new RegExp(`(^|\\s)#${type.tag.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(\\s|$)`).test(text)) pieces.push(`#${type.tag}`);
    if (pieces.length) {
      const trimmed = text.replace(/[ \t]+$/, "");
      const anchor = ANCHOR_TAIL_RE.exec(trimmed);
      const at = line.from + (anchor ? anchor.index : trimmed.length);
      const insert = ` ${pieces.join(" ")}`;
      const tr = st.update({ changes: { from: at, insert } });
      changes = changes.compose(tr.changes);
      caret = firstEmpty >= 0 ? at + 1 + firstEmpty : at + insert.length;
    }
  }
  if (changes.empty) return null;
  const head = caret ?? changes.mapPos(state.selection.main.head, 1);
  return { changes, selection: { anchor: head }, userEvent: "block.type" };
}
