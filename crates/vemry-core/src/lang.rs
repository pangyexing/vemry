//! What a note reads as: the language it declares, or the one its letters give away,
//! and the writing system it is set in. The index carries the verdict on every note
//! summary so a surface that shows a note's name or a line from it (the file tree, the
//! task views) can set it in the right faces without reading the note. Mirrors
//! `src/lib/notelang.ts`'s `noteLanguage()` rule for rule; change both together.

use serde::Serialize;

/// Below this many letters a text follows the interface.
const MIN_LETTERS: usize = 300;
/// Enough letters to settle the ratio; the scan stops there.
const SCAN_LETTERS: usize = 4000;

/// Characters written only in Traditional Chinese, paired position for position with
/// their Simplified forms below.
const TRAD: &str = "們個這說時國會來對學為與後麼樣還開經從現關點動應將過發機長問頭實體當無兩見電認沒進給產業義讓總條話區種邊聽寫書幾讀變請師壓樂氣歲顯論選聯護聲導隊術劃錢廣報際覺轉鐵夠極質據戰識標響環線須難驗觀團費館飛東車馬門語樹萬單買賣筆紅綠藍島鳥魚雲風處親愛歡記頁圖裡級歷務員層網張並嗎誰";
const SIMP: &str = "们个这说时国会来对学为与后么样还开经从现关点动应将过发机长问头实体当无两见电认没进给产业义让总条话区种边听写书几读变请师压乐气岁显论选联护声导队术划钱广报际觉转铁够极质据战识标响环线须难验观团费馆飞东车马门语树万单买卖笔红绿蓝岛鸟鱼云风处亲爱欢记页图里级历务员层网张并吗谁";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Script {
    Cjk,
    Latin,
    Rtl,
    Other,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct NoteLanguage {
    pub lang: Option<String>,
    pub script: Option<Script>,
}

/// The writing system a BCP 47 tag is written in.
pub fn script_of_lang(lang: &str) -> Script {
    let l = lang.to_ascii_lowercase();
    let primary = l.split(['-', '_']).next().unwrap_or("");
    match primary {
        "zh" | "ja" | "ko" => Script::Cjk,
        "ar" | "he" | "fa" | "ur" | "yi" | "ps" | "iw" => Script::Rtl,
        "ru" | "uk" | "be" | "bg" | "sr" | "mk" | "el" | "th" | "hi" | "bn" | "ta" | "te" | "ka" | "hy" | "km" | "lo"
        | "my" | "si" | "am" => Script::Other,
        _ => Script::Latin,
    }
}

fn is_han(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x3134F)
}
fn is_kana(c: char) -> bool {
    matches!(c as u32, 0x3041..=0x309F | 0x30A0..=0x30FF | 0x31F0..=0x31FF | 0xFF66..=0xFF9F)
}
fn is_hangul(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x11FF | 0x3130..=0x318F | 0xAC00..=0xD7AF)
}
fn is_arabic(c: char) -> bool {
    matches!(c as u32, 0x0600..=0x06FF | 0x0750..=0x077F | 0x08A0..=0x08FF | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF)
}
fn is_hebrew(c: char) -> bool {
    matches!(c as u32, 0x0590..=0x05FF | 0xFB1D..=0xFB4F)
}
fn is_latin(c: char) -> bool {
    c.is_ascii_alphabetic() || matches!(c as u32, 0x00C0..=0x024F | 0x1E00..=0x1EFF)
}

/// What a note's raw text reads as: `lang` from the frontmatter (already parsed by the
/// caller) wins; otherwise the letters of the body decide.
pub fn note_language(declared: Option<&str>, body: &str) -> NoteLanguage {
    if let Some(tag) = declared.map(str::trim).filter(|t| !t.is_empty()) {
        let tag = tag.replace('_', "-");
        let script = script_of_lang(&tag);
        return NoteLanguage { lang: Some(tag), script: Some(script) };
    }
    let (mut han, mut kana, mut hangul, mut arabic, mut hebrew, mut latin, mut other) = (0usize, 0, 0, 0, 0, 0, 0);
    let (mut trad, mut simp, mut letters) = (0usize, 0, 0);
    for c in body.chars() {
        if !c.is_alphabetic() {
            continue;
        }
        letters += 1;
        if is_han(c) {
            han += 1;
            if TRAD.contains(c) {
                trad += 1;
            } else if SIMP.contains(c) {
                simp += 1;
            }
        } else if is_kana(c) {
            kana += 1;
        } else if is_hangul(c) {
            hangul += 1;
        } else if is_arabic(c) {
            arabic += 1;
        } else if is_hebrew(c) {
            hebrew += 1;
        } else if is_latin(c) {
            latin += 1;
        } else {
            other += 1;
        }
        if letters >= SCAN_LETTERS {
            break;
        }
    }
    if letters < MIN_LETTERS {
        return NoteLanguage::default();
    }
    let cjk = (han + kana + hangul) * 5;
    let rtl = arabic + hebrew;
    let total = (cjk + rtl + latin + other) as f64;
    let found = |lang: Option<&str>, script| NoteLanguage { lang: lang.map(String::from), script: Some(script) };
    if rtl as f64 >= total * 0.3 {
        return found(Some(if arabic >= hebrew { "ar" } else { "he" }), Script::Rtl);
    }
    if cjk as f64 >= total * 0.5 {
        if hangul >= han + kana {
            return found(Some("ko"), Script::Cjk);
        }
        // Japanese is never written without kana; a handful in a Chinese note are a quoted name
        if kana >= 20 || (kana > 0 && kana as f64 >= (han + kana) as f64 * 0.05) {
            return found(Some("ja"), Script::Cjk);
        }
        if trad + simp >= 5 {
            return found(Some(if trad > simp { "zh-Hant" } else { "zh-Hans" }), Script::Cjk);
        }
        return found(None, Script::Cjk);
    }
    if latin as f64 >= total * 0.5 {
        return found(None, Script::Latin);
    }
    found(None, Script::Other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_wins() {
        let nl = note_language(Some("de_AT"), "短");
        assert_eq!(nl.lang.as_deref(), Some("de-AT"));
        assert_eq!(nl.script, Some(Script::Latin));
    }

    #[test]
    fn short_text_follows_the_interface() {
        assert_eq!(note_language(None, "A heading and two lines."), NoteLanguage::default());
    }

    #[test]
    fn letters_decide() {
        let ja = "これは日本語の文章です。睡眠と記憶について書いています。".repeat(12);
        assert_eq!(note_language(None, &ja).lang.as_deref(), Some("ja"));
        let ko = "이것은 한국어 문장입니다 수면과 기억에 대해 씁니다 ".repeat(20);
        assert_eq!(note_language(None, &ko).lang.as_deref(), Some("ko"));
        let hant = "這是一個關於睡眠與記憶的筆記，我們會說明學習時間的問題。".repeat(12);
        assert_eq!(note_language(None, &hant).lang.as_deref(), Some("zh-Hant"));
        let hans = "这是一个关于睡眠与记忆的笔记，我们会说明学习时间的问题。".repeat(12);
        assert_eq!(note_language(None, &hans).lang.as_deref(), Some("zh-Hans"));
        let he = "זהו טקסט בעברית על שינה וזיכרון ".repeat(20);
        assert_eq!(note_language(None, &he), NoteLanguage { lang: Some("he".into()), script: Some(Script::Rtl) });
        let en = "Sleep consolidates memory during the night. ".repeat(12);
        assert_eq!(note_language(None, &en), NoteLanguage { lang: None, script: Some(Script::Latin) });
        // a Chinese note full of English terms still reads as Chinese
        let mixed = "读 Walker 2017 的第 6 章，关于 REM sleep 和 memory consolidation 的讨论。".repeat(10);
        assert_eq!(note_language(None, &mixed).script, Some(Script::Cjk));
    }
}
