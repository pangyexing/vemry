// CodeMirror assembly for the note editor and the plain/code file editor.
import { t } from "$lib/i18n/i18n.svelte";
import { EditorState, type Extension } from "@codemirror/state";
import { EditorView, keymap, drawSelection, highlightSpecialChars, placeholder } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { markdown, markdownLanguage } from "@codemirror/lang-markdown";
import { syntaxHighlighting, HighlightStyle, LanguageDescription } from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { autocompletion, completionKeymap, closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { tags } from "@lezer/highlight";

import { wikilinkExtension, type WikilinkHost } from "./wikilink";
import { mdlinkExtension } from "./mdlink";
import { linkAt } from "./linkat";
import { livePreview, type AssetLoader, type ImageCopier } from "./livepreview";
import { tagCompletions, calloutCompletions, atCompletions } from "./completions";
import { refCompletions } from "./blockref";
import { slashCompletions } from "./slash";
import { embedAt, embedExtension, openEmbedSource, type EmbedHost } from "./embed";
import { provenanceExtension } from "./provenance";
import { Highlight, highlightTag } from "./markdown-ext";
import { queryBlockExtension, type QueryHost } from "./queryblock";
import { runBlockExtension, type RunHost } from "./runblock";
import { blockSelection } from "./blocksel";
import { foldExtension } from "./fold";
import { blockHandle, type BlockHost } from "./blockhandle";
import { dragSourceField } from "./blockdrag";

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
  { tag: tags.quote, class: "cm-md-quote" },
  { tag: tags.emphasis, fontStyle: "italic" },
  { tag: tags.strong, fontWeight: "600" },
  { tag: [tags.url, tags.link], class: "cm-wikilink" },
  { tag: tags.strikethrough, class: "cm-lp-strike" },
  { tag: tags.processingInstruction, class: "cm-md-quote" },
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
  /** note mode: vault tag universe for `#` completion (kept fresh by caller) */
  tags?: () => string[];
  /** note mode: live `vemry-query` blocks (query + result-row navigation) */
  queryHost?: QueryHost | null;
  /** note mode: run button on `js` blocks (one-shot read-only sandbox) */
  runHost?: RunHost | null;
  /** note mode: the hover block handle and its menu (null: no handle) */
  blockHost?: BlockHost | null;
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

function filePaste(handler?: (files: File[]) => Promise<string | null>): Extension {
  if (!handler) return [];
  return EditorView.domEventHandlers({
    paste: (event, view) => {
      const dt = event.clipboardData;
      if (!dt || !isFileLikePaste(dt)) return false;
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
      markdown({ base: markdownLanguage, extensions: [Highlight] }),
      syntaxHighlighting(mdHighlight),
      livePreview({ loadAsset: opts.loadAsset, copyImage: opts.copyImage }),
      foldExtension(),
      blockSelection(),
      autocompletion({
        override: [
          refCompletions(host),
          wikilink.completions,
          atCompletions(host.targets),
          slashCompletions(),
          tagCompletions(opts.tags ?? (() => [])),
          calloutCompletions,
        ],
        icons: false,
        // a picked `@note` chains into the heading/block stage
        activateOnCompletion: (c) => c.type === "chain",
      }),
      wikilink.extension,
      keymap.of(completionKeymap),
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
      filePaste(opts.onPasteMarkdown),
    );
    if (opts.onOpenLink) mode.push(mdlinkExtension(opts.onOpenLink, singleClick));
    if (opts.queryHost) mode.push(queryBlockExtension(opts.queryHost));
    if (opts.runHost) mode.push(runBlockExtension(opts.runHost));
    if (opts.blockHost) mode.push(blockHandle(opts.blockHost), dragSourceField);
    if (opts.embedHost) mode.push(embedExtension(opts.embedHost));
    if (opts.blockHost) mode.push(provenanceExtension());
  } else {
    if (opts.language) mode.push(opts.language);
    mode.push(codeFont, syntaxHighlighting(codeHighlight));
  }
  return EditorState.create({
    doc: opts.doc,
    extensions: [
      history(),
      drawSelection(),
      highlightSpecialChars(),
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
    ],
  });
}
