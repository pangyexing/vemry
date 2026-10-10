// A note's remote images (`![](https://…)`): which may load, and through what.
//
// Opening a note makes no network request (the network-exit list, exit 4): a remote
// image is first asked of the vemry-asset proxy's disk cache alone, and shows if it is
// there — an image seen in the Reader, or loaded once before. One that is not stays a
// placeholder until the user lets it load: that image, every image in the note, or every
// remote image always (the `note_remote_images` setting). An image the user pastes in is
// theirs to see at once, the same trust as clipping the page it came from. Whatever loads
// goes through the proxy — no cookies, SSRF validation, and the cache that makes the next
// opening offline again — never straight from the webview, which the CSP forbids anyway.
import { EditorView } from "@codemirror/view";
import { convertFileSrc } from "@tauri-apps/api/core";

/** Images the user let load in this run of the app (decoded URLs, as the widget sees them). */
const allowed = new Set<string>();
/** The setting: every remote image loads without asking. */
let always = false;
/** Placeholders on screen, each waiting to hear that its image may now load. */
const waiting = new Set<() => void>();

/** prefs.refresh mirrors the setting here; turning it on wakes every placeholder. */
export function setAlwaysLoadRemote(on: boolean) {
  const woke = on && !always;
  always = on;
  if (woke) wake();
}

export function mayLoadRemote(url: string): boolean {
  return always || allowed.has(url);
}

/** Let these images load for the rest of the run, and wake the placeholders showing them. */
export function allowRemote(urls: Iterable<string>) {
  let added = false;
  for (const u of urls) {
    if (allowed.has(u)) continue;
    allowed.add(u);
    added = true;
  }
  if (added) wake();
}

function wake() {
  for (const f of [...waiting]) f();
}

/** A placeholder subscribes while it is mounted; the return unsubscribes. */
export function onRemoteAllowed(f: () => void): () => void {
  waiting.add(f);
  return () => waiting.delete(f);
}

const REMOTE_IMAGE_RE = /!\[[^\]\n]*\]\(\s*<?(https?:\/\/[^\s)>]+)>?(?:\s+"[^"\n]*")?\s*\)/gi;

/** The http(s) image targets in a stretch of Markdown, each once, decoded the way the
 *  live preview decodes a target before it builds the widget (so the two compare equal). */
export function remoteImageTargets(text: string): string[] {
  const out = new Set<string>();
  for (const m of text.matchAll(REMOTE_IMAGE_RE)) {
    let url = m[1];
    try {
      url = decodeURIComponent(url);
    } catch {
      /* not encoded by us, so it is kept as written */
    }
    out.add(url);
  }
  return [...out];
}

/** What a paste or a drop brings in is the user's own to see: its remote images may
 *  load at once. The widgets for them are built in the same update, before this runs,
 *  which is why a placeholder looks again before it shows (livepreview's ImageWidget). */
export const allowPastedRemoteImages = EditorView.updateListener.of((u) => {
  for (const tr of u.transactions) {
    if (!tr.isUserEvent("input.paste") && !tr.isUserEvent("input.drop")) continue;
    tr.changes.iterChanges((_fa, _ta, _fb, _tb, inserted) => {
      if (inserted.length) allowRemote(remoteImageTargets(inserted.toString()));
    });
  }
});

/** The proxy address for a note's remote image; `cacheOnly` asks the disk cache and
 *  nothing else (an unseen image answers 404). `referer` is the note's source page, or "". */
export function noteImageUrl(url: string, referer: string, cacheOnly: boolean): string {
  let q = `u=${encodeURIComponent(url)}&k=note`;
  if (referer) q += `&r=${encodeURIComponent(referer)}`;
  if (cacheOnly) q += "&c=1";
  return `${convertFileSrc("", "vemry-asset")}?${q}`;
}
