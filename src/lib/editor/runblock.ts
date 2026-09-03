// The "run" button of a js code block: a click sends the block's contents to a
// one-shot read-only sandbox on the Rust side (a 2s fuse, no write API), and the
// output is rendered in a block-level widget after the closing fence.
// The output exists only in the editor's state — it never reaches disk and vanishes
// when the document closes. This button is the only entry point of the whole chain:
// there is no automatic execution path at all (opening a note is not running code).
import { t } from "$lib/i18n/i18n.svelte";
import { blockRoot } from "./blockwidget";
import { syntaxTree } from "@codemirror/language";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { type EditorState, type Extension, StateEffect, StateField } from "@codemirror/state";
import { fencedCodeAt, fencedCodeText } from "./fenced";

export interface RunHost {
  evalBlock(code: string): Promise<{ output: string; error: string | null }>;
}

const setOutput = StateEffect.define<{ at: number; output: string; error: string | null }>({
  map: (v, m) => ({ ...v, at: m.mapPos(v.at) }),
});
const clearOutput = StateEffect.define<{ at: number }>({
  map: (v, m) => ({ at: m.mapPos(v.at) }),
});

/** The code of the fenced block the button belongs to, plus the output anchor (the end of the closing fence line) */
function blockAt(view: EditorView, pos: number): { code: string; endPos: number } | null {
  const n = fencedCodeAt(view.state, pos);
  if (!n) return null;
  const code = fencedCodeText(view.state.doc, n);
  const endPos = view.state.doc.lineAt(Math.min(n.to, view.state.doc.length)).to;
  return { code, endPos };
}

class RunWidget extends WidgetType {
  constructor(readonly host: RunHost) {
    super();
  }
  eq() {
    return true;
  }
  toDOM(view: EditorView) {
    const btn = document.createElement("button");
    btn.className = "cm-lp-run";
    btn.textContent = t("runBlock.run");
    btn.onmousedown = (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (btn.disabled) return;
      let info: { code: string; endPos: number } | null;
      try {
        info = blockAt(view, view.posAtDOM(btn));
      } catch {
        return;
      }
      if (!info) return;
      btn.disabled = true;
      btn.textContent = t("runBlock.running");
      // The document may have changed while it ran: on completion the button's block
      // is located again. Should the button have left the document (a different note
      // was opened, the block was deleted) the output is discarded — the editor view
      // is reused across notes, and falling back to the anchor from click time would
      // inject the output at an arbitrary position in another note
      const anchor = (): number | null => {
        try {
          const relocated = blockAt(view, view.posAtDOM(btn));
          return relocated ? relocated.endPos : null;
        } catch {
          return null;
        }
      };
      void this.host
        .evalBlock(info.code)
        .then((r) => {
          const at = anchor();
          if (at != null) view.dispatch({ effects: setOutput.of({ at, output: r.output, error: r.error }) });
        })
        .catch((e) => {
          const at = anchor();
          if (at != null) view.dispatch({ effects: setOutput.of({ at, output: "", error: String(e) }) });
        })
        .finally(() => {
          btn.disabled = false;
          btn.textContent = t("runBlock.run");
        });
    };
    return btn;
  }
}

class OutputWidget extends WidgetType {
  constructor(
    readonly output: string,
    readonly error: string | null,
  ) {
    super();
  }
  eq(other: OutputWidget) {
    return other.output === this.output && other.error === this.error;
  }
  ignoreEvent() {
    return true;
  }
  toDOM(view: EditorView) {
    const wrap = document.createElement("div");
    wrap.className = "cm-runblock-out";
    if (this.error) wrap.classList.add("cm-runblock-err");
    const pre = document.createElement("pre");
    pre.textContent = [this.output, this.error ?? ""].filter(Boolean).join("\n") || t("runBlock.noOutput");
    wrap.appendChild(pre);
    const clear = document.createElement("button");
    clear.className = "cm-runblock-clear";
    clear.textContent = "✕";
    clear.title = t("runBlock.clearOutput");
    clear.onmousedown = (e) => {
      e.preventDefault();
      e.stopPropagation();
      try {
        view.dispatch({ effects: clearOutput.of({ at: view.posAtDOM(wrap) }) });
      } catch {
        /* the widget has left the document */
      }
    };
    wrap.appendChild(clear);
    return blockRoot(wrap);
  }
}

const JS_HEAD_RE = /^\s*(`{3,}|~{3,})\s*(js|javascript)\s*$/i;

function buildButtons(state: EditorState, deco: Decoration): DecorationSet {
  const doc = state.doc;
  const decos: ReturnType<Decoration["range"]>[] = [];
  syntaxTree(state).iterate({
    enter(node) {
      if (node.name !== "FencedCode") return;
      const firstLine = doc.lineAt(node.from);
      if (JS_HEAD_RE.test(firstLine.text)) decos.push(deco.range(firstLine.from));
      return false;
    },
  });
  return Decoration.set(decos, true);
}

export function runBlockExtension(host: RunHost): Extension {
  // side -2: it sorts before livepreview's copy button (-1), and the CSS gives each its own top-right slot
  const runDeco = Decoration.widget({ widget: new RunWidget(host), side: -2 });
  const buttons = StateField.define<DecorationSet>({
    create: (state) => buildButtons(state, runDeco),
    update(value, tr) {
      const treeChanged = syntaxTree(tr.state) !== syntaxTree(tr.startState);
      if (!tr.docChanged && !treeChanged) return value;
      return buildButtons(tr.state, runDeco);
    },
    provide: (f) => EditorView.decorations.from(f),
  });
  const outputs = StateField.define<DecorationSet>({
    create: () => Decoration.none,
    update(value, tr) {
      value = value.map(tr.changes);
      for (const e of tr.effects) {
        if (e.is(setOutput)) {
          const at = Math.min(e.value.at, tr.state.doc.length);
          value = value.update({
            filterFrom: at,
            filterTo: at,
            filter: () => false,
            add: [
              Decoration.widget({
                widget: new OutputWidget(e.value.output, e.value.error),
                side: 1,
                block: true,
              }).range(at),
            ],
          });
        } else if (e.is(clearOutput)) {
          value = value.update({ filterFrom: e.value.at, filterTo: e.value.at, filter: () => false });
        }
      }
      return value;
    },
    provide: (f) => EditorView.decorations.from(f),
  });
  return [buttons, outputs];
}
