// Click / ⌘/Ctrl+click navigation for standard markdown links `[label](target)`
// / `![](target)`, autolinks `<https://…>`, and bare URLs in notes. No
// decoration needed — the markdown highlighter already styles tags.url/
// tags.link as .cm-wikilink (accent + pointer).
// Division of labor: [[wikilinks]] are semantic note references (index,
// aliases); markdown links are literal references to vault files or URLs.
import { type EditorState } from "@codemirror/state";
import { stripTrailingPunct } from "$lib/markdown";
import { inCode } from "./fenced";
import { linkClickHandlers, matchAt, type LinkHit } from "./clickopen";

const MD_LINK_RE = /!?\[[^\]\n]*\]\(([^)\n]+)\)/g;
// A bare URL in the editor was pasted by hand and need not be percent-encoded, so
// this is wider than the chat panel's BARE_URL (markdown.ts, an ASCII allowlist): the
// open character class admits CJK paths and an unmatched '(' (an unencoded query
// string), while ')' is only taken in when it pairs with a '(' (a Wikipedia
// disambiguation page; a ')' that ends a sentence is left to stripTrailingPunct).
// The difference is deliberate, and a change to the boundary rule goes through both
const URL_RE = /https?:\/\/(?:\([^\s<>()]*\)?|[^\s<>()]+)+/g;

/** The escape a clipped page writes before a character of its text (`attention\_tf.py`):
 *  inside a URL the backslash is not part of the address. The click below drops it and
 *  the live preview hides it, so the two must name the same characters. */
export const CLIP_ESCAPE_RE = /\\([_*~#])/g;

/** The Markdown link or bare URL at pos (an inline scan). Mod-Enter uses it too. */
export function mdLinkAt(state: EditorState, pos: number): LinkHit | null {
  const line = state.doc.lineAt(pos);
  const m = matchAt(line, MD_LINK_RE, pos);
  if (m) {
    const target = m[1].trim();
    const from = line.from + m.index;
    return target ? { target, from, to: from + m[0].length, embed: m[0].startsWith("!") } : null;
  }
  // autolink `<https://…>` / a bare URL (a URL inside code is literal text, not a link)
  const u = matchAt(line, URL_RE, pos);
  if (u) {
    if (inCode(state, pos)) return null;
    const from = line.from + u.index;
    // Punctuation, CJK or otherwise, often sits right against a URL — the same boundary rule as the chat panel
    // A clipped page escapes `_` in its text; in a URL the backslash is not part of the address
    return { target: stripTrailingPunct(u[0]).replace(CLIP_ESCAPE_RE, "$1"), from, to: from + u[0].length };
  }
  return null;
}

export function mdlinkExtension(open: (target: string, invert: boolean) => void, singleClick: () => boolean) {
  return linkClickHandlers((view, pos) => mdLinkAt(view.state, pos), open, singleClick);
}
