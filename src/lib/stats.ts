// The document word count (for the status line). The rule matches what a word count
// is usually taken to mean for Markdown:
// the scripts written without word spacing (Han, kana) count 1 per character, and
// everything else — Latin, Cyrillic, Greek, and Hangul, which is written with spaces —
// counts 1 per word;
// the Markdown syntax is not stripped — what is counted is the source, so comments
// and link targets are writing too.
//
// Code points are compared one by one rather than through a regex match/replace: this
// runs behind a 200ms input debounce, and the old form would materialise an array of
// "every CJK character in the document" plus a copy of the whole text for each count
// (several MB for a document of a few hundred thousand characters), while a single
// pass allocates nothing and measures 6× faster.
import { i18n } from "$lib/i18n/i18n.svelte";

/** The scripts counted per character: Han (plus the compatibility ideographs and the
 *  extension A block), Hiragana, Katakana and half-width Katakana. Hangul is not here —
 *  Korean puts spaces between its words, so a run of Hangul is one word, like Latin. */
const isPerChar = (c: number) =>
  (c >= 0x4e00 && c <= 0x9fff) || // CJK unified ideographs
  (c >= 0xf900 && c <= 0xfaff) || // CJK compatibility ideographs
  (c >= 0x3400 && c <= 0x4dbf) || // extension A
  (c >= 0x3040 && c <= 0x30ff) || // Hiragana + Katakana
  (c >= 0xff66 && c <= 0xff9f) || // half-width Katakana
  (c >= 0x20000 && c <= 0x3ffff); // extensions B and beyond (astral)

/** The word-character set: any letter or digit of any script (so Korean, Cyrillic,
 *  Greek, Devanagari all count as words) minus the per-character scripts above, plus
 *  the marks that hold a word together. */
const WORDISH = /[\p{L}\p{N}\p{M}]/u;
const isWordChar = (c: number) =>
  !isPerChar(c) &&
  (c === 0x5f || // _
    c === 0x27 || // '
    c === 0x2d || // -
    c === 0x2019 || // ’
    WORDISH.test(String.fromCodePoint(c)));

/** The degradation threshold for a very long document: a full scan behind a 200ms debounce would stall typing, so it falls back to counting characters alone. */
const DEGRADE_CHARS = 500_000;

export function countWords(text: string): number {
  if (text.length > DEGRADE_CHARS) return text.length;
  let n = 0;
  let inWord = false;
  for (const ch of text) {
    const c = ch.codePointAt(0)!;
    if (isPerChar(c)) {
      n++;
      inWord = false;
    } else if (isWordChar(c)) {
      // A run of word characters adds 1 at the start of the word only
      if (!inWord) n++;
      inWord = true;
    } else {
      inWord = false;
    }
  }
  return n;
}

// One formatter per locale and precision: `Intl.NumberFormat` re-parses its options on
// every construction, and the gauges rebuild on each streamed token.
const NUMBER_FORMATS = new Map<string, Intl.NumberFormat>();
function compact(digits: number): Intl.NumberFormat {
  const key = `${i18n.locale}/${digits}`;
  let f = NUMBER_FORMATS.get(key);
  if (!f) {
    f = new Intl.NumberFormat(i18n.locale, { notation: "compact", maximumFractionDigits: digits });
    NUMBER_FORMATS.set(key, f);
  }
  return f;
}

/** A count as a gauge shows it: `847`, then the locale's own compact form (`1.2K`,
 *  `124K` in English, `1.2万` in Chinese). The chat's usage gauge, its thinking counter
 *  and the translation's token count read this one rule. */
export function fmtCount(n: number): string {
  if (n < 1000) return String(n);
  return compact(n >= 10_000 ? 0 : 1).format(n);
}
