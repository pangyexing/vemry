// The Markdown subset renderer of the AI chat panel. Written by hand rather than
// taken from a library: chat only needs a subset (paragraphs, headings, lists,
// quotes, code, tables, bold and italic, links), and the renderer itself is the XSS
// trust surface — every piece of text is escaped first and only the tags generated
// here are emitted; a link is only recognised as http(s) or a [[wikilink]] and is
// handed to the panel's click delegation through data-url / data-wiki;
// an image folds into a placeholder chip that makes no request (the main webview
// makes zero remote requests, the same fallback shape the reading projection uses).
// The one library exception: typesetting formulas is delegated to a local KaTeX
// (renderMathHtml, called only on the $…$ fragments recognised, with trust:false) —
// TeX typesetting is not realistically something to write by hand.
//
// Citation numbering: a `[title](url)` or a bare URL in the body folds into an inline
// marker whose number comes from extractCitations' document order — the source cards
// use the same extraction, so the two always agree.
// The model likes writing its sources as standalone list items — when a line holds
// nothing but a link the marker keeps the title text, and the block level then merges
// a pure-source item back into the previous one rather than rendering a lone bullet
// holding a single number.

import { t } from "$lib/i18n/i18n.svelte";
import { convertFileSrc } from "@tauri-apps/api/core";
import katex from "katex";
import { toast } from "$lib/stores/toast.svelte";
import { CALLOUT_GROUP, CALLOUT_HEAD_RE } from "$lib/callout";
import { scanFields } from "$lib/fields";

export interface Citation {
  title: string;
  url: string;
}

// The one place the link and image regexes are built: the label tolerates \[ \]
// escapes, and after the URL a trailing "title" is tolerated (the model writes both
// often, and without tolerance the whole run would show as source). The four variants
// differ only in the leading ! of an image and in the target subpattern — changing
// how tolerant the boundaries are touches this one place.
const linkRe = (image: boolean, target: string) =>
  new RegExp(
    `${image ? "!" : ""}\\[((?:\\\\.|[^\\]\\\\])*)\\]\\(\\s*(${target})(?:\\s+(?:"[^"]*"|'[^']*'))?\\s*\\)`,
    "g",
  );
// The chat target: absolute http(s) only; one level of paired parentheses is
// tolerated inside the URL (a Wikipedia disambiguation page,
// /wiki/Rust_(programming_language)), or both the marker and the source card would
// hold half a URL
const HTTP_TARGET = String.raw`https?:\/\/(?:[^\s()]|\([^\s()]*\))+`;
// The reading projection's target: a page body commonly holds relative paths, so any target is taken, resolved against the page URL first and then filtered by the allowlist
const ANY_TARGET = String.raw`(?:[^)\s(]|\([^)\s]*\))+?`;
const MD_LINK = linkRe(false, HTTP_TARGET);
// An image reference in chat: rendered as a placeholder chip rather than an <img>, and it does not enter the source cards (an image is not a citation source)
const MD_IMAGE = linkRe(true, HTTP_TARGET);
// A bare URL: ASCII URL characters only — the URLs a model outputs are in fact
// percent-encoded ASCII, and opening this up would swallow the CJK sentence that
// follows; the lookbehind avoids a Markdown link's "(", and trailing punctuation is
// removed by strip. The editor's URL_RE for ⌘click (mdlink.ts) faces URLs pasted by
// hand and unencoded and is deliberately wider — a change to the boundary rule goes
// through both
const BARE_URL = /(?<![([])https?:\/\/(?:[A-Za-z0-9\-._~:/?#@!$&*+,;=%]|\([A-Za-z0-9\-._~:/?#@!$&*+,;=%]*\))+/g;

/** A bare URL is often ended by punctuation, CJK or otherwise — which is stripped.
 *  The chat markers and source cards share this URL boundary rule with the editor's
 *  ⌘click (mdlink.ts). */
export function stripTrailingPunct(url: string): string {
  const punct = /[)）\]】』」》>"'.,，。;；:：!！?？]/;
  let end = url.length;
  while (end > 0 && punct.test(url[end - 1])) {
    // A trailing ')' of a Wikipedia-style URL is content rather than punctuation: it is kept when the prefix still holds an unmatched '('
    if (url[end - 1] === ")") {
      const prefix = url.slice(0, end - 1);
      const opens = (prefix.match(/\(/g) ?? []).length;
      const closes = (prefix.match(/\)/g) ?? []).length;
      if (opens > closes) break;
    }
    end--;
  }
  return url.slice(0, end);
}

export function domainOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return url;
  }
}

/** Extract the body's source links (Markdown links plus bare URLs) in document order,
 *  deduplicating by URL and keeping the first non-empty title. The marker numbering
 *  and the source cards share this one ordering. */
export function extractCitations(text: string): Citation[] {
  const hits: { index: number; title: string; url: string }[] = [];
  // An image is not a source: MD_IMAGE and MD_LINK come from the same builder and
  // differ only in the leading !, so a link match preceded by a ! is an image — it
  // takes up the range so the bare-URL scan skips it and it never enters a source card
  const linkSpans: [number, number][] = [];
  const inside = (spans: [number, number][], at: number) => spans.some(([s, e]) => at >= s && at < e);
  for (const m of text.matchAll(MD_LINK)) {
    const at = m.index ?? 0;
    if (at > 0 && text[at - 1] === "!") {
      linkSpans.push([at - 1, at + m[0].length]);
      continue;
    }
    hits.push({ index: at, title: plainLabel(m[1]), url: stripTrailingPunct(m[2]) });
    linkSpans.push([at, at + m[0].length]);
  }
  for (const m of text.matchAll(BARE_URL)) {
    const at = m.index ?? 0;
    if (inside(linkSpans, at)) continue;
    hits.push({ index: at, title: "", url: stripTrailingPunct(m[0]) });
  }
  hits.sort((a, b) => a.index - b.index);
  const seen = new Map<string, Citation>();
  for (const h of hits) {
    if (!h.url) continue;
    const prev = seen.get(h.url);
    if (!prev) seen.set(h.url, { title: h.title, url: h.url });
    else if (!prev.title && h.title) prev.title = h.title;
  }
  return [...seen.values()];
}

/** Inline fields (`key:: value`, `[key:: value]`) as the chip the editor shows — a
 *  read-only rendering must not expose the markup the editor hides. Runs after code
 *  spans are stashed (a field never sits in code) and before links, since scanFields
 *  skips `[[…]]` by itself. */
function stashFields(s: string, stash: (html: string) => string): string {
  if (!s.includes("::")) return s;
  const marks = scanFields(s);
  if (!marks.length) return s;
  let out = "";
  let at = 0;
  for (const m of marks) {
    const chip = `<span class="md-field"><span class="md-field-key">${escapeHtml(m.key)}</span>${escapeHtml(m.value)}</span>`;
    out += s.slice(at, m.from) + stash(chip);
    at = m.to;
  }
  return out + s.slice(at);
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/** Undo Markdown punctuation escapes (htmd's output and the model's both write things
 *  like \[ \*).
 *  Only punctuation escapes are recognised, so a real backslash such as a Windows path
 *  is left alone. */
const unescapeMd = (s: string) => s.replace(/\\([\\`*_{}[\]()#+.!|~<>-])/g, "$1");

/** The plain-text form of a marker's, a source card's or an image chip's title, and of
 *  an Agenda task row: escapes and the emphasis and code markers (**bold**, *italic*,
 *  `code`) are stripped, so they are not parsed a second time inside a tag attribute
 *  or a chip's text. */
export function plainLabel(s: string): string {
  return unescapeMd(s)
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/\*([^*]+)\*/g, "$1")
    .replace(/`([^`]+)`/g, "$1")
    .trim();
}

/** The plain-text form of the projection layer (claim and evidence cards, Agenda and
 *  kanban task rows, the outline): on top of plainLabel it also folds the link family
 *  — an image → its alt, [text](url) → text (READ_LINK takes any target, so
 *  `[note](note.md)` folds too), a bare URL → its domain, [[a|b]] → b, [[a]] → a — and
 *  finally collapses the whitespace. Every replacement swaps in the label in place
 *  without padding spaces: "([a](u), [b](u))" has to fold into "(a, b)". The order is
 *  a hard constraint: images before links (or READ_LINK eats [alt](url) and leaves a
 *  lone !), and links before bare URLs. plainLabel's existing consumers (a marker's
 *  label, an image chip) are unchanged, and this layer sits on top of it.
 *  READ_LINK/READ_IMAGE are defined in the reading-mode section below (this is only
 *  called once the module has finished evaluating). */
export function plainText(s: string): string {
  let out = s.replace(READ_IMAGE, (_, alt: string) => alt.trim() || t("editor.image"));
  out = out.replace(READ_LINK, (_, label: string) => label);
  out = out.replace(BARE_URL, (m) => {
    const url = stripTrailingPunct(m);
    return domainOf(url) + m.slice(url.length); // the trailing punctuation that was stripped stays in the text
  });
  out = out.replace(/\[\[([^\]|]+)\|([^\]]+)\]\]/g, "$2").replace(/\[\[([^\]]+)\]\]/g, "$1");
  return plainLabel(out).replace(/\s{2,}/g, " ").trim();
}

/** Inline rendering: the opaque fragments (code, links, wikilinks) are cut out into
 *  placeholders first, everything is escaped, bold and italic are applied, and the
 *  fragments are filled back in — which guarantees escaping always precedes any tag
 *  generation. */
function renderInline(raw: string, cite: (url: string, title: string, standalone: boolean) => string): string {
  // A line holding nothing but links and separating punctuation = the model wrote its
  // sources as their own line or list item, and folding it to a bare number would be
  // unreadable — cite is told to keep the title, and the block level then decides
  // whether to merge it into the previous item
  const standalone =
    raw.replace(MD_LINK, "").replace(BARE_URL, "").replace(/[\s。;；,，.、·:：]+/g, "") === "";
  const slots: string[] = [];
  const stash = (html: string) => `${slots.push(html) - 1}`;
  // Any sentinel code points the model's own text carries are removed first, or they would be filled back in as placeholders (as in renderReadingInline)
  let s = raw.replace(/[]/g, "");
  s = s.replace(/`([^`\n]+)`/g, (_, code: string) => stash(`<code>${escapeHtml(code)}</code>`));
  s = stashFields(s, stash);
  // Formulas before images and links (the same pair of rules as renderReadingInline):
  // a $E=mc^2$ the model wrote used to be plain text in chat — KaTeX renders it
  // locally, with trust:false
  s = s.replace(/\$\$([^$\n]+?)\$\$/g, (_, tex: string) => stash(renderMathHtml(tex, true)));
  s = s.replace(/\$(?![\s$])((?:\\.|[^$\n\\])+?)\$(?!\d)/g, (m, tex: string) =>
    /\s$/.test(tex) ? m : stash(renderMathHtml(tex, false)),
  );
  // Images are cut out before links, or MD_LINK would eat [alt](url) and leave a lone
  // "!"; the includes gate is the same idea as in extractCitations — a line with no
  // image should not go through the image regex
  if (s.includes("!["))
    s = s.replace(MD_IMAGE, (_, alt: string, url: string) =>
      stash(imageChip(stripTrailingPunct(url), plainLabel(alt) || t("editor.image"))),
    );
  s = s.replace(MD_LINK, (_, title: string, url: string) => stash(cite(stripTrailingPunct(url), title, standalone)));
  s = s.replace(BARE_URL, (m: string) => stash(cite(stripTrailingPunct(m), "", standalone)));
  s = s.replace(/\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g, (_, target: string, alias?: string) => stash(wikiAnchor(target.trim(), alias)));
  s = escapeHtml(s);
  s = s.replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>");
  s = s.replace(/(?<![*\w])\*([^*\n]+)\*(?!\*)/g, "<em>$1</em>");
  s = s.replace(/(\d+)/g, (_, i: string) => slots[Number(i)] ?? "");
  return s;
}

/** The block form: one piece of HTML per top-level block. The panel renders block by
 *  block with {@html} — while streaming, only the last unclosed block changes and the
 *  DOM of the blocks before it is untouched (selected text is no longer replaced
 *  wholesale, and the re-render cost drops from the whole message to the last
 *  block). */
/** Block-level state carried across calls. The streaming prefix-block cache splits one
 *  message into several render calls, so any state where "what came before decides how
 *  what follows renders" has to travel through here, or block-by-block rendering and
 *  whole-message rendering would produce different results for the same text (losing
 *  citeZone once merged a deliberate bibliography list into a string of markers). */
export interface BlockRenderState {
  citeZone: boolean;
}

export function renderAssistantMarkdownBlocks(
  text: string,
  numberOf: Map<string, number>,
  state?: BlockRenderState,
): string[] {
  const cite = (url: string, title: string, standalone: boolean): string => {
    const n = numberOf.get(url);
    // The title is cleaned to plain text: **emphasis** inside a label is not parsed a second time, and leaving it in is noise
    const clean = plainLabel(title);
    const label = clean || domainOf(url);
    if (n == null) {
      return `<a class="md-link" data-url="${escapeHtml(url)}" title="${escapeHtml(url)}">${escapeHtml(label)}</a>`;
    }
    const tip = clean ? `${clean} · ${url}` : url;
    const chip = `<a class="md-cite" data-url="${escapeHtml(url)}" title="${escapeHtml(tip)}">${n}</a>`;
    if (!standalone) return chip;
    // A pure source line: the title stays in the body (still clickable) with the marker appended, matching the source card
    return `<a class="md-link" data-url="${escapeHtml(url)}" title="${escapeHtml(url)}">${escapeHtml(label)}</a>${chip}`;
  };
  return renderBlockList(text, (line) => renderInline(line, cite), state);
}

/** A note's Markdown as the editor's embed widget shows it: the chat renderer's
 *  block and inline rules with plain links (no citation numbering), and nested
 *  `![[…]]` folded into ⧉ wikilinks — so an embed is one level deep by
 *  construction and can never loop. Same escaping contract as the chat: every
 *  character is escaped first, only tags generated here are emitted. */
export function renderNoteMarkdown(text: string): string {
  const plainLink = (url: string, title: string): string => {
    const label = plainLabel(title) || domainOf(url);
    return `<a class="md-link" data-url="${escapeHtml(url)}" title="${escapeHtml(url)}">${escapeHtml(label)}</a>`;
  };
  return renderBlocks(text, (line) => renderInline(foldNestedEmbeds(line), plainLink));
}

/** A wikilink anchor. A plain `[[Note]]` is the link it always was; a target with a
 *  fragment (`[[Note#^id]]`, `[[Note#bN]]`, `[[Note#Heading]]`) becomes a block
 *  citation chip — a ¶ (block) or › (heading) glyph and the note's name — carrying
 *  data-note / data-frag for the hover card and the click. */
export function wikiAnchor(target: string, alias?: string): string {
  const hash = target.indexOf("#");
  if (hash < 0) return `<a class="md-wiki" data-wiki="${escapeHtml(target)}">${escapeHtml(alias ?? target)}</a>`;
  const note = target.slice(0, hash).trim();
  const frag = target.slice(hash + 1).trim();
  const isBlock = frag.startsWith("^") || /^b\d+$/.test(frag);
  const stem = (note.split("/").pop() ?? note).replace(/\.md$/i, "");
  const label = alias ?? (isBlock ? stem || t("chat.thisNote") : `${stem ? `${stem} › ` : ""}${frag}`);
  return (
    `<a class="md-wiki md-blockref" data-wiki="${escapeHtml(target)}" data-note="${escapeHtml(note)}" data-frag="${escapeHtml(frag)}" title="${escapeHtml(target)}">` +
    `<span class="md-blockref-glyph">${isBlock ? "¶" : "›"}</span>${escapeHtml(label)}</a>`
  );
}

/** `![[t]]` / `![[t|a]]` → `[[t|⧉ a]]`: an embed inside embedded content shows as a link. */
function foldNestedEmbeds(line: string): string {
  return line.replace(/!\[\[([^\]|\n]+)(?:\|([^\]\n]+))?\]\]/g, (_, target: string, alias?: string) => `[[${target}|⧉ ${alias ?? target}]]`);
}

// ---- merging a pure-source item into the previous one (chat only: the reading projection emits no md-cite and never triggers it) ----

const CITE_CHIP = /<a class="md-cite"[^>]*>[^<]*<\/a>/g;
const CITE_OR_LINK = /<a class="md-(?:link|cite)"[^>]*>[^<]*<\/a>/g;

/** When a rendered list item or paragraph line holds nothing but link anchors, marker
 *  chips and separating punctuation, it is a "pure source item" — its markers are
 *  returned; otherwise null. */
function pureCiteChips(html: string): string[] | null {
  if (!html.includes('class="md-cite"')) return null;
  if (html.replace(CITE_OR_LINK, "").replace(/[\s。;；,，.、·:：]+/g, "") !== "") return null;
  return html.match(CITE_CHIP);
}

/** Append a pure-source item's markers to the end of the previous item (which is where
 *  a lone numeric bullet is eliminated). When the previous item is a pure-source item
 *  itself nothing is merged — the whole run is a source list, and each keeping its
 *  title reads better.
 *  Returning true means it was merged and the caller does not make an item of it. */
function appendCites(items: string[], item: string): boolean {
  const chips = pureCiteChips(item);
  if (!chips || !items.length) return false;
  const prev = items[items.length - 1];
  if (pureCiteChips(prev)) return false;
  items[items.length - 1] = prev + chips.join("");
  return true;
}

/** A code fence's opening line (the block-level state machine and stableBlockPrefix share this one grammar). */
const FENCE_RE = /^(`{3,})/;

/** The one grammar of a task list's check prefix (`[ ]`/`[x]`): the block-level
 *  rendering's taskify and the panel's plan-card parsePlan share it, so extending the
 *  check markers changes this one place. */
export const TASK_PREFIX_RE = /^\[( |x|X)\]\s+/;

/** The safe split point of streaming rendering: it returns a prefix length that ends
 *  at a blank line and does not sit inside an unclosed code fence (0 = there is none).
 *  The fence pairing rule matches renderBlockList's — a closing fence has to be at
 *  least as long as the opening one, and a shorter ``` line inside the content is not
 *  a closer. The panel's streaming prefix-block cache uses this as its boundary, so a
 *  change to the block grammar needs no separate change to the component's
 *  heuristics. */
export function stableBlockPrefix(text: string): number {
  let inCode = false;
  let codeFence = 3;
  let offset = 0;
  let cut = 0;
  for (const line of text.split("\n")) {
    const fence = FENCE_RE.exec(line);
    if (inCode) {
      if (fence && fence[1].length >= codeFence) inCode = false;
    } else if (fence) {
      inCode = true;
      codeFence = fence[1].length;
    } else if (!line.trim()) {
      cut = offset + line.length + 1; // after the blank line (its trailing newline included)
    }
    offset += line.length + 1;
  }
  return Math.min(cut, text.length);
}

/** The click handling of the code block's copy button the renderer generates
 *  (emitCode's button[data-copy]). The button's DOM is generated in this module, so
 *  its behaviour lives here too — the click delegation of both the chat panel and the
 *  reading projection comes through here first, and true means it was handled. */
export function handleCodeCopyClick(e: MouseEvent): boolean {
  if (!(e.target instanceof Element)) return false;
  const btn = e.target.closest("button[data-copy]");
  if (!btn) return false;
  const pre = btn.parentElement?.querySelector("pre");
  if (pre) void navigator.clipboard.writeText(pre.textContent ?? "").then(() => toast.push(t("editor.copiedCode")));
  return true;
}

/** The block-level state machine: shared by chat and the reading projection, with the inline rendering injected by the caller. */
function renderBlocks(text: string, inline: (line: string) => string): string {
  return renderBlockList(text, inline).join("");
}

function renderBlockList(
  text: string,
  inline: (line: string) => string,
  state?: BlockRenderState,
): string[] {
  const out: string[] = [];
  let para: string[] = [];
  let quote: string[] = [];
  /** The list nesting stack: an indent ≥ the level above plus 2 nests (every level used
   *  to be flattened into one, which flattened every multi-level outline the AI
   *  produced). Closing a level merges its rendered result into the parent's last
   *  item. */
  type ListFrame = { ordered: boolean; start: number; indent: number; items: string[] };
  const listStack: ListFrame[] = [];
  let code: string[] | null = null;
  // The opening fence's length: the clipping side upgrades it to ```` when the content
  // holds ``` (vemry-clip's code_handler), and a closer has to be no shorter than the
  // opener, or a ``` line inside the content would close it early
  let codeFence = 3;
  /** The "bibliography zone": when the previous heading holds a word like
   *  references/sources/bibliography, the model is deliberately listing its sources —
   *  and appendCites must not merge that list into a string of markers.
   *  The initial value comes from state: under block-by-block rendering the heading may
   *  lie in the prefix a previous call already froze. */
  let citeZone = state?.citeZone ?? false;

  const flushPara = () => {
    if (para.length) out.push(`<p>${para.join("<br>")}</p>`);
    para = [];
  };
  // A quote whose first line is a callout head renders as the callout the editor
  // shows (tinted, the title as its head), not as a blockquote wearing `[!type]`
  const flushQuote = () => {
    if (!quote.length) return;
    const head = CALLOUT_HEAD_RE.exec(quote[0]);
    if (head) {
      const group = CALLOUT_GROUP[head[1].toLowerCase()] ?? "info";
      const title = head[3].trim() ? inline(head[3].trim()) : escapeHtml(head[1]);
      const body = quote.slice(1).map(inline).join("<br>");
      out.push(`<div class="md-callout md-co-${group}"><div class="md-co-head">${title}</div>${body ? `<p>${body}</p>` : ""}</div>`);
    } else {
      out.push(`<blockquote>${quote.map(inline).join("<br>")}</blockquote>`);
    }
    quote = [];
  };
  const closeTopList = () => {
    const f = listStack.pop();
    if (!f) return;
    const tag = f.ordered ? "ol" : "ul";
    // An ordered list is often cut into several <ol>s by a paragraph or a table, and without a start number they would all begin at 1 again
    const start = f.ordered && f.start !== 1 ? ` start="${f.start}"` : "";
    const html = `<${tag}${start}>${f.items.map((i) => `<li>${i}</li>`).join("")}</${tag}>`;
    const parent = listStack[listStack.length - 1];
    if (parent) {
      // A sublist merges into the parent's last item (a malformed indent with no preceding item becomes an item of its own)
      if (parent.items.length) parent.items[parent.items.length - 1] += html;
      else parent.items.push(html);
    } else {
      out.push(html);
    }
  };
  const flushList = () => {
    while (listStack.length) closeTopList();
  };
  /** The single entry point for a list line: a pure-source item first tries to merge
   *  into the current level's previous item (outside the bibliography zone), and the
   *  indent then decides nesting, the same level, or closing a level. */
  const pushListItem = (ordered: boolean, startNum: number, indent: number, item: string) => {
    flushPara();
    flushQuote();
    const top = listStack[listStack.length - 1];
    if (!citeZone && top && appendCites(top.items, item)) return;
    while (listStack.length && listStack[listStack.length - 1].indent > indent) closeTopList();
    const cur = listStack[listStack.length - 1];
    if (cur && indent >= cur.indent + 2) {
      listStack.push({ ordered, start: startNum, indent, items: [] });
    } else if (cur && cur.ordered !== ordered) {
      closeTopList();
      listStack.push({ ordered, start: startNum, indent, items: [] });
    } else if (!cur) {
      listStack.push({ ordered, start: startNum, indent, items: [] });
    }
    listStack[listStack.length - 1].items.push(item);
  };
  /** A task list item: the `[ ]`/`[x]` prefix renders as a non-interactive check mark
   *  (both the 4.3 plan card and the AI's todo lists depend on it; it used to render a
   *  literal [ ]). */
  const taskify = (raw: string): { rest: string; prefix: string; done: boolean } | null => {
    const m = TASK_PREFIX_RE.exec(raw);
    if (!m) return null;
    const done = m[1].toLowerCase() === "x";
    return { rest: raw.slice(m[0].length), prefix: `<span class="md-check${done ? " done" : ""}">${done ? "☑" : "☐"}</span> `, done };
  };
  /** The raw lines of a pipe table; on flush it becomes a table when the second line is a separator row, and falls back to a paragraph otherwise. */
  let tableRows: string[] | null = null;
  // Split on unescaped | — a pipe inside a cell is escaped to \| by the clipping side
  // (vemry-clip's table handler) and restored after splitting
  const splitRow = (line: string) =>
    line
      .trim()
      .replace(/^\|/, "")
      .replace(/\|$/, "")
      .split(/(?<!\\)\|/)
      .map((c) => c.trim().replace(/\\\|/g, "|"));
  const flushTable = () => {
    if (!tableRows) return;
    const raw = tableRows;
    tableRows = null;
    const rows = raw.map(splitRow);
    const isSep = (cells: string[]) => cells.length > 0 && cells.every((c) => /^:?-+:?$/.test(c));
    if (rows.length >= 2 && isSep(rows[1])) {
      const head = rows[0];
      const body = rows.slice(2);
      const tr = (cells: string[], tag: string) =>
        `<tr>${head.map((_, i) => `<${tag}>${inline(cells[i] ?? "")}</${tag}>`).join("")}</tr>`;
      out.push(
        `<div class="md-tablewrap"><table><thead>${tr(head, "th")}</thead><tbody>${body
          .map((r) => tr(r, "td"))
          .join("")}</tbody></table></div>`,
      );
    } else {
      out.push(`<p>${raw.map(inline).join("<br>")}</p>`);
    }
  };
  const flushAll = () => {
    flushPara();
    flushQuote();
    flushList();
    flushTable();
  };
  const emitCode = () => {
    // The copy button: static HTML going through the panel's or the projection's click
    // delegation (button[data-copy]), and what it copies is the <pre> text that follows
    if (code)
      out.push(
        `<div class="md-codewrap"><button class="md-copy" data-copy title="${escapeHtml(t("editor.copyCode"))}">${escapeHtml(t("editor.copyCodeShort"))}</button><pre class="md-code"><code>${escapeHtml(code.join("\n"))}</code></pre></div>`,
      );
    code = null;
  };

  for (const line of text.split("\n")) {
    const fence = FENCE_RE.exec(line);
    if (code) {
      if (fence && fence[1].length >= codeFence) emitCode();
      else code.push(line);
      continue;
    }
    if (fence) {
      flushAll();
      code = [];
      codeFence = fence[1].length;
      continue;
    }
    if (/^\s*\|.*\|\s*$/.test(line)) {
      flushPara();
      flushQuote();
      flushList();
      (tableRows ??= []).push(line);
      continue;
    }
    flushTable();
    if (/^\s*(-{3,}|\*{3,})\s*$/.test(line)) {
      flushAll();
      out.push(`<hr class="md-hr">`);
      continue;
    }
    const h = /^(#{1,6})\s+(.*)$/.exec(line);
    if (h) {
      flushAll();
      citeZone = /参考|来源|出处|references|sources|bibliography/i.test(h[2]);
      out.push(`<div class="md-h md-h${h[1].length}">${inline(h[2])}</div>`);
      continue;
    }
    const ul = /^(\s*)[-*]\s+(.*)$/.exec(line);
    if (ul) {
      const task = taskify(ul[2]);
      const body = task ? inline(task.rest) : inline(ul[2]);
      const item = task
        ? `${task.prefix}${task.done ? `<span class="md-taskdone">${body}</span>` : body}`
        : body;
      pushListItem(false, 1, ul[1].replace(/\t/g, "  ").length, item);
      continue;
    }
    const ol = /^(\s*)(\d+)[.、)]\s+(.*)$/.exec(line);
    if (ol) {
      pushListItem(true, parseInt(ol[2], 10), ol[1].replace(/\t/g, "  ").length, inline(ol[3]));
      continue;
    }
    const q = /^>\s?(.*)$/.exec(line);
    if (q) {
      flushPara();
      flushList();
      quote.push(q[1]);
      continue;
    }
    if (!line.trim()) {
      flushAll();
      continue;
    }
    flushQuote();
    flushList();
    const pl = inline(line);
    if (citeZone || !appendCites(para, pl)) para.push(pl);
  }
  flushAll();
  emitCode(); // while streaming, the fence may not be closed yet, so it is rendered as code
  if (state) state.citeZone = citeZone;
  return out;
}

// ---- the reading projection's rendering (v0.5 P2) ----

/** A relative link is resolved against the page URL; only http(s) is allowed, and anything else is null (handled as plain text). */
function resolveHttp(href: string, base: string): string | null {
  try {
    const u = new URL(href, base);
    return u.protocol === "http:" || u.protocol === "https:" ? u.href : null;
  } catch {
    return null;
  }
}

/** A projection image's address → the vemry-asset proxy protocol (on the Rust side:
 *  SSRF validation, no cookies, the page's Referer, a disk cache). The platform
 *  difference (Windows goes through http://<scheme>.localhost) is given authoritatively
 *  by convertFileSrc rather than sniffed from the UA. */
export function proxiedImageUrl(url: string, referer: string): string {
  const q = `u=${encodeURIComponent(url)}&r=${encodeURIComponent(referer)}`;
  return `${convertFileSrc("", "vemry-asset")}?${q}`;
}

/** The image placeholder chip: the fallback when the proxy is off or the load failed; a click opens a browser window to see the original. */
export function imageChip(url: string, label: string): string {
  const tip = t("editor.imageNotLoaded", { url });
  return `<a class="md-img" data-url="${escapeHtml(url)}" title="${escapeHtml(tip)}">⧉ ${escapeHtml(label)}</a>`;
}

/** LaTeX → KaTeX HTML. The hand-written renderer's only third-party HTML injection
 *  point: called on recognised formula fragments alone, with trust:false forbidding
 *  dangerous commands such as \href; a syntax error renders as the source in red
 *  (throwOnError:false) rather than throwing and taking the whole document down. */
function renderMathHtml(tex: string, display: boolean): string {
  try {
    return katex.renderToString(tex.trim(), {
      displayMode: display,
      throwOnError: false,
      trust: false,
      output: "html",
    });
  } catch {
    return `<code>${escapeHtml(tex)}</code>`;
  }
}

// Reading mode's links and images: from the same builder as chat's MD_LINK/MD_IMAGE
// (linkRe), differing only in that the target is not limited to absolute http(s)
const READ_LINK = linkRe(false, ANY_TARGET);
const READ_IMAGE = linkRe(true, ANY_TARGET);

/** Reading mode's inline rendering. It follows the same safety contract as the chat
 *  rendering (escape first, emit only the tags generated here, recognise a link only
 *  once it resolves to http(s)), with these differences:
 *  - an image folds into a placeholder chip that makes no request — the main webview
 *    keeps making zero remote requests;
 *  - a link keeps its full text and does not fold into a marker;
 *  - wikilinks are not recognised: a [[..]] in a page's body is ordinary text.
 *  The placeholder sentinel uses an explicit / (the same idea as renderInline's
 *  invisible sentinel). */
function renderReadingInline(raw: string, base: string): string {
  const slots: string[] = [];
  const stash = (html: string) => `${slots.push(html) - 1}`;
  let s = raw.replace(/[]/g, "");
  s = s.replace(/`([^`\n]+)`/g, (_, code: string) => stash(`<code>${escapeHtml(code)}</code>`));
  // Formulas before images and links: the subscripts and braces inside $…$ must not be
  // touched by the later rules. Block level ($$) comes first; inline requires
  // non-whitespace at both ends and no digit after the closing $ — a dollar amount in
  // the body is not a formula
  s = s.replace(/\$\$([^$\n]+?)\$\$/g, (_, tex: string) => stash(renderMathHtml(tex, true)));
  s = s.replace(/\$(?![\s$])((?:\\.|[^$\n\\])+?)\$(?!\d)/g, (m, tex: string) =>
    /\s$/.test(tex) ? m : stash(renderMathHtml(tex, false)),
  );
  s = s.replace(READ_IMAGE, (_, alt: string, href: string) => {
    const label = unescapeMd(alt).trim() || t("editor.image");
    // A data: image embedded in the page is rendered directly — it makes no network request anyway, so it needs no proxy
    if (/^data:image\//i.test(href)) {
      return stash(`<img class="md-imgload" src="${escapeHtml(href)}" alt="${escapeHtml(label)}">`);
    }
    const url = resolveHttp(href, base);
    if (!url) return stash(escapeHtml(label));
    // The image loads through the vemry-asset proxy (SSRF validation, no cookies and a
    // disk cache on the Rust side); a failed load is swapped back to a placeholder chip
    // by ReadingView's capture listener
    return stash(
      `<img class="md-imgload" src="${escapeHtml(proxiedImageUrl(url, base))}" alt="${escapeHtml(label)}" loading="lazy" data-url="${escapeHtml(url)}">`,
    );
  });
  s = s.replace(READ_LINK, (_, text: string, href: string) => {
    // The link's text may hold an already-folded fragment sentinel (an image inside a link): it is stripped, and an empty result falls back to the domain
    const inner = text.match(/(\d+)/);
    const bare = unescapeMd(text).replace(/\d+/g, "").trim();
    // An in-page anchor (a Wikipedia citation marker and the like): the projection has no anchors, so it renders as plain text
    if (href.startsWith("#")) return stash(escapeHtml(bare));
    // A link that is nothing but an image (a Wikipedia thumbnail, a lead image): the image itself is rendered and the click belongs to it (opening a browser window)
    if (!bare && inner && slots[Number(inner[1])]?.startsWith("<img")) return text;
    const url = resolveHttp(stripTrailingPunct(href), base);
    const label = bare || (url ? domainOf(url) : "");
    if (!url) return stash(escapeHtml(label));
    return stash(`<a class="md-link" data-url="${escapeHtml(url)}" title="${escapeHtml(url)}">${escapeHtml(label)}</a>`);
  });
  s = s.replace(BARE_URL, (m: string) => {
    const url = stripTrailingPunct(m);
    return stash(`<a class="md-link" data-url="${escapeHtml(url)}" title="${escapeHtml(url)}">${escapeHtml(url)}</a>`);
  });
  s = unescapeMd(s);
  s = escapeHtml(s);
  s = s.replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>");
  s = s.replace(/(?<![*\w])\*([^*\n]+)\*(?!\*)/g, "<em>$1</em>");
  s = s.replace(/(\d+)/g, (_, i: string) => slots[Number(i)] ?? "");
  return s;
}

/** The projection's body → HTML. `base` is the page's actual URL (after redirects), and relative links are resolved against it. */
export function renderReadingMarkdown(text: string, base: string): string {
  return renderBlocks(text, (line) => renderReadingInline(line, base));
}
