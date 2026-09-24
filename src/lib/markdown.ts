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
// A block citation (`[[note#^id]]`, `[[note#bN]]`, `[[note#Heading]]`) folds the same
// way into a numbered marker of its own series (¶1, ¶2 — the ¶ keeps it apart from the
// web sources' numbers), and the answer closes with the list of the notes it cited.

import { t } from "$lib/i18n/i18n.svelte";
import { convertFileSrc } from "@tauri-apps/api/core";
import { closesMathBlock, foldMath, mathBlockTex, opensMathBlock, renderMathHtml, replaceMath } from "$lib/math";
import { toast } from "$lib/stores/toast.svelte";
import { CALLOUT_GROUP, CALLOUT_HEAD_RE, calloutDefaultTitle, calloutIconSvg } from "$lib/callout";
import { scanFields, stateStepOf } from "$lib/fields";
import { stateGlyphSvg, TASK_STEPS, taskStep } from "$lib/chips";
import { types } from "$lib/stores/types.svelte";
import { POSITIONAL_RE } from "$lib/citation";

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
/** `*…*` whose opening `*` does not sit inside a word. The word characters are letters
 *  and digits of any script, not only ASCII's, so a Cyrillic or Greek word is read the
 *  way an English one is — with Han and kana deliberately left out: the parser refuses
 *  emphasis flanked by them and the editor rescues it on purpose (livepreview's
 *  CJK_EMPH_RE), so `这是*重点*了` has to keep rendering as emphasis here too.
 *  The three lookbehinds are the start of the string, a non-word character, and a
 *  per-character script; all are zero-width, so the span is still group 1. */
const EM_RE =
  /(?:(?<![\s\S])|(?<=[^*\p{L}\p{N}_])|(?<=[\p{sc=Han}\p{sc=Hiragana}\p{sc=Katakana}]))\*([^*\n]+)\*(?!\*)/gu;

const MD_LINK = linkRe(false, HTTP_TARGET);
// An image reference in chat: rendered as a placeholder chip rather than an <img>, and it does not enter the source cards (an image is not a citation source)
const MD_IMAGE = linkRe(true, HTTP_TARGET);
// A note's own image, `![alt](clips/assets/x.png)` — a vault-relative path. Only a note
// embed renders it (renderNoteMarkdown, as an <img> its host loads from the vault);
// in chat such a line stays text
const NOTE_IMAGE = linkRe(true, ANY_TARGET);
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
    const state = stateStepOf(types.enabled, m.key, m.value);
    const glyph = state ? `<span class="md-field-glyph">${stateGlyphSvg(state.step, state.of)}</span>` : "";
    const cls = state ? " md-field-state" : "";
    const chip = `<span class="md-field${cls}">${glyph}<span class="md-field-key">${escapeHtml(m.key)}</span>${escapeHtml(m.value)}</span>`;
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
/** The same escapes, for the inline renderers (which stash the protected character). */
const ESCAPE_RE = /\\([\\`*_{}[\]()#+.!|~<>-])/g;

/** The emphasis family the editor hides the markers of: **bold** and __bold__, *italic*
 *  and _italic_, ~~struck~~, ==highlighted==. An underscore only counts at a word's
 *  edge (`snake_case_name` is a word, as the renderer reads it), and a strike or a
 *  highlight needs its words right inside the markers (`a == b` is not highlighted). */
const EMPHASIS: [RegExp, string][] = [
  [/\*\*([^*]+)\*\*/g, "$1"],
  [/__([^_]+)__/g, "$1"],
  [/\*([^*]+)\*/g, "$1"],
  [/(^|[^\p{L}\p{N}_])_([^_\n]+)_(?![\p{L}\p{N}_])/gu, "$1$2"],
  [/~~(?=\S)([^~\n]*?\S)~~/g, "$1"],
  [/==(?=\S)([^=\n]*?\S)==/g, "$1"],
];

/** The plain-text form of a marker's, a source card's or an image chip's title, and of
 *  an Agenda task row: escapes, the emphasis markers (EMPHASIS) and the code markers
 *  are stripped, so they are not parsed a second time inside a tag attribute or a
 *  chip's text. What sits inside a code span is left as written — `a_b_c` or `**` in
 *  code is the text itself. */
export function plainLabel(s: string): string {
  return unescapeMd(s)
    .split(/(`[^`]+`)/)
    .map((part, i) => (i % 2 ? part.slice(1, -1) : EMPHASIS.reduce((acc, [re, to]) => acc.replace(re, to), part)))
    .join("")
    .trim();
}

/** The character references a line of Markdown carries in practice; an unknown name is
 *  left as written rather than guessed. */
const ENTITIES: Record<string, string> = {
  amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " ", ensp: " ", emsp: " ", thinsp: " ",
  mdash: "—", ndash: "–", hellip: "…", middot: "·", bull: "•", times: "×", divide: "÷", deg: "°",
  plusmn: "±", copy: "©", reg: "®", trade: "™", laquo: "«", raquo: "»", ldquo: "“", rdquo: "”",
  lsquo: "‘", rsquo: "’", larr: "←", rarr: "→", uarr: "↑", darr: "↓", harr: "↔",
};

/** `&amp;`, `&#8212;`, `&#x2014;` → the character. */
function decodeEntities(s: string): string {
  if (!s.includes("&")) return s;
  return s.replace(/&(#\d{1,7}|#x[0-9a-f]{1,6}|[a-z]{2,8});/gi, (m, ref: string) => {
    if (ref[0] !== "#") return ENTITIES[ref.toLowerCase()] ?? m;
    const code = ref[1] === "x" || ref[1] === "X" ? parseInt(ref.slice(2), 16) : parseInt(ref.slice(1), 10);
    return code > 0 && code <= 0x10ffff ? String.fromCodePoint(code) : m;
  });
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
 *  What the editor hides in a line and a projection must not show either: an autolink's
 *  angle brackets, a footnote mark `[^1]`, inline HTML tags (`<mark>`, `<br>`), and —
 *  last, so a decoded `&lt;b&gt;` or `&#42;` stays text — character references.
 *  READ_LINK/READ_IMAGE are defined in the reading-mode section below (this is only
 *  called once the module has finished evaluating). */
export function plainText(s: string): string {
  // `<https://…>`: the URL alone, which the bare-URL fold below reads
  let out = s.replace(/<(https?:\/\/[^\s<>]+)>/g, "$1");
  out = out.replace(READ_IMAGE, (_, alt: string) => alt.trim() || t("editor.image"));
  out = out.replace(READ_LINK, (_, label: string) => label);
  out = out.replace(BARE_URL, (m) => {
    const url = stripTrailingPunct(m);
    return domainOf(url) + m.slice(url.length); // the trailing punctuation that was stripped stays in the text
  });
  // Formulas: the plain approximation, never the $ delimiters ($\sqrt{d_k}$ → √d_k)
  out = foldMath(out);
  // Inline fields: the chip's wording, `status: reading`, never the `[key:: value]` markup
  out = foldFields(out);
  // Embeds and wikilinks: the label the editor shows — the alias, else the note's stem
  // with › heading or ¶ block; an embed is marked ⧉, as it is inside embedded content
  out = out.replace(/(!?)\[\[([^\]|\n]+)(?:\|([^\]\n]+))?\]\]/g, (_, bang: string, target: string, alias?: string) => `${bang ? "⧉ " : ""}${wikiLabel(target.trim(), alias?.trim())}`);
  // A footnote mark is a pointer to text elsewhere, not a word of this one
  out = out.replace(/\[\^[^\]\s]+\]/g, "");
  // Inline HTML: a comment goes whole, a tag leaves its words (a line break a space)
  out = out.replace(/<!--[\s\S]*?-->/g, "").replace(/<\/?([A-Za-z][A-Za-z0-9-]*)(?:\s[^<>]*)?\/?>/g, (_, tag: string) => (tag.toLowerCase() === "br" ? " " : ""));
  return decodeEntities(plainLabel(out)).replace(/\s{2,}/g, " ").trim();
}

/** `[key:: value]`, `(key:: value)` and `key:: value` as the chip reads them. */
function foldFields(s: string): string {
  if (!s.includes("::")) return s;
  let out = "";
  let at = 0;
  for (const m of scanFields(s)) {
    out += s.slice(at, m.from) + `${m.key}: ${m.value}`;
    at = m.to;
  }
  return out + s.slice(at);
}

/** A wikilink target read apart: the note before the `#`, the fragment after it, the
 *  note's stem (last path segment, no `.md`), and whether the fragment names a block
 *  rather than a heading. The label and the anchor below both turn on this. */
function splitWikiTarget(target: string) {
  const hash = target.indexOf("#");
  const note = hash < 0 ? target : target.slice(0, hash).trim();
  const frag = hash < 0 ? "" : target.slice(hash + 1).trim();
  return {
    note,
    frag,
    stem: (note.split("/").pop() ?? note).replace(/\.md$/i, ""),
    isBlock: frag.startsWith("^") || POSITIONAL_RE.test(frag),
  };
}

/** The plain label of a wikilink target: the alias when there is one, else the note's
 *  stem — alone, with `› heading`, or with `¶` for a block. The same shape the editor's
 *  folded link shows. */
function wikiLabel(target: string, alias?: string): string {
  if (alias) return alias;
  const { frag, stem, isBlock } = splitWikiTarget(target);
  if (!frag) return stem || target;
  if (isBlock) return stem ? `${stem} ¶` : "¶";
  return stem ? `${stem} › ${frag}` : frag;
}

/** Inline rendering: the opaque fragments (code, links, wikilinks) are cut out into
 *  placeholders first, everything is escaped, bold and italic are applied, and the
 *  fragments are filled back in — which guarantees escaping always precedes any tag
 *  generation. */
function renderInline(
  raw: string,
  cite: (url: string, title: string, standalone: boolean) => string,
  localImage?: (target: string, alt: string) => string,
  wiki: (target: string, alias?: string) => string = wikiAnchor,
): string {
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
  s = replaceMath(s, (f) => stash(renderMathHtml(f.tex, f.display)));
  // Backslash escapes (`2\.`, `\_`, `\*`): the protected character alone, stashed so the
  // rules below cannot read it as markup. After math (a formula's backslashes are
  // TeX), before links and emphasis
  s = s.replace(ESCAPE_RE, (_, ch: string) => stash(escapeHtml(ch)));
  // Images are cut out before links, or MD_LINK would eat [alt](url) and leave a lone
  // "!"; the includes gate is the same idea as in extractCitations — a line with no
  // image should not go through the image regex
  if (s.includes("!["))
    s = s.replace(MD_IMAGE, (_, alt: string, url: string) =>
      stash(imageChip(stripTrailingPunct(url), plainLabel(alt) || t("editor.image"))),
    );
  // What is left with a `![` is a vault path (the http ones are stashed by now)
  if (localImage && s.includes("!["))
    s = s.replace(NOTE_IMAGE, (_, alt: string, target: string) => stash(localImage(target, plainLabel(alt) || t("editor.image"))));
  s = s.replace(MD_LINK, (_, title: string, url: string) => stash(cite(stripTrailingPunct(url), title, standalone)));
  s = s.replace(BARE_URL, (m: string) => stash(cite(stripTrailingPunct(m), "", standalone)));
  s = s.replace(/\[\[([^\]|]+)(?:\|([^\]]+))?\]\]/g, (_, target: string, alias?: string) => stash(wiki(target.trim(), alias)));
  s = escapeHtml(s);
  s = s.replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>");
  s = s.replace(EM_RE, "<em>$1</em>");
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
  /** The block citations numbered so far, as `note#fragment` in document order: a
   *  citation's number is its place here plus one. It only ever grows, so a block frozen
   *  with its numbers stays right as later blocks cite more; a caller that renders a tail
   *  it will render again passes a copy. */
  notes?: string[];
}

/** An answer's markdown as blocks of HTML. Called with a state, it renders one piece of a
 *  message that the caller assembles (the streaming panel), and the caller closes the
 *  finished message with citedNotesHtml itself; called without one, the text is a whole
 *  message and its last block is that list. */
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
  const notes = state ? (state.notes ??= []) : [];
  // A citation with a fragment becomes a numbered marker; an alias the model gave it is
  // prose it meant to be read, so it stays as linked words with the marker after them.
  // A plain [[Note]] is a link, not a citation
  const wiki = (target: string, alias?: string): string => {
    const { note, frag } = splitWikiTarget(target);
    if (!frag) return wikiAnchor(target, alias);
    const key = `${note}#${frag}`;
    const n = notes.indexOf(key) + 1 || notes.push(key);
    const mark = noteCiteMark(note, frag, n);
    return alias ? `<a class="md-wiki" data-wiki="${escapeHtml(target)}">${escapeHtml(alias)}</a>${mark}` : mark;
  };
  const out = renderBlockList(text, (line) => renderInline(line, cite, undefined, wiki), state);
  if (!state && notes.length) out.push(citedNotesHtml(notes));
  return out;
}

/** A block citation's marker: `¶n`, carrying the target for the click and the hover card
 *  (data-note / data-frag, as the chip of an embed does). The accessible name says which
 *  note it is, since the number alone does not. */
function noteCiteMark(note: string, frag: string, n: number): string {
  const target = `${note}#${frag}`;
  const name = splitWikiTarget(target).stem || t("chat.thisNote");
  return (
    `<a class="md-cite md-notecite" data-wiki="${escapeHtml(target)}" data-note="${escapeHtml(note)}" data-frag="${escapeHtml(frag)}" aria-label="${escapeHtml(`${name} ¶${n}`)}">` +
    `¶${n}</a>`
  );
}

/** The list that closes an answer citing notes: one line per note, in the order each
 *  was first cited, with the note's name (opens it) and the markers that cite it (each
 *  lands on its block and shows the passage on hover, as in the text). `notes` is
 *  BlockRenderState.notes of the whole message. */
export function citedNotesHtml(notes: string[]): string {
  const byNote = new Map<string, { frag: string; n: number }[]>();
  notes.forEach((key, i) => {
    const hash = key.indexOf("#");
    const note = key.slice(0, hash);
    const list = byNote.get(note) ?? [];
    list.push({ frag: key.slice(hash + 1), n: i + 1 });
    byNote.set(note, list);
  });
  const rows = [...byNote].map(([note, cites]) => {
    const name = splitWikiTarget(note).stem || t("chat.thisNote");
    const link = note ? `<a class="md-wiki" data-wiki="${escapeHtml(note)}">${escapeHtml(name)}</a>` : escapeHtml(name);
    return `<li>${link}${cites.map((c) => noteCiteMark(note, c.frag, c.n)).join("")}</li>`;
  });
  return `<div class="md-citednotes"><div class="md-citednotes-head">${escapeHtml(t("chat.citedNotes"))}</div><ul>${rows.join("")}</ul></div>`;
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
  // The note's own images come out as <img data-asset>, and the embed widget loads
  // them the way the editor's image widget does (a blob URL from the vault file)
  const localImage = (target: string, alt: string) => `<img class="md-img" data-asset="${escapeHtml(target)}" alt="${escapeHtml(alt)}">`;
  return renderBlocks(text, (line) => renderInline(foldNestedEmbeds(line), plainLink, localImage));
}

/** A wikilink anchor. A plain `[[Note]]` is the link it always was; a target with a
 *  fragment (`[[Note#^id]]`, `[[Note#bN]]`, `[[Note#Heading]]`) becomes a block
 *  citation chip — a ¶ (block) or › (heading) glyph and the note's name — carrying
 *  data-note / data-frag for the hover card and the click. */
export function wikiAnchor(target: string, alias?: string): string {
  const hash = target.indexOf("#");
  if (hash < 0) return `<a class="md-wiki" data-wiki="${escapeHtml(target)}">${escapeHtml(alias ?? target)}</a>`;
  const { note, frag, stem, isBlock } = splitWikiTarget(target);
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
export const TASK_PREFIX_RE = /^\[( |x|X|\/|-)\]\s+/;

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
  // shows (tinted, the type's icon, the title as its head), not as a blockquote
  // wearing `[!type]`
  const flushQuote = () => {
    if (!quote.length) return;
    const head = CALLOUT_HEAD_RE.exec(quote[0]);
    if (head) {
      const type = head[1].toLowerCase();
      const group = CALLOUT_GROUP[type] ?? "info";
      const title = head[3].trim() ? inline(head[3].trim()) : escapeHtml(calloutDefaultTitle(head[1]));
      const body = quote.slice(1).map(inline).join("<br>");
      out.push(
        `<div class="md-callout md-co-${group}"><div class="md-co-head"><span class="md-co-icon">${calloutIconSvg(type, group)}</span>${title}</div>${body ? `<p>${body}</p>` : ""}</div>`,
      );
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
  /** A task list item: the checkbox prefix renders as the non-interactive state glyph
   *  (chips.ts — the editor's checkbox, so a task reads the same in a projection and in
   *  the note; both the 4.3 plan card and the AI's todo lists depend on it). All four
   *  states, as the editor parses them (markdown-ext.ts): `[/]` in progress and `[-]`
   *  cancelled used to come out as literal brackets here. The state is read from the
   *  character in place rather than through tasks.ts's stateOfChar: tasks.ts imports
   *  this module, and the mapping is four characters. `done` covers cancelled too — a
   *  closed task's words are struck, whichever way it was closed, as in the editor. */
  const taskify = (raw: string): { rest: string; prefix: string; done: boolean } | null => {
    const m = TASK_PREFIX_RE.exec(raw);
    if (!m) return null;
    const c = m[1];
    const state = c === "x" || c === "X" ? "done" : c === "/" ? "doing" : c === "-" ? "cancelled" : "todo";
    const glyph = stateGlyphSvg(taskStep(state), TASK_STEPS, state === "cancelled");
    return { rest: raw.slice(m[0].length), prefix: `<span class="md-check">${glyph}</span> `, done: state === "done" || state === "cancelled" };
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
  /** The lines of a display formula whose `$$` fences sit on their own lines (the
   *  editor renders the same block as one widget, editor/math.ts); the one-line
   *  `$$…$$` form is inline's business. */
  let math: string[] | null = null;
  const emitMath = () => {
    if (math) out.push(`<div class="md-math">${renderMathHtml(mathBlockTex(math), true)}</div>`);
    math = null;
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
    if (math) {
      if (!line.trim()) {
        // no closer in the paragraph: the lines are ordinary text after all (the same
        // verdict the editor reaches, so an embed and the editor agree)
        for (const l of math) para.push(inline(l));
        math = null;
        flushAll();
        continue;
      }
      math.push(line);
      if (closesMathBlock(line)) emitMath();
      continue;
    }
    if (opensMathBlock(line)) {
      flushAll();
      math = [line];
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
  emitMath(); // likewise an unclosed display block renders what it holds
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
  // touched by the later rules (scanInlineMath states the order and the rules)
  s = replaceMath(s, (f) => stash(renderMathHtml(f.tex, f.display)));
  s = s.replace(ESCAPE_RE, (_, ch: string) => stash(escapeHtml(ch)));
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
  s = s.replace(EM_RE, "<em>$1</em>");
  s = s.replace(/(\d+)/g, (_, i: string) => slots[Number(i)] ?? "");
  return s;
}

/** The projection's body → HTML. `base` is the page's actual URL (after redirects), and relative links are resolved against it. */
export function renderReadingMarkdown(text: string, base: string): string {
  return renderBlocks(text, (line) => renderReadingInline(line, base));
}

/** The part of a still-streaming text that is safe to render: a tail whose inline syntax
 *  has not closed yet is held back, so the reader never sees `[title](htt` flash as source
 *  and then collapse into a link, or a run of `**` turn the rest of the line bold and back.
 *  Only the last line is examined (an unclosed construct cannot span a newline in the
 *  renderers here), and the held text appears as soon as it closes or the line ends.
 *  A pipe table whose header row has arrived but whose separator row has not is completed
 *  with a separator, so the table takes its shape at once instead of arriving as a
 *  paragraph of pipes and then reflowing; a half-arrived table row is held back.
 *  Shared by the chat's streaming answer and the translation view's streaming cells. */
export function streamingTail(text: string): string {
  const nl = text.lastIndexOf("\n");
  const head = text.slice(0, nl + 1);
  let last = text.slice(nl + 1);

  // Inside an open fence everything is literal: nothing to hold back
  const fences = (head.match(/^\s*(```|~~~)/gm) ?? []).length;
  if (fences % 2 === 1) return text;

  // A table row that has not reached its closing pipe yet
  if (/^\s*\|/.test(last) && !/\|\s*$/.test(last.trimEnd().slice(1))) last = "";

  let cut = last.length;
  const hold = (i: number) => {
    if (i >= 0 && i < cut) cut = i;
  };
  // Inline code first: an odd backtick opens a span nothing after it may be parsed in
  const ticks = [...last.matchAll(/`+/g)];
  if (ticks.length % 2 === 1) hold(ticks[ticks.length - 1].index!);
  const scan = last.slice(0, cut);
  // [[wikilink without its ]]
  const wl = scan.lastIndexOf("[[");
  if (wl >= 0 && scan.indexOf("]]", wl) < 0) hold(wl);
  // [text](url without its ) — and a [ that has not reached its ] yet
  for (const m of scan.matchAll(/(?<!\[)\[(?!\[)/g)) {
    const rest = scan.slice(m.index!);
    const close = /^\[[^\]]*\](\([^)]*(\))?)?/.exec(rest);
    if (!close) hold(m.index!); // no ] yet
    else if (close[1] !== undefined && close[2] === undefined) hold(m.index!); // ]( without )
    else if (close[0].length === rest.length && close[1] === undefined) hold(m.index!); // ] is the last char: a ( may follow
  }
  // Paired emphasis markers: an odd count means the last one is still open
  for (const mark of ["**", "__", "~~", "=="]) {
    const at: number[] = [];
    let i = scan.indexOf(mark);
    while (i >= 0 && i < cut) {
      at.push(i);
      i = scan.indexOf(mark, i + mark.length);
    }
    if (at.length % 2 === 1) hold(at[at.length - 1]);
  }
  // Inline math: an odd count of unescaped single $ (a $$ pair is display math, left alone)
  const dollars = [...scan.slice(0, cut).matchAll(/(?<![\\$])\$(?!\$)/g)];
  if (dollars.length % 2 === 1) hold(dollars[dollars.length - 1].index!);

  last = last.slice(0, cut);

  // A header row with no separator under it yet: supply one so the table keeps its shape
  const lines = head.split("\n");
  lines.pop(); // the empty string after head's final newline
  const prev = lines[lines.length - 1] ?? "";
  const beforePrev = lines[lines.length - 2] ?? "";
  if (last === "" && /^\s*\|.*\|\s*$/.test(prev) && !/^\s*\|/.test(beforePrev)) {
    const cells = prev.trim().replace(/^\|/, "").replace(/\|$/, "").split(/(?<!\\)\|/).length;
    return `${head}|${" --- |".repeat(cells)}\n`;
  }
  return head + last;
}
