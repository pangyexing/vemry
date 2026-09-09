// Which script a text is mostly written in — the one question the status bar's
// "translate" chip asks. A ratio of characters, nothing more: no request, no
// dictionary, and a short text answers "unknown" rather than guessing.

export type Script = "cjk" | "latin" | "other";

/** How many letters the verdict needs; below it the text is too short to say. */
const MIN_LETTERS = 300;

const CJK = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u;
const LATIN = /\p{Script=Latin}/u;
const OTHER_LETTER = /\p{L}/u;

/** The script most of the letters belong to, or null when the text is too short. */
export function dominantScript(text: string): Script | null {
  let cjk = 0;
  let latin = 0;
  let other = 0;
  for (const ch of text) {
    if (CJK.test(ch)) cjk++;
    else if (LATIN.test(ch)) latin++;
    else if (OTHER_LETTER.test(ch)) other++;
    // the ratio has long settled by here, and this runs over a whole note on every save
    if (cjk + latin + other >= MIN_LETTERS * 4) break;
  }
  // A CJK character is a word; a Latin letter is a fifth of one — weighed so a
  // Chinese note with English terms in it still reads as Chinese
  const cjkWeight = cjk * 5;
  const total = cjkWeight + latin + other;
  if (cjk + latin + other < MIN_LETTERS) return null;
  if (cjkWeight >= total * 0.5) return "cjk";
  if (latin >= total * 0.5) return "latin";
  return "other";
}

/** The script a target language is written in. */
export function scriptOf(lang: string): Script {
  const l = lang.toLowerCase();
  if (l.startsWith("zh") || l.startsWith("ja") || l.startsWith("ko")) return "cjk";
  return "latin";
}

/** Whether a note reads as written in a language other than `target`'s: the
 *  status bar shows its chip on this alone. Frontmatter is left out — its keys
 *  are always Latin. */
export function readsForeign(text: string, target: string): boolean {
  const body = text.startsWith("---\n") ? text.slice(text.indexOf("\n---", 4) + 4) : text;
  const script = dominantScript(body);
  return script !== null && script !== scriptOf(target);
}
