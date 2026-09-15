/** The language a note declares for itself: a `lang:` key in its frontmatter with a
 *  BCP 47 tag (`lang: ja`, `lang: de-AT`). The editor body carries it as a `lang`
 *  attribute so fonts, line breaking and hyphenation follow the note rather than the
 *  interface (app.css `[lang]`); a note without one follows the interface. */
export function noteLang(head: string): string | null {
  if (!head.startsWith("---")) return null;
  const end = head.search(/\r?\n---\s*(?:\r?\n|$)/);
  if (end === -1) return null;
  const fm = head.slice(3, end);
  const m = /(?:^|\r?\n)lang:[ \t]*["']?([A-Za-z]{2,3}(?:[-_][A-Za-z0-9]{2,8})*)["']?[ \t]*(?=\r?\n|$)/.exec(fm);
  return m ? m[1].replace("_", "-") : null;
}
