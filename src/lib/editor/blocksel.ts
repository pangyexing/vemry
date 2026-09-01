// Block-level keyboard commands in text mode: move the block at the cursor with
// ⌘⇧↑ / ⌘⇧↓ and duplicate it with ⌘⇧D — keys defaultKeymap leaves free (⌥↑/⌥↓
// stay CodeMirror's line moves). The block selection mode (Esc selects the
// block, ↑/↓ walk, ⌫ deletes, ⌘C copies Markdown…) grows on this file in the
// next batch; the transforms all come from blockops.ts.
import { Prec } from "@codemirror/state";
import { EditorView, keymap, type Command } from "@codemirror/view";
import { blockAt, ensureParsed } from "./blocks";
import { duplicateBlocks, moveBlocks } from "./blockops";

function withBlock(f: (view: EditorView, b: ReturnType<typeof blockAt>) => boolean): Command {
  return (view) => {
    const head = view.state.selection.main.head;
    if (!ensureParsed(view.state, Math.min(view.state.doc.length, head + 5000))) view.dispatch({});
    const b = blockAt(view.state, head);
    if (!b) return false;
    return f(view, b);
  };
}

const moveUp: Command = withBlock((view, b) => {
  const spec = moveBlocks(view.state, [b!], -1);
  if (spec) view.dispatch(spec);
  return true;
});

const moveDown: Command = withBlock((view, b) => {
  const spec = moveBlocks(view.state, [b!], 1);
  if (spec) view.dispatch(spec);
  return true;
});

const duplicate: Command = withBlock((view, b) => {
  const spec = duplicateBlocks(view.state, [b!]);
  if (spec) view.dispatch(spec);
  return true;
});

/** The text-mode block keymap (highest precedence so ⌘⇧↑ is not swallowed by a
 *  selection extension binding elsewhere). */
export function blockKeys() {
  return Prec.highest(
    keymap.of([
      { key: "Mod-Shift-ArrowUp", run: moveUp },
      { key: "Mod-Shift-ArrowDown", run: moveDown },
      { key: "Mod-Shift-d", run: duplicate },
    ]),
  );
}
