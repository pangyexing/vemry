//! The attributes a task line carries after its text, and how they are read.
//!
//! Every attribute has an emoji spelling and a bracketed `[key:: value]` spelling; both
//! parse to the same value. The attributes are the line's trailing run: read from the
//! end backwards, a token that is an attribute, a `#tag`, a `[key:: value]` field or the
//! `^anchor` continues the run, and the first token that is none of these ends it —
//! everything before is the description, and a `📅` in the description is a word.
//! Within the run the same attribute twice reads as the last one (the editor underlines
//! the other). An attribute vemry adds to a line goes into the order of `AttrKind`'s
//! declaration — id, depends on, priority, repeat, removed once done, estimate,
//! created, start, scheduled, due, cancelled, done — and the anchor stays last.
//! The frontend mirrors this file in `views/tasks.ts` (scanTaskAttrs, insertAttr) for
//! the editor's chips and the views' rewrites — change the two together.
//!
//! | attribute        | emoji | `[key:: v]`      | value                                   |
//! |------------------|-------|------------------|-----------------------------------------|
//! | id               | 🆔    | `id::`           | one token                               |
//! | depends on       | ⛔    | `dependsOn::`    | ids, comma-separated                    |
//! | priority         | ❗❗ ❗ ❕ ❕❕ | `priority::` | `highest` `high` `low` `lowest`   |
//! | repeat           | 🔁    | `repeat::`       | the rule's text, see `repeat.rs`        |
//! | delete when done | 🚮    | `onCompletion::` | `delete`                                |
//! | estimate         | 🕐    | `estimate::`     | `30m` `2h` `1d` (`d` = 8h)              |
//! | created          | ➕    | `created::`      | `YYYY-MM-DD`                            |
//! | start            | ⏩    | `start::`        | `YYYY-MM-DD`                            |
//! | scheduled        | ⏳    | `scheduled::`    | `YYYY-MM-DD`                            |
//! | due              | 📅    | `due::`          | `YYYY-MM-DD`                            |
//! | cancelled        | ❌    | `cancelled::`    | `YYYY-MM-DD`                            |
//! | done             | ✅    | `completion::`   | `YYYY-MM-DD`                            |
//!
//! Reading rules, the same for every attribute: a U+FE0F after an emoji is ignored; a
//! date is `YYYY-MM-DD` exactly; a marker whose token is not a value (`⏳ waiting`) is
//! text; the space between an emoji and its value is optional, and an emoji glued to
//! the word before it (`写周报📅 …`) still opens an attribute; a bare `key:: value` is an
//! ordinary field, not an attribute; a repeat rule's text runs to the next attribute,
//! field, tag or anchor. The second and later entries of `AttrKind::emoji` and the
//! `medium` priority word are read as well; nothing writes them, and they are test
//! material rather than a promise.

use crate::block::{BlockKind, Priority, TaskState};
use crate::parser::scan_fields;
use chrono::NaiveDate;
use std::ops::Range;

/// The attributes, in the order vemry writes them onto a line (`Ord` is that order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AttrKind {
    Id,
    Depends,
    Priority,
    Repeat,
    Delete,
    Estimate,
    Created,
    Start,
    Scheduled,
    Due,
    Cancelled,
    Done,
}

impl AttrKind {
    pub const ALL: [AttrKind; 12] = [
        AttrKind::Id,
        AttrKind::Depends,
        AttrKind::Priority,
        AttrKind::Repeat,
        AttrKind::Delete,
        AttrKind::Estimate,
        AttrKind::Created,
        AttrKind::Start,
        AttrKind::Scheduled,
        AttrKind::Due,
        AttrKind::Cancelled,
        AttrKind::Done,
    ];
    const DATES: [AttrKind; 6] = [AttrKind::Created, AttrKind::Start, AttrKind::Scheduled, AttrKind::Due, AttrKind::Cancelled, AttrKind::Done];

    /// The emoji spellings: the first is the one written, the rest are read as well.
    pub fn emoji(self) -> &'static [&'static str] {
        match self {
            AttrKind::Id => &["🆔"],
            AttrKind::Depends => &["⛔"],
            AttrKind::Priority => &["❗", "❕", "🔺", "⏫", "🔼", "🔽", "⏬"],
            AttrKind::Repeat => &["🔁"],
            AttrKind::Delete => &["🚮", "🏁"],
            AttrKind::Estimate => &["🕐"],
            AttrKind::Created => &["➕"],
            AttrKind::Start => &["⏩", "🛫"],
            AttrKind::Scheduled => &["⏳", "⌛"],
            AttrKind::Due => &["📅"],
            AttrKind::Cancelled => &["❌"],
            AttrKind::Done => &["✅"],
        }
    }

    /// The `key::` name, lowercase (`completion` for the done date).
    pub fn key(self) -> &'static str {
        match self {
            AttrKind::Id => "id",
            AttrKind::Depends => "dependson",
            AttrKind::Priority => "priority",
            AttrKind::Repeat => "repeat",
            AttrKind::Delete => "oncompletion",
            AttrKind::Estimate => "estimate",
            AttrKind::Created => "created",
            AttrKind::Start => "start",
            AttrKind::Scheduled => "scheduled",
            AttrKind::Due => "due",
            AttrKind::Cancelled => "cancelled",
            AttrKind::Done => "completion",
        }
    }

    pub fn from_key(key: &str) -> Option<AttrKind> {
        let lower = key.to_ascii_lowercase();
        AttrKind::ALL.into_iter().find(|k| k.key() == lower)
    }

    pub fn is_date(self) -> bool {
        AttrKind::DATES.contains(&self)
    }
}

/// Minutes in an estimate's `d`: a working day.
pub const DAY_MINUTES: u32 = 8 * 60;

/// Everything a task line says besides its checkbox and its text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskAttrs {
    pub due: Option<NaiveDate>,
    pub scheduled: Option<NaiveDate>,
    pub start: Option<NaiveDate>,
    pub created: Option<NaiveDate>,
    pub done: Option<NaiveDate>,
    pub cancelled: Option<NaiveDate>,
    pub priority: Priority,
    pub repeat: Option<String>,
    pub delete_on_done: bool,
    pub task_id: Option<String>,
    pub depends_on: Vec<String>,
    /// Minutes.
    pub estimate: Option<u32>,
}

impl TaskAttrs {
    pub fn into_kind(self, state: TaskState) -> BlockKind {
        BlockKind::Task {
            state,
            due: self.due,
            scheduled: self.scheduled,
            start: self.start,
            created: self.created,
            done: self.done,
            cancelled: self.cancelled,
            priority: self.priority,
            repeat: self.repeat,
            delete_on_done: self.delete_on_done,
            task_id: self.task_id,
            depends_on: self.depends_on,
            estimate: self.estimate,
        }
    }

}

/// One attribute occurrence on a line, as the scanner found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttrMark {
    pub kind: AttrKind,
    /// Byte range within the scanned text: the marker through its value, a bracketed
    /// field with its brackets.
    pub span: Range<usize>,
    /// The value as read: a date as written, a priority word, a rule's text, an id, ids
    /// joined by `,`, an estimate token; empty for 🚮.
    pub value: String,
    /// The value reads: a real date, a priority word, a duration, a non-empty id or rule.
    pub valid: bool,
    /// In the trailing run, where an attribute counts (implies `valid`).
    pub tail: bool,
    /// What the parser reads: in the tail, and the last of its kind there.
    pub active: bool,
}

/// Without U+FE0F: an emoji typed with the variation selector reads as the bare one.
/// Borrowed when there is none, which is the overwhelmingly common token — this runs
/// several times per token of every task line in the vault.
pub fn strip_vs16(s: &str) -> std::borrow::Cow<'_, str> {
    if s.contains('\u{FE0F}') {
        std::borrow::Cow::Owned(s.chars().filter(|&c| c != '\u{FE0F}').collect())
    } else {
        std::borrow::Cow::Borrowed(s)
    }
}

/// `YYYY-MM-DD` exactly, and a real calendar date.
///
/// The input is NFKC-normalised first, so the full-width digits and dashes an IME
/// emits (`２０２６－０９－１０`) parse. Only the reading is tolerant: nothing here
/// writes, and the file keeps whatever the user typed.
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    let s = crate::search::normalize(s);
    let s: &str = &s;
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' || !b.iter().enumerate().all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()) {
        return None;
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// A token that is a priority by itself.
fn priority_token(tok: &str) -> Option<Priority> {
    let mut chars = tok.chars();
    let first = chars.next()?;
    if (first == '❗' || first == '❕') && chars.clone().all(|c| c == first) {
        let n = tok.chars().count();
        return Some(match (first, n) {
            ('❗', 1) => Priority::High,
            ('❗', _) => Priority::Highest,
            ('❕', 1) => Priority::Low,
            _ => Priority::Lowest,
        });
    }
    match tok {
        "🔺" => Some(Priority::Highest),
        "⏫" | "🔼" => Some(Priority::High),
        "🔽" => Some(Priority::Low),
        "⏬" => Some(Priority::Lowest),
        _ => None,
    }
}

/// A `priority::` value.
pub fn priority_word(s: &str) -> Option<Priority> {
    match s.trim().to_ascii_lowercase().as_str() {
        "highest" | "urgent" => Some(Priority::Highest),
        "high" | "medium" => Some(Priority::High),
        "normal" | "none" => Some(Priority::Normal),
        "low" => Some(Priority::Low),
        "lowest" => Some(Priority::Lowest),
        _ => None,
    }
}

/// The word of a priority, as `priority::` writes it.
pub fn priority_name(p: Priority) -> &'static str {
    match p {
        Priority::Highest => "highest",
        Priority::High => "high",
        Priority::Normal => "normal",
        Priority::Low => "low",
        Priority::Lowest => "lowest",
    }
}

/// `30m`, `2h`, `1.5h`, `2h30m`, `1d`, `1d2h`: minutes. None when the token is not a duration.
pub fn parse_estimate(s: &str) -> Option<u32> {
    // Full-width digits and letters from an IME read as the ASCII ones (parse_date's rule).
    let s = crate::search::normalize(s);
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut total = 0f64;
    let mut num = String::new();
    let mut any = false;
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let per_unit = match c {
            'm' => 1.0,
            'h' => 60.0,
            'd' => DAY_MINUTES as f64,
            _ => return None,
        };
        let n: f64 = num.parse().ok()?;
        num.clear();
        total += n * per_unit;
        any = true;
    }
    if !num.is_empty() || !any || total <= 0.0 {
        return None;
    }
    Some(total.round() as u32)
}

/// Minutes back to the shortest spelling: `45m`, `2h`, `2h30m`, `1d`, `1d2h`.
pub fn format_estimate(minutes: u32) -> String {
    let days = minutes / DAY_MINUTES;
    let hours = (minutes % DAY_MINUTES) / 60;
    let mins = minutes % 60;
    let mut out = String::new();
    if days > 0 {
        out.push_str(&format!("{days}d"));
    }
    if hours > 0 {
        out.push_str(&format!("{hours}h"));
    }
    if mins > 0 || out.is_empty() {
        out.push_str(&format!("{mins}m"));
    }
    out
}

fn split_ids(s: &str) -> Vec<String> {
    s.split([',', ' ', '\t']).map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

/// A token that ends a repeat rule's text and that a marker takes no value from: the
/// next attribute, a field, a tag or an anchor.
fn is_stop(tok: &str) -> bool {
    let bare = strip_vs16(tok);
    starts_marker(&bare) || bare.starts_with(['[', '(', '^', '#']) || bare.contains("::")
}

/// Whether an attribute's emoji opens `s` — the union of the table's own spellings, so
/// a new one is a row in it and nothing else. An ASCII byte there answers no without a
/// comparison: every marker is well above it.
fn starts_marker(s: &str) -> bool {
    !s.is_empty() && !s.as_bytes()[0].is_ascii() && AttrKind::ALL.iter().flat_map(|k| k.emoji()).any(|m| s.starts_with(m))
}

/// A `key::` or emoji value read for a kind: (the value kept, whether it reads).
fn read_value(kind: AttrKind, raw: &str) -> (String, bool) {
    let raw = raw.trim();
    match kind {
        AttrKind::Created | AttrKind::Start | AttrKind::Scheduled | AttrKind::Due | AttrKind::Cancelled | AttrKind::Done => (raw.to_string(), parse_date(raw).is_some()),
        AttrKind::Priority => match priority_word(raw) {
            Some(p) if p != Priority::Normal => (priority_name(p).to_string(), true),
            _ => (raw.to_string(), false),
        },
        AttrKind::Repeat => (raw.to_string(), !raw.is_empty()),
        AttrKind::Delete => (String::new(), raw.eq_ignore_ascii_case("delete")),
        AttrKind::Id => (raw.to_string(), !raw.is_empty() && !raw.contains(char::is_whitespace)),
        AttrKind::Depends => {
            let ids = split_ids(raw);
            (ids.join(","), !ids.is_empty())
        }
        AttrKind::Estimate => (raw.to_string(), parse_estimate(raw).is_some()),
    }
}

struct Tok<'a> {
    text: &'a str,
    at: usize,
}

/// `❗❗`: a second mark right after the same one (a U+FE0F between them ignored)
/// continues the first rather than opening a new token.
fn same_mark_run(text: &str, start: usize, at: usize) -> bool {
    let c = text[at..].chars().next().unwrap_or(' ');
    (c == '❗' || c == '❕') && text[start..at].chars().rev().find(|&x| x != '\u{FE0F}') == Some(c)
}

/// Whitespace-separated tokens, each split again where an attribute emoji starts
/// inside it (`写周报📅` is a word and a marker), so a marker needs no space before it.
fn tokenize(text: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        let end = text[i..].find(char::is_whitespace).map(|e| i + e).unwrap_or(text.len());
        let mut start = i;
        let mut j = i;
        while j < end {
            if j > start && starts_marker(&text[j..end]) && !same_mark_run(text, start, j) {
                out.push(Tok { text: &text[start..j], at: start });
                start = j;
            }
            j += text[j..].chars().next().unwrap().len_utf8();
        }
        out.push(Tok { text: &text[start..end], at: start });
        i = end;
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Unit {
    /// A word of the description: ends the trailing run.
    Plain,
    /// A tag, a field, the anchor, or a marker with nothing after it yet (`📅` awaiting
    /// its date from the menu): the run continues through it.
    Through,
    /// An attribute that reads.
    Attr,
}

/// Every attribute occurrence of a task line's text (the part after the checkbox), in
/// line order, with the trailing run and the winners marked.
pub fn scan_task_attrs(text: &str) -> Vec<AttrMark> {
    let toks = tokenize(text);
    let fields = scan_fields(text, 0);
    // anchor_start is the offset of the space before the caret; the token starts at the caret
    let anchor_caret = crate::blockid::anchor_start(text).map(|a| a + (text[a..].len() - text[a..].trim_start_matches([' ', '\t']).len()));
    let mut marks: Vec<AttrMark> = Vec::new();
    let mut mark_unit: Vec<usize> = Vec::new();
    let mut units: Vec<Unit> = Vec::new();
    let tok_end = |i: usize| toks[i].at + toks[i].text.len();
    // A marker's value: the rest of its token, or the next token when that is not a stop.
    let value_of = |i: usize, marker: &str| -> Option<(String, usize, usize)> {
        let bare = strip_vs16(toks[i].text);
        let rem = bare.strip_prefix(marker)?;
        if !rem.is_empty() {
            return Some((rem.to_string(), i + 1, tok_end(i)));
        }
        match toks.get(i + 1) {
            Some(next) if !is_stop(next.text) => Some((strip_vs16(next.text).into_owned(), i + 2, tok_end(i + 1))),
            _ => Some((String::new(), i + 1, tok_end(i))),
        }
    };
    let mut push = |marks: &mut Vec<AttrMark>, units: &mut Vec<Unit>, kind: AttrKind, span: Range<usize>, raw: &str| {
        let (value, valid) = read_value(kind, raw);
        let bare = raw.is_empty();
        marks.push(AttrMark { kind, span, value, valid, tail: false, active: false });
        mark_unit.push(units.len());
        units.push(if valid { Unit::Attr } else if bare { Unit::Through } else { Unit::Plain });
    };
    let mut i = 0;
    while i < toks.len() {
        let tok = &toks[i];
        // a field is one unit through its close; the bracketed spelling of an attribute key is that attribute
        if let Some(f) = fields.iter().find(|f| f.span.start <= tok.at && tok.at < f.span.end) {
            let bracketed = text[f.span.start..].starts_with(['[', '(']);
            match AttrKind::from_key(&f.key).filter(|_| bracketed) {
                Some(kind) => push(&mut marks, &mut units, kind, f.span.clone(), &f.value),
                None => units.push(Unit::Through),
            }
            while i < toks.len() && toks[i].at < f.span.end {
                i += 1;
            }
            continue;
        }
        let bare = strip_vs16(tok.text);
        if let Some(p) = priority_token(&bare) {
            push(&mut marks, &mut units, AttrKind::Priority, tok.at..tok_end(i), priority_name(p));
            i += 1;
            continue;
        }
        if let Some((kind, marker)) = AttrKind::DATES.into_iter().find_map(|k| k.emoji().iter().find(|m| bare.starts_with(*m)).map(|m| (k, *m))) {
            let (v, next, end) = value_of(i, marker).unwrap();
            push(&mut marks, &mut units, kind, tok.at..end, &v);
            i = next;
            continue;
        }
        if let Some(rem) = bare.strip_prefix("🔁") {
            let mut parts: Vec<String> = Vec::new();
            if !rem.is_empty() {
                parts.push(rem.to_string());
            }
            let mut j = i + 1;
            let mut end = tok_end(i);
            while j < toks.len() && !is_stop(toks[j].text) {
                parts.push(toks[j].text.to_string());
                end = tok_end(j);
                j += 1;
            }
            push(&mut marks, &mut units, AttrKind::Repeat, tok.at..end, &parts.join(" "));
            i = j;
            continue;
        }
        if bare == "🚮" {
            push(&mut marks, &mut units, AttrKind::Delete, tok.at..tok_end(i), "delete");
            i += 1;
            continue;
        }
        if bare.starts_with("🏁") {
            let (v, next, end) = value_of(i, "🏁").unwrap();
            push(&mut marks, &mut units, AttrKind::Delete, tok.at..end, if v.eq_ignore_ascii_case("keep") { "keep" } else { "delete" });
            i = next;
            continue;
        }
        if let Some((v, next, end)) = value_of(i, "🆔") {
            push(&mut marks, &mut units, AttrKind::Id, tok.at..end, &v);
            i = next;
            continue;
        }
        if let Some((v, mut next, mut end)) = value_of(i, "⛔") {
            let mut ids = split_ids(&v);
            let mut last = v;
            // `⛔ a, b`: a trailing comma carries on to the next token
            while last.ends_with(',') && next < toks.len() && !is_stop(toks[next].text) {
                last = toks[next].text.to_string();
                ids.extend(split_ids(&last));
                end = tok_end(next);
                next += 1;
            }
            push(&mut marks, &mut units, AttrKind::Depends, tok.at..end, &ids.join(","));
            i = next;
            continue;
        }
        if let Some((v, next, end)) = value_of(i, "🕐") {
            push(&mut marks, &mut units, AttrKind::Estimate, tok.at..end, &v);
            i = next;
            continue;
        }
        let through = (bare.starts_with('#') && bare.len() > 1) || anchor_caret == Some(tok.at);
        units.push(if through { Unit::Through } else { Unit::Plain });
        i += 1;
    }
    let tail_start = units.iter().rposition(|u| *u == Unit::Plain).map(|p| p + 1).unwrap_or(0);
    let mut seen: Vec<AttrKind> = Vec::new();
    for (idx, m) in marks.iter_mut().enumerate().rev() {
        m.tail = m.valid && mark_unit[idx] >= tail_start;
        m.active = m.tail && !seen.contains(&m.kind);
        if m.active {
            seen.push(m.kind);
        }
    }
    marks
}

/// Read every attribute of a task line's text (the part after the checkbox).
pub fn parse_task_attrs(rest: &str) -> TaskAttrs {
    let mut a = TaskAttrs::default();
    for m in scan_task_attrs(rest).into_iter().filter(|m| m.active) {
        match m.kind {
            AttrKind::Due => a.due = parse_date(&m.value),
            AttrKind::Scheduled => a.scheduled = parse_date(&m.value),
            AttrKind::Start => a.start = parse_date(&m.value),
            AttrKind::Created => a.created = parse_date(&m.value),
            AttrKind::Done => a.done = parse_date(&m.value),
            AttrKind::Cancelled => a.cancelled = parse_date(&m.value),
            AttrKind::Priority => a.priority = priority_word(&m.value).unwrap_or_default(),
            AttrKind::Repeat => a.repeat = Some(m.value),
            AttrKind::Delete => a.delete_on_done = true,
            AttrKind::Id => a.task_id = Some(m.value),
            AttrKind::Depends => a.depends_on = m.value.split(',').map(str::to_string).collect(),
            AttrKind::Estimate => a.estimate = parse_estimate(&m.value),
        }
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Option<NaiveDate> {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
    }

    #[test]
    fn a_full_emoji_line() {
        let a = parse_task_attrs("write the scanner #vemry 🆔 scan01 ❗ 🔁 every week 🕐 45m ⏩ 2026-09-10 ⏳ 2026-09-12 📅 2026-09-15 ^t1");
        assert_eq!(a.task_id.as_deref(), Some("scan01"));
        assert_eq!(a.priority, Priority::High);
        assert_eq!(a.repeat.as_deref(), Some("every week"));
        assert_eq!(a.estimate, Some(45));
        assert_eq!(a.start, d("2026-09-10"));
        assert_eq!(a.scheduled, d("2026-09-12"));
        assert_eq!(a.due, d("2026-09-15"));
        assert!(!a.delete_on_done && a.depends_on.is_empty() && a.created.is_none());
    }

    #[test]
    fn the_bracketed_key_spellings_read_the_same_and_the_bare_one_is_a_field() {
        let a = parse_task_attrs("x [id:: scan01] [priority:: high] [repeat:: every week] [estimate:: 45m] [start:: 2026-09-10] [scheduled:: 2026-09-12] [due:: 2026-09-15] [onCompletion:: delete] [dependsOn:: a, b]");
        assert_eq!(a.task_id.as_deref(), Some("scan01"));
        assert_eq!(a.priority, Priority::High);
        assert_eq!(a.repeat.as_deref(), Some("every week"));
        assert_eq!(a.estimate, Some(45));
        assert_eq!(a.start, d("2026-09-10"));
        assert_eq!(a.scheduled, d("2026-09-12"));
        assert_eq!(a.due, d("2026-09-15"));
        assert!(a.delete_on_done);
        assert_eq!(a.depends_on, vec!["a", "b"]);
        assert_eq!(parse_task_attrs("x (due:: 2026-09-15)").due, d("2026-09-15"));
        // a bare `key:: value` is an ordinary field, not an attribute
        let b = parse_task_attrs("x due:: 2026-09-15 priority:: lowest");
        assert!(b.due.is_none() && b.priority == Priority::Normal);
    }

    #[test]
    fn priority_steps_and_the_other_spellings_read() {
        assert_eq!(parse_task_attrs("a ❗❗").priority, Priority::Highest);
        assert_eq!(parse_task_attrs("a ❕").priority, Priority::Low);
        assert_eq!(parse_task_attrs("a ❕❕").priority, Priority::Lowest);
        assert_eq!(parse_task_attrs("a ❗\u{FE0F}").priority, Priority::High);
        assert_eq!(parse_task_attrs("a ❗\u{FE0F}❗\u{FE0F}").priority, Priority::Highest);
        assert_eq!(parse_task_attrs("a 🔺").priority, Priority::Highest);
        assert_eq!(parse_task_attrs("a 🔼").priority, Priority::High);
        assert_eq!(parse_task_attrs("a ⏬").priority, Priority::Lowest);
        assert_eq!(parse_task_attrs("a [priority:: medium]").priority, Priority::High);
        assert_eq!(parse_task_attrs("a [priority:: 3]").priority, Priority::Normal);
        let other = parse_task_attrs("a 🛫 2026-09-10 ⌛ 2026-09-12 🏁 delete");
        assert_eq!(other.start, d("2026-09-10"));
        assert_eq!(other.scheduled, d("2026-09-12"));
        assert!(other.delete_on_done);
        assert!(!parse_task_attrs("a 🏁 keep").delete_on_done);
        assert!(parse_task_attrs("a 🚮").delete_on_done);
    }

    #[test]
    fn a_repeat_rule_runs_to_the_next_attribute() {
        assert_eq!(parse_task_attrs("a 🔁 every 2 weeks when done 📅 2026-09-15").repeat.as_deref(), Some("every 2 weeks when done"));
        assert_eq!(parse_task_attrs("a 🔁every month #tag").repeat.as_deref(), Some("every month"));
        assert_eq!(parse_task_attrs("a 🔁 every day [owner:: me]").repeat.as_deref(), Some("every day"));
        assert_eq!(parse_task_attrs("a 🔁 every day ^k1").repeat.as_deref(), Some("every day"));
        assert!(parse_task_attrs("a 🔁").repeat.is_none());
    }

    #[test]
    fn ids_and_dependencies() {
        assert_eq!(parse_task_attrs("a ⛔ k3x9pa, m2v8qr 📅 2026-09-30").depends_on, vec!["k3x9pa", "m2v8qr"]);
        assert_eq!(parse_task_attrs("a ⛔ k3x9pa,m2v8qr").depends_on, vec!["k3x9pa", "m2v8qr"]);
        assert_eq!(parse_task_attrs("a ⛔k3x9pa").depends_on, vec!["k3x9pa"]);
        assert_eq!(parse_task_attrs("a 🆔abc").task_id.as_deref(), Some("abc"));
        assert!(parse_task_attrs("a 🆔 📅 2026-09-30").task_id.is_none());
    }

    #[test]
    fn fullwidth_dates_parse() {
        use chrono::Datelike;
        let d = parse_date("２０２６－０９－１０").expect("full-width ISO date");
        assert_eq!((d.year(), d.month(), d.day()), (2026, 9, 10));
        assert_eq!(parse_date("2026-09-10"), Some(d));
        assert_eq!(parse_date("2026-9-10"), None);
    }

    #[test]
    fn estimates() {
        assert_eq!(parse_estimate("30m"), Some(30));
        assert_eq!(parse_estimate("2h"), Some(120));
        assert_eq!(parse_estimate("1.5h"), Some(90));
        assert_eq!(parse_estimate("2h30m"), Some(150));
        assert_eq!(parse_estimate("1d"), Some(480));
        assert_eq!(parse_estimate("1d2h"), Some(600));
        assert_eq!(parse_estimate("soon"), None);
        // full-width, as an IME emits it
        assert_eq!(parse_estimate("３０ｍ"), Some(30));
        assert_eq!(parse_estimate("２ｈ"), Some(120));
        assert_eq!(parse_estimate("2"), None);
        assert_eq!(format_estimate(150), "2h30m");
        assert_eq!(format_estimate(480), "1d");
        assert_eq!(format_estimate(45), "45m");
        assert_eq!(parse_task_attrs("a 🕐45m").estimate, Some(45));
        assert_eq!(parse_task_attrs("a 🕐 later").estimate, None);
    }

    #[test]
    fn only_the_trailing_run_counts_and_the_last_of_a_kind_wins() {
        // a marker in the description is a word
        assert!(parse_task_attrs("call 📅 2026-09-15 about the invoice").due.is_none());
        assert_eq!(parse_task_attrs("a ❗❗ b").priority, Priority::Normal);
        // tags, fields and the anchor carry the run through
        let a = parse_task_attrs("a #t 📅 2026-09-15 [owner:: me] ✅ 2026-09-16 ^k1");
        assert_eq!(a.due, d("2026-09-15"));
        assert_eq!(a.done, d("2026-09-16"));
        // the last of a kind wins, whichever spelling
        let b = parse_task_attrs("a ⏳ waiting for a reply 📅 2026-09-15 [due:: 2026-09-16]");
        assert!(b.scheduled.is_none());
        assert_eq!(b.due, d("2026-09-16"));
        assert_eq!(parse_task_attrs("a 📅 2026-09-15 📅 2026-09-20").due, d("2026-09-20"));
        // a marker glued to the word before it still opens an attribute
        assert_eq!(parse_task_attrs("写周报📅 2026-09-15").due, d("2026-09-15"));
        assert_eq!(parse_task_attrs("买菜❗").priority, Priority::High);
        // a bad token is a word (and it ends the run)
        assert!(parse_task_attrs("a 📅 2026-02-30").due.is_none());
        assert!(parse_task_attrs("a 📅 2026-9-5").due.is_none());
        assert!(parse_task_attrs("a ✅ 2026-09-14 📅 next Friday").done.is_none());
        assert_eq!(parse_task_attrs("a ✅ 2026-09-14 ❌ 2026-09-13").done, d("2026-09-14"));
        assert_eq!(parse_task_attrs("a ✅ 2026-09-14 ❌ 2026-09-13").cancelled, d("2026-09-13"));
        assert_eq!(parse_task_attrs("a ➕ 2026-09-01").created, d("2026-09-01"));
    }

    #[test]
    fn the_marks_say_what_was_read_and_what_was_not() {
        let text = "call 📅 2026-09-15 later ⏳ soon 📅 2026-09-16 📅 2026-09-17";
        let marks = scan_task_attrs(text);
        let flags: Vec<(bool, bool, bool)> = marks.iter().map(|m| (m.valid, m.tail, m.active)).collect();
        // the first is in the description, the ⏳ a bad token (a word), the third a duplicate, the last the winner
        assert_eq!(flags, vec![(true, false, false), (false, false, false), (true, true, false), (true, true, true)]);
        assert_eq!(&text[marks[1].span.clone()], "⏳ soon");
        assert_eq!(&text[marks[3].span.clone()], "📅 2026-09-17");
        // a bad token at the end ends the run: what stands before it is description
        assert!(scan_task_attrs("a 📅 2026-09-15 ⏳ soon").iter().all(|m| !m.tail));
        // a bare marker awaiting its value does not: the attributes before it still read
        assert_eq!(parse_task_attrs("a ⏳ 2026-09-12 📅").scheduled, d("2026-09-12"));
        assert_eq!(parse_task_attrs("a 📅 2026-09-15 ⛔ ^k1").due, d("2026-09-15"));
        let m = scan_task_attrs("x [dependsOn:: a, b] ^k1");
        assert_eq!((m[0].kind, m[0].value.as_str(), m[0].active), (AttrKind::Depends, "a,b", true));
        assert_eq!(&"x [dependsOn:: a, b] ^k1"[m[0].span.clone()], "[dependsOn:: a, b]");
    }
}
