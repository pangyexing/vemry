// Cross-note embeds: `![[Note]]`, `![[Note#Heading]]`, `![[Note#^id]]` render the
// target's current text read-only inside the editor — a live projection of the
// source, nothing copied into this file. Alone on its line an embed is a block
// widget with a header that jumps to the source; inside a line it is a compact
// chip. Depth is structurally one: the body is rendered by renderNoteMarkdown,
// which folds nested `![[…]]` into ⧉ links, so an embed can never loop; an embed
// whose source contains this very line renders a marker instead. Content is
// resolved through `resolve_ref` (UI semantics: private notes are not
// filtered, like every view) and cached per index version; heights are cached
// per target so a widget mounts at its last size and scrolling does not jump.
// A widget is equal to itself shifted by an edit elsewhere (eq ignores where it
// stands), so typing above an embed keeps its DOM; whatever needs its position
// reads it at event time with posAtDOM. It is equal to itself across index versions
// too: a refresh re-resolves inside the mounted widget and swaps a changed body in
// whole, with its images already loaded (blockwidget.ts, the live half). The body's
// words are selectable; opening the source is the header row's.
import { t } from "$lib/i18n/i18n.svelte";
import { blockRoot, liveRefresh, queryRefresh, recordHeight, selectableIsland, selectedWithin, swapInPlace, unwatchLive, watchLive } from "./blockwidget";
import { focusEditor } from "./clickguard";
import { syntaxTree } from "@codemirror/language";
import { type EditorState, type Extension, type Line, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import type { RefResolution } from "$lib/api";
import { handleCodeCopyClick, renderNoteMarkdown } from "$lib/markdown";
import { byteToChar } from "$lib/bytes";
import { stripAnchors } from "$lib/fields";
import { IMAGE_EXT_RE, imageDimsOf, inCode, noteImageDims } from "./livepreview";
import { openImageLightbox } from "$lib/lightbox";
import { applyLangAttrs, type NoteLanguage } from "$lib/notelang";
import { selectionReveals, selectionTouches } from "./reveal";
import { WIKI_RE_SOURCE } from "./wikilink";

export interface EmbedHost {
  /** `resolve_ref(target, currentNote)`. */
  resolve(target: string): Promise<RefResolution>;
  follow(target: string): void;
  openUrl(url: string): void;
  openBlockAt(note: string, byteStart: number, content?: string): void;
  currentNote(): string | null;
  /** A vault image's path → a loadable URL, the editor's own asset loader; without
   *  one, the note's images show as their alt. */
  loadAsset?(target: string): Promise<string | null>;
  /** What a note reads as, from the index (NoteSummary.lang/script); absent or null,
   *  the body follows the note it is shown in. */
  langOf?(note: string): NoteLanguage | null;
}

const NO_LANG: NoteLanguage = { lang: null, script: null };

export const EMBED_RE = new RegExp("!" + WIKI_RE_SOURCE, "g");

/** The inside of `![[…]]`: the target and an optional label (a purely numeric
 *  suffix is the image-width syntax and means nothing on a note embed). */
export function parseEmbed(inner: string): { target: string; label: string | null } {
  const pipe = inner.lastIndexOf("|");
  if (pipe < 0) return { target: inner.trim(), label: null };
  const label = inner.slice(pipe + 1).trim();
  return { target: inner.slice(0, pipe).trim(), label: /^\d+$/.test(label) || !label ? null : label };
}

/** The embed at pos, if any (its full inner text, unlike wikiLinkAt's target). */
export function embedAt(state: EditorState, pos: number): { inner: string; from: number; to: number } | null {
  const line = state.doc.lineAt(pos);
  EMBED_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = EMBED_RE.exec(line.text))) {
    const from = line.from + m.index;
    const to = from + m[0].length;
    if (pos >= from && pos <= to) return { inner: m[1], from, to };
  }
  return null;
}

interface CacheEntry {
  version: number;
  promise: Promise<RefResolution>;
  value?: RefResolution;
}

const resolved = new Map<string, CacheEntry>();
const heights = new Map<string, number>();

/** A placeholder body's height, held until the real one is measured: the widget's last
 *  measured height less the chrome (the crumb row and the borders) that sits outside
 *  the body. Both placeholders — the unresolved one and the one waiting on its images —
 *  reserve the same space. */
const bodyMinHeight = (known: number) => `${Math.max(0, known - 40)}px`;

/** Invalidated when the vault changes (the same trap as the image size cache). */
export function clearEmbedCaches() {
  resolved.clear();
  heights.clear();
}

function lookup(host: EmbedHost, target: string, version: number): CacheEntry {
  const key = `${host.currentNote() ?? ""}|${target}`;
  const hit = resolved.get(key);
  if (hit && hit.version === version) return hit;
  const entry: CacheEntry = { version, promise: host.resolve(target) };
  entry.promise.then((v) => void (entry.value = v)).catch(() => undefined);
  resolved.set(key, entry);
  return entry;
}

/** Jump to an embed's source (Mod-Enter on the line, the chip, the header). */
export async function openEmbedSource(host: EmbedHost, inner: string): Promise<void> {
  const { target } = parseEmbed(inner);
  const r = await host.resolve(target);
  if (r.state === "resolved") host.openBlockAt(r.note, r.span.start, r.content);
}

/** The slice as it is shown: trailing block anchors dropped (an id is an internal
 *  name), and a heading target loses its first line (the header row names it). */
function prepareSlice(r: RefResolution & { state: "resolved" }): string {
  let text = stripAnchors(r.content);
  if (r.target.kind === "heading") text = text.split("\n").slice(1).join("\n");
  return text;
}

function crumbOf(target: string, label: string | null, r?: RefResolution): { title: string; sep: string; sub: string } {
  const hash = target.indexOf("#");
  const note = hash >= 0 ? target.slice(0, hash) : target;
  const frag = hash >= 0 ? target.slice(hash + 1) : "";
  const stem = note.split("/").pop() || t("embed.thisNote");
  const title = r && r.state === "resolved" ? r.note_title : label ?? stem;
  if (frag.startsWith("^")) return { title, sep: "¶", sub: "" };
  if (frag) return { title, sep: "›", sub: r && r.state === "resolved" && r.target.kind === "heading" ? r.target.text : frag };
  return { title, sep: "", sub: "" };
}

function wire(view: EditorView, el: HTMLElement, host: EmbedHost, inner: string) {
  el.addEventListener("mousedown", (e) => {
    const target = e.target as HTMLElement;
    // The body's words can be selected (a selectable island): its press is the
    // browser's, and what it holds — a link, an image — acts on the click
    if (target.closest(".cm-embed-body")) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.button !== 0) return;
    if (target.closest(".cm-embed-edit")) {
      // the widget's current line: it outlives position shifts, so a position baked
      // in at creation would go stale
      view.dispatch({ selection: { anchor: view.posAtDOM(el) } });
      focusEditor(view);
      return;
    }
    // Opening is the header row's alone (and the chip's, which is all header)
    if (target.closest(".cm-embed-head, .cm-embed-chip")) void openEmbedSource(host, inner);
  });
  el.addEventListener("click", (e) => {
    if (handleCodeCopyClick(e) || e.button !== 0) return;
    const target = e.target as HTMLElement;
    const body = target.closest<HTMLElement>(".cm-embed-body");
    // a press that selected words is a selection, not a click on what it started on
    if (!body || selectedWithin(body)) return;
    const wiki = target.closest<HTMLElement>("a[data-wiki]");
    if (wiki?.dataset.wiki) {
      host.follow(wiki.dataset.wiki);
      return;
    }
    const url = target.closest<HTMLElement>("a[data-url]");
    if (url?.dataset.url) {
      host.openUrl(url.dataset.url);
      return;
    }
    const img = target.closest("img");
    if (img?.src) openImageLightbox(img.src, img.alt);
  });
}

/** Load a rendered body's own images (a paper's figure, an embedded diagram) the way
 *  the editor's image widget loads them — a blob URL from the vault file, the size it is
 *  known to have set before it arrives — and settle once every one has loaded or been
 *  replaced by its chip. A rejected loader settles too: a count that never reached zero
 *  would leave the body reserving a figure's height for good. */
function loadImages(body: HTMLElement, host: EmbedHost): Promise<void> {
  const imgs = [...body.querySelectorAll<HTMLImageElement>("img[data-asset]")];
  return Promise.all(
    imgs.map((img) => {
      const target = img.dataset.asset ?? "";
      const dims = imageDimsOf(target);
      if (dims) {
        img.width = dims.w;
        img.height = dims.h;
      }
      const url = host.loadAsset?.(target) ?? Promise.resolve(null);
      return url
        .catch(() => null)
        .then(
          (u) =>
            new Promise<void>((done) => {
              if (!u) {
                img.replaceWith(`⧉ ${img.alt || target}`);
                return done();
              }
              img.addEventListener(
                "load",
                () => {
                  noteImageDims(target, img.naturalWidth, img.naturalHeight);
                  done();
                },
                { once: true },
              );
              img.addEventListener("error", () => done(), { once: true });
              img.src = u;
            }),
        );
    }),
  ).then(() => undefined);
}

/** What a resolution shows: a marker line (the source gone, the anchor gone, the embed
 *  inside its own source), or the rendered slice. */
type Shown = { kind: "marker"; cls: string; text: string } | { kind: "body"; html: string; lang: NoteLanguage };

class EmbedWidget extends WidgetType {
  /** The height cache's key: the embed text within this note, wherever it stands. */
  readonly key: string;
  constructor(
    readonly inner: string,
    /** Where the embed stood when built: the first fill's recursion check (a refresh
     *  reads where the mounted widget stands now). */
    readonly lineFrom: number,
    readonly host: EmbedHost,
    /** The index version the editor last announced, read at mount. */
    readonly versionOf: (state: EditorState) => number,
  ) {
    super();
    this.key = `${host.currentNote() ?? ""}|${inner}`;
  }
  eq(other: EmbedWidget) {
    // not lineFrom: the same embed shifted by an edit elsewhere keeps its DOM; and not
    // the index version: a refresh re-resolves inside the mounted widget (watchLive)
    return other.inner === this.inner;
  }
  get estimatedHeight() {
    return heights.get(this.key) ?? 72;
  }
  ignoreEvent() {
    return true;
  }
  destroy(dom: HTMLElement) {
    unwatchLive(dom);
  }

  /** `at`: where the embed stands now (the widget outlives edits above it). */
  private shownOf(view: EditorView, r: RefResolution, at: number): Shown {
    if (r.state === "note_missing") return { kind: "marker", cls: "cm-embed-missing", text: t("embed.noteNotFound") };
    if (r.state === "anchor_missing") return { kind: "marker", cls: "cm-embed-missing", text: t("embed.notFound") };
    const self = this.host.currentNote();
    let recursive = false;
    if (self && r.note === self) {
      if (r.target.kind === "note") recursive = true;
      else {
        const text = view.state.doc.toString();
        const from = byteToChar(text, r.span.start);
        const to = byteToChar(text, r.span.end);
        recursive = at >= from && at <= to;
      }
    }
    if (recursive) return { kind: "marker", cls: "cm-embed-recursive", text: t("embed.recursive") };
    // The source note's language, as the index read it: the embed's fonts and line
    // breaking follow the note the words come from, not the one they are shown in
    return { kind: "body", html: renderNoteMarkdown(prepareSlice(r)), lang: this.host.langOf?.(r.note) ?? NO_LANG };
  }

  toDOM(view: EditorView) {
    const { target, label } = parseEmbed(this.inner);
    const wrap = document.createElement("div");
    wrap.className = "cm-embed";
    const root = blockRoot(wrap);
    const head = document.createElement("div");
    head.className = "cm-embed-head";
    const title = document.createElement("span");
    title.className = "cm-embed-title";
    const sep = document.createElement("span");
    sep.className = "cm-embed-sep";
    const sub = document.createElement("span");
    sub.className = "cm-embed-sub";
    const open = document.createElement("button");
    open.className = "cm-embed-open";
    open.title = t("embed.open");
    open.textContent = "↗";
    const edit = document.createElement("button");
    edit.className = "cm-embed-edit";
    edit.title = t("embed.edit");
    edit.textContent = "✎";
    head.append(title, sep, sub, open, edit);
    let body: HTMLElement = document.createElement("div");
    body.className = "cm-embed-body md-body";
    selectableIsland(body);
    const note = document.createElement("div");
    note.className = "cm-embed-note";
    note.textContent = t("embed.loading");
    wrap.append(head, body, note);
    const setCrumb = (r?: RefResolution) => {
      const c = crumbOf(target, label, r);
      title.textContent = c.title;
      sep.textContent = c.sep;
      sub.textContent = c.sub;
      sep.hidden = !c.sep;
      sub.hidden = !c.sub;
    };
    setCrumb();
    wire(view, wrap, this.host, this.inner);

    /** The resolution on screen, compared whole: an unchanged answer leaves the DOM alone. */
    let shownSig = "";
    /** Put a resolution on screen: the crumb, the marker or the body (`next`, already
     *  rendered, when the refresh prepared it off the page). */
    const paint = (r: RefResolution, shown: Shown, next?: HTMLElement) => {
      setCrumb(r);
      wrap.classList.remove("cm-embed-missing", "cm-embed-recursive");
      if (shown.kind === "marker") {
        wrap.classList.add(shown.cls);
        body.replaceChildren();
        note.textContent = shown.text;
        if (!note.isConnected) wrap.appendChild(note);
        return;
      }
      if (next) {
        body.replaceWith(next);
        body = next;
      } else body.innerHTML = shown.html;
      applyLangAttrs(body, shown.lang);
      note.remove();
    };

    // The first fill. Until a body's images have loaded it keeps the embed's known
    // height: a widget scrolled back into view is built afresh, and measured before its
    // figure arrives it is a fraction of its size — the height map shrinks by the
    // difference, and with the view at the note's end the scroll offset is clamped past
    // the document, where an empty viewport renders nothing and so never measures the
    // figure again (the note stays blank until a scroll)
    const fill = (r: RefResolution) => {
      const shown = this.shownOf(view, r, this.lineFrom);
      shownSig = JSON.stringify(r);
      body.style.minHeight = "";
      paint(r, shown);
      if (shown.kind !== "body" || !body.querySelector("img[data-asset]")) return;
      const known = heights.get(this.key);
      if (known) body.style.minHeight = bodyMinHeight(known);
      const filled = body;
      void loadImages(filled, this.host).then(() => {
        filled.style.minHeight = "";
        if (!root.isConnected) return;
        heights.set(this.key, root.offsetHeight);
        view.requestMeasure();
      });
    };

    let loaded = this.versionOf(view.state);
    const entry = lookup(this.host, target, loaded);
    if (entry.value) {
      // A cached resolution (the target embedded twice, a widget scrolled back into the
      // viewport, a re-mounted tab): fill the DOM now, before CodeMirror mounts it. No
      // placeholder, and no isConnected guard here — the element is not attached yet,
      // and a guard at this point would leave the placeholder up for good.
      fill(entry.value);
      // The cache is what a rebuilt widget estimates its height from before it is measured
      // (a vault switch, a tab re-mounted): filled here as well, once the DOM is attached,
      // or an embed of 1800px estimates 72 and the height map above the viewport shrinks
      // by the difference — with the view scrolled below it, past the document
      recordHeight(view, root, heights, this.key);
    } else {
      const known = heights.get(this.key);
      if (known) body.style.minHeight = bodyMinHeight(known);
      const finish = (r: RefResolution) => {
        // dropped before the source arrived, or a refresh's newer answer already painted
        if (!wrap.isConnected || shownSig) return;
        swapInPlace(view, root, () => fill(r));
        heights.set(this.key, root.offsetHeight);
      };
      entry.promise.then(finish, () => finish({ state: "note_missing" }));
    }

    // A refresh: re-resolved in the background with the old body on screen; the same
    // answer changes nothing, a new one is rendered off the page, its images loaded, and
    // swapped in at once — never a "loading" line, never a figure-less body
    let seq = 0;
    watchLive(view, root, (version) => {
      if (version === loaded) return; // announced before this widget mounted: already current
      loaded = version;
      const my = ++seq;
      const settle = async (r: RefResolution) => {
        if (my !== seq || !root.isConnected) return;
        const sig = JSON.stringify(r);
        if (sig === shownSig) return;
        const shown = this.shownOf(view, r, view.posAtDOM(root));
        let next: HTMLElement | undefined;
        if (shown.kind === "body") {
          next = document.createElement("div");
          next.className = body.className;
          selectableIsland(next);
          next.innerHTML = shown.html;
          await loadImages(next, this.host);
          if (my !== seq || !root.isConnected) return;
        }
        shownSig = sig;
        swapInPlace(view, root, () => paint(r, shown, next));
        recordHeight(view, root, heights, this.key);
      };
      lookup(this.host, target, version).promise.then(settle, () => settle({ state: "note_missing" }));
    });
    return root;
  }
}

class EmbedChipWidget extends WidgetType {
  constructor(
    readonly inner: string,
    readonly host: EmbedHost,
    readonly versionOf: (state: EditorState) => number,
  ) {
    super();
  }
  eq(other: EmbedChipWidget) {
    return other.inner === this.inner;
  }
  ignoreEvent() {
    return true;
  }
  destroy(dom: HTMLElement) {
    unwatchLive(dom);
  }
  toDOM(view: EditorView) {
    const { target, label } = parseEmbed(this.inner);
    const el = document.createElement("span");
    el.className = "cm-embed-chip";
    const c = crumbOf(target, label);
    el.textContent = `⧉ ${c.title}${c.sep ? ` ${c.sep} ${c.sub}` : ""}`.trim();
    el.title = t("embed.open");
    wire(view, el, this.host, this.inner);
    const mark = (r: RefResolution) => {
      el.classList.toggle("cm-embed-chip-missing", r.state !== "resolved");
      const cc = crumbOf(target, label, r);
      el.textContent = `⧉ ${cc.title}${cc.sep ? ` ${cc.sep} ${cc.sub}` : ""}`.trim();
    };
    let loaded = this.versionOf(view.state);
    const entry = lookup(this.host, target, loaded);
    // a cached resolution is applied before the chip is mounted (no isConnected
    // guard, as for EmbedWidget); a late one only if the chip is still there
    if (entry.value) mark(entry.value);
    else entry.promise.then((r) => void (el.isConnected && mark(r))).catch(() => undefined);
    watchLive(view, el, (version) => {
      if (version === loaded) return;
      loaded = version;
      lookup(this.host, target, version)
        .promise.then((r) => void (el.isConnected && mark(r)))
        .catch(() => undefined);
    });
    return el;
  }
}

interface BuildResult {
  decos: DecorationSet;
  hasEmbed: boolean;
}

function build(state: EditorState, host: EmbedHost, versionOf: (state: EditorState) => number): BuildResult {
  const doc = state.doc;
  const decos: ReturnType<Decoration["range"]>[] = [];
  let hasEmbed = false;
  const text = doc.toString();
  EMBED_RE.lastIndex = 0;
  let m: RegExpExecArray | null;
  while ((m = EMBED_RE.exec(text))) {
    const start = m.index;
    const end = start + m[0].length;
    const { target } = parseEmbed(m[1]);
    if (IMAGE_EXT_RE.test(target.split("#")[0])) continue;
    if (inCode(state, start + 1)) continue;
    hasEmbed = true;
    const line: Line = doc.lineAt(start);
    if (line.text.trim() === m[0]) {
      if (selectionTouches(state, line.from, line.to)) continue;
      decos.push(Decoration.replace({ widget: new EmbedWidget(m[1], line.from, host, versionOf), block: true }).range(line.from, line.to));
    } else {
      if (selectionReveals(state, start, end)) continue;
      decos.push(Decoration.replace({ widget: new EmbedChipWidget(m[1], host, versionOf) }).range(start, end));
    }
  }
  return { decos: Decoration.set(decos, true), hasEmbed };
}

/** The StateField (a block-level replace may not come from a ViewPlugin). Shares
 *  queryRefresh with the query block: an index version bump re-resolves the mounted
 *  widgets in place, and the field only records the version for later mounts. */
export function embedExtension(host: EmbedHost): Extension {
  const field: StateField<{ version: number } & BuildResult> = StateField.define({
    create: (state) => ({ version: 0, ...build(state, host, versionOf) }),
    update(value, tr) {
      let version = value.version;
      for (const e of tr.effects) if (e.is(queryRefresh)) version = e.value;
      const treeChanged = syntaxTree(tr.state) !== syntaxTree(tr.startState);
      if ((!tr.docChanged && !tr.selection && !treeChanged) || (!value.hasEmbed && !tr.docChanged && !treeChanged)) {
        return version === value.version ? value : { ...value, version };
      }
      return { version, ...build(tr.state, host, versionOf) };
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });
  function versionOf(state: EditorState): number {
    return state.field(field, false)?.version ?? 0;
  }
  return [field, liveRefresh];
}
