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
import { CALLOUT_RE, frontmatterEnd } from "./blocks";
import { syntaxTree } from "@codemirror/language";
import {
  type Command,
  Decoration,
  type DecorationSet,
  EditorView,
  ViewPlugin,
  type ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import { type EditorState, type Extension, RangeSetBuilder, StateField } from "@codemirror/state";
import { ancestorNamed, contentStart, fencedCodeAt, fencedCodeText, inCode } from "./fenced";
import { CLIP_ESCAPE_RE } from "./mdlink";
import { revealArmed, revealRanges, selectionReveals, selectionTouches } from "./reveal";
import { tableField } from "./table";
import { LINK_RE as WIKI_RE } from "./wikilink";
import {
  attrIconSvg,
  type AttrKind,
  checkboxState,
  completeTaskSlice,
  DATE_FIELDS,
  dateFieldLabel,
  type DateField,
  EDIT_FIELDS,
  type EditField,
  formatEstimate,
  isClosed,
  isUrgent,
  localDate,
  parseEstimate,
  type Priority,
  priorityLabel,
  ATTR_KEY_NAME,
  isUnitMark,
  markerGlyphLength,
  nextTaskState,
  restOffset,
  taskToggleTitle,
  scanTaskAttrs,
  STAMP_RE,
  stateOfChar,
  TASK_ATTR_KEYS,
  TASK_LINE_RE,
  type TaskAttrMark,
  type TaskState,
  taskStateIconSvg,
} from "$lib/views/tasks";
import { type StateStep, stateGlyphSvg } from "$lib/chips";
import { scanFields } from "$lib/fields";
import { privateRangeAt } from "./privateblock";
import { blockRoot } from "./blockwidget";
import { focusEditor } from "./clickguard";
import { openImageLightbox } from "$lib/lightbox";
import { copyText } from "$lib/copytext";
import { scanInlineMath } from "$lib/math";
import { domainOf } from "$lib/markdown";
import { allowRemote, mayLoadRemote, noteImageUrl, onRemoteAllowed } from "./remoteimages";
import { MathWidget, mathBlockField, mathBlocksOf } from "./math";

/** An image target inside the vault → a displayable URL (a blob); http(s) does not go through it */
export type AssetLoader = (target: string) => Promise<string | null>;

/** An image target inside the vault → written to the system clipboard (as a bitmap),
 *  with success or failure fed back to the button; not offered for an http(s) image
 *  (the editor makes no network request just to copy) */
export type ImageCopier = (target: string) => Promise<boolean>;

// inCode lives in fenced.ts (editor/math.ts needs it without a cycle); the importers here keep their path
export { inCode };
// The parser's rule (parser.rs): a `#` opens a tag unless the char before it is a
// letter, a digit, `#` or `&` — so `（#tag` and `「#tag` are tags here too, and `##h`
// and `&#39;` are not.
const TAG_RE = /(^|[^\p{L}\p{N}#&])#([\p{L}\p{N}_][\p{L}\p{N}_/-]*)/gu;
/** An emphasis span the parser may have refused: `**…**` or `*…*` on one line, no `*` inside */
const CJK_EMPH_RE = /(\*\*|\*)([^*\n]+?)\1/g;
const CJK_PUNCT = /[。，、；：！？）」』】》〉…—～（「『【《〈]/;
/** What may not sit outside the delimiter for the rescue to apply: the span has to run
 *  into a word, or CommonMark's flanking rule would have taken it already. */
const WORD_OUTSIDE = /[\s\p{P}\p{S}]/u;
const cjkStrongDeco = Decoration.mark({ class: "cm-lp-cjk-strong" });
/** The lenient emphasis span above; it touches CJK punctuation by construction, but its
 *  words may still be Latin, so it takes the same per-span verdict as a parsed one. */
const cjkEmDeco = Decoration.mark({ class: "cm-md-em cm-lp-em-cjk" });
const latinEmDeco = Decoration.mark({ class: "cm-md-em" });
/** A span holding a Han, kana or hangul letter. Its emphasis is drawn with the emphasis
 *  marks of the note's language (app.css .cm-lp-em-cjk), not slanted: CJK faces have no
 *  italic, and the engine's synthetic slant smears the strokes. A Latin-only span in the
 *  same note keeps its italic — the verdict is per span, not per note. */
const CJK_TEXT = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u;
const cjkEmphDeco = Decoration.mark({ class: "cm-lp-em-cjk" });
/** A bare URL's extent for the escape fold above (the click boundary itself is mdlink's) */
const BARE_URL_RE = /https?:\/\/[^\s<>]+/g;
/** The whole-line gate for that fold: the same characters CLIP_ESCAPE_RE names, derived
 *  from it rather than spelled a second time, and without the `g` flag — matchAll below
 *  starts from the regex's own lastIndex, which a `test` on a global one would move. */
const CLIP_ESCAPE_TEST = new RegExp(CLIP_ESCAPE_RE.source);
// The character class mirrors Rust's block-id rule (blockid.rs::is_valid_block_id): a badge must never claim an id the parser rejects
const ANCHOR_RE = /[ \t]\^([A-Za-z0-9-]+)$/gm;
export const IMAGE_EXT_RE = /\.(png|jpe?g|gif|webp|svg|bmp|ico|avif|tiff?)$/i;
// CALLOUT_RE lives in blocks.ts (the block model, fold.ts and this renderer share it)

const hideDeco = Decoration.replace({});
const doneDeco = Decoration.mark({ class: "cm-lp-done" });
/** A task's `- [ ] ` while the cursor is in it: the source in the checkbox's box (at
 *  least; the source may be wider, and then pushes the text). */
const taskSrcDeco = Decoration.mark({ class: "cm-lp-lmark cm-lp-lmark-task" });
const tagDeco = Decoration.mark({ class: "cm-lp-tag" });
/** The `#` of a tag: kept in the text, faded by CSS (the name carries the capsule) */
const tagHashDeco = Decoration.mark({ class: "cm-lp-tag-hash" });
const anchorDeco = Decoration.mark({ class: "cm-lp-anchor" });
/** Inline code, backticks included: the ground and the padding sit on the whole span. */
const inlineCodeDeco = Decoration.mark({ class: "cm-lp-icode" });
/** The line holding a bare cursor (the armed reveal rule's lines): what shows only
 *  where the reader is — a block's id, an AI block's accept and reject — keys on it. */
const curLineDeco = Decoration.line({ class: "cm-lp-curline" });
/** A blank line between two text lines: a paragraph gap rather than a whole empty line
 *  (app.css .cm-lp-gap, 0.9em — the gap the rendered surfaces put between paragraphs). */
const gapLineDeco = Decoration.line({ class: "cm-lp-gap" });
const badDateDeco = Decoration.mark({
  class: "cm-lp-baddate",
  attributes: { title: t("editor.badDate") },
});
/** A valid attribute the parser does not read: one in the description (the trailing run
 *  is read from the end of the line), or the earlier of two of a kind (the last counts).
 *  Underlined with the reason, so a missing chip is never a mystery. */
const attrOutsideDeco = Decoration.mark({ class: "cm-lp-attr-unread", attributes: { title: t("editor.attrOutside") } });
const attrDuplicateDeco = Decoration.mark({ class: "cm-lp-attr-unread", attributes: { title: t("editor.attrDuplicate") } });
const stampDeco = Decoration.mark({ class: "cm-lp-stamp" });
const quoteLine = Decoration.line({ class: "cm-lp-quote" });
const codeLine = Decoration.line({ class: "cm-lp-code" });
const codeHeadLine = Decoration.line({ class: "cm-lp-code-head" });
const codeLastLine = Decoration.line({ class: "cm-lp-code-last" });
/** A fence line while it is hidden: only the block's inner margin is left of it. */
const codeFenceLine = Decoration.line({ class: "cm-lp-code-fence" });
/** The head line of the block holding the cursor: its copy and run buttons show. */
const codeCurLine = Decoration.line({ class: "cm-lp-code-cur" });
const tableSrcLine = Decoration.line({ class: "cm-lp-tablesrc" });
/** The vertical rhythm of a heading line (the space above; the values live in app.css's .cm-lp-h1..h6); the index is the level minus 1 */
const headingLines = [1, 2, 3, 4, 5, 6].map((n) => Decoration.line({ class: `cm-lp-h${n}` }));

// callout type → colour group and head icon: one vocabulary with the read-only
// renderers (callout.ts), so a `[!tip]` looks the same wherever it appears
import { CALLOUT_GROUP, calloutDefaultTitle, calloutIconSvg } from "$lib/callout";

interface Entry {
  from: number;
  to: number;
  deco: Decoration;
}

/** The `- [ ]` checkbox: the state glyph (chips.ts) — the same drawing a card, an
 *  Agenda row, a query row and the reading projection show, so a task looks the same
 *  wherever it is met. A click rewrites the state in the text underneath: a plain click
 *  flips done, ⌥ flips "in progress" (`[/]`, the half-filled box — the same two
 *  gestures as a query row's glyph, and the one way from the editor to the state the
 *  task views' menus set). A span rather than a native input: the input was never
 *  focused (mousedown is taken over below) and only lent its :checked/:indeterminate
 *  selectors to a second, hand-styled copy of the drawing, which had drifted from the
 *  views' one. The glyph sits in a box that also holds the gap after it: the cursor
 *  just past the widget is drawn at the box's edge, so on a task with no text yet it
 *  stands where the first character will go, not against the glyph. */
class CheckboxWidget extends WidgetType {
  constructor(readonly state: TaskState) {
    super();
  }
  eq(other: CheckboxWidget) {
    return other.state === this.state;
  }
  toDOM(view: EditorView) {
    const wrap = document.createElement("span");
    wrap.className = "cm-lp-taskbox";
    const box = document.createElement("span");
    box.className = "cm-lp-checkbox";
    box.setAttribute("role", "checkbox");
    box.setAttribute("aria-checked", this.state === "done" ? "true" : this.state === "doing" ? "mixed" : "false");
    box.innerHTML = taskStateIconSvg(this.state);
    box.title = taskToggleTitle(this.state);
    // only the glyph toggles: a click in the gap places the cursor, as on the text
    box.onmousedown = (e) => {
      // Handled on mousedown: preventDefault keeps the editor's selection, or the
      // cursor landing inside the checkbox's range would trigger reveal and replace
      // this widget before the click arrived
      e.preventDefault();
      e.stopPropagation();
      toggleTaskAt(view, view.posAtDOM(wrap), e.altKey);
    };
    wrap.appendChild(box);
    return wrap;
  }
}

/** The icon the `[!type]` marker is replaced with; with an empty title the type name stands in as one */
class CalloutIconWidget extends WidgetType {
  constructor(
    readonly type: string,
    readonly group: string,
    readonly label: string,
  ) {
    super();
  }
  eq(other: CalloutIconWidget) {
    return other.type === this.type && other.group === this.group && other.label === this.label;
  }
  toDOM() {
    const el = document.createElement("span");
    el.className = "cm-lp-co-icon";
    el.innerHTML = calloutIconSvg(this.type, this.group);
    // the private block's badge explains itself on hover, like every other privacy badge
    if (this.group === "private") el.title = t("editor.privateBlockHint");
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
const bulletDotDeco = Decoration.replace({ widget: new TextBadgeWidget("•", "cm-lp-lmark cm-lp-bulletdot") });
/** A bullet's `- ` while the cursor is on it: the source in the same box the dot takes. */
const bulletMarkDeco = Decoration.mark({ class: "cm-lp-lmark cm-lp-bullet" });

/** The width of a list marker's box. The item's first line starts at the marker and the
 *  text of every line after it — a wrapped line or a continuation line — hangs at the
 *  box's right edge; the depth adds 1.5em per level (app.css .cm-lp-li). A bullet's box
 *  and a task's are app.css tokens (--li-box-bullet, --li-box-task); a number's grows
 *  with its digits (tabular figures, 0.6em each, plus the stop and the space). */
const BULLET_BOX = "var(--li-box-bullet)";
const TASK_BOX = "var(--li-box-task)";
const numberBox = (mark: string) => `${Math.round(((mark.length - 1) * 0.6 + 0.9) * 10) / 10}em`;

const liDecos = new Map<string, Decoration>();
/** The line decoration of a list item's line (`cont`: a continuation line of the item,
 *  which only takes the hanging position). Cached by shape: a long list reuses a handful. */
function liLine(depth: number, box: string, cont: boolean): Decoration {
  const key = `${cont ? "c" : "m"}${depth}|${box}`;
  let d = liDecos.get(key);
  if (!d) {
    d = Decoration.line({ class: cont ? "cm-lp-li cm-lp-li-cont" : "cm-lp-li", attributes: { style: `--li-depth: ${depth}; --li-mark: ${box}` } });
    liDecos.set(key, d);
  }
  return d;
}
const numberDecos = new Map<string, Decoration>();
/** An ordered item's number in its box (shown as written, cursor or not). */
function numberMark(box: string): Decoration {
  let d = numberDecos.get(box);
  if (!d) {
    d = Decoration.mark({ class: "cm-lp-lmark cm-lp-lnum", attributes: { style: `min-width: ${box}` } });
    numberDecos.set(box, d);
  }
  return d;
}

/** A block id while the cursor is elsewhere: no ink in the reading column, and no width
 *  either — a zero-width pin at the end of the text, whose badge floats above the line's
 *  end while the pointer is over the block (app.css .cm-lp-anchor-pin). */
class AnchorPinWidget extends WidgetType {
  constructor(readonly id: string) {
    super();
  }
  eq(other: AnchorPinWidget) {
    return other.id === this.id;
  }
  toDOM() {
    const pin = document.createElement("span");
    pin.className = "cm-lp-anchor-pin";
    const tip = document.createElement("span");
    tip.className = "cm-lp-anchor cm-lp-anchor-tip";
    tip.textContent = `^${this.id}`;
    pin.appendChild(tip);
    return pin;
  }
}

/** A task attribute chip: a valid date (📅 ⏳ ⏩ … or its `key::` spelling) renders as a
 *  small capsule looking exactly like Agenda's, an overdue (and open) due date coloured
 *  danger; the other attributes — a priority, a repeat rule, an estimate, an id, what
 *  the task waits on, "removed once done" — draw their icon and value in the
 *  attribute's tone (app.css). A click on a settable date lands beside the chip and
 *  raises its menu; the source can be edited once the line reveals */
class AttrChipWidget extends WidgetType {
  constructor(
    readonly kind: AttrKind,
    readonly text: string,
    readonly cls: string,
    readonly title: string,
    readonly icon: string,
    readonly onChip?: FieldChipHandler,
    readonly chipValue = "",
  ) {
    super();
  }
  eq(other: AttrChipWidget) {
    return other.kind === this.kind && other.text === this.text && other.cls === this.cls && other.icon === this.icon;
  }
  toDOM(view: EditorView) {
    const el = document.createElement("span");
    el.className = `cm-lp-datechip cm-lp-attr-${this.kind} ${this.cls}${this.onChip ? " cm-lp-chip-click" : ""}`;
    el.title = this.title;
    el.innerHTML = this.icon;
    if (this.text) el.append(this.text);
    else el.classList.add("cm-lp-datechip-bare");
    wireChipClick(el, view, this.onChip, this.kind, this.chipValue);
    return el;
  }
}

/** The emoji of an attribute while its source is being edited (the cursor touches the
 *  chip, or the attribute is one the parser does not read): the glyph alone is replaced
 *  by the icon and the attribute's key name — `due`, `repeat`, `priority high` — and the
 *  value after it stays plain editable text. So the emoji spelling on disk is never on
 *  screen, and the source reads as words; Backspace over the atom removes the emoji. */
class MarkerAtomWidget extends WidgetType {
  constructor(
    readonly kind: AttrKind,
    readonly label: string,
    readonly cls: string,
    readonly icon: string,
    readonly onChip?: FieldChipHandler,
    readonly chipValue = "",
  ) {
    super();
  }
  eq(other: MarkerAtomWidget) {
    return other.kind === this.kind && other.label === this.label && other.cls === this.cls && other.chipValue === this.chipValue;
  }
  toDOM(view: EditorView) {
    const el = document.createElement("span");
    el.className = `cm-lp-attr-atom cm-lp-attr-${this.kind} ${this.cls}${this.onChip ? " cm-lp-chip-click" : ""}`;
    el.innerHTML = this.icon;
    el.append(this.label);
    wireChipClick(el, view, this.onChip, this.kind, this.chipValue);
    return el;
  }
  ignoreEvent() {
    return false;
  }
}

/** The handler a chip or atom gets, for the attributes a click may set and no others. */
const chipHandler = (kind: AttrKind, onFieldChip?: FieldChipHandler) =>
  EDIT_FIELDS.includes(kind as EditField) ? onFieldChip : undefined;

/** The atom for a mark's glyph, or null for the bracketed spelling (already words). A
 *  settable date's atom opens its menu on a click, like its chip. */
function markerAtom(am: TaskAttrMark, text: string, from: number, onFieldChip?: FieldChipHandler): Entry | null {
  const n = markerGlyphLength(am.kind, text);
  if (!n) return null;
  const p = am.kind === "priority" ? (am.value as Priority) : undefined;
  const label = p ? `${ATTR_KEY_NAME.priority} ${p}` : ATTR_KEY_NAME[am.kind];
  const cls = p ? (isUrgent(p) ? "cm-lp-attr-urgent" : "cm-lp-attr-faint") : "";
  const onChip = chipHandler(am.kind, onFieldChip);
  return { from, to: from + n, deco: Decoration.replace({ widget: new MarkerAtomWidget(am.kind, label, cls, attrIconSvg(am.kind, p), onChip, am.date ?? "") }) };
}

/** Backspace right after a unit mark (`📅 ` awaiting its date, `❗❗ `, `🚮`) — at its end
 *  or past the one space after it — removes the whole mark and that space in one
 *  stroke, the way a chip reads as one thing: char by char the emoji would first
 *  become its atom and then vanish, two keystrokes with nothing visibly deleted by the
 *  first. A marker with a value is left to the ordinary Backspace, which eats the value
 *  first. */
export const deleteTaskMarkerBackward: Command = (view) => {
  const { state } = view;
  const sel = state.selection.main;
  if (!sel.empty) return false;
  const line = state.doc.lineAt(sel.head);
  if (!TASK_LINE_RE.test(line.text)) return false;
  const at0 = line.from + restOffset(line.text);
  const rest = line.text.slice(at0 - line.from);
  const caret = sel.head - at0;
  for (const am of scanTaskAttrs(rest)) {
    if (!isUnitMark(am) || am.end > caret) continue;
    if (!/^[ \t]?$/.test(rest.slice(am.end, caret))) continue;
    view.dispatch({ changes: { from: at0 + am.start, to: sel.head }, userEvent: "delete.backward" });
    return true;
  }
  return false;
};

interface ChipParts {
  text: string;
  cls: string;
  title: string;
  icon: string;
}

/** The text, tone class, tooltip and icon of a non-date attribute chip. */
function attrChipParts(am: TaskAttrMark): ChipParts {
  const parts = (text: string, cls: string, title: string, icon: string): ChipParts => ({ text, cls, title, icon });
  switch (am.kind) {
    case "priority": {
      const p = am.value as Priority;
      return parts("", isUrgent(p) ? "cm-lp-attr-urgent" : "cm-lp-attr-faint", priorityLabel(p), attrIconSvg("priority", p));
    }
    case "repeat":
      return parts(am.value, "", t("task.chip.repeat", { rule: am.value }), attrIconSvg("repeat"));
    case "delete":
      return parts("", "", t("task.chip.deleteOnDone"), attrIconSvg("delete"));
    case "id":
      return parts(am.value, "", t("task.chip.id", { id: am.value }), attrIconSvg("id"));
    case "depends": {
      const ids = am.value.split(",").join(", ");
      return parts(ids, "cm-lp-attr-urgent", t("task.chip.blockedBy", { ids }), attrIconSvg("depends"));
    }
    case "estimate": {
      const v = formatEstimate(parseEstimate(am.value) ?? 0);
      return parts(v, "", t("task.chip.estimate", { v }), attrIconSvg("estimate"));
    }
    default:
      return parts(am.value, "", "", "");
  }
}

/** A chip's click: the host opens the menu for the field at that position (`pos` is
 *  where the field's source starts; `value` is "" for an empty placeholder; `at` is
 *  where the menu anchors — the click, or the caret for a typed close). */
export type FieldChipHandler = (view: EditorView, pos: number, key: string, value: string, at: { x: number; y: number }, opts?: { keyboard?: boolean }) => void;

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
    /** The value's step when the key is a state field (the host's fieldState): the chip
     *  then carries the glyph and the step's tone. */
    readonly state?: StateStep | null,
  ) {
    super();
  }
  eq(other: FieldChipWidget) {
    return other.key === this.key && other.value === this.value && other.state?.step === this.state?.step && other.state?.of === this.state?.of;
  }
  toDOM(view: EditorView) {
    const el = document.createElement("span");
    const empty = this.value === "";
    el.className = `cm-lp-field${this.onChip ? " cm-lp-chip-click" : ""}${empty ? " cm-lp-field-empty" : ""}`;
    if (this.state) {
      el.classList.add("cm-lp-field-state");
      const g = document.createElement("span");
      g.className = "cm-lp-field-glyph";
      g.innerHTML = stateGlyphSvg(this.state.step, this.state.of);
      el.appendChild(g);
    }
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
/** A formula's source while the cursor is on it: monospace and muted, like a revealed field */
const mathSrcDeco = Decoration.mark({ class: "cm-lp-math-src" });

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

/** The size an image target is known to have — read from its header when its note
 *  opened, or measured when it loaded — for every surface drawing a vault image (this
 *  widget, an embed's body). */
export function imageDimsOf(target: string): { w: number; h: number } | undefined {
  return imageDims.get(target);
}

/** Record an image's intrinsic size once it has loaded. */
export function noteImageDims(target: string, w: number, h: number): void {
  if (w > 0 && h > 0) imageDims.set(target, { w, h });
}

// The vault image references a note's text holds: `![[a.png]]` / `![[a.png|300]]`, and a
// one-line `![alt](path)`. Read from the raw text before the editor exists, so a match
// inside a code block only costs a header read.
const WIKI_IMAGE_RE = /!\[\[([^\]|#\n]+)(?:#[^\]|\n]*)?(?:\|[^\]\n]*)?\]\]/g;
const MD_IMAGE_RE = /!\[[^\]\n]*\]\(\s*<?([^)\s>]+)>?(?:\s+"[^"\n]*")?\s*\)/g;

/** Read the size of every vault image a note shows that is not known yet from the file
 *  headers (commands.rs image_sizes). Started as the note opens, alongside the editor
 *  being built rather than before it (a jump into the note right after opening expects
 *  the editor to hold it at once); the answer is back within a few milliseconds, as a
 *  rule before the first frame, and fitImagePlaceholders hands it to the images already
 *  mounted. `resolve` maps a target to its vault path as the asset loader does; `read`
 *  is the IPC. Resolves to whether anything new was learnt. */
export async function primeImageDims(
  doc: string,
  resolve: (target: string) => string | null,
  read: (paths: string[]) => Promise<([number, number] | null)[]>,
): Promise<boolean> {
  const byPath = new Map<string, string[]>();
  // keyed as the image widget keys its target: a `![](…)` URL decoded, a `![[…]]` as written
  const want = (raw: string, encoded: boolean) => {
    let target = raw.trim();
    if (encoded) {
      try {
        target = decodeURIComponent(target);
      } catch {
        /* not encoded: kept as written */
      }
    }
    if (/^[a-z][a-z0-9+.-]*:/i.test(target) || !IMAGE_EXT_RE.test(target) || imageDims.has(target)) return;
    const path = resolve(target);
    if (!path) return;
    const targets = byPath.get(path);
    if (!targets) byPath.set(path, [target]);
    else if (!targets.includes(target)) targets.push(target);
  };
  for (const m of doc.matchAll(WIKI_IMAGE_RE)) want(m[1], false);
  for (const m of doc.matchAll(MD_IMAGE_RE)) want(m[1], true);
  if (!byPath.size) return false;
  const paths = [...byPath.keys()];
  const sizes = await read(paths).catch(() => null);
  if (!sizes) return false;
  let learnt = false;
  paths.forEach((path, i) => {
    const s = sizes[i];
    if (!s) return;
    for (const target of byPath.get(path)!) noteImageDims(target, s[0], s[1]);
    learnt = true;
  });
  return learnt;
}

/** Give the image widgets still waiting on their first load the size now known for
 *  them: the guessed placeholder box becomes the image's own aspect ratio, and the
 *  editor measures again. */
export function fitImagePlaceholders(view: EditorView): void {
  let fitted = false;
  for (const img of view.contentDOM.querySelectorAll<HTMLImageElement>("img.cm-lp-loading[data-target]")) {
    const dims = imageDims.get(img.dataset.target!);
    if (!dims) continue;
    img.width = dims.w;
    img.height = dims.h;
    img.classList.remove("cm-lp-loading");
    fitted = true;
  }
  if (fitted) view.requestMeasure();
}

/** A mounted remote-image placeholder's subscription (remoteimages.ts), dropped when the widget is. */
const remoteSubs = new WeakMap<HTMLElement, () => void>();

/** An inline image; a vault file becomes a blob URL through loadAsset, and an http(s)
 *  one goes through the vemry-asset proxy — from its cache alone until the user lets it
 *  load, so that opening a note makes no network request (remoteimages.ts) */
class ImageWidget extends WidgetType {
  constructor(
    readonly target: string,
    readonly alt: string,
    readonly width: number,
    readonly load?: AssetLoader,
    readonly copy?: ImageCopier,
    readonly openUrl?: (url: string) => void,
    readonly remote?: RemoteImageHost,
  ) {
    super();
  }
  eq(other: ImageWidget) {
    return other.target === this.target && other.alt === this.alt && other.width === this.width;
  }
  destroy(dom: HTMLElement) {
    remoteSubs.get(dom)?.();
    remoteSubs.delete(dom);
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
      // the key fitImagePlaceholders finds it by, should the header's answer come after the mount
      img.dataset.target = this.target;
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
      // The placeholder reads like the projection's chip: the alt, else the host — never
      // a bare URL across the line; a remote one opens in the browser window on click
      // (hotlink protection or a host gone is the usual reason it did not load)
      const remoteMissing = /^https?:\/\//i.test(this.target);
      const label = this.alt || (remoteMissing ? domainOf(this.target) : this.target);
      wrap.textContent = `⧉ ${label}`;
      wrap.title = remoteMissing ? t("editor.imageNotLoaded", { url: this.target }) : this.target;
      if (remoteMissing && this.openUrl) {
        const open = this.openUrl;
        wrap.style.cursor = "pointer";
        wrap.onmousedown = (e) => {
          if (e.button !== 0) return;
          e.preventDefault();
          e.stopPropagation();
          open(this.target);
        };
      }
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
    if (!remote) {
      const url = this.load?.(this.target) ?? Promise.resolve(null);
      void url.then((u) => (u ? (img.src = u) : missing()));
      return wrap;
    }
    // A remote image: the proxy's cache first, which makes no request; an image that is
    // not there waits as a placeholder until it may load (a click, the whole note, or the
    // setting), and only then is it fetched — through the proxy, never by the webview
    const referer = this.remote?.referer() ?? "";
    let ask: HTMLElement | null = null;
    const fetchIt = () => {
      remoteSubs.get(wrap)?.();
      remoteSubs.delete(wrap);
      ask?.remove();
      ask = null;
      wrap.classList.remove("cm-lp-image-ask");
      img.classList.add("cm-lp-loading");
      img.onerror = missing;
      img.src = noteImageUrl(this.target, referer, false);
    };
    if (mayLoadRemote(this.target)) {
      fetchIt();
      return wrap;
    }
    img.onerror = () => {
      // Allowed while the cache answered (a paste lands its images in the same update
      // that built this widget): fetch rather than ask
      if (mayLoadRemote(this.target)) return fetchIt();
      img.classList.remove("cm-lp-loading");
      wrap.classList.add("cm-lp-image-ask");
      ask = this.placeholder(view);
      wrap.appendChild(ask);
      remoteSubs.set(
        wrap,
        onRemoteAllowed(() => {
          if (mayLoadRemote(this.target)) fetchIt();
        }),
      );
      view.requestMeasure();
    };
    img.src = noteImageUrl(this.target, referer, true);
    return wrap;
  }

  /** A remote image not loaded: what it is, a Load button (the whole chip loads it too),
   *  and the note-wide choices under ⋯ and on a right-click. A mousedown is swallowed, as
   *  on the image itself, so the cursor does not land in the source and reveal it. */
  private placeholder(view: EditorView): HTMLElement {
    const box = document.createElement("span");
    box.className = "cm-lp-remote";
    box.title = t("editor.remoteImageAsk", { url: this.target });
    const label = document.createElement("span");
    label.textContent = `⧉ ${this.alt || domainOf(this.target)}`;
    const load = document.createElement("button");
    load.className = "cm-lp-remote-btn";
    load.textContent = t("editor.remoteImageLoad");
    box.append(label, load);
    box.onmousedown = (e) => {
      if (e.button !== 0) return;
      e.preventDefault();
      e.stopPropagation();
      allowRemote([this.target]);
    };
    const host = this.remote;
    if (host) {
      const more = document.createElement("button");
      more.className = "cm-lp-remote-btn";
      more.textContent = "⋯";
      more.dataset.tip = t("common.more");
      more.onmousedown = (e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.stopPropagation();
        host.menu(e, view);
      };
      box.append(more);
      box.oncontextmenu = (e) => {
        e.stopPropagation();
        host.menu(e, view);
      };
    }
    return box;
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

/** Frontmatter keys the app writes for itself and reads on its own: a note that declares
 *  only these shows no property line at all (its `lang:` is already at work in the page). */
const APP_KEYS = new Set(["lang"]);

/** The folded frontmatter property line; a click moves the cursor into the block and
 *  expands the source. With no key of the reader's own it folds to nothing, and the
 *  source still opens when the cursor moves into it. */
class FrontmatterWidget extends WidgetType {
  constructor(readonly keys: string[]) {
    super();
  }
  eq(other: FrontmatterWidget) {
    return other.keys.join("\n") === this.keys.join("\n");
  }
  get estimatedHeight() {
    return this.keys.length ? 30 : 0;
  }
  toDOM(view: EditorView) {
    const el = document.createElement("div");
    el.className = this.keys.length ? "cm-lp-frontmatter" : "cm-lp-frontmatter-none";
    if (!this.keys.length) return blockRoot(el);
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
      focusEditor(view);
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
    btn.classList.add("cm-lp-code-btn");
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

/** A click on the checkbox rewrites the whole line, as the task views do: a closing
 *  state leaves its ✅ / ❌ stamp, a repeating task ticked gets its next occurrence above
 *  it, and a 🚮 task ticked goes, its line break with it. */
function toggleTaskAt(view: EditorView, pos: number, alt = false) {
  const line = view.state.doc.lineAt(pos);
  const state = checkboxState(line.text);
  if (state === null) return;
  const out = completeTaskSlice(line.text, nextTaskState(state, alt), localDate());
  if (out === "") {
    const from = line.from > 0 ? line.from - 1 : line.from;
    const to = line.from > 0 ? line.to : Math.min(line.to + 1, view.state.doc.length);
    view.dispatch({ changes: { from, to, insert: "" }, userEvent: "delete" });
    return;
  }
  if (out !== line.text) view.dispatch({ changes: { from: line.from, to: line.to, insert: out }, userEvent: "input.task" });
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
  // The display blocks the field already found: the inline scan skips their lines, the
  // ones left as source included, so their `$$` fences never read as inline delimiters
  const mathBlocks = mathBlocksOf(state);

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
          case "Emphasis": {
            if (CJK_TEXT.test(doc.sliceString(node.from, node.to))) marks.push({ from: node.from, to: node.to, deco: cjkEmphDeco });
            break;
          }
          case "InlineCode": {
            marks.push({ from: node.from, to: node.to, deco: inlineCodeDeco });
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
            // CommonMark allows a newline inside the label or the (url), but a
            // ViewPlugin's replace decoration spanning lines makes CodeMirror throw a
            // RangeError outright — a link across lines keeps its source
            if (doc.lineAt(n.from).number !== doc.lineAt(n.to).number) break;
            const linkMarks = n.getChildren("LinkMark");
            if (!n.getChild("URL") || linkMarks.length < 4) break;
            if (!revealedEl(n.from, n.to)) {
              hide(linkMarks[0].from, linkMarks[0].to);
              hide(linkMarks[1].from, n.to);
              break;
            }
            // The cursor in the label shows the brackets but keeps the address folded to
            // "(…)": a click into a link no longer spills a long URL over several lines and
            // reflows the paragraph. The address opens once the cursor reaches it — from
            // the label one more step past the `]`, from outside the link at the `)`
            const open = linkMarks[2];
            if (!revealedEl(open.from, n.to))
              hides.push({ from: open.from, to: n.to, deco: Decoration.replace({ widget: new TextBadgeWidget("(…)", "cm-lp-urlfold") }) });
            break;
          }
          case "Autolink": {
            if (revealedEl(node.from, node.to)) break;
            hide(node.from, node.from + 1); // <
            hide(node.to - 1, node.to); // >
            break;
          }
          case "Escape": {
            // `\.` `\_` `\*`: the backslash is syntax and the character it protects is
            // content — a clipped "2\. Attention…" reads "2. Attention…" until the cursor
            // is on it (CommonMark's own verdict; the projection renderer folds it too)
            if (revealedEl(node.from, node.to)) break;
            hide(node.from, node.from + 1);
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
                widget: new ImageWidget(target, alt, width, opts.loadAsset, opts.copyImage, opts.openUrl, opts.remoteImages),
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
            // the space after the box is the box's own, like a bullet's (the list's hanging indent counts it in)
            const boxTo = doc.sliceString(node.to, node.to + 1) === " " ? node.to + 1 : node.to;
            const marker = doc.sliceString(node.from, node.to);
            const taskState: TaskState = stateOfChar(marker.length >= 3 ? marker[1] : " ");
            const closed = isClosed(taskState);
            const task = node.node.parent;
            if (closed && task && task.to > node.to + 1)
              marks.push({ from: node.to + 1, to: task.to, deco: doneDeco });
            // Only the task's **first line** is scanned: a Rust task block is a single
            // line (the parser recognises tasks line by line), and a marker on a lazy
            // continuation line is something the parser never reads, so drawing a chip
            // for it would display an attribute that does not exist in the index
            const lineEnd = doc.lineAt(node.from).to;
            const rest = doc.sliceString(node.to, task ? Math.min(task.to, lineEnd) : lineEnd);
            for (const am of scanTaskAttrs(rest)) {
              const aFrom = node.to + am.start;
              const aTo = node.to + am.end;
              // Wherever the source is on screen — the cursor in the chip, an attribute
              // the parser does not read, a malformed value — the emoji glyph itself is
              // still drawn as the icon with the attribute's name (markerAtom); only the
              // value is raw text
              const atom = () => {
                const a = markerAtom(am, rest.slice(am.start, am.end), aFrom, opts.onFieldChip);
                if (a) hides.push(a);
              };
              // A unit mark (a priority, 🚮, a marker still without its value) counts the
              // one space after it as its own: the cursor just past that space is at the
              // mark, the atom shows, and Backspace there takes the whole mark
              // (deleteTaskMarkerBackward) — otherwise the mark would flip between chip
              // and atom as the cursor crosses an invisible space
              const revealed = revealedEl(aFrom, isUnitMark(am) && rest[am.end] === " " ? aTo + 1 : aTo);
              if (!am.valid) {
                // A marker with nothing after it yet — the `📅 ` an `@due` pick leaves
                // while its date menu is open, a `⛔ ` awaiting the id to paste — draws as
                // the bare chip, the icon alone; a date's bare chip opens its menu like a
                // full one
                if (am.value === "" && am.kind !== "priority" && am.kind !== "delete") {
                  if (revealed) {
                    atom();
                    continue;
                  }
                  const isDate = am.kind in DATE_FIELDS;
                  const onChip = isDate ? chipHandler(am.kind, opts.onFieldChip) : undefined;
                  const title = isDate ? dateFieldLabel(am.kind as DateField) : attrChipParts(am).title;
                  hides.push({ from: aFrom, to: aTo, deco: Decoration.replace({ widget: new AttrChipWidget(am.kind, "", "", title, attrIconSvg(am.kind), onChip, "") }) });
                  continue;
                }
                // A malformed date (starting with a digit, or the explicit `[key:: …]`
                // syntax): the parser ignores it silently and the task vanishes from
                // Agenda with nothing to see — so a warning is drawn (on the cursor's
                // line too: a typo should be visible the moment it is made). A decorative
                // emoji (⏳ and the like) draws no warning: the parser is just as silent
                // about it, and a warning would only be a false alarm
                if (am.attempted) {
                  atom();
                  marks.push({ from: aFrom, to: aTo, deco: badDateDeco });
                }
                continue;
              }
              if (!am.active) {
                // Only the winner the parser actually read renders a chip (looking
                // exactly like Agenda's); a valid attribute it does not read — one in
                // the description, or the earlier of two of a kind — is ordinary text,
                // and a chip there would misreport the index, so it is underlined with
                // the reason instead
                atom();
                marks.push({ from: aFrom, to: aTo, deco: am.tail ? attrDuplicateDeco : attrOutsideDeco });
                continue;
              }
              // The source is only shown for editing when the cursor touches the chip's
              // range (clicking a chip folds to its boundary)
              if (revealed) {
                atom();
                continue;
              }
              if (am.kind in DATE_FIELDS) {
                const field = am.kind as DateField;
                const overdue = field === "due" && !closed && am.date! < today;
                // the dates a person sets open their menu on a click; a stamp does not
                const onChip = chipHandler(field, opts.onFieldChip);
                hides.push({
                  from: aFrom,
                  to: aTo,
                  deco: Decoration.replace({
                    widget: new AttrChipWidget(field, am.date!, overdue ? "cm-lp-datechip-overdue" : "", dateFieldLabel(field), attrIconSvg(field), onChip, am.date!),
                  }),
                });
                continue;
              }
              const c = attrChipParts(am);
              hides.push({ from: aFrom, to: aTo, deco: Decoration.replace({ widget: new AttrChipWidget(am.kind, c.text, c.cls, c.title, c.icon) }) });
            }
            if (revealedEl(bulletFrom, node.to)) {
              marks.push({ from: bulletFrom, to: boxTo, deco: taskSrcDeco });
              break;
            }
            hides.push({
              from: bulletFrom,
              to: boxTo,
              deco: Decoration.replace({ widget: new CheckboxWidget(taskState) }),
            });
            break;
          }
          case "ListItem": {
            // The hanging indent: the item's line starts with its marker in a box of the
            // marker's width, and the text of every later line of the item — wrapped or a
            // continuation — lines up at the box's edge. The source's own indentation is
            // not drawn (the depth is), so revealing a marker moves no text sideways
            const item = node.node;
            const mark = item.getChild("ListMark");
            if (!mark) break;
            let depth = 0;
            for (let p = item.parent; p; p = p.parent) if (p.name === "ListItem") depth++;
            const markText = doc.sliceString(mark.from, mark.to);
            const bullet = /^[-*+]$/.test(markText);
            const task = item.getChild("Task") != null;
            const num = bullet ? "" : numberBox(markText);
            const box = task ? (bullet ? TASK_BOX : `calc(${num} + ${TASK_BOX})`) : bullet ? BULLET_BOX : num;
            const markEnd = doc.sliceString(mark.to, mark.to + 1) === " " ? mark.to + 1 : mark.to;
            const first = doc.lineAt(mark.from);
            if (first.from >= range.from && first.from <= range.to) {
              addLine(first.from, liLine(depth, box, false), "li");
              if (mark.from > first.from && /^[ \t]+$/.test(doc.sliceString(first.from, mark.from))) hide(first.from, mark.from);
              // a task's box belongs to TaskMarker (a bullet task's box is the checkbox)
              if (!bullet) marks.push({ from: mark.from, to: markEnd, deco: numberMark(num) });
              else if (!task) {
                if (revealedEl(mark.from, mark.to)) marks.push({ from: mark.from, to: markEnd, deco: bulletMarkDeco });
                else hides.push({ from: mark.from, to: markEnd, deco: bulletDotDeco });
              }
            }
            for (let c = item.firstChild; c; c = c.nextSibling) {
              if (c.name !== "Paragraph" && c.name !== "Task") continue;
              const to = Math.min(c.to, range.to);
              for (let pos = Math.max(c.from, range.from); pos <= to; ) {
                const l = doc.lineAt(pos);
                if (l.from !== first.from) {
                  addLine(l.from, liLine(depth, box, true), "li");
                  const lead = /^[ \t]*/.exec(l.text)![0].length;
                  if (lead) hide(l.from, l.from + lead);
                }
                if (l.to >= to) break;
                pos = l.to + 1;
              }
            }
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
            const type = co[3].toLowerCase();
            const group = CALLOUT_GROUP[type] ?? "info";
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
                  widget: new CalloutIconWidget(type, group, title ? "" : calloutDefaultTitle(co[3])),
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
            // the cursor anywhere in the block shows its buttons (the pointer does too, codeHover)
            if (lineReveals.some(([a, b]) => a <= n.to && b >= n.from)) addLine(firstLine.from, codeCurLine, "cc");
            // A hidden fence line keeps only the block's inner margin (8px): the empty
            // full-height lines above and below the code read as a gap inside the block
            if (!revealedLine(firstLine.from, firstLine.to)) {
              hide(firstLine.from, firstLine.to);
              addLine(firstLine.from, codeFenceLine, "cf");
            }
            const lastLine = doc.lineAt(n.to);
            if (lastLine.from > firstLine.from) addLine(lastLine.from, codeLastLine, "cl");
            // The start of the line tolerates a quote marker (contentStart, the same rule
            // as the button anchor's retry): a fence from a clipped selection lives
            // inside a quote block, where the closing line is "> ```"
            if (
              lastLine.from > firstLine.from &&
              /^(`{3,}|~{3,})\s*$/.test(lastLine.text.slice(contentStart(lastLine.text))) &&
              !revealedLine(lastLine.from, lastLine.to)
            ) {
              hide(lastLine.from, lastLine.to);
              addLine(lastLine.from, codeFenceLine, "cf");
            }
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
              widget: new ImageWidget(target, target, width, opts.loadAsset, opts.copyImage, opts.openUrl, opts.remoteImages),
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

    // Formulas: `$…$` and the one-line `$$…$$` fold into typeset widgets (math.ts). The
    // lines of a multi-line display block are mathBlockField's and are skipped here —
    // its `$$` fences would otherwise read as inline delimiters once the block is
    // revealed
    if (text.includes("$")) {
      for (let pos = range.from; pos <= range.to; ) {
        const line = doc.lineAt(pos);
        if (line.text.includes("$") && !mathBlocks.some((b) => line.from >= b.from && line.to <= b.to)) {
          for (const f of scanInlineMath(line.text)) {
            const start = line.from + f.start;
            const end = line.from + f.end;
            if (isCode(start)) continue;
            if (revealedEl(start, end)) {
              marks.push({ from: start, to: end, deco: mathSrcDeco });
              continue;
            }
            hides.push({ from: start, to: end, deco: Decoration.replace({ widget: new MathWidget(f.tex, f.display) }) });
          }
        }
        if (line.to >= range.to) break;
        pos = line.to + 1;
      }
    }

    // A backslash escape inside a bare URL (`attention\_tf.py`: a clip's escaping of the
    // `_` in its text) is not an Escape node — the parser sees a URL — so it is folded
    // here; the click path drops the backslash the same way (mdlink.ts)
    if (CLIP_ESCAPE_TEST.test(text)) {
      BARE_URL_RE.lastIndex = 0;
      while ((m = BARE_URL_RE.exec(text))) {
        const start = range.from + m.index;
        if (isCode(start) || revealedEl(start, start + m[0].length)) continue;
        for (const e of m[0].matchAll(CLIP_ESCAPE_RE)) hide(start + e.index, start + e.index + 1);
      }
    }

    // `**…分析。**当然`: CJK punctuation inside the delimiter leaves the span unparsed
    // (CommonMark's flanking rule; new clips move the punctuation out, this reads what
    // is already on disk). A span the tree did not take, whose delimiter touches CJK
    // punctuation on the inside and a word on the outside, folds like the real one
    if (text.includes("*")) {
      CJK_EMPH_RE.lastIndex = 0;
      while ((m = CJK_EMPH_RE.exec(text))) {
        const start = range.from + m.index;
        const end = start + m[0].length;
        const run = m[1].length;
        // The character test first: it rejects every span in a document without CJK
        // punctuation, and the two tree walks below cost a resolveInner each
        const inner = m[2];
        const after = text[m.index + m[0].length] ?? "";
        const before = m.index > 0 ? text[m.index - 1] : "";
        const cjkTail = CJK_PUNCT.test(inner[inner.length - 1]) && after !== "" && !WORD_OUTSIDE.test(after);
        const cjkHead = CJK_PUNCT.test(inner[0]) && before !== "" && !WORD_OUTSIDE.test(before);
        if (!cjkTail && !cjkHead) continue;
        if (isCode(start)) continue;
        if (ancestorNamed(state, start + run, "Emphasis", "StrongEmphasis")) continue; // the parser took it: nothing to do here
        marks.push({ from: start + run, to: end - run, deco: run === 2 ? cjkStrongDeco : CJK_TEXT.test(inner) ? cjkEmDeco : latinEmDeco });
        if (revealedEl(start, end)) continue;
        hide(start, start + run);
        hide(end - run, end);
      }
    }

    // #tag chips (the cursor's line keeps the styling too; only decorations are added and no character is hidden)
    TAG_RE.lastIndex = 0;
    while ((m = TAG_RE.exec(text))) {
      if (/^[\d/]+$/.test(m[2])) continue; // digits and slashes alone are not a tag
      const start = range.from + m.index + m[1].length;
      if (isCode(start)) continue;
      marks.push({ from: start, to: start + 1 + m[2].length, deco: tagDeco });
      marks.push({ from: start, to: start + 1, deco: tagHashDeco });
    }

    // Where a block anchor is defined, "… ^id" is metadata a reader has no use for: on
    // the cursor's line it is a small badge (the marker a [[note#^id]] can cite, there to
    // be edited), elsewhere it folds, with its space, into a zero-width pin whose badge
    // shows over the line's end on hover — an imported paper has one on every paragraph.
    // The whitespace is limited to [ \t]: the parser requires a space before the ^
    ANCHOR_RE.lastIndex = 0;
    while ((m = ANCHOR_RE.exec(text))) {
      const start = range.from + m.index + 1; // the leading whitespace is not styled
      const end = start + 1 + m[1].length;
      if (isCode(start)) continue;
      if (revealedLine(start, end)) marks.push({ from: start, to: end, deco: anchorDeco });
      else hides.push({ from: start - 1, to: end, deco: Decoration.replace({ widget: new AnchorPinWidget(m[1]) }) });
    }

    // Line by line: the cursor's lines, and the blank lines that separate two text
    // lines, drawn as a paragraph gap. A blank line with the cursor on it keeps its full
    // height (the reader is about to type there); a run of several blank lines is left
    // as written; code, the frontmatter and a formula block's lines are not blank lines
    // of the prose
    const fmEndGap = frontmatterEnd(doc);
    for (let pos = range.from; pos <= range.to; ) {
      const line = doc.lineAt(pos);
      if (revealedLine(line.from, line.to)) addLine(line.from, curLineDeco, "cur");
      else if (
        line.number > 1 &&
        line.number < doc.lines &&
        line.from >= fmEndGap &&
        !line.text.trim() &&
        doc.line(line.number - 1).text.trim() &&
        doc.line(line.number + 1).text.trim() &&
        !isCode(line.from) &&
        !mathBlocks.some((b) => line.from >= b.from && line.to <= b.to)
      )
        addLine(line.from, gapLineDeco, "gap");
      if (line.to >= range.to) break;
      pos = line.to + 1;
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
            // The bracketed spelling of an attribute key IS the attribute and drew its chip
            // above; a bare `due:: 2026-08-20` is a field only — the parser reads no date out
            // of it (parser.rs's task_due_is_both_a_date_and_a_field) — so it keeps the
            // ordinary field chip rather than falling between the two loops and showing raw
            if (task && (f.form === "bracket" || f.form === "paren") && TASK_ATTR_KEYS.has(f.key.toLowerCase()) && f.value !== "") continue;
            if (revealedEl(f.from, f.to)) {
              marks.push({ from: f.from, to: f.to, deco: fieldSrcDeco });
              continue;
            }
            const state = f.value !== "" && opts.fieldState ? opts.fieldState(f.key, f.value) : null;
            hides.push({ from: f.from, to: f.to, deco: Decoration.replace({ widget: new FieldChipWidget(f.key, f.value, opts.onFieldChip, state) }) });
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
    if (key && !APP_KEYS.has(key[1])) keys.push(key[1]);
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

/** What a remote image in the note needs from its host (remoteimages.ts). */
export interface RemoteImageHost {
  /** The note's source page (frontmatter `source:`), sent as the Referer when one of its
   *  remote images loads, or "" for none. */
  referer: () => string;
  /** The note-wide choices behind a placeholder's ⋯ and its right-click: load every
   *  remote image in the note, save them into the vault, always load them. */
  menu: (e: MouseEvent, view: EditorView) => void;
}

export interface LivePreviewOptions {
  loadAsset?: AssetLoader;
  copyImage?: ImageCopier;
  /** A remote image that did not load: its placeholder opens the URL in the browser window. */
  openUrl?: (url: string) => void;
  /** A remote image not loaded yet: its placeholder's note-wide choices. */
  remoteImages?: RemoteImageHost;
  /** A click on a chip: a task's due/scheduled date, or any inline `key:: value` field. */
  onFieldChip?: FieldChipHandler;
  /** The step of a field's value when its key is a state field (fields.ts's stateStepOf
   *  over the vault's types): the chip then draws the glyph. Absent, no chip does. */
  fieldState?: (key: string, value: string) => StateStep | null;
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

/** A code block's copy and run buttons are not drawn over the code while nobody is
 *  using them: they show while the cursor is in the block (cm-lp-code-cur) and while the
 *  pointer is over any of its lines — this plugin. The lines of a block are consecutive
 *  siblings, so the head is the nearest line above that carries cm-lp-code-head. The
 *  mark goes on the buttons (widget DOM, which CodeMirror leaves alone), not on the line,
 *  whose attributes it rewrites on every sync. */
const codeHover = ViewPlugin.fromClass(
  class {
    hot: Element | null = null;
    /** The line last looked up: a move within one line settles nothing new. */
    over: Element | null = null;
    constructor(readonly view: EditorView) {}
    set(head: Element | null) {
      if (head === this.hot) return;
      for (const b of this.hot?.querySelectorAll(".cm-lp-code-btn") ?? []) b.classList.remove("cm-lp-hot");
      this.hot = head;
      for (const b of head?.querySelectorAll(".cm-lp-code-btn") ?? []) b.classList.add("cm-lp-hot");
    }
    destroy() {
      this.set(null);
    }
  },
  {
    eventHandlers: {
      mousemove(e) {
        let line = e.target instanceof Element ? e.target.closest(".cm-line") : null;
        if (line === this.over) return;
        this.over = line;
        if (line && !line.classList.contains("cm-lp-code")) line = null;
        while (line && !line.classList.contains("cm-lp-code-head")) {
          const prev: Element | null = line.previousElementSibling;
          line = prev?.classList.contains("cm-lp-code") ? prev : null;
        }
        this.set(line);
      },
      mouseleave() {
        this.over = null;
        this.set(null);
      },
    },
  },
);

export function livePreview(opts: LivePreviewOptions = {}): Extension {
  // revealArmed is registered here (rather than in setup.ts): every consumer of the
  // reveal predicates (livepreview/table/queryblock/frontmatter/clickopen) lives in
  // note mode alone, and note mode always carries livePreview()
  return [revealArmed, livePreviewPlugin(opts), compositionRelease, frontmatterField, tableField, mathBlockField, emptyFieldCloser(opts.onFieldChip), codeHover];
}
