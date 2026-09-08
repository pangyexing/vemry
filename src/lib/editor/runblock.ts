// The "run" button of a js code block: a click sends the block's contents to a
// one-shot read-only sandbox on the Rust side (a 2s fuse, no write API), and the
// output is rendered in a block-level widget after the closing fence.
// The output exists only in the editor's state — it never reaches disk and vanishes
// when the document closes. This button is the only entry point of the whole chain:
// there is no automatic execution path at all (opening a note is not running code).
// A block that called vemry.lab.* comes back with a lab spec, and its output widget
// is a live session (lab.ts): the controls it declared re-run the block while the
// session is on, still through this sandbox and still only after that first click.
import { t } from "$lib/i18n/i18n.svelte";
import type { BlockEvalResult, LabParams } from "$lib/api";
import { blockRoot } from "./blockwidget";
import { LabSession } from "./lab";
import { syntaxTree } from "@codemirror/language";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { type EditorState, type Extension, StateEffect, StateField } from "@codemirror/state";
import { fencedCodeAt, fencedCodeText } from "./fenced";

export interface RunHost {
  /** `params` are the lab controls' current values on a re-run; null on the first run. */
  evalBlock(code: string, params: LabParams | null): Promise<BlockEvalResult>;
}

const setOutput = StateEffect.define<{ at: number; output: string; error: string | null; session: LabSession | null }>({
  map: (v, m) => ({ ...v, at: m.mapPos(v.at) }),
});
const clearOutput = StateEffect.define<{ at: number }>({
  map: (v, m) => ({ at: m.mapPos(v.at) }),
});

interface Located {
  code: string;
  /** The output anchor: the end of the closing fence line. */
  endPos: number;
  /** The code's span, when it is one contiguous piece the pin can replace (a fence
   *  inside a quote is cut into pieces by the parser and cannot be). */
  range: { from: number; to: number } | null;
}

/** The fenced block at pos: its code, the output anchor and the replaceable range. */
function blockAt(view: EditorView, pos: number): Located | null {
  const n = fencedCodeAt(view.state, pos);
  if (!n) return null;
  const code = fencedCodeText(view.state.doc, n);
  const endPos = view.state.doc.lineAt(Math.min(n.to, view.state.doc.length)).to;
  const pieces = n.getChildren("CodeText");
  let range: { from: number; to: number } | null = null;
  if (pieces.length) {
    const from = pieces[0].from;
    const to = pieces[pieces.length - 1].to;
    if (view.state.doc.sliceString(from, to) === code) range = { from, to };
  }
  return { code, endPos, range };
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
      let info: Located | null;
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
        .evalBlock(info.code, null)
        .then((r) => {
          const at = anchor();
          if (at == null) return;
          // A lab block gets a live session: the host it needs finds the block again
          // from where the widget sits, so an edited block re-runs as edited and a
          // deleted one is simply not run
          let session: LabSession | null = null;
          if (r.lab) {
            const h = labHost(view, this.host);
            session = new LabSession(h, r.lab, r.output);
            h.bind(session.root);
          }
          view.dispatch({ effects: setOutput.of({ at, output: r.output, error: r.error, session }) });
        })
        .catch((e) => {
          const at = anchor();
          if (at != null) view.dispatch({ effects: setOutput.of({ at, output: "", error: String(e), session: null }) });
        })
        .finally(() => {
          btn.disabled = false;
          btn.textContent = t("runBlock.run");
        });
    };
    return btn;
  }
}

/** The editor side of a lab session. The session's root is the widget's DOM, so its
 *  position in the document is where the block's closing fence ends — the block is
 *  found from there at every call, never from a position remembered at click time. */
function labHost(view: EditorView, host: RunHost) {
  let root: HTMLElement | null = null;
  const at = (): number | null => {
    if (!root) return null;
    try {
      return view.posAtDOM(root);
    } catch {
      return null;
    }
  };
  const located = (): Located | null => {
    const pos = at();
    if (pos == null) return null;
    // the widget sits at the end of the closing fence line; that line's start is inside the fence node
    return blockAt(view, view.state.doc.lineAt(pos).from);
  };
  return {
    bind: (el: HTMLElement) => {
      root = el;
    },
    locate: () => {
      const l = located();
      return l ? { code: l.code, range: l.range } : null;
    },
    evalBlock: (code: string, params: LabParams | null) => host.evalBlock(code, params),
    replaceCode: (range: { from: number; to: number }, code: string) => {
      view.dispatch({ changes: { from: range.from, to: range.to, insert: code } });
    },
    close: () => {
      const pos = at();
      if (pos != null) view.dispatch({ effects: clearOutput.of({ at: pos }) });
    },
  };
}

class OutputWidget extends WidgetType {
  constructor(
    readonly output: string,
    readonly error: string | null,
    readonly session: LabSession | null,
  ) {
    super();
  }
  eq(other: OutputWidget) {
    return other.output === this.output && other.error === this.error && other.session === this.session;
  }
  ignoreEvent() {
    return true;
  }
  toDOM(view: EditorView) {
    if (this.session) {
      this.session.attach(view);
      return blockRoot(this.session.root);
    }
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
  destroy() {
    this.session?.destroy();
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
                widget: new OutputWidget(e.value.output, e.value.error, e.value.session),
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
