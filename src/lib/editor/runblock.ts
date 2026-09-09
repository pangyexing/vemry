// The "run" button of a js code block: a click sends the block's contents to a
// one-shot read-only sandbox on the Rust side (a 2s fuse, no write API), and the
// output is rendered in a block-level widget after the closing fence.
// The output exists only in the editor's state — it never reaches disk and vanishes
// when the document closes. This button is the only entry point of the whole chain:
// there is no automatic execution path at all (opening a note is not running code).
//
// A block that calls vemry.lab.* is a lab, and it is shown as its card rather than
// its code (the reader wants the experiment; the code is for editing): with the
// cursor outside the fence the whole block is replaced by a card — before the first
// run a one-line cold card (the block's leading comment as its caption, a Run
// button), after it the live session (lab.ts) — and with the cursor inside, the
// source, as a query block or a table. Code or card, never both; the session lives
// in this file's state field across that, keyed by the fence's start, so the card
// comes back with its controls where they were. A block edited after a run says so
// on its card and waits for Run: a re-run only ever follows a person's move.
import { t } from "$lib/i18n/i18n.svelte";
import type { BlockEvalResult, LabParams } from "$lib/api";
import { blockRoot } from "./blockwidget";
import { ICON_EDIT, LabSession, type LabHost, cardReveal, iconButton, joinOutput, labInteracting } from "./lab";
import { swallowPress } from "$lib/lab/dom";
import { focusEditor } from "./clickguard";
import { syntaxTree } from "@codemirror/language";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import { EditorState, type Extension, RangeSet, RangeValue, StateEffect, StateField, type Transaction } from "@codemirror/state";
import { fencedCodeAt, fencedCodeText } from "./fenced";
import { selectionReveals, selectionTouches } from "./reveal";
import { foldRangesOf, insideFold } from "./table";

export interface RunHost {
  /** `params` are the lab controls' current values on a re-run; null on the first run. */
  evalBlock(code: string, params: LabParams | null): Promise<BlockEvalResult>;
}

const setOutput = StateEffect.define<{ at: number; output: string; error: string | null }>({
  map: (v, m) => ({ ...v, at: m.mapPos(v.at) }),
});
const clearOutput = StateEffect.define<{ at: number }>({
  map: (v, m) => ({ at: m.mapPos(v.at) }),
});
/** A lab session begins (anchored at the fence's start) or ends. */
const setLab = StateEffect.define<{ at: number; session: LabSession }>({
  map: (v, m) => ({ ...v, at: m.mapPos(v.at) }),
});
const clearLab = StateEffect.define<{ at: number }>({
  map: (v, m) => ({ at: m.mapPos(v.at) }),
});

interface Located {
  code: string;
  /** The fence's start: the start of its first line (where a lab session is anchored). */
  start: number;
  /** The first code line's start: where ✎ lands the cursor. */
  codeStart: number;
  /** The output anchor: the end of the closing fence line. */
  endPos: number;
  /** The code's span, when it is one contiguous piece the pin can replace (a fence
   *  inside a quote is cut into pieces by the parser and cannot be). */
  range: { from: number; to: number } | null;
}

/** The fenced block at pos: its code, the anchors and the replaceable range. */
function blockAt(view: EditorView, pos: number): Located | null {
  const n = fencedCodeAt(view.state, pos);
  if (!n) return null;
  const doc = view.state.doc;
  const code = fencedCodeText(doc, n);
  const first = doc.lineAt(n.from);
  const endPos = doc.lineAt(Math.min(n.to, doc.length)).to;
  const pieces = n.getChildren("CodeText");
  let range: { from: number; to: number } | null = null;
  if (pieces.length) {
    const from = pieces[0].from;
    const to = pieces[pieces.length - 1].to;
    if (doc.sliceString(from, to) === code) range = { from, to };
  }
  return { code, start: first.from, codeStart: Math.min(first.to + 1, endPos), endPos, range };
}

/** The fenced block an element inside a card or a button stands in, or null once it has
 *  left the document. Its position is its line's start either way — a `side: -2` widget
 *  and a block replace both anchor at the fence's first line. */
function blockAtDOM(view: EditorView, el: HTMLElement): Located | null {
  try {
    return blockAt(view, view.state.doc.lineAt(view.posAtDOM(el)).from);
  } catch {
    return null;
  }
}

/** Whether a block is a lab: it calls the lab API somewhere in its code. */
const LAB_RE = /\bvemry\.lab\./;

/** The caption of a cold card: the block's leading `//` comment, first line only. */
function captionOf(code: string): string {
  const nl = code.indexOf("\n");
  const m = /^\s*\/\/\s*(.+)$/.exec(nl < 0 ? code : code.slice(0, nl));
  return m ? m[1].trim() : "";
}

/** Run the block at `info` and place the result: a lab starts a session anchored at
 *  the fence's start (and, from the source state, the cursor steps out of the fence
 *  so the card can show), anything else is text after the fence. Shared by the Run
 *  button on the source and the cold card's Run. */
async function runBlock(view: EditorView, host: RunHost, info: Located, relocate: () => Located | null, fromSource: boolean) {
  let r: BlockEvalResult;
  try {
    r = await host.evalBlock(info.code, null);
  } catch (e) {
    const now = relocate();
    if (now) view.dispatch({ effects: setOutput.of({ at: now.endPos, output: "", error: String(e) }) });
    return;
  }
  // The document may have changed while it ran: the block is located again. Should
  // it have left the document (a different note was opened, the block was deleted)
  // the result is discarded — the editor view is reused across notes, and an anchor
  // from click time could land the output in another note
  const now = relocate();
  if (!now) return;
  if (!r.lab) {
    view.dispatch({ effects: setOutput.of({ at: now.endPos, output: r.output, error: r.error }) });
    return;
  }
  const h = labHost(view, host);
  const session = new LabSession(h, r.lab, r.output, now.code);
  h.bind(session.root);
  // From the source: the cursor steps to the line after the fence (or before it at
  // the document's end), so the block folds into its card at once — Run is the
  // moment the reader stops editing and starts looking
  let selection: { anchor: number } | undefined;
  if (fromSource) {
    const doc = view.state.doc;
    if (now.endPos < doc.length) selection = { anchor: now.endPos + 1 };
    else if (now.start > 0) selection = { anchor: now.start - 1 };
  }
  view.dispatch({ effects: setLab.of({ at: now.start, session }), selection });
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
      const relocate = () => blockAtDOM(view, btn);
      const info = relocate();
      if (!info) return;
      btn.disabled = true;
      btn.textContent = t("runBlock.running");
      void runBlock(view, this.host, info, relocate, true).finally(() => {
        btn.disabled = false;
        btn.textContent = t("runBlock.run");
      });
    };
    return btn;
  }
}

/** The editor side of a lab session. The session's root is the card's DOM, so its
 *  position in the document is the fence's start — the block is found from there at
 *  every call, never from a position remembered at click time. */
function labHost(view: EditorView, host: RunHost): LabHost & { bind: (el: HTMLElement) => void } {
  let root: HTMLElement | null = null;
  const located = () => (root ? blockAtDOM(view, root) : null);
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
    reveal: () => {
      const l = located();
      if (!l) return;
      view.dispatch({ selection: { anchor: l.codeStart }, annotations: cardReveal.of(true) });
      focusEditor(view);
    },
    close: () => {
      const l = located();
      if (l) view.dispatch({ effects: clearLab.of({ at: l.start }) });
    },
  };
}

const ICON_FLASK =
  '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 2v7.31"/><path d="M14 9.3V1.99"/><path d="M8.5 2h7"/><path d="M14 9.3a6.5 6.5 0 1 1-4 0"/><path d="M5.52 16h12.96"/></svg>';

/** The card a lab block folds into: the live session when there is one, else the
 *  cold card — the caption and a Run button. Its DOM is the session's own root, so
 *  a fold after a reveal brings the controls back where they were. */
class LabCardWidget extends WidgetType {
  constructor(
    readonly host: RunHost,
    readonly session: LabSession | null,
    readonly caption: string,
    readonly stale: boolean,
    /** A range selection reaches across the card: drawn as selected, since the editor draws nothing over a replaced block. */
    readonly selected = false,
  ) {
    super();
  }
  eq(other: LabCardWidget) {
    return other.session === this.session && other.caption === this.caption && other.stale === this.stale && other.selected === this.selected;
  }
  ignoreEvent() {
    return true;
  }
  get estimatedHeight() {
    return this.session ? 320 : 44;
  }
  toDOM(view: EditorView) {
    if (this.session) {
      this.session.attach(view);
      this.session.setStale(this.stale);
      this.session.root.classList.toggle("cm-lab-selected", this.selected);
      return blockRoot(this.session.root);
    }
    const card = document.createElement("div");
    card.className = "cm-lab cm-lab-cold";
    card.classList.toggle("cm-lab-selected", this.selected);
    const head = document.createElement("div");
    head.className = "cm-lab-head";
    const kind = document.createElement("span");
    kind.className = "cm-lab-live";
    kind.innerHTML = ICON_FLASK;
    kind.append(t("runBlock.experiment"));
    const caption = document.createElement("span");
    caption.className = "cm-lab-caption";
    caption.textContent = this.caption;
    caption.title = this.caption;
    const tools = document.createElement("span");
    tools.className = "cm-lab-tools";
    const locate = () => blockAtDOM(view, card);
    const run = document.createElement("button");
    run.className = "cm-lab-run";
    run.textContent = t("runBlock.run");
    swallowPress(run);
    run.addEventListener("click", (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (run.disabled) return;
      const info = locate();
      if (!info) return;
      run.disabled = true;
      run.textContent = t("runBlock.running");
      void runBlock(view, this.host, info, locate, false).finally(() => {
        run.disabled = false;
        run.textContent = t("runBlock.run");
      });
    });
    // the same ✎ as the live card's, annotation included: without it the pointer guard
    // below may drop the selection this dispatches
    const edit = iconButton("cm-lab-tool", ICON_EDIT, t("runBlock.editCode"), () => {
      const info = locate();
      if (!info) return;
      view.dispatch({ selection: { anchor: info.codeStart }, annotations: cardReveal.of(true) });
      focusEditor(view);
    });
    tools.append(run, edit);
    head.append(kind, caption, tools);
    card.appendChild(head);
    return blockRoot(card);
  }
  // A fold after a reveal re-creates the widget around the same session: nothing to
  // destroy here — the session ends with clearLab (✕) or the document
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
    pre.textContent = joinOutput(this.output, this.error) || t("runBlock.noOutput");
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

interface CardResult {
  decos: DecorationSet;
  /** Whether the document holds a js fence at all (a revealed one counts): without one,
   *  a cursor move cannot make a card appear, so the tree is not walked again. */
  hasFence: boolean;
}

/** A session in the labs range set, anchored at its fence's start. */
class SessionMark extends RangeValue {
  constructor(readonly session: LabSession) {
    super();
  }
  eq(other: RangeValue) {
    return other instanceof SessionMark && other.session === this.session;
  }
}

/** The lab cards: every js fence that calls the lab API is replaced whole by its card
 *  unless a bare cursor stands inside it (the keyboard's way in, and the ✎'s) or it is
 *  inside a fold. A range selection that reaches across the card does not open it —
 *  the table's block rule, made for a block with no other way in, would show a
 *  hundred lines of JavaScript to a reader who dragged past — the card is marked
 *  selected instead, since the editor draws no selection over a replaced block. */
function buildCards(state: EditorState, labs: RangeSet<SessionMark>, host: RunHost): CardResult {
  const doc = state.doc;
  const decos: ReturnType<Decoration["range"]>[] = [];
  let hasFence = false;
  syntaxTree(state).iterate({
    enter(node) {
      if (node.name !== "FencedCode") return;
      const firstLine = doc.lineAt(node.from);
      if (!JS_HEAD_RE.test(firstLine.text)) return false;
      hasFence = true;
      const from = firstLine.from;
      const to = doc.lineAt(Math.min(node.to, doc.length)).to;
      // the reveal and the fold decide before the body is read: whether the block is a
      // lab does not change the answer, and reading it materialises the whole fence
      if (selectionReveals(state, from, to) || insideFold(state, from, to)) return false;
      const code = fencedCodeText(doc, node.node);
      if (!LAB_RE.test(code)) return false;
      const selected = selectionTouches(state, from, to);
      let session: LabSession | null = null;
      labs.between(from, from, (f, _t, mark) => {
        if (f === from) session = mark.session;
      });
      const stale = session ? (session as LabSession).lastCode !== code : false;
      decos.push(Decoration.replace({ widget: new LabCardWidget(host, session, captionOf(code), stale, selected), block: true }).range(from, to));
      return false;
    },
  });
  return { decos: Decoration.set(decos, true), hasFence };
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
            add: [Decoration.widget({ widget: new OutputWidget(e.value.output, e.value.error), side: 1, block: true }).range(at)],
          });
        } else if (e.is(clearOutput)) {
          value = value.update({ filterFrom: e.value.at, filterTo: e.value.at, filter: () => false });
        }
      }
      return value;
    },
    provide: (f) => EditorView.decorations.from(f),
  });
  // The sessions, by fence start; a deleted fence takes its session with it
  const labs = StateField.define<RangeSet<SessionMark>>({
    create: () => RangeSet.empty,
    update(value, tr) {
      value = value.map(tr.changes);
      for (const e of tr.effects) {
        if (e.is(setLab)) {
          const at = Math.min(e.value.at, tr.state.doc.length);
          value = value.update({ filterFrom: at, filterTo: at, filter: () => false, add: [new SessionMark(e.value.session).range(at)], sort: true });
        } else if (e.is(clearLab)) {
          value = value.update({
            filterFrom: e.value.at,
            filterTo: e.value.at,
            filter: (_f, _t, mark) => {
              mark.session.destroy();
              return false;
            },
          });
        }
      }
      return value;
    },
  });
  const labChanged = (tr: Transaction) => tr.effects.some((e) => e.is(setLab) || e.is(clearLab));
  const cards = StateField.define<CardResult>({
    create: (state) => buildCards(state, state.field(labs), host),
    update(value, tr) {
      const treeChanged = syntaxTree(tr.state) !== syntaxTree(tr.startState);
      const foldChanged = foldRangesOf(tr).length > 0;
      if (!tr.docChanged && !tr.selection && !treeChanged && !foldChanged && !labChanged(tr)) return value;
      // The common case — a note with no js fence: a cursor move needs no rescan (a
      // document or syntax-tree change still does, since a fence may have just been typed)
      if (!value.hasFence && !tr.docChanged && !treeChanged && !foldChanged) return value;
      return buildCards(tr.state, tr.state.field(labs), host);
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });
  // The pointer guard, two rules for a selection-only transaction (the ✎ reveal is
  // annotated and passes; a doc change and an effect pass):
  // - while a pointer is (or just was) down on a card, it is the browser's caret
  //   settling beside the widget, not the reader's intent — dropped whole, so the
  //   cursor stays where it was;
  // - a click that lands on a fence the card still covers can only have come from the
  //   margin beside the card (the card swallows its own presses), and goes to the line
  //   after the card instead of revealing: the card has its ✎ for that. A table has no
  //   ✎ and keeps the block rule; keyboard navigation onto the fence, a drag across the
  //   card and a click on code already shown all still reveal.
  const guard = EditorState.transactionFilter.of((tr) => {
    if (tr.docChanged || !tr.selection || tr.effects.length || tr.annotation(cardReveal)) return tr;
    if (labInteracting()) return [];
    const main = tr.newSelection.main;
    if (!tr.isUserEvent("select.pointer") || !main.empty) return tr;
    const doc = tr.startState.doc;
    let landing: number | null = null;
    tr.startState.field(cards).decos.between(main.from, main.from, (from, to) => {
      landing = to < doc.length ? to + 1 : from > 0 ? from - 1 : null;
      return false;
    });
    return landing == null ? tr : { selection: { anchor: landing }, userEvent: "select.pointer" };
  });
  return [buttons, outputs, labs, cards, guard];
}
