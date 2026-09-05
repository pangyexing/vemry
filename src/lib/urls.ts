// Small URL helpers shared by the places that list addresses (the palette's web space,
// the browser panel's new-tab page) and the inputs that accept one.

/** A URL's display and filter form: the scheme is dropped and the percent encoding decoded
 *  (so a non-ASCII path is readable and can be matched by a query in that script); a failed
 *  decode (a malformed sequence) is kept as written. Opening still uses the raw url. */
export function prettyUrl(u: string): string {
  const s = u.replace(/^https?:\/\//, "");
  try {
    return decodeURI(s);
  } catch {
    return s;
  }
}

/** Whether typed text is an address rather than a query: a scheme, or a domain with an alphabetic top-level label plus an optional path. */
export function isUrlLike(s: string): boolean {
  const it = s.trim();
  if (!it || /\s/.test(it)) return false;
  return /^https?:\/\/\S+$/i.test(it) || /^[\w-]+(\.[\w-]+)*\.[a-z]{2,}(\/\S*)?$/i.test(it);
}

/** An entry of the address lists: a bookmark or a history entry, with its display form and filter key ready. */
export type WebCandidate = { url: string; title: string; starred: boolean; pretty: string; key: string };

/** The pool the address lists draw on (the palette's web space, the new-tab page):
 *  bookmarks first plus the history, deduplicated by url (first seen wins = bookmarks take
 *  precedence), with the display form decoded and the filter key lowercased once per
 *  entry. Built from the two mirrors alone, never per keystroke — decoding up to 500
 *  URLs on every character typed is what this avoids. */
export function webCandidates(bookmarks: { url: string; title: string }[], history: { url: string; title: string }[]): WebCandidate[] {
  const seen = new Set<string>();
  return [
    ...bookmarks.map((b) => ({ url: b.url, title: b.title, starred: true })),
    ...history.map((h) => ({ url: h.url, title: h.title, starred: false })),
  ]
    .filter((c) => (seen.has(c.url) ? false : (seen.add(c.url), true)))
    .map((c) => {
      const pretty = prettyUrl(c.url);
      return { ...c, pretty, key: `${c.title} ${pretty}`.toLowerCase() };
    });
}
