// A one-shot highlight of the landing block after a jump (a citation click, a
// query card, a wikilink with a fragment): the lines flash for FLASH_MS and the
// decoration is removed by a timer. The field maps the range through edits so a
// keystroke during the flash does not strand the highlight.
import { type Extension, StateEffect, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView } from "@codemirror/view";

export const FLASH_MS = 1200;

const flashEffect = StateEffect.define<{ from: number; to: number } | null>({
  map: (v, m) => (v ? { from: m.mapPos(v.from), to: m.mapPos(v.to) } : null),
});

const flashLine = Decoration.line({ class: "cm-flash" });

export const flashField: Extension = StateField.define<DecorationSet>({
  create: () => Decoration.none,
  update(decos, tr) {
    decos = decos.map(tr.changes);
    for (const e of tr.effects) {
      if (!e.is(flashEffect)) continue;
      if (!e.value) return Decoration.none;
      const doc = tr.state.doc;
      const from = Math.min(e.value.from, doc.length);
      const to = Math.min(Math.max(e.value.to, from), doc.length);
      const first = doc.lineAt(from).number;
      const last = doc.lineAt(to).number;
      const out = [];
      for (let n = first; n <= last; n++) out.push(flashLine.range(doc.line(n).from));
      return Decoration.set(out);
    }
    return decos;
  },
  provide: (f) => EditorView.decorations.from(f),
});

export function flashRange(view: EditorView, from: number, to: number) {
  view.dispatch({ effects: flashEffect.of({ from, to }) });
  setTimeout(() => {
    try {
      view.dispatch({ effects: flashEffect.of(null) });
    } catch {
      // the view may have been destroyed meanwhile
    }
  }, FLASH_MS);
}
