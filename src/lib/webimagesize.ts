// The sizes of the web pictures the reading projection has shown, remembered by URL, so
// the next time the page is projected its <img> carries width and height and holds the
// picture's place before it arrives. Without them a picture lazily loaded above the
// reader — a page reopened at its remembered position, a quick scroll past — pushed the
// text being read down by its whole height. A picture seen for the first time still
// arrives unsized: its URL says nothing of its size, and the projection makes no request
// of its own to find out.
// The URLs are kept hashed (this is a record of what was read, not a history to show),
// in the webview's storage, bounded, the oldest dropped first; a storage that cannot be
// read or written (a private window) just means no sizes.

const KEY = "vemry-web-image-sizes";
const MAX = 1500;

let table: Map<string, [number, number]> | null = null;
let saveTimer: ReturnType<typeof setTimeout> | null = null;

/** 53 bits of FNV-1a over the URL: a key, not something that can be read back. */
function keyOf(url: string): string {
  let h1 = 0x811c9dc5;
  let h2 = 0x01000193;
  for (let i = 0; i < url.length; i++) {
    const c = url.charCodeAt(i);
    h1 = Math.imul(h1 ^ c, 0x01000193);
    h2 = Math.imul(h2 ^ c, 0x5bd1e995);
  }
  return ((h2 >>> 0) & 0x1fffff).toString(36) + (h1 >>> 0).toString(36);
}

function sizes(): Map<string, [number, number]> {
  if (table) return table;
  table = new Map();
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) for (const [k, w, h] of JSON.parse(raw) as [string, number, number][]) table.set(k, [w, h]);
  } catch {
    /* unreadable storage: start empty */
  }
  return table;
}

function save() {
  saveTimer = null;
  try {
    localStorage.setItem(KEY, JSON.stringify([...sizes()].map(([k, [w, h]]) => [k, w, h])));
  } catch {
    /* storage full or blocked: the sizes stay for this session */
  }
}

/** The size a web picture had when last shown, if it was. */
export function webImageSize(url: string): [number, number] | undefined {
  return sizes().get(keyOf(url));
}

function remember(url: string, w: number, h: number) {
  if (!(w > 0 && h > 0)) return;
  const t = sizes();
  const k = keyOf(url);
  const old = t.get(k);
  if (old && old[0] === w && old[1] === h) return;
  t.delete(k);
  t.set(k, [w, h]);
  while (t.size > MAX) t.delete(t.keys().next().value!);
  saveTimer ??= setTimeout(save, 1000);
}

/** Record each projected picture's size as it loads (a capture-phase listener on the
 *  document, installed once by main.ts). */
export function installWebImageSizes(): void {
  document.addEventListener(
    "load",
    (e) => {
      const img = e.target;
      if (img instanceof HTMLImageElement && img.classList.contains("md-imgload") && img.dataset.url) {
        remember(img.dataset.url, img.naturalWidth, img.naturalHeight);
      }
    },
    true,
  );
}
