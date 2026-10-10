import type { FieldStat, BlockType } from "$lib/api";
// CodeMirror assembly for the note editor and the plain/code file editor.
import { t } from "$lib/i18n/i18n.svelte";
import { Compartment, EditorSelection, EditorState, Prec, RangeSetBuilder, type Extension } from "@codemirror/state";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  keymap,
  drawSelection,
  highlightSpecialChars,
  placeholder,
  ViewPlugin,
  type Command,
  type ViewUpdate,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { markdown, markdownLanguage, insertNewlineContinueMarkupCommand, deleteMarkupBackward } from "@codemirror/lang-markdown";
import { syntaxHighlighting, HighlightStyle, LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { autocompletion, completionKeymap, closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { tags } from "@lezer/highlight";

import { wikilinkExtension, type WikilinkHost } from "./wikilink";
import { mdlinkExtension } from "./mdlink";
import { linkAt } from "./linkat";
import { livePreview, type AssetLoader, type ImageCopier, type FieldChipHandler, type LivePreviewOptions, deleteTaskMarkerBackward } from "./livepreview";
import { allowPastedRemoteImages } from "./remoteimages";
import { tagCompletions, calloutCompletions, atCompletions, attrIconOption, fieldCompletions, imeCompletionGuard, acceptWithTab } from "./completions";
import { refCompletions } from "./blockref";
import { slashCompletions } from "./slash";
import { embedAt, embedExtension, openEmbedSource, type EmbedHost } from "./embed";
import { provenanceExtension, type ProvenanceHost } from "./provenance";
import { flashField } from "./flash";
import { Highlight, highlightTag, TaskStates } from "./markdown-ext";
import { queryBlockExtension, type QueryHost } from "./queryblock";
import { runBlockExtension, type RunHost } from "./runblock";
import { blockSelection } from "./blocksel";
import { foldExtension } from "./fold";
import { blockHandle, type BlockHost } from "./blockhandle";
import { dragSourceField } from "./blockdrag";
import { clickGuard } from "./clickguard";

// The font and line height have to be declared in the theme layer: CM's baseTheme
// injects monospace and a 1.4 line height at .ͼx .cm-scroller (0,2,0), which app.css's
// bare .cm-scroller class cannot outweigh (the same cascade note as .cm-content's
// column width); EditorView.theme's rules come after baseTheme, and at equal
// specificity the later one wins
const noteFont = EditorView.theme({
  ".cm-scroller": { fontFamily: "var(--font-ui)", lineHeight: "1.75" },
});
const codeFont = EditorView.theme({
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.6" },
});

// What CodeMirror's own base themes colour, taken over in the theme layer for the same
// cascade reason as the font above: its light and dark defaults are scoped one class
// deeper than a bare app.css rule, so the completion menu's selected row stayed its
// default bright blue with white text (the app's rule never applied), and the menu's
// ground was its grey. Here the row is the app's active ground with the body ink, in
// both themes. The colours are the tokens, so the theme needs no dark variant of its own.
const chromeTheme = EditorView.theme({
  ".cm-tooltip": {
    background: "var(--bg-popover)",
    color: "var(--fg)",
    border: "1px solid var(--border)",
  },
  ".cm-tooltip.cm-tooltip-autocomplete": {
    borderRadius: "var(--radius)",
    boxShadow: "var(--shadow)",
    fontFamily: "var(--font-ui)",
    overflow: "hidden",
  },
  ".cm-tooltip-autocomplete ul li[aria-selected]": {
    background: "var(--bg-active)",
    color: "var(--fg)",
  },
  ".cm-completionMatchedText": {
    textDecoration: "none",
    fontWeight: "600",
  },
});

// Whether the editor is on a dark ground, told to CodeMirror (EditorView.darkTheme) so
// the defaults it still owns take their dark values rather than light ones on a dark
// page. It follows the system scheme live: each view listens and reconfigures its
// compartment when the scheme flips (the app sets no theme of its own; the tokens follow
// prefers-color-scheme, and so does this).
const darkMode = new Compartment();
const darkQuery = typeof window !== "undefined" && window.matchMedia ? window.matchMedia("(prefers-color-scheme: dark)") : null;
const darkFacet = () => EditorView.darkTheme.of(!!darkQuery?.matches);
const followScheme = ViewPlugin.fromClass(
  class {
    constructor(readonly view: EditorView) {
      darkQuery?.addEventListener("change", this.flip);
    }
    flip = () => {
      this.view.dispatch({ effects: darkMode.reconfigure(darkFacet()) });
    };
    destroy() {
      darkQuery?.removeEventListener("change", this.flip);
    }
  },
);

/** The scrollbar shows while the note scrolls, not only while the pointer is over it
 *  (app.css draws the thumb on :hover): a keyboard scroll, or the view following the
 *  cursor, shows where in the note the reader is, and the thumb fades once it settles. */
const scrollingThumb = ViewPlugin.fromClass(
  class {
    timer = 0;
    constructor(readonly view: EditorView) {}
    destroy() {
      clearTimeout(this.timer);
    }
  },
  {
    eventObservers: {
      scroll() {
        this.view.scrollDOM.classList.add("cm-scrolling");
        clearTimeout(this.timer);
        this.timer = window.setTimeout(() => this.view.scrollDOM.classList.remove("cm-scrolling"), 900);
      },
    },
  },
);

// The characters drawn as a visible placeholder: the control characters, and the ones
// that can make a line say something other than it looks (the bidi overrides, the
// interlinear annotation marks). Left out, and so drawn as the browser draws them: the
// zero-width space and the BOM (invisible; a clip carries them in from the web, and a
// red dot for each read as damage), the soft hyphen (a hyphenation hint the browser
// honours at a line end), and the direction marks and isolates (invisible, and what
// keeps a mixed right-to-left line in order). The g flag is required: the decorator
// behind this throws on a pattern without it, and the whole editor fails to open.
const SPECIAL_CHARS = /[\u0000-\u0008\u000a-\u001f\u007f-\u009f\u2028\u2029\u202d\u202e\ufff9-\ufffc]/gu;

/** A right-to-left letter: Hebrew, Arabic, Syriac, Thaana, N'Ko and their presentation forms. */
const RTL_CHAR = /[\u0590-\u08ff\ufb1d-\ufdff\ufe70-\ufefc]/;
const autoDirLine = Decoration.line({ attributes: { dir: "auto" } });

function bidiDecorations(view: EditorView): DecorationSet {
  const builder = new RangeSetBuilder<Decoration>();
  const doc = view.state.doc;
  for (const { from, to } of view.visibleRanges) {
    for (let pos = from; pos <= to; ) {
      const line = doc.lineAt(pos);
      if (RTL_CHAR.test(line.text)) builder.add(line.from, line.from, autoDirLine);
      if (line.to >= to) break;
      pos = line.to + 1;
    }
  }
  return builder.finish();
}

/** Each line takes the direction of its own first strong letter: a Hebrew or Arabic
 *  paragraph runs right to left and aligns right, its full stop and its "ב-2013" in
 *  order, while the lines around it stay left to right. Only a line holding a
 *  right-to-left letter gets `dir="auto"` (a Chinese or English line is untouched), and
 *  perLineTextDirection makes the cursor motion read each line's own direction. */
const perLineDirection: Extension = [
  EditorView.perLineTextDirection.of(true),
  ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      constructor(view: EditorView) {
        this.decorations = bidiDecorations(view);
      }
      update(u: ViewUpdate) {
        if (u.docChanged || u.viewportChanged) this.decorations = bidiDecorations(u.view);
      }
    },
    { decorations: (v) => v.decorations },
  ),
];

// The heading scale lives entirely in app.css's .cm-md-h1..h5 classes. A TagStyle's
// class and its style properties are mutually exclusive: once a class is set, the
// fontSize and friends of the same rule are silently dropped (in
// @codemirror/language, `style.class || def(...)`) — the old 1.35em never took effect,
// and "a heading only looks bold" was rooted in writing both into one rule. Do not put
// fontSize back here
const mdHighlight = HighlightStyle.define([
  { tag: tags.heading1, class: "cm-md-heading cm-md-h1" },
  { tag: tags.heading2, class: "cm-md-heading cm-md-h2" },
  { tag: tags.heading3, class: "cm-md-heading cm-md-h3" },
  { tag: tags.heading4, class: "cm-md-heading cm-md-h4" },
  { tag: tags.heading5, class: "cm-md-heading cm-md-h5" },
  { tag: tags.heading, class: "cm-md-heading" }, // the h6 fallback: 1em, bold only
  { tag: tags.monospace, class: "cm-md-mono" },
  // A quote's text gets no class: it reads in the body ink, and a callout's head in its
  // group's colour. A muted class on every span of the quote outranked the line's colour
  // and turned every callout head and every clipped paragraph grey. Only the syntax marks
  // (`>`, `#`, `**`, a list marker in source) step back, as cm-md-meta.
  // Emphasis is a class so app.css can draw it per writing system: italic in Latin, the
  // emphasis marks of the language in CJK (livepreview marks a span holding CJK text).
  { tag: tags.emphasis, class: "cm-md-em" },
  { tag: tags.strong, fontWeight: "600" },
  { tag: [tags.url, tags.link], class: "cm-wikilink" },
  { tag: tags.strikethrough, class: "cm-lp-strike" },
  { tag: tags.processingInstruction, class: "cm-md-meta" },
  { tag: highlightTag, class: "cm-md-mark" },
]);

// class-based so app.css can theme it for light/dark
const codeHighlight = HighlightStyle.define([
  { tag: tags.keyword, class: "cm-code-keyword" },
  { tag: [tags.string, tags.special(tags.string), tags.regexp], class: "cm-code-string" },
  { tag: [tags.comment, tags.meta], class: "cm-code-comment" },
  { tag: [tags.number, tags.bool, tags.atom, tags.null], class: "cm-code-atom" },
  { tag: [tags.typeName, tags.className, tags.namespace], class: "cm-code-type" },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName)], class: "cm-code-func" },
  { tag: [tags.propertyName, tags.attributeName, tags.labelName], class: "cm-code-prop" },
]);

/** Language support for a non-note file, by filename (null: plain text). */
export async function languageFor(path: string): Promise<Extension | null> {
  const desc = LanguageDescription.matchFilename(languages, path.split("/").pop() ?? path);
  if (!desc) return null;
  try {
    return await desc.load();
  } catch {
    return null;
  }
}

export interface EditorOptions {
  parent: HTMLElement;
  doc: string;
  /** note mode (wikilinks, markdown) when set; plain/code file mode when null */
  host: WikilinkHost | null;
  /** code-file language support (ignored in note mode) */
  language?: Extension | null;
  /** note mode: a rich-text paste (the clipboard carries text/html) → Markdown to insert;
   *  null keeps the native plain-text paste. Runs only when the clipboard is not file-like. */
  onPasteHtml?: (html: string, plain: string) => Promise<string | null>;
  /** note mode: turn a paste of files/images into markdown to insert at the
   *  cursor; resolve null when the clipboard held nothing file-like */
  onPasteMarkdown?: (files: File[]) => Promise<string | null>;
  /** note mode: click / ⌘/Ctrl+click on a `[label](target)` markdown link or
   *  bare URL; invert = "the other way of opening it" (a modified click, inverting the
   *  url_open funnel) */
  onOpenLink?: (target: string, invert?: boolean) => void;
  /** note mode: whether an ordinary click on a rendered link opens it directly (the
   *  live value of the click_opens_links setting, read at click time so a change takes
   *  effect immediately); off by default */
  linkSingleClick?: () => boolean;
  /** note mode: resolve an embedded image target to a displayable URL */
  loadAsset?: AssetLoader;
  /** note mode: copy an embedded vault image to the system clipboard */
  copyImage?: ImageCopier;
  /** A remote image that did not load: its placeholder opens the URL in the browser window. */
  openUrl?: (url: string) => void;
  /** note mode: a remote image not loaded yet — the note's Referer and the placeholder's note-wide choices */
  remoteImages?: LivePreviewOptions["remoteImages"];
  /** note mode: vault tag universe for `#` completion (kept fresh by caller) */
  tags?: () => string[];
  /** note mode: inline-field keys and values for `key::` completion (kept fresh by the caller) */
  fieldStats?: () => FieldStat[] | Promise<FieldStat[]>;
  /** note mode: the vault's enabled block types (the `/` menu's "types" section) */
  blockTypes?: () => BlockType[];
  /** note mode: a click on a chip (a task date, or any inline `key:: value` field) */
  onFieldChip?: FieldChipHandler;
  /** note mode: the step of a state field's value, for the chip's glyph (fields.ts's stateStepOf over the vault's types) */
  fieldState?: LivePreviewOptions["fieldState"];
  /** note mode: live `vemry-query` blocks (query + result-row navigation) */
  queryHost?: QueryHost | null;
  /** note mode: run button on `js` blocks (one-shot read-only sandbox) */
  runHost?: RunHost | null;
  /** note mode: the hover block handle and its menu, which also marks AI blocks reviewed (null: no handle) */
  blockHost?: (BlockHost & ProvenanceHost) | null;
  /** note mode: cross-note embeds `![[Note#^id]]` rendered read-only (null: plain links) */
  embedHost?: EmbedHost | null;
  onSave: (content: string) => void;
  onDirty: () => void;
  /** Cursor (selection head) change notification, shared by the outline panel
   *  highlighting the current section and the status line showing line and column
   *  (both 1-based; the column counts UTF-16 units). */
  onCursor?: (head: number, line: number, col: number) => void;
  /** Main selection range change notification (from===to means no selection); the snapshot source of `@ current selection`. */
  onSelection?: (from: number, to: number) => void;
}

/** Does this paste maybe carry files or images? Finder-copied files show their
 *  paths only on the native pasteboard (async IPC), but preventDefault must be
 *  decided synchronously — so the gate is broad ("Files"/uri-list/image types)
 *  and every caller falls back to inserting the clipboard's plain text when
 *  nothing file-like was actually there (e.g. a copied browser URL).
 *  Shared with the agent composer: one clipboard-type fix, both paste paths. */
export function isFileLikePaste(dt: DataTransfer | null): boolean {
  return (
    !!dt &&
    (dt.files.length > 0 ||
      dt.types.includes("Files") ||
      dt.types.includes("text/uri-list") ||
      Array.from(dt.items).some((i) => i.type.startsWith("image/")))
  );
}

/** Clipboard HTML that is code rather than prose: a `<pre>`, a preformatted white-space
 *  rule, or a monospace font — the shape code editors put on the clipboard (per-line div
 *  blocks with non-breaking-space indentation and a colour span per token). Converting
 *  that to Markdown yields NBSP-indented lines with a blank line between each; the plain
 *  text is the code itself, so the native paste is the right one. */
const CODE_HTML_RE = /<pre[\s>]|white-space:\s*pre|font-family:[^;"']*\b(?:mono|courier|consolas|menlo|monaco)/i;

function filePaste(handler?: (files: File[]) => Promise<string | null>, htmlHandler?: (html: string, plain: string) => Promise<string | null>): Extension {
  if (!handler && !htmlHandler) return [];
  return EditorView.domEventHandlers({
    paste: (event, view) => {
      const dt = event.clipboardData;
      if (!dt) return false;
      if (!isFileLikePaste(dt)) {
        // Rich text: the HTML fragment is only readable during dispatch, so it is taken here
        // and converted asynchronously. The decision to take over the paste has to be made
        // now, before the conversion: when nothing converts, the plain text is inserted the
        // way the native paste would have (the same user event, so undo and the input
        // handlers see a paste)
        const html = dt.getData("text/html");
        const plain = dt.getData("text/plain");
        if (!htmlHandler || !html || !plain.trim() || CODE_HTML_RE.test(html)) return false;
        event.preventDefault();
        void htmlHandler(html, plain).then((md) => {
          const insert = md ?? plain;
          if (!insert) return;
          view.dispatch(view.state.replaceSelection(insert), { userEvent: "input.paste", scrollIntoView: true });
        });
        return true;
      }
      if (!handler) return false;
      const text = dt.getData("text/plain"); // only readable during dispatch
      event.preventDefault();
      void handler(Array.from(dt.files)).then((md) => {
        const insert = md ?? text;
        if (!insert) return;
        // insert at the selection as of completion — the doc may have changed
        // while the attachment was being written
        const { from, to } = view.state.selection.main;
        view.dispatch({ changes: { from, to, insert }, selection: { anchor: from + insert.length } });
      });
      return true;
    },
  });
}

// Enter in a list: CodeMirror's continuation command, with lists kept tight. Its
// defaults loosen a list in two ways — Enter on an empty second item inserts a
// blank line above it instead of ending the list (the `nonTightLists` option), and
// once any two items are separated by a blank line, every further Enter inserts a
// blank line along with the marker. Under the seam rule (blockops.ts: an item packs
// against an item) that is one stray blank line spreading down the list with each
// keystroke. The command has no option for the second behaviour, so its transaction
// is rewritten: an insertion shaped "\n<blank line>\n<markers>" loses the blank
// line, wherever it sits among the transaction's changes (in an ordered list the
// command renumbers the following items in the same transaction; with several
// cursors there is one continuation per cursor). A blank line between items is
// still typed the same way as anywhere else — Enter on an empty item ends the list,
// Enter again opens the gap.
const continueMarkup = insertNewlineContinueMarkupCommand({ nonTightLists: false });
const LOOSE_CONTINUATION = /^\n[ \t>]*\n/;
const continueListTight: Command = (view) =>
  continueMarkup({
    state: view.state,
    dispatch: (tr) => {
      const changes: { from: number; to: number; insert: string }[] = [];
      const cursorsAt: number[] = [];
      tr.changes.iterChanges((from, to, _f, _t, inserted) => {
        const text = inserted.toString();
        const m = LOOSE_CONTINUATION.exec(text);
        if (m) cursorsAt.push(changes.length);
        changes.push({ from, to, insert: m ? "\n" + text.slice(m[0].length) : text });
      });
      if (cursorsAt.length === 0) {
        view.dispatch(tr);
        return;
      }
      const set = view.state.changes(changes);
      const selection = EditorSelection.create(cursorsAt.map((i) => EditorSelection.cursor(set.mapPos(changes[i].to, 1))));
      view.dispatch({ changes: set, selection, scrollIntoView: true, userEvent: "input" });
    },
  });
const listKeymap = Prec.high(
  keymap.of([
    { key: "Enter", run: continueListTight },
    // a task attribute without a value goes as one unit, before the list markup rule sees the key
    { key: "Backspace", run: deleteTaskMarkerBackward },
    { key: "Backspace", run: deleteMarkupBackward },
  ]),
);

export function createEditor(opts: EditorOptions): EditorView {
  const view = new EditorView({ parent: opts.parent, state: buildState(opts) });
  return view;
}

export function resetDoc(view: EditorView, opts: EditorOptions) {
  view.setState(buildState(opts));
}

function buildState(opts: EditorOptions): EditorState {
  const mode: Extension[] = [];
  if (opts.host) {
    const host = opts.host;
    const singleClick = opts.linkSingleClick ?? (() => false);
    const wikilink = wikilinkExtension(host, singleClick);
    mode.push(
      noteFont,
      placeholder(t("editor.placeholder")),
      markdown({ base: markdownLanguage, extensions: [Highlight, TaskStates], addKeymap: false }),
      listKeymap,
      syntaxHighlighting(mdHighlight),
      livePreview({
        loadAsset: opts.loadAsset,
        copyImage: opts.copyImage,
        onFieldChip: opts.onFieldChip,
        fieldState: opts.fieldState,
        openUrl: opts.openUrl,
        remoteImages: opts.remoteImages,
      }),
      allowPastedRemoteImages,
      foldExtension(),
      blockSelection(),
      flashField,
      imeCompletionGuard(),
      autocompletion({
        override: [
          refCompletions(host),
          wikilink.completions,
          atCompletions(host.targets, opts.onFieldChip),
          slashCompletions(opts.blockTypes ?? (() => [])),
          fieldCompletions(opts.fieldStats ?? (() => []), opts.blockTypes ?? (() => []), opts.onFieldChip),
          tagCompletions(opts.tags ?? (() => [])),
          calloutCompletions,
        ],
        icons: false,
        // the `@` palette's task attributes carry their drawn icon
        addToOptions: [attrIconOption],
        // a picked `@note` chains into the heading/block stage
        activateOnCompletion: (c) => c.type === "chain",
      }),
      wikilink.extension,
      // Tab accepts too (before the base keymap's indentWithTab, which it falls through to)
      keymap.of([...completionKeymap, { key: "Tab", run: acceptWithTab }]),
      // Open the link at the cursor: in the raw state (while editing) a mouse click
      // means editing, and this is the keyboard exit.
      // With no link it falls through to defaultKeymap's Mod-Enter (insertBlankLine)
      keymap.of([
        {
          key: "Mod-Enter",
          run: (v) => {
            const head = v.state.selection.main.head;
            // an embed line opens its source
            if (opts.embedHost) {
              const em = embedAt(v.state, head);
              if (em) {
                void openEmbedSource(opts.embedHost, em.inner);
                return true;
              }
            }
            const found = linkAt(v.state, head);
            if (!found) return false;
            if (found.flavor === "wiki") {
              host.follow(found.hit.target);
              return true;
            }
            if (!opts.onOpenLink) return false;
            opts.onOpenLink(found.hit.target, false);
            return true;
          },
        },
      ]),
      filePaste(opts.onPasteMarkdown, opts.onPasteHtml),
    );
    if (opts.onOpenLink) mode.push(mdlinkExtension(opts.onOpenLink, singleClick));
    if (opts.queryHost) mode.push(queryBlockExtension(opts.queryHost));
    if (opts.runHost) mode.push(runBlockExtension(opts.runHost));
    if (opts.blockHost) mode.push(blockHandle(opts.blockHost), dragSourceField);
    if (opts.embedHost) mode.push(embedExtension(opts.embedHost));
    if (opts.blockHost) mode.push(provenanceExtension(opts.blockHost));
  } else {
    if (opts.language) mode.push(opts.language);
    mode.push(codeFont, syntaxHighlighting(codeHighlight));
  }
  return EditorState.create({
    doc: opts.doc,
    extensions: [
      history(),
      drawSelection(),
      clickGuard(),
      highlightSpecialChars({ specialChars: SPECIAL_CHARS }),
      chromeTheme,
      darkMode.of(darkFacet()),
      followScheme,
      perLineDirection,
      scrollingThumb,
      EditorView.lineWrapping,
      closeBrackets(),
      ...mode,
      keymap.of([
        {
          key: "Mod-s",
          run: (v) => {
            opts.onSave(v.state.doc.toString());
            return true;
          },
        },
        ...closeBracketsKeymap, // Backspace deletes a bracket pair ([[]] goes in one press)
        ...defaultKeymap,
        ...historyKeymap,
        indentWithTab,
      ]),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) opts.onDirty();
        if (update.selectionSet || update.docChanged) {
          const main = update.state.selection.main;
          const ln = update.state.doc.lineAt(main.head);
          opts.onCursor?.(main.head, ln.number, main.head - ln.from + 1);
          opts.onSelection?.(main.from, main.to);
        }
      }),
      emptyViewportGuard,
    ],
  });
}

/** A viewport that is empty while the document is not: the height map shrank under a
 *  scroll offset at the note's end (a block widget rebuilt above it measured shorter than
 *  before — an embed whose figure had not loaded), the browser clamped the offset past
 *  the document, and with nothing rendered nothing gets measured again — the note stays
 *  blank until the reader scrolls. Pull the view back onto the document's end: the last
 *  lines render, and whatever re-enters the viewport is measured on the way. */
const emptyViewportGuard = EditorView.updateListener.of((update) => {
  if (!update.viewportChanged) return;
  const v = update.view;
  if (v.viewport.from !== v.viewport.to || v.state.doc.length === 0) return;
  requestAnimationFrame(() => {
    if (v.viewport.from !== v.viewport.to) return;
    // the scroller's own numbers: documentTop is a screen coordinate (the content box's
    // top, which is negative once scrolled) and does not belong in a scrollTop
    const el = v.scrollDOM;
    el.scrollTop = Math.max(0, el.scrollHeight - el.clientHeight);
  });
});
