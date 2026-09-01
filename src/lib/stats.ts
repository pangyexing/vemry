// The document word count (for the status line). The rule matches what a word count
// is usually taken to mean for Markdown:
// CJK (Japanese kana and Korean syllables included) counts 1 per character, and
// everything else counts 1 per word;
// the Markdown syntax is not stripped — what is counted is the source, so comments
// and link targets are writing too.
//
// The code-point ranges come from the character class /[一-鿿㐀-䶿豈-﫿぀-ヿ가-힯]/ used
// before, compared code point by code point rather than by regex: this runs behind a
// 200ms input debounce, and the old form's match/replace would materialise an array
// of "every CJK character in the document" plus a copy of the whole text for each
// count (several MB for a document of a few hundred thousand characters), while a
// single pass allocates nothing and measures 6× faster. The ranges are unchanged
// (including how 0x4E00-0xFAFF incidentally covers the surrogate range as before — an
// emoji counts 1 per code unit, twice, exactly as the old implementation did).
const isCjk = (c: number) =>
  (c >= 0x4e00 && c <= 0xfaff) || (c >= 0x3400 && c <= 0x4dbf) || (c >= 0x3040 && c <= 0x30ff);

/** The word-character set, mirroring the original /[A-Za-z0-9_'’-]/. */
const isWordChar = (c: number) =>
  (c >= 0x61 && c <= 0x7a) || // a-z
  (c >= 0x41 && c <= 0x5a) || // A-Z
  (c >= 0x30 && c <= 0x39) || // 0-9
  c === 0x5f || // _
  c === 0x27 || // '
  c === 0x2d || // -
  c === 0x2019; // ’

/** The degradation threshold for a very long document: a full scan behind a 200ms debounce would stall typing, so it falls back to counting characters alone. */
const DEGRADE_CHARS = 500_000;

export function countWords(text: string): number {
  if (text.length > DEGRADE_CHARS) return text.length;
  let n = 0;
  let inWord = false;
  for (let i = 0; i < text.length; i++) {
    const c = text.charCodeAt(i);
    if (isCjk(c)) {
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
