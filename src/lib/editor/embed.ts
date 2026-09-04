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
// reads it at event time with posAtDOM.
import { t } from "$lib/i18n/i18n.svelte";
import { blockRoot } from "./blockwidget";
import { focusEditor } from "./clickguard";
import { syntaxTree } from "@codemirror/language";
import { type EditorState, type Extension, type Line, StateField } from "@codemirror/state";
import { Decoration, type DecorationSet, EditorView, WidgetType } from "@codemirror/view";
import type { RefResolution } from "$lib/api";
import { handleCodeCopyClick, renderNoteMarkdown } from "$lib/markdown";
import { byteToChar } from "$lib/bytes";
import { IMAGE_EXT_RE, inCode } from "./livepreview";
import { queryRefresh } from "./queryblock";
import { selectionReveals, selectionTouches } from "./reveal";
import { WIKI_RE_SOURCE } from "./wikilink";

export interface EmbedHost {
  /** `resolve_ref(target, currentNote)`. */
  resolve(target: string): Promise<RefResolution>;
  follow(target: string): void;
  openUrl(url: string): void;
  openBlockAt(note: string, byteStart: number, content?: string): void;
  currentNote(): string | null;
}

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
  let text = r.content.replace(/[ \t]\^[A-Za-z0-9-]+$/gm, "");
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
    if (target.closest(".cm-embed-head, .cm-embed-chip")) {
      void openEmbedSource(host, inner);
      return;
    }
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
  });
  el.addEventListener("click", (e) => {
    handleCodeCopyClick(e);
  });
}

class EmbedWidget extends WidgetType {
  /** The height cache's key: the embed text within this note, wherever it stands. */
  readonly key: string;
  constructor(
    readonly inner: string,
    /** Where the embed stood when built; only the recursion check reads it (an
     *  edit that changes the answer also bumps the index version and rebuilds). */
    readonly lineFrom: number,
    readonly version: number,
    readonly host: EmbedHost,
  ) {
    super();
    this.key = `${host.currentNote() ?? ""}|${inner}`;
  }
  eq(other: EmbedWidget) {
    // not lineFrom: the same embed shifted by an edit elsewhere keeps its DOM
    return other.inner === this.inner && other.version === this.version;
  }
  get estimatedHeight() {
    return heights.get(this.key) ?? 72;
  }
  ignoreEvent() {
    return true;
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
    const body = document.createElement("div");
    body.className = "cm-embed-body md-body";
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
    const fill = (r: RefResolution) => {
      setCrumb(r);
      body.style.minHeight = "";
      if (r.state === "note_missing") {
        wrap.classList.add("cm-embed-missing");
        note.textContent = t("embed.noteNotFound");
      } else if (r.state === "anchor_missing") {
        wrap.classList.add("cm-embed-missing");
        note.textContent = t("embed.notFound");
      } else {
        const self = this.host.currentNote();
        let recursive = false;
        if (self && r.note === self) {
          if (r.target.kind === "note") recursive = true;
          else {
            const text = view.state.doc.toString();
            const from = byteToChar(text, r.span.start);
            const to = byteToChar(text, r.span.end);
            recursive = this.lineFrom >= from && this.lineFrom <= to;
          }
        }
        if (recursive) {
          wrap.classList.add("cm-embed-recursive");
          note.textContent = t("embed.recursive");
        } else {
          body.innerHTML = renderNoteMarkdown(prepareSlice(r));
          note.remove();
        }
      }
    };
    const entry = lookup(this.host, target, this.version);
    if (entry.value) {
      // A cached resolution (the target embedded twice, a widget scrolled back into the
      // viewport, a re-mounted tab): fill the DOM now, before CodeMirror mounts it. No
      // placeholder, and no isConnected guard here — the element is not attached yet,
      // and a guard at this point would leave the placeholder up for good.
      fill(entry.value);
    } else {
      const known = heights.get(this.key);
      if (known) body.style.minHeight = `${Math.max(0, known - 40)}px`;
      const finish = (r: RefResolution) => {
        if (!wrap.isConnected) return; // dropped before the source arrived
        fill(r);
        heights.set(this.key, root.offsetHeight);
        view.requestMeasure();
      };
      entry.promise.then(finish, () => finish({ state: "note_missing" }));
    }
    return root;
  }
}

class EmbedChipWidget extends WidgetType {
  constructor(
    readonly inner: string,
    readonly version: number,
    readonly host: EmbedHost,
  ) {
    super();
  }
  eq(other: EmbedChipWidget) {
    return other.inner === this.inner && other.version === this.version;
  }
  ignoreEvent() {
    return true;
  }
  toDOM(view: EditorView) {
    const { target, label } = parseEmbed(this.inner);
    const el = document.createElement("span");
    el.className = "cm-embed-chip";
    const c = crumbOf(target, label);
    el.textContent = `⧉ ${c.title}${c.sep ? ` ${c.sep} ${c.sub}` : ""}`.trim();
    el.title = t("embed.open");
    wire(view, el, this.host, this.inner);
    const entry = lookup(this.host, target, this.version);
    const mark = (r: RefResolution) => {
      if (r.state !== "resolved") el.classList.add("cm-embed-chip-missing");
      else {
        const cc = crumbOf(target, label, r);
        el.textContent = `⧉ ${cc.title}${cc.sep ? ` ${cc.sep} ${cc.sub}` : ""}`.trim();
      }
    };
    // a cached resolution is applied before the chip is mounted (no isConnected
    // guard, as for EmbedWidget); a late one only if the chip is still there
    if (entry.value) mark(entry.value);
    else entry.promise.then((r) => void (el.isConnected && mark(r))).catch(() => undefined);
    return el;
  }
}

interface BuildResult {
  decos: DecorationSet;
  hasEmbed: boolean;
}

function build(state: EditorState, version: number, host: EmbedHost): BuildResult {
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
      decos.push(Decoration.replace({ widget: new EmbedWidget(m[1], line.from, version, host), block: true }).range(line.from, line.to));
    } else {
      if (selectionReveals(state, start, end)) continue;
      decos.push(Decoration.replace({ widget: new EmbedChipWidget(m[1], version, host) }).range(start, end));
    }
  }
  return { decos: Decoration.set(decos, true), hasEmbed };
}

/** The StateField (a block-level replace may not come from a ViewPlugin). Shares
 *  queryRefresh with the query block: an index version bump re-resolves. */
export function embedExtension(host: EmbedHost): Extension {
  return StateField.define<{ version: number } & BuildResult>({
    create: (state) => ({ version: 0, ...build(state, 0, host) }),
    update(value, tr) {
      let version = value.version;
      let refreshed = false;
      for (const e of tr.effects) {
        if (e.is(queryRefresh)) {
          version = e.value;
          refreshed = true;
        }
      }
      const treeChanged = syntaxTree(tr.state) !== syntaxTree(tr.startState);
      if (!tr.docChanged && !tr.selection && !treeChanged && !refreshed) return value;
      if (!value.hasEmbed && !tr.docChanged && !treeChanged) {
        return version === value.version ? value : { ...value, version };
      }
      return { version, ...build(tr.state, version, host) };
    },
    provide: (f) => EditorView.decorations.from(f, (v) => v.decos),
  });
}
