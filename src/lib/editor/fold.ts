// Folding: a heading folds its section, a callout or quote folds its body. A
// chevron appears at the start of a foldable line on hover (no fold gutter —
// the note column keeps its left edge; it hangs into the content padding, in
// the slot right of the block handle, and stays visible while the handle marks
// it with cm-fold-hover), and a folded range shows a small
// "n lines" chip that unfolds on click. `[!type]-` callouts start folded when
// the document opens; `+` or no marker means expanded. Fold state lives only
// in the editor state (it survives a suspended tab, never the file).
// CodeMirror's fold field unfolds any fold the selection head enters, so the
// outline's and the search's jumps need nothing extra.
import { t } from "$lib/i18n/i18n.svelte";
import { codeFolding, foldedRanges, foldEffect, foldService, syntaxTree, unfoldEffect } from "@codemirror/language";
import { RangeSetBuilder, type EditorState, type Extension, type Line, type StateEffect } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, ViewPlugin, type ViewUpdate, WidgetType } from "@codemirror/view";
import { blockAt, ensureParsed, frontmatterEnd, sectionEnd, topBlocks } from "./blocks";

/** The range a fold on `line` would hide: a heading's section, a callout's or
 *  quote's body. Null when the line is not foldable or the section is empty. */
export function foldRangeAt(state: EditorState, line: Line): { from: number; to: number } | null {
  if (line.from < frontmatterEnd(state.doc)) return null;
  const b = blockAt(state, line.to, { top: true });
  if (!b || b.from !== line.from) return null;
  if (b.kind === "heading") {
    const end = sectionEnd(state, b);
    return end > b.to ? { from: line.to, to: end } : null;
  }
  if ((b.kind === "callout" || b.kind === "quote") && b.to > line.to) return { from: line.to, to: b.to };
  return null;
}

function foldedAt(state: EditorState, from: number): { from: number; to: number } | null {
  let hit: { from: number; to: number } | null = null;
  foldedRanges(state).between(from, from, (f, t) => {
    if (f === from) hit = { from: f, to: t };
  });
  return hit;
}

/** Fold or unfold the foldable line at pos; false when there is nothing to fold. */
export function toggleFoldAt(view: EditorView, pos: number): boolean {
  const line = view.state.doc.lineAt(pos);
  const r = foldRangeAt(view.state, line);
  if (!r) return false;
  const folded = foldedAt(view.state, r.from);
  view.dispatch({ effects: folded ? unfoldEffect.of(folded) : foldEffect.of(r) });
  return true;
}

/** Fold effects for every `[!type]-` callout (the marker means "start folded"). */
export function defaultFolds(state: EditorState): StateEffect<{ from: number; to: number }>[] {
  const out: StateEffect<{ from: number; to: number }>[] = [];
  for (const b of topBlocks(state)) {
    if (b.kind !== "callout" || b.foldMarker !== "-") continue;
    const r = foldRangeAt(state, state.doc.lineAt(b.from));
    if (r) out.push(foldEffect.of(r));
  }
  return out;
}

/** Apply the default folds once a document has been created (not on a
 *  restored state, which keeps its own folds). */
export function foldOnLoad(view: EditorView): void {
  // the tree of a freshly created state may cover the first screen only: push
  // the parse to the end (bounded) and land it with an empty transaction so
  // the callouts below the fold are seen
  ensureParsed(view.state, view.state.doc.length);
  view.dispatch({});
  const effects = defaultFolds(view.state);
  if (effects.length) view.dispatch({ effects });
}

class FoldToggleWidget extends WidgetType {
  constructor(readonly folded: boolean) {
    super();
  }
  eq(other: FoldToggleWidget) {
    return other.folded === this.folded;
  }
  ignoreEvent() {
    return true;
  }
  toDOM(view: EditorView) {
    const el = document.createElement("button");
    el.className = "cm-fold-toggle" + (this.folded ? " cm-fold-folded" : "");
    const tip = t(this.folded ? "block.unfold" : "block.fold");
    el.dataset.tip = tip; // the block grip's tooltip (app.css), not the browser's title
    el.setAttribute("aria-label", tip);
    el.innerHTML =
      '<svg viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M7 5 L12 10 L7 15"/></svg>';
    el.onmousedown = (e) => {
      e.preventDefault();
      e.stopPropagation();
      toggleFoldAt(view, view.posAtDOM(el));
    };
    return el;
  }
}

const foldableLine = Decoration.line({ class: "cm-foldable" });

function toggles(view: EditorView): DecorationSet {
  const b = new RangeSetBuilder<Decoration>();
  const state = view.state;
  const seen = new Set<number>();
  for (const { from, to } of view.visibleRanges) {
    for (let pos = from; pos <= to; ) {
      const line = state.doc.lineAt(pos);
      if (!seen.has(line.from)) {
        seen.add(line.from);
        const r = line.text.trim() ? foldRangeAt(state, line) : null;
        if (r) {
          b.add(line.from, line.from, foldableLine);
          b.add(line.from, line.from, Decoration.widget({ widget: new FoldToggleWidget(foldedAt(state, r.from) != null), side: -1 }));
        }
      }
      if (line.to >= to) break;
      pos = line.to + 1;
    }
  }
  return b.finish();
}

const foldToggles = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = toggles(view);
    }
    update(u: ViewUpdate) {
      const foldChanged = foldedRanges(u.state) !== foldedRanges(u.startState);
      // The tree comparison is what catches background parse progress — the same test the
      // four sibling extensions use. "Any transaction carrying any effect" also fired on
      // the completion popup's per-keystroke effects, on closeBrackets and on block mode.
      if (u.docChanged || u.viewportChanged || foldChanged || syntaxTree(u.state) !== syntaxTree(u.startState)) {
        this.decorations = toggles(u.view);
      }
    }
  },
  { decorations: (v) => v.decorations },
);

export function foldExtension(): Extension {
  return [
    foldService.of((state, from) => foldRangeAt(state, state.doc.lineAt(from))),
    codeFolding({
      preparePlaceholder: (state, range) => state.doc.lineAt(range.to).number - state.doc.lineAt(range.from).number,
      placeholderDOM(_view, onclick, prepared) {
        const el = document.createElement("span");
        el.className = "cm-fold-chip";
        el.textContent = t("block.foldedLines", { n: Number(prepared) || 0 });
        el.onclick = onclick;
        return el;
      },
    }),
    foldToggles,
  ];
}
