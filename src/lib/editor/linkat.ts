// The one definition of "which link is at the cursor or the clicked point": wiki
// before md, link text inside code is a literal, and `![[embed]]` does not count (the
// lesson being that followLink would create a note of the same name by mistake). The
// ⌘Enter keyboard exit and the editor context menu's "open link" share this order and
// these two exemptions.
// It does not live in clickopen.ts: wikilink and mdlink both take matchAt from there,
// and importing back would make a cycle.
import type { EditorState } from "@codemirror/state";
import type { LinkHit } from "./clickopen";
import { inCode } from "./livepreview";
import { mdLinkAt } from "./mdlink";
import { wikiLinkAt } from "./wikilink";

/** flavor decides how it opens: wiki goes through vault.followLink (which may create), md through openMdLink. */
export function linkAt(state: EditorState, pos: number): { flavor: "wiki" | "md"; hit: LinkHit } | null {
  if (inCode(state, pos)) return null;
  const wiki = wikiLinkAt(state, pos);
  if (wiki && !wiki.embed) return { flavor: "wiki", hit: wiki };
  const md = mdLinkAt(state, pos);
  return md ? { flavor: "md", hit: md } : null;
}
