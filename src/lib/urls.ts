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
