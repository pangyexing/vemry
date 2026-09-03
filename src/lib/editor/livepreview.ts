// Live preview for notes: on lines the cursor is not on, the Markdown syntax markers
// are hidden — a heading's #, the */_ of bold and italic, ==highlight==, the
// backticks of inline code, the brackets and alias prefix of a [[wikilink]], the
// target part of [label](url), a quote's >, a code block's fences; a task item
// renders as a clickable checkbox, a horizontal rule as a solid line, and the YAML
// frontmatter at the head of the document folds into a property strip;
// ![](path) and ![[img.png]] render inline as images, a > [!type] quote renders as a
// callout, and a #tag renders as a chip. Tables live in table.ts.
// These are view-layer decorations alone and never modify the document; an element
// the cursor or selection touches shows its source (block-level markers such as
// headings and quotes reveal by whole line).
import { t } from "$lib/i18n/i18n.svelte";
import { privacyGlyphSvg } from "$lib/privacyGlyph";
import { CALLOUT_RE, frontmatterEnd } from "./blocks";
import { syntaxTree } from "@codemirror/language";
import {
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import { type EditorState, type Extension, RangeSetBuilder, StateField } from "@codemirror/state";
import { contentStart, fencedCodeAt, fencedCodeText } from "./fenced";
import { revealArmed, revealRanges, selectionReveals, selectionTouches } from "./reveal";
import { tableField } from "./table";
import { LINK_RE as WIKI_RE } from "./wikilink";
import { DATE_FIELDS, dateFieldLabel, type DateField, dateIconSvg, localDate, scanTaskDates, STAMP_RE, TASK_LINE_RE } from "$lib/views/tasks";
import { scanFields } from "$lib/fields";
import { privateRangeAt } from "./privateblock";
import { blockRoot } from "./blockwidget";
import { openImageLightbox } from "$lib/lightbox";
import { copyText } from "$lib/copytext";

/** An image target inside the vault → a displayable URL (a blob); http(s) does not go through it */
export type AssetLoader = (target: string) => Promise<string | null>;

/** An image target inside the vault → written to the system clipboard (as a bitmap),
 *  with success or failure fed back to the button; not offered for an http(s) image
 *  (the editor makes no network request just to copy) */
export type ImageCopier = (target: string) => Promise<boolean>;

/** Whether pos falls inside code (inline code, a fence, an indented code block).
 *  mdlink's ⌘click uses this same one, so "what counts as code" has a single answer. */
export function inCode(state: EditorState, pos: number): boolean {
  const tree = syntaxTree(state);
  for (let n: ReturnType<typeof tree.resolveInner> | null = tree.resolveInner(pos, 1); n; n = n.parent)
    if (n.name === "InlineCode" || n.name === "FencedCode" || n.name === "CodeBlock") return true;
  return false;
}
const TAG_RE = /(^|\s)#([\p{L}\p{N}_][\p{L}\p{N}_/-]*)/gu;
// The character class mirrors Rust's block-id rule (blockid.rs::is_valid_block_id): a badge must never claim an id the parser rejects
const ANCHOR_RE = /[ \t]\^([A-Za-z0-9-]+)$/gm;
export const IMAGE_EXT_RE = /\.(png|jpe?g|gif|webp|svg|bmp|ico|avif|tiff?)$/i;
// CALLOUT_RE lives in blocks.ts (the block model, fold.ts and this renderer share it)

const hideDeco = Decoration.replace({});
const bulletDeco = Decoration.mark({ class: "cm-lp-bullet" });
const doneDeco = Decoration.mark({ class: "cm-lp-done" });
const tagDeco = Decoration.mark({ class: "cm-lp-tag" });
const anchorDeco = Decoration.mark({ class: "cm-lp-anchor" });
const badDateDeco = Decoration.mark({
  class: "cm-lp-baddate",
  attributes: { title: t("editor.badDate") },
});
const stampDeco = Decoration.mark({ class: "cm-lp-stamp" });
const quoteLine = Decoration.line({ class: "cm-lp-quote" });
const codeLine = Decoration.line({ class: "cm-lp-code" });
const codeHeadLine = Decoration.line({ class: "cm-lp-code-head" });
const tableSrcLine = Decoration.line({ class: "cm-lp-tablesrc" });
/** The vertical rhythm of a heading line (the space above; the values live in app.css's .cm-lp-h1..h6); the index is the level minus 1 */
const headingLines = [1, 2, 3, 4, 5, 6].map((n) => Decoration.line({ class: `cm-lp-h${n}` }));

// callout type → colour and icon group (the common alias table plus vemry's
// claim/evidence)
// exported: the completion source uses it as the list of callout type candidates
import { CALLOUT_GROUP } from "$lib/callout";

// lucide-style stroke icons (a 24 viewBox)
const CALLOUT_ICONS: Record<string, string> = {
  info: '<circle cx="12" cy="12" r="10"/><path d="M12 16v-4M12 8h.01"/>',
  tip: '<path d="M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z"/>',
  success: '<path d="M20 6 9 17l-5-5"/>',
  question: '<circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3M12 17h.01"/>',
  warn: '<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 20h16a2 2 0 0 0 1.73-2ZM12 9v4M12 17h.01"/>',
  danger: '<path d="M13 2 3 14h9l-1 8 10-12h-9l1-8z"/>',
  example: '<path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"/>',
  quote: '<path d="M17 6H3M21 12H8M21 18H8"/>',
  claim: '<path d="M4 15s1-1 4-1 5 2 8 2 4-1 4-1V3s-1 1-4 1-5-2-8-2-4 1-4 1zM4 22v-7"/>',
  evidence:
    '<path d="m16 16 3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1ZM2 16l3-8 3 8c-.87.65-1.92 1-3 1s-2.13-.35-3-1ZM7 21h10M12 3v18M3 7h2c2 0 5-1 7-2 2 1 5 2 7 2h2"/>',
};

interface Entry {
  from: number;
  to: number;
  deco: Decoration;
}

/** The `- [ ]` checkbox; a click rewrites the checked state in the text underneath */
class CheckboxWidget extends WidgetType {
  constructor(readonly checked: boolean) {
    super();
  }
  eq(other: CheckboxWidget) {
    return other.checked === this.checked;
  }
  toDOM(view: EditorView) {
    const box = document.createElement("input");
    box.type = "checkbox";
    box.className = "cm-lp-checkbox";
    box.checked = this.checked;
    box.onmousedown = (e) => {
      // Handled on mousedown: preventDefault keeps the editor's selection, or the
      // cursor landing inside the checkbox's range would trigger reveal and replace
      // this widget before the click arrived
      e.preventDefault();
      e.stopPropagation();
      toggleTaskAt(view, view.posAtDOM(box));
    };
    return box;
  }
}

/** The icon the `[!type]` marker is replaced with; with an empty title the type name stands in as one */
class CalloutIconWidget extends WidgetType {
  constructor(readonly group: string, readonly label: string) {
    super();
  }
  eq(other: CalloutIconWidget) {
    return other.group === this.group && other.label === this.label;
  }
  toDOM() {
    const el = document.createElement("span");
    el.className = "cm-lp-co-icon";
    if (this.group === "private") {
      // the same struck-out sparkle as every other privacy badge (privacyGlyph.ts)
      el.innerHTML = privacyGlyphSvg(15);
      el.title = t("editor.privateBlockHint");
    } else {
      el.innerHTML = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">${CALLOUT_ICONS[this.group] ?? CALLOUT_ICONS.info}</svg>`;
    }
    if (this.label) el.append(this.label);
    return el;
  }
}

/** A plain-text replacement badge: the friendly display of a wikilink's suffix (› / ¶) and of the supports: prefix */
class TextBadgeWidget extends WidgetType {
  constructor(
    readonly text: string,
    readonly cls: string,
  ) {
    super();
  }
  eq(other: TextBadgeWidget) {
    return other.text === this.text && other.cls === this.cls;
  }
  toDOM() {
    const el = document.createElement("span");
    el.className = this.cls;
    el.textContent = this.text;
    return el;
  }
}

// The glyphs of -/*/+ drift with the platform's fonts (on Windows the * is small and
// high and reads as unformatted), while a round dot is the stable "formatted" shape;
// an ordered number keeps its source
const bulletDotDeco = Decoration.replace({ widget: new TextBadgeWidget("•", "cm-lp-bulletdot") });

/** The task date chip: a valid 📅/⏳/due::/scheduled:: renders as a small capsule
 *  looking exactly like Agenda's, and an overdue (and unfinished) one is coloured
 *  danger; a click lands beside the chip, and the source can be edited once the line
 *  reveals */
class DateChipWidget extends WidgetType {
  constructor(
    readonly kind: DateField,
    readonly date: string,
    readonly overdue: boolean,
    readonly onChip?: FieldChipHandler,
  ) {
    super();
  }
  eq(other: DateChipWidget) {
    return other.kind === this.kind && other.date === this.date && other.overdue === this.overdue;
  }
  toDOM(view: EditorView) {
    const el = document.createElement("span");
    el.className = `cm-lp-datechip${this.onChip ? " cm-lp-chip-click" : ""}${this.overdue ? " cm-lp-datechip-overdue" : ""}`;
    el.title = dateFieldLabel(this.kind);
    el.innerHTML = dateIconSvg(this.kind);
    el.append(this.date);
    wireChipClick(el, view, this.onChip, this.kind, this.date);
    return el;
  }
}

/** A chip's click: the host opens the menu for the field at that position (`pos` is
 *  where the field's source starts; `value` is "" for an empty placeholder; `at` is
 *  where the menu anchors — the click, or the caret for a typed close). */
export type FieldChipHandler = (view: EditorView, pos: number, key: string, value: string, at: { x: number; y: number }) => void;

function wireChipClick(el: HTMLElement, view: EditorView, onChip: FieldChipHandler | undefined, key: string, value: string) {
  if (!onChip) return;
  el.addEventListener("mousedown", (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const pos = view.posAtDOM(el);
    onChip(view, pos, key, value, { x: e.clientX, y: e.clientY });
  });
}

/** Typing the `]` (or `)`) that closes an empty bracket field raises its menu at the
 *  caret at once, the way a click on the chip would: the user has just declined to
 *  type the value, and while the caret touches the field it is source, not a chip —
 *  without this the route is click away, click back. Only a typed close counts
 *  (input.type: an inserted bracket, or the type-over of an auto-closed one), and
 *  only with the caret coming from just inside; an arrow across the bracket, a paste
 *  or an undo do not. The menu is raised after the update settles — it is the host's,
 *  and it takes the focus. */
function emptyFieldCloser(onChip?: FieldChipHandler): Extension {
  if (!onChip) return [];
  const open = onChip;
  return EditorView.updateListener.of((update) => {
    if (!update.selectionSet || !update.transactions.some((tr) => tr.isUserEvent("input.type"))) return;
    const { state, startState } = update;
    const sel = state.selection.main;
    const prev = startState.selection.main;
    if (!sel.empty || !prev.empty || update.changes.mapPos(prev.head, -1) !== sel.head - 1) return;
    const head = sel.head;
    const closer = state.sliceDoc(head - 1, head);
    if (closer !== "]" && closer !== ")") return;
    if (head < frontmatterEnd(state.doc) || inCode(state, head)) return;
    const line = state.doc.lineAt(head);
    const mark = scanFields(line.text, line.from, { empty: true }).find(
      (f) => f.to === head && f.value === "" && (f.form === "bracket" || f.form === "paren"),
    );
    if (!mark) return;
    const view = update.view;
    setTimeout(() => {
      if (view.state.doc !== state.doc || view.state.selection.main.head !== head) return;
      const c = view.coordsAtPos(mark.from);
      if (!c) return;
      open(view, mark.from, mark.key, "", { x: c.left, y: c.bottom + 2 });
    }, 0);
  });
}

/** An inline `key:: value` field as a chip: the key muted, the value plain. A click
 *  hands it to the host, which raises the value menu (a date value gets the date
 *  menu, the same one the task chips use). An empty placeholder — the `[key:: ]` a
 *  block type leaves behind — draws a faint "empty" slot so it can be filled by a
 *  click. The source is shown once the cursor touches the chip's range. */
class FieldChipWidget extends WidgetType {
  constructor(
    readonly key: string,
    readonly value: string,
    readonly onChip?: FieldChipHandler,
  ) {
    super();
  }
  eq(other: FieldChipWidget) {
    return other.key === this.key && other.value === this.value;
  }
  toDOM(view: EditorView) {
    const el = document.createElement("span");
    const empty = this.value === "";
    el.className = `cm-lp-field${this.onChip ? " cm-lp-chip-click" : ""}${empty ? " cm-lp-field-empty" : ""}`;
    const k = document.createElement("span");
    k.className = "cm-lp-field-key";
    k.textContent = this.key;
    const v = document.createElement("span");
    v.className = "cm-lp-field-val";
    v.textContent = empty ? t("field.empty") : this.value;
    el.append(k, v);
    el.title = empty ? t("field.fillHint") : `${this.key}:: ${this.value}`;
    wireChipClick(el, view, this.onChip, this.key, this.value);
    return el;
  }
}

const fieldSrcDeco = Decoration.mark({ class: "cm-lp-field-src" });

/* The copy button is always an icon (stacked squares), with the words demoted to a
   hover tip (data-tip → CSS ::after); after a click it briefly swaps to a result icon
   and wording, restored after 1.2s */
const ICON_COPY =
  '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="9" y="9" width="13" height="13" rx="2"/><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1"/></svg>';
const ICON_OK =
  '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>';
const ICON_FAIL =
  '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>';
const ICON_ZOOM =
  '<svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/><line x1="11" y1="8" x2="11" y2="14"/><line x1="8" y1="11" x2="14" y2="11"/></svg>';

function copyIconButton(tip: string): { btn: HTMLButtonElement; flash: (ok: boolean) => void } {
  const btn = document.createElement("button");
  btn.className = "cm-lp-copy";
  btn.dataset.tip = tip;
  btn.innerHTML = ICON_COPY;
  let timer = 0;
  const flash = (ok: boolean) => {
    btn.innerHTML = ok ? ICON_OK : ICON_FAIL;
    btn.dataset.tip = t(ok ? "editor.copied" : "editor.copyFailed");
    btn.classList.toggle("cm-lp-copy-ok", ok);
    btn.classList.toggle("cm-lp-copy-fail", !ok);
    clearTimeout(timer);
    timer = window.setTimeout(() => {
      btn.innerHTML = ICON_COPY;
      btn.dataset.tip = tip;
      btn.classList.remove("cm-lp-copy-ok", "cm-lp-copy-fail");
    }, 1200);
  };
  return { btn, flash };
}

/** A cache of intrinsic image sizes, keyed by target (a blob URL changes every time;
 *  only the target is stable).
 *  CM does its virtual scrolling from measured heights: a widget mounts at height 0,
 *  the images load one after another and push it open, the total height above grows,
 *  and the scroll position is pushed back down (in an image-heavy document, scrolling
 *  down snaps back to the top). A cache hit fixes the aspect ratio through the width
 *  and height attributes, so the line height is right before the load; an image seen
 *  for the first time falls back to a CSS placeholder box (.cm-lp-loading) for an
 *  approximate height, narrowing the jump to one small correction. */
const imageDims = new Map<string, { w: number; h: number }>();

/** Invalidated when the vault changes (called by the editor's vault-switch effect,
 *  the same trap as assetUrls): the same relative path would take the previous
 *  vault's size as its placeholder, and the further the aspect ratios differ the
 *  bigger the jump after loading; without clearing it, it would also grow without
 *  bound for the life of the process. */
export function clearImageDims() {
  imageDims.clear();
}

/** An inline image; a vault file becomes a blob URL through loadAsset, and http(s) is used directly */
class ImageWidget extends WidgetType {
  constructor(
    readonly target: string,
    readonly alt: string,
    readonly width: number,
    readonly load?: AssetLoader,
    readonly copy?: ImageCopier,
  ) {
    super();
  }
  eq(other: ImageWidget) {
    return other.target === this.target && other.alt === this.alt && other.width === this.width;
  }
  toDOM(view: EditorView) {
    const wrap = document.createElement("span");
    wrap.className = "cm-lp-image";
    const img = document.createElement("img");
    img.alt = this.alt;
    if (this.width > 0) img.style.width = `${this.width}px`;
    const dims = imageDims.get(this.target);
    if (dims) {
      // Attributes rather than style: CSS's height:auto lets it return to the real
      // intrinsic ratio after loading, and the attributes only hold the aspect-ratio
      // placeholder before it (a built-in browser behaviour)
      img.width = dims.w;
      img.height = dims.h;
    } else {
      img.classList.add("cm-lp-loading");
    }
    // Clicking an image in the rendered state opens the lightbox, with or without a
    // modifier (there is no longer a "⌘ opens the file itself" branch — that would
    // jump out of the current note). The mousedown has to be swallowed: without that,
    // the cursor would land in the source text and trigger reveal, and once the widget
    // is destroyed the click would never happen at all. The price is the same as for a
    // folded link: a drag-selection cannot start on the image. stopPropagation: do not
    // let clickopen work the coordinates back out a second time
    wrap.onmousedown = (e) => {
      if (e.button !== 0 || e.target !== img) return;
      e.preventDefault();
      e.stopPropagation();
    };
    // click rather than mousedown: when the mouseup happens outside the image the
    // click lands on the common ancestor and the target is no longer the img — dragging
    // away cancels naturally, the same button semantics as clickopen
    wrap.onclick = (e) => {
      if (e.target !== img || !img.src) return;
      e.preventDefault();
      e.stopPropagation();
      openImageLightbox(img.src, this.alt);
    };
    const missing = () => {
      // Assigning textContent would clear the copy button along with it — with no image at all, the button should not be there
      img.remove();
      wrap.classList.add("cm-lp-image-missing");
      wrap.textContent = this.alt || this.target;
    };
    // The size changes as it loads, so the editor is asked to measure again and the
    // line height keeps up; the intrinsic size is recorded in passing, so the next
    // mount (an edit rebuilding it, a scroll repaint) already holds the right height
    // before loading. Should the file have been replaced, the old cache holds the wrong
    // size, and it converges on the real value and corrects the attributes
    img.onload = () => {
      img.classList.remove("cm-lp-loading");
      const w = img.naturalWidth;
      const h = img.naturalHeight;
      if (w > 0 && h > 0) {
        imageDims.set(this.target, { w, h });
        img.width = w;
        img.height = h;
      }
      view.requestMeasure();
    };
    img.onerror = missing;
    wrap.appendChild(img);
    const remote = /^https?:\/\//i.test(this.target);
    const copy = remote ? undefined : this.copy;
    if (copy) {
      const { btn, flash } = copyIconButton(t("editor.copyImage"));
      btn.classList.add("cm-lp-imgbtn");
      btn.onmousedown = (e) => {
        e.preventDefault();
        e.stopPropagation();
        void copy(this.target).then(flash);
      };
      wrap.appendChild(btn);
    }
    // The zoom button: on the same row as the copy button (appearing on hover); a remote image has no copy button and it takes the top-right corner back
    const zoom = document.createElement("button");
    zoom.className = "cm-lp-copy cm-lp-imgbtn cm-lp-imgzoom";
    if (copy) zoom.classList.add("cm-lp-imgzoom-shift");
    zoom.dataset.tip = t("editor.zoom");
    zoom.innerHTML = ICON_ZOOM;
    zoom.onmousedown = (e) => {
      e.preventDefault();
      e.stopPropagation();
      if (img.src) openImageLightbox(img.src, this.alt);
    };
    wrap.appendChild(zoom);
    const url = remote ? Promise.resolve(this.target) : (this.load?.(this.target) ?? Promise.resolve(null));
    void url.then((u) => (u ? (img.src = u) : missing()));
    return wrap;
  }
}

/** A trailing `alt|300` segment is taken as the width when it is a positive number */
function parseWidth(raw: string): [string, number] {
  const pipe = raw.lastIndexOf("|");
  if (pipe >= 0) {
    const w = Number(raw.slice(pipe + 1));
    if (Number.isFinite(w) && w > 0) return [raw.slice(0, pipe), w];
  }
  return [raw, 0];
}

class HrWidget extends WidgetType {
  eq() {
    return true;
  }
  toDOM() {
    const el = document.createElement("span");
    el.className = "cm-lp-hr";
    return el;
  }
}

/** The folded frontmatter property strip; a click moves the cursor into the block and expands the source */
class FrontmatterWidget extends WidgetType {
  constructor(readonly keys: string[]) {
    super();
  }
  eq(other: FrontmatterWidget) {
    return other.keys.join("\n") === this.keys.join("\n");
  }
  get estimatedHeight() {
    return 34;
  }
  toDOM(view: EditorView) {
    const el = document.createElement("div");
    el.className = "cm-lp-frontmatter";
    const label = document.createElement("span");
    label.className = "cm-lp-fm-label";
    label.textContent = t("editor.properties");
    const keys = document.createElement("span");
    keys.className = "cm-lp-fm-keys";
    keys.textContent = this.keys.join(" · ");
    el.append(label, keys);
    el.onmousedown = (e) => {
      e.preventDefault();
      const doc = view.state.doc;
      const anchor = doc.lines >= 2 && doc.line(1).text === "---" ? doc.line(2).from : 0;
      view.dispatch({ selection: { anchor } });
      view.focus();
    };
    return blockRoot(el);
  }
}

/** A fenced code block's content (without the fence lines or the quote markers); pos is the button's anchor (the start of the block's first line) */
function codeBlockAt(view: EditorView, pos: number): string | null {
  const fence = fencedCodeAt(view.state, pos);
  return fence ? fencedCodeText(view.state.doc, fence) : null;
}

/** The one-click copy in a code block's top-right corner; the content is taken from the syntax tree at click time and is not rebuilt as the document changes */
class CopyWidget extends WidgetType {
  eq() {
    return true;
  }
  toDOM(view: EditorView) {
    const { btn, flash } = copyIconButton(t("editor.copyCode"));
    btn.onmousedown = (e) => {
      e.preventDefault();
      e.stopPropagation();
      const code = codeBlockAt(view, view.posAtDOM(btn));
      if (code == null) return;
      void copyText(code).then(flash);
    };
    return btn;
  }
}

// side -1: with the fence line hidden by a replace, the button is only rendered when it sits before the hidden range
const copyDeco = Decoration.widget({ widget: new CopyWidget(), side: -1 });

function toggleTaskAt(view: EditorView, pos: number) {
  const line = view.state.doc.lineAt(pos);
  const m = /^(?:(?:[-*+]|\d+[.)])\s+)?\[( |x|X)\]/.exec(line.text.slice(pos - line.from));
  if (!m) return;
  const box = pos + m[0].length - 2; // the check character sits before the "?]" at the end of the match
  view.dispatch({ changes: { from: box, to: box + 1, insert: m[1] === " " ? "x" : " " } });
}

function buildDecorations(view: EditorView, opts: LivePreviewOptions = {}): DecorationSet {
  const state = view.state;
  const doc = state.doc;
  const tree = syntaxTree(state);
  const today = localDate(); // the overdue check; it refreshes with the next rebuild after midnight, the same nature as Agenda's

  // Two granularities: an inline element uses the element-level selectionReveals (the
  // one definition), while a block-level marker (a heading, a quote, an hr, a fence, a
  // callout) uses line ranges (revealRanges is precomputed to avoid evaluating it
  // repeatedly)
  const lineReveals = revealRanges(state);
  const revealedLine = (from: number, to: number) =>
    lineReveals.some(([a, b]) => from <= b && to >= a);
  const revealedEl = (from: number, to: number) => selectionReveals(state, from, to);

  const isCode = (pos: number) => inCode(state, pos);

  const hides: Entry[] = [];
  const marks: Entry[] = [];
  const seenLines = new Set<string>(); // nested nodes are entered more than once, so line decorations are deduplicated
  const hide = (from: number, to: number) => {
    if (to > from) hides.push({ from, to, deco: hideDeco });
  };
  const addLine = (pos: number, deco: Decoration, tag: string) => {
    const key = `${pos}/${tag}`;
    if (seenLines.has(key)) return;
    seenLines.add(key);
    marks.push({ from: pos, to: pos, deco });
  };
  /** Add a line decoration to every line in the intersection of [from,to] with the visible range */
  const decorateLines = (
    nodeFrom: number,
    nodeTo: number,
    rangeFrom: number,
    rangeTo: number,
    deco: Decoration,
    tag: string,
  ) => {
    const from = Math.max(nodeFrom, rangeFrom);
    const to = Math.min(nodeTo, rangeTo);
    for (let pos = doc.lineAt(from).from; pos <= to; ) {
      const line = doc.lineAt(pos);
      addLine(line.from, deco, tag);
      if (line.to >= to) break;
      pos = line.to + 1;
    }
  };

  for (const range of view.visibleRanges) {
    tree.iterate({
      from: range.from,
      to: range.to,
      enter: (node) => {
        switch (node.name) {
          case "ATXHeading1":
          case "ATXHeading2":
          case "ATXHeading3":
          case "ATXHeading4":
          case "ATXHeading5":
          case "ATXHeading6": {
            // A heading line only gets space above and none below: in the source there
            // is usually a blank line after a heading, and that line is the space below
            // — adding more would make the two equal, which is exactly the problem
            // being fixed.
            // The reveal state is not consulted — a cursor landing on a heading merely
            // exposes the #, and the line height should not jump with it; the
            // document's first line is not left hanging. A Setext heading (the
            // underlined kind) is not covered here
            const line = doc.lineAt(node.from);
            if (line.number > 1) addLine(line.from, headingLines[+node.name.slice(-1) - 1], "h");
            break;
          }
          case "HeaderMark": {
            // Only an ATX heading's # is handled (a Setext underline keeps its whole line)
            if (!node.node.parent?.name.startsWith("ATXHeading")) break;
            if (revealedLine(node.from, node.to)) break;
            let from = node.from;
            let to = node.to;
            if (doc.sliceString(to, to + 1) === " ") to += 1; // a leading "# "
            else if (doc.sliceString(from - 1, from) === " ") from -= 1; // a trailing " #"
            hide(from, to);
            break;
          }
          case "EmphasisMark":
          case "StrikethroughMark":
          case "HighlightMark": {
            // The verdict is taken over the parent node
            // (Emphasis/StrongEmphasis/Strikethrough/Highlight): a cursor in the middle
            // of bold text has to expose the markers on both sides
            const p = node.node.parent;
            if (!revealedEl(p?.from ?? node.from, p?.to ?? node.to)) hide(node.from, node.to);
            break;
          }
          case "CodeMark": {
            // The ``` of a fenced block is handled per line by the FencedCode branch
            const p = node.node.parent;
            if (p?.name !== "InlineCode") break;
            if (!revealedEl(p.from, p.to)) hide(node.from, node.to);
            break;
          }
          case "Link": {
            // Inline [label](url) only: the [ and the ](url) are hidden and the label
            // keeps the link styling; a reference or shortcut link has a different
            // structure and is shown as written
            const n = node.node;
            if (revealedEl(n.from, n.to)) break;
            // CommonMark allows a newline inside the label or the (url), but a
            // ViewPlugin's replace decoration spanning lines makes CodeMirror throw a
            // RangeError outright — a link across lines keeps its source
            if (doc.lineAt(n.from).number !== doc.lineAt(n.to).number) break;
            const linkMarks = n.getChildren("LinkMark");
            if (!n.getChild("URL") || linkMarks.length < 4) break;
            hide(linkMarks[0].from, linkMarks[0].to);
            hide(linkMarks[1].from, n.to);
            break;
          }
          case "Autolink": {
            if (revealedEl(node.from, node.to)) break;
            hide(node.from, node.from + 1); // <
            hide(node.to - 1, node.to); // >
            break;
          }
          case "Image": {
            // ![alt](target): a vault path goes by its extension, while http(s) is
            // always treated as an embedded image; a non-image target keeps its source
            const n = node.node;
            if (revealedEl(n.from, n.to)) break;
            // As with Link: a ![alt](…) across lines cannot use a ViewPlugin replace and keeps its source
            if (doc.lineAt(n.from).number !== doc.lineAt(n.to).number) break;
            const urlNode = n.getChild("URL");
            const linkMarks = n.getChildren("LinkMark");
            if (!urlNode || linkMarks.length < 2) break;
            let target = doc.sliceString(urlNode.from, urlNode.to);
            try {
              target = decodeURIComponent(target);
            } catch {
              /* not encoded by us, so it is kept as written */
            }
            if (!/^https?:\/\//i.test(target) && !IMAGE_EXT_RE.test(target)) break;
            const [alt, width] = parseWidth(doc.sliceString(linkMarks[0].to, linkMarks[1].from));
            hides.push({
              from: n.from,
              to: n.to,
              deco: Decoration.replace({
                widget: new ImageWidget(target, alt, width, opts.loadAsset, opts.copyImage),
              }),
            });
            break;
          }
          case "TaskMarker": {
            const item = node.node.parent?.parent; // Task → ListItem
            const listMark = item?.getChild("ListMark");
            const bulletFrom =
              listMark && /^[-*+]$/.test(doc.sliceString(listMark.from, listMark.to))
                ? listMark.from
                : node.from; // an ordered task only swaps the [ ] itself and keeps the number
            const checked = /[xX]/.test(doc.sliceString(node.from, node.to));
            const task = node.node.parent;
            if (checked && task && task.to > node.to + 1)
              marks.push({ from: node.to + 1, to: task.to, deco: doneDeco });
            // Only the task's **first line** is scanned: a Rust task block is a single
            // line (the parser recognises tasks line by line), and a date marker on a
            // lazy continuation line is something the parser never reads, so drawing a
            // chip for it would display a date that does not exist in the index
            const lineEnd = doc.lineAt(node.from).to;
            const rest = doc.sliceString(node.to, task ? Math.min(task.to, lineEnd) : lineEnd);
            for (const dm of scanTaskDates(rest)) {
              const dFrom = node.to + dm.start;
              const dTo = node.to + dm.end;
              if (dm.date === null) {
                // A malformed date (starting with a digit, or an explicit attr::): the
                // parser ignores it silently and the task vanishes from Agenda with
                // nothing to see — so a warning is drawn (on the cursor's line too: a
                // typo should be visible the moment it is made). A decorative emoji (⏳
                // and the like) draws no warning: the parser is just as silent about it,
                // and a warning would only be a false alarm
                if (dm.attempted) marks.push({ from: dFrom, to: dTo, deco: badDateDeco });
                continue;
              }
              // Only the winner the parser actually read renders a chip (looking exactly
              // like Agenda's); a valid date that is not the winner is ordinary text, and
              // a chip there would misreport the index. The source is only shown for
              // editing when the cursor touches the chip's range (clicking a chip folds
              // to its boundary)
              if (!dm.active || revealedEl(dFrom, dTo)) continue;
              hides.push({
                from: dFrom,
                to: dTo,
                deco: Decoration.replace({
                  widget: new DateChipWidget(dm.field, dm.date, dm.field === "due" && !checked && dm.date < today, opts.onFieldChip),
                }),
              });
            }
            if (revealedEl(bulletFrom, node.to)) break;
            hides.push({
              from: bulletFrom,
              to: node.to,
              deco: Decoration.replace({ widget: new CheckboxWidget(checked) }),
            });
            break;
          }
          case "ListMark": {
            if (node.node.parent?.getChild("Task")) break; // a task item belongs to TaskMarker
            marks.push({ from: node.from, to: node.to, deco: bulletDeco });
            if (!revealedEl(node.from, node.to) && /^[-*+]$/.test(doc.sliceString(node.from, node.to)))
              hides.push({ from: node.from, to: node.to, deco: bulletDotDeco });
            break;
          }
          case "Blockquote": {
            const n = node.node;
            const first = doc.lineAt(n.from);
            const co = CALLOUT_RE.exec(first.text);
            if (!co) {
              decorateLines(node.from, node.to, range.from, range.to, quoteLine, "q");
              break;
            }
            // > [!type] title → a callout: the whole block is coloured and the first line is the title line
            const group = CALLOUT_GROUP[co[3].toLowerCase()] ?? "info";
            const title = first.text.slice(co[0].length).trim();
            const from = Math.max(n.from, range.from);
            // A private block's tint stops where the index's block stops: privateblock.ts
            // mirrors the Rust parser and is what the status bar and the selection guard
            // read, so the tint asks it rather than walking `>` lines again. (Lezer folds a
            // lazy continuation line into the quote and the parser does not — an untinted
            // lazy line is visibly public.)
            const end = group === "private" ? (privateRangeAt(doc, first.from)?.to ?? n.to) : n.to;
            const to = Math.min(end, range.to);
            for (let pos = doc.lineAt(from).from; pos <= to; ) {
              const line = doc.lineAt(pos);
              let cls = `cm-lp-callout cm-lp-co-${group}`;
              if (line.from === first.from) cls += " cm-lp-co-head cm-lp-co-first";
              if (line.to >= end) cls += " cm-lp-co-last";
              addLine(line.from, Decoration.line({ class: cls }), "co");
              if (line.to >= to) break;
              pos = line.to + 1;
            }
            if (!revealedLine(first.from, first.to))
              hides.push({
                from: first.from + co[1].length,
                to: first.from + co[0].length,
                deco: Decoration.replace({
                  widget: new CalloutIconWidget(group, title ? "" : co[3]),
                }),
              });
            break;
          }
          case "Table": {
            // With the selection inside the table its source is shown, and a monospace
            // font keeps the pipes roughly aligned;
            // outside it, the whole table is taken over by table.ts's block-level widget
            decorateLines(node.from, node.to, range.from, range.to, tableSrcLine, "t");
            break;
          }
          case "QuoteMark": {
            if (revealedLine(node.from, node.to)) break;
            const to = doc.sliceString(node.to, node.to + 1) === " " ? node.to + 1 : node.to;
            hide(node.from, to);
            break;
          }
          case "HorizontalRule": {
            if (revealedLine(node.from, node.to)) break;
            hides.push({
              from: node.from,
              to: node.to,
              deco: Decoration.replace({ widget: new HrWidget() }),
            });
            break;
          }
          case "FencedCode": {
            const n = node.node;
            decorateLines(n.from, n.to, range.from, range.to, codeLine, "c");
            const firstLine = doc.lineAt(n.from);
            addLine(firstLine.from, codeHeadLine, "ch");
            addLine(firstLine.from, copyDeco, "copy");
            if (!revealedLine(firstLine.from, firstLine.to)) hide(firstLine.from, firstLine.to);
            const lastLine = doc.lineAt(n.to);
            // The start of the line tolerates a quote marker (contentStart, the same rule
            // as the button anchor's retry): a fence from a clipped selection lives
            // inside a quote block, where the closing line is "> ```"
            if (
              lastLine.from > firstLine.from &&
              /^(`{3,}|~{3,})\s*$/.test(lastLine.text.slice(contentStart(lastLine.text))) &&
              !revealedLine(lastLine.from, lastLine.to)
            )
              hide(lastLine.from, lastLine.to);
            break;
          }
        }
      },
    });

    // A wikilink is not in the syntax tree (wikilink.ts decorates by regex), so the brackets are hidden by regex here too
    const text = doc.sliceString(range.from, range.to);
    WIKI_RE.lastIndex = 0;
    let m: RegExpExecArray | null;
    while ((m = WIKI_RE.exec(text))) {
      const start = range.from + m.index;
      const end = start + m[0].length;
      // The embed form's replace range includes the leading "!", so the reveal check has to cover it or the cursor cannot reach it from the left
      const embed = doc.sliceString(start - 1, start) === "!";
      if (revealedEl(embed ? start - 1 : start, end) || isCode(start)) continue;
      // ![[img.png]] / ![[img.png|300]] embed an image; a non-image ![[…]] belongs to
      // embed.ts (a block widget or a chip) — this scan neither hides its brackets nor
      // draws › / ¶ for it, so the two decorators never overlap
      if (embed) {
        const parts = m[1].split("|");
        const target = parts[0].split("#")[0].trim();
        if (IMAGE_EXT_RE.test(target)) {
          const w = parts.length > 1 ? Number(parts[parts.length - 1]) : 0;
          const width = Number.isFinite(w) && w > 0 ? w : 0;
          hides.push({
            from: start - 1,
            to: end,
            deco: Decoration.replace({
              widget: new ImageWidget(target, target, width, opts.loadAsset, opts.copyImage),
            }),
          });
        }
        continue;
      }
      // The supports:[[…]] prefix is the marker of an evidence's supporting relation and
      // renders as a citation badge (the badge's range lies outside [start,end], so it
      // gets its own reveal check)
      const SUPPORTS = "supports:";
      if (
        m.index >= SUPPORTS.length &&
        text.slice(m.index - SUPPORTS.length, m.index) === SUPPORTS &&
        !revealedEl(start - SUPPORTS.length, start)
      ) {
        hides.push({
          from: start - SUPPORTS.length,
          to: start,
          deco: Decoration.replace({ widget: new TextBadgeWidget("supports →", "cm-lp-supports") }),
        });
      }
      hide(start, start + 2);
      const pipe = m[1].indexOf("|");
      if (pipe >= 0) {
        hide(start + 2, start + 2 + pipe + 1); // "target|" leaves the alias alone
      } else {
        const hash = m[1].indexOf("#");
        if (hash >= 0) {
          const hashFrom = start + 2 + hash;
          if (m[1][hash + 1] === "^") {
            // [[note#^id]] → "note ¶" — a block id is an internal anchor name and is hidden entirely while reading
            hides.push({
              from: hashFrom,
              to: end - 2,
              deco: Decoration.replace({ widget: new TextBadgeWidget("¶", "cm-lp-blockref") }),
            });
          } else {
            // [[note#heading]] → "note › heading"
            hides.push({
              from: hashFrom,
              to: hashFrom + 1,
              deco: Decoration.replace({ widget: new TextBadgeWidget("›", "cm-lp-linksep") }),
            });
          }
        }
      }
      hide(end - 2, end);
    }

    // #tag chips (the cursor's line keeps the styling too; only decorations are added and no character is hidden)
    TAG_RE.lastIndex = 0;
    while ((m = TAG_RE.exec(text))) {
      if (/^[\d/]+$/.test(m[2])) continue; // digits and slashes alone are not a tag
      const start = range.from + m.index + m[1].length;
      if (isCode(start)) continue;
      marks.push({ from: start, to: start + 1 + m[2].length, deco: tagDeco });
    }

    // Where a block anchor is defined, "… ^id" renders as a small badge (the marker a
    // [[note#^id]] can cite); like a tag it is a mark decoration and keeps its styling
    // on the cursor's line. The whitespace is limited to [ \t]: the parser requires a
    // space before the ^
    ANCHOR_RE.lastIndex = 0;
    while ((m = ANCHOR_RE.exec(text))) {
      const start = range.from + m.index + 1; // the leading whitespace is not styled
      if (isCode(start)) continue;
      marks.push({ from: start, to: start + 1 + m[1].length, deco: anchorDeco });
    }

    // The *(YYYY-MM-DD HH:MM)* signature of a memo or a clip: demoted to small, faint
    // text, so metadata does not compete with the body (the cursor's line keeps the
    // styling too; only decorations are added and no character is hidden, and the
    // italic asterisks belong to EmphasisMark)
    STAMP_RE.lastIndex = 0;
    while ((m = STAMP_RE.exec(text))) {
      const start = range.from + m.index;
      if (isCode(start)) continue;
      marks.push({ from: start, to: start + m[0].length, deco: stampDeco });
    }

    // Inline fields `key:: value` → chips (line by line: the scanner's rules are per
    // line, and a fenced block or the frontmatter is skipped whole). A task line's
    // due::/scheduled:: are the date chips above and are not drawn twice. An empty
    // placeholder is a chip too (the index does not hold it; the chip is how it gets
    // filled) — an empty `[due:: ]` included, which the date scanner cannot see.
    if (text.includes("::")) {
      const fmEnd = frontmatterEnd(doc);
      for (let pos = range.from; pos <= range.to; ) {
        const line = doc.lineAt(pos);
        if (line.to > range.to && line.from > range.from) break;
        if (line.text.includes("::") && line.from >= fmEnd && !isCode(line.from)) {
          const task = TASK_LINE_RE.test(line.text);
          for (const f of scanFields(line.text, line.from, { empty: true })) {
            if (task && f.key in DATE_FIELDS && f.value !== "") continue;
            if (revealedEl(f.from, f.to)) {
              marks.push({ from: f.from, to: f.to, deco: fieldSrcDeco });
              continue;
            }
            hides.push({ from: f.from, to: f.to, deco: Decoration.replace({ widget: new FieldChipWidget(f.key, f.value, opts.onFieldChip) }) });
          }
        }
        if (line.to >= range.to) break;
        pos = line.to + 1;
      }
    }
  }

  // Replace decorations may not partially overlap: after sorting by from ascending and to descending, the larger range that came first is kept
  hides.sort((a, b) => a.from - b.from || b.to - a.to);
  const cleanHides: Entry[] = [];
  let coveredTo = -1;
  for (const h of hides) {
    if (h.from < coveredTo) continue;
    cleanHides.push(h);
    coveredTo = h.to;
  }

  const all = [...cleanHides, ...marks].sort(
    (a, b) => a.from - b.from || a.deco.startSide - b.deco.startSide,
  );
  const builder = new RangeSetBuilder<Decoration>();
  for (const e of all) builder.add(e.from, e.to, e.deco);
  return builder.finish();
}

/** What the decorations depend on in the selection is "which bare cursors are
 *  revealing" — under an element-level verdict even a move within a line changes the
 *  output, so the key encodes only the effective reveal points (ranges that are armed
 *  and empty), and only an update whose output cannot have changed is skipped:
 *  unarmed it is always the empty string, and a non-empty range during a drag never
 *  enters the key → no full rebuild.
 *  An armed flip always comes with a tr.selection, so the selectionSet branch is
 *  certain to see the key change. */
function revealKey(state: EditorState): string {
  if (!(state.field(revealArmed, false) ?? true)) return "";
  return state.selection.ranges
    .filter((r) => r.empty)
    .map((r) => `${r.from}`)
    .join(",");
}

function livePreviewPlugin(opts: LivePreviewOptions) {
  return ViewPlugin.fromClass(
    class {
      decorations: DecorationSet;
      key: string;
      /** Decorations were held still through an IME composition and must be rebuilt once it ends. */
      held = false;
      constructor(view: EditorView) {
        this.key = revealKey(view.state);
        this.decorations = buildDecorations(view, opts);
      }
      update(update: ViewUpdate) {
        // An open IME composition: nothing may be hidden or replaced on the line the
        // browser is composing in. The first composed letter turns `[[|]]` into the
        // wikilink `[[c]]`; folding its brackets at that moment overlaps the text node
        // the IME holds, the line shows `[c]`, and from then on the DOM and the
        // document disagree (WebKit keeps composing into a node the editor no longer
        // owns). The decorations only travel with the changes until compositionend
        if (update.view.composing) {
          if (update.docChanged) this.decorations = this.decorations.map(update.changes);
          this.held = true;
          return;
        }
        const treeChanged = syntaxTree(update.state) !== syntaxTree(update.startState);
        if (update.docChanged || update.viewportChanged || treeChanged || this.held) {
          this.held = false;
          this.key = revealKey(update.state);
          this.decorations = buildDecorations(update.view, opts);
        } else if (update.selectionSet) {
          // A selectionSet transaction need not really have changed the selection (a click in place, say), so an identical key skips the rebuild
          const key = revealKey(update.state);
          if (key !== this.key) {
            this.key = key;
            this.decorations = buildDecorations(update.view, opts);
          }
        }
      }
    },
    { decorations: (v) => v.decorations },
  );
}

// The frontmatter fold spans lines, and a block-level replace has to come from a StateField (a ViewPlugin is not allowed)
function frontmatterDeco(state: EditorState): DecorationSet {
  const doc = state.doc;
  if (doc.lines < 2 || doc.line(1).text !== "---") return Decoration.none;
  let end = -1;
  const keys: string[] = [];
  const maxLine = Math.min(doc.lines, 100);
  for (let i = 2; i <= maxLine; i++) {
    const text = doc.line(i).text;
    if (text === "---" || text === "...") {
      end = doc.line(i).to;
      break;
    }
    const key = /^([\w-]+)\s*:/.exec(text);
    if (key) keys.push(key[1]);
  }
  if (end < 0) return Decoration.none;
  // A selection touching the block expands the source (block-level selectionTouches:
  // the armed gate stops the initial cursor of a freshly opened document and a
  // programmatic jump, and an intersecting range selection expands it too)
  if (selectionTouches(state, 0, end)) return Decoration.none;
  return Decoration.set([
    Decoration.replace({ widget: new FrontmatterWidget(keys), block: true }).range(0, end),
  ]);
}

const frontmatterField = StateField.define<DecorationSet>({
  create: frontmatterDeco,
  update(value, tr) {
    if (tr.docChanged || tr.selection) return frontmatterDeco(tr.state);
    return value;
  },
  provide: (f) => EditorView.decorations.from(f),
});

export interface LivePreviewOptions {
  loadAsset?: AssetLoader;
  copyImage?: ImageCopier;
  /** A click on a chip: a task's due/scheduled date, or any inline `key:: value` field. */
  onFieldChip?: FieldChipHandler;
}

/** After a composition ends without a document change (a cancelled one), no
 *  transaction follows to release the held decorations: an empty one does. */
const compositionRelease = EditorView.domEventHandlers({
  compositionend(_e, view) {
    setTimeout(() => {
      if (!view.composing) view.dispatch({});
    }, 0);
    return false;
  },
});

export function livePreview(opts: LivePreviewOptions = {}): Extension {
  // revealArmed is registered here (rather than in setup.ts): every consumer of the
  // reveal predicates (livepreview/table/queryblock/frontmatter/clickopen) lives in
  // note mode alone, and note mode always carries livePreview()
  return [revealArmed, livePreviewPlugin(opts), compositionRelease, frontmatterField, tableField, emptyFieldCloser(opts.onFieldChip)];
}
