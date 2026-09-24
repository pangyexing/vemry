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

/** The writing system a text is set in, which is what the column width, the direction
 *  and the line-breaking rules depend on (app.css `[data-script]`). */
export type NoteScript = "cjk" | "latin" | "rtl" | "other";

/** What a piece of text reads as: a BCP 47 tag when one is known (declared, or one of the
 *  languages the script alone gives away), and the writing system. Both null when the text
 *  is too short to say: the surface then follows the interface, as before. */
export interface NoteLanguage {
  lang: string | null;
  script: NoteScript | null;
}

const NONE: NoteLanguage = { lang: null, script: null };

/** Below this many letters a text follows the interface: a heading and two lines of Latin
 *  in a Chinese note must not flip the whole note to a Latin column. */
const MIN_LETTERS = 300;
/** Enough letters to settle the ratio; the scan stops there (this runs on every open). */
const SCAN_LETTERS = 4000;

const HAN = /\p{Script=Han}/u;
const KANA = /[\p{Script=Hiragana}\p{Script=Katakana}]/u;
const HANGUL = /\p{Script=Hangul}/u;
const ARABIC = /\p{Script=Arabic}/u;
const HEBREW = /\p{Script=Hebrew}/u;
const LATIN = /\p{Script=Latin}/u;
const LETTER = /\p{L}/u;

// Characters written only in one of the two Chinese standards, among the most frequent
// few hundred of each: which set a text draws from tells Traditional from Simplified.
// Pairs are aligned (個/个 …) so neither side is favoured by list length. Mirrored in
// crates/vemry-core/src/lang.rs, which gives the index the same verdict.
const TRAD = new Set(
  "們個這說時國會來對學為與後麼樣還開經從現關點動應將過發機長問頭實體當無兩見電認沒進給產業義讓總條話區種邊聽寫書幾讀變請師壓樂氣歲顯論選聯護聲導隊術劃錢廣報際覺轉鐵夠極質據戰識標響環線須難驗觀團費館飛東車馬門語樹萬單買賣筆紅綠藍島鳥魚雲風處親愛歡記頁圖裡級歷務員層網張並嗎誰"
);
const SIMP = new Set(
  "们个这说时国会来对学为与后么样还开经从现关点动应将过发机长问头实体当无两见电认没进给产业义让总条话区种边听写书几读变请师压乐气岁显论选联护声导队术划钱广报际觉转铁够极质据战识标响环线须难验观团费馆飞东车马门语树万单买卖笔红绿蓝岛鸟鱼云风处亲爱欢记页图里级历务员层网张并吗谁"
);

/** The frontmatter block's length, so detection reads the body only (its keys are Latin). */
function bodyStart(text: string): number {
  if (!text.startsWith("---")) return 0;
  const m = /\r?\n---[ \t]*(?:\r?\n|$)/.exec(text.slice(3));
  return m ? 3 + m.index + m[0].length : 0;
}

/** The writing system a BCP 47 tag is written in. */
export function scriptOfLang(lang: string): NoteScript {
  const l = lang.toLowerCase();
  if (/^(zh|ja|ko)(-|$)/.test(l)) return "cjk";
  if (/^(ar|he|fa|ur|yi|ps|iw)(-|$)/.test(l)) return "rtl";
  if (/^(ru|uk|be|bg|sr|mk|el|th|hi|bn|ta|te|ka|hy|km|lo|my|si|am)(-|$)/.test(l)) return "other";
  return "latin";
}

/** What a note (or any text: a chat answer, a translated cell) reads as. A `lang:` in the
 *  frontmatter wins; otherwise the letters decide: kana means Japanese, hangul Korean,
 *  ideographs alone Chinese (Traditional or Simplified by which standard's own characters
 *  it draws on), Arabic or Hebrew letters their language; Latin and the other alphabets
 *  only give the writing system, since the letters do not say which language it is.
 *  Callers read this once when a surface opens and do not re-read it while the reader
 *  edits: a column that changed width mid-sentence would move the text under the cursor. */
export function noteLanguage(text: string): NoteLanguage {
  const declared = noteLang(text.slice(0, 2000));
  if (declared) return { lang: declared, script: scriptOfLang(declared) };

  let han = 0, kana = 0, hangul = 0, arabic = 0, hebrew = 0, latin = 0, other = 0;
  let trad = 0, simp = 0;
  let letters = 0;
  for (const ch of text.slice(bodyStart(text))) {
    if (!LETTER.test(ch)) continue;
    letters++;
    if (HAN.test(ch)) {
      han++;
      if (TRAD.has(ch)) trad++;
      else if (SIMP.has(ch)) simp++;
    } else if (KANA.test(ch)) kana++;
    else if (HANGUL.test(ch)) hangul++;
    else if (ARABIC.test(ch)) arabic++;
    else if (HEBREW.test(ch)) hebrew++;
    else if (LATIN.test(ch)) latin++;
    else other++;
    if (letters >= SCAN_LETTERS) break;
  }
  if (letters < MIN_LETTERS) return NONE;

  // An ideograph, a kana or a hangul syllable is roughly a word; a Latin letter a fifth of
  // one — weighed so a Chinese note full of English terms still reads as Chinese
  const cjk = (han + kana + hangul) * 5;
  const rtl = arabic + hebrew;
  const total = cjk + rtl + latin + other;
  if (rtl >= total * 0.3) return { lang: arabic >= hebrew ? "ar" : "he", script: "rtl" };
  if (cjk >= total * 0.5) {
    if (hangul >= han + kana) return { lang: "ko", script: "cjk" };
    // Japanese is never written without kana; a handful in a Chinese note are a quoted name
    if (kana >= 20 || (kana > 0 && kana >= (han + kana) * 0.05)) return { lang: "ja", script: "cjk" };
    if (trad + simp >= 5) return { lang: trad > simp ? "zh-Hant" : "zh-Hans", script: "cjk" };
    return { lang: null, script: "cjk" };
  }
  if (latin >= total * 0.5) return { lang: null, script: "latin" };
  return { lang: null, script: "other" };
}

/** The attributes a surface stamps on its text container: `lang` so fonts and line breaking
 *  follow the text, `data-script` so the column width follows its writing system. A key
 *  whose value is unknown is absent, and the interface's own `lang` on :root applies. */
export function langAttrs(nl: NoteLanguage): { lang?: string; "data-script"?: NoteScript } {
  const out: { lang?: string; "data-script"?: NoteScript } = {};
  if (nl.lang) out.lang = nl.lang;
  if (nl.script) out["data-script"] = nl.script;
  return out;
}

/** Stamp (or clear) those attributes on an element directly, for a surface built outside
 *  Svelte's templates (the editor's content element, an embed widget). */
export function applyLangAttrs(el: HTMLElement, nl: NoteLanguage): void {
  if (nl.lang) el.setAttribute("lang", nl.lang);
  else el.removeAttribute("lang");
  if (nl.script) el.setAttribute("data-script", nl.script);
  else el.removeAttribute("data-script");
}
