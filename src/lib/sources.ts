// The three ways a web source is acted on — open it, clip it, open its clipped note —
// shared by the source cards in the conversation and the session's sources workbench,
// so the "already clipped → the local note" rule and the clip toast live once.
import { api } from "$lib/api";
import { hostDispatch } from "$lib/host";
import { t } from "$lib/i18n/i18n.svelte";
import { clips } from "$lib/stores/clips.svelte";
import { toast } from "$lib/stores/toast.svelte";

/** Open a citation: already clipped → the local note (invert forces the web page); everything
 *  else goes through the url_open funnel (respecting the setting and not stealing the focus —
 *  reading is something the user deliberately switched to). */
export function openCitation(url: string, invert: boolean) {
  const local = clips.pathFor(url);
  if (local && !invert) {
    void hostDispatch({ kind: "open-note", path: local });
    return;
  }
  void api.urlOpen(url, invert, false).catch((e) => toast.push(String(e), "error"));
}

/** One-click clipping of a source: the reference entry lands in clips/ and the citation becomes a local link. */
export async function clipSource(url: string, title: string) {
  try {
    const path = await api.clipUrl(url, title);
    clips.markClipped(url, path);
    toast.push(t("chat.clipped", { path }));
  } catch (e) {
    toast.push(String(e), "error");
  }
}

/** Open the clipped note behind a source (a no-op when it has none). */
export function openClipped(url: string) {
  const p = clips.pathFor(url);
  if (p) void hostDispatch({ kind: "open-note", path: p });
}
