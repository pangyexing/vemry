//! The attributes a task line carries after its text, and how they are read.
//!
//! Every attribute has an emoji spelling and a `key::` spelling; both parse to the same
//! value, and the emoji is read first (a line carrying both takes the emoji's). A few
//! spellings other tools write are read as well (the "also read" column) so that a
//! vault written elsewhere opens as is; vemry itself writes the first spelling only.
//! The frontend mirrors this table in `views/tasks.ts` (TASK_ATTRS) for the editor's
//! chips and the views' rewrites — change the two together.
//!
//! | attribute        | emoji | also read      | key::          | value                                   |
//! |------------------|-------|----------------|----------------|-----------------------------------------|
//! | due              | 📅    |                | `due::`        | `YYYY-MM-DD`                            |
//! | scheduled        | ⏳    | ⌛             | `scheduled::`  | `YYYY-MM-DD`                            |
//! | start            | ⏩    | 🛫             | `start::`      | `YYYY-MM-DD`                            |
//! | created          | ➕    |                | `created::`    | `YYYY-MM-DD`                            |
//! | done             | ✅    |                | `completion::` | `YYYY-MM-DD`                            |
//! | cancelled        | ❌    |                | `cancelled::`  | `YYYY-MM-DD`                            |
//! | priority         | ❗❗ ❗ ❕ ❕❕ | 🔺 ⏫ 🔼 🔽 ⏬ | `priority::` | `highest` `high` `low` `lowest` (`medium` reads as high) |
//! | repeat           | 🔁    |                | `repeat::`     | the rule's text, see `repeat.rs`         |
//! | delete when done | 🚮    | 🏁             | `onCompletion::` | `delete` / `keep`                     |
//! | id               | 🆔    |                | `id::`         | one token                               |
//! | depends on       | ⛔    |                | `dependsOn::`  | ids, comma-separated                    |
//! | estimate         | 🕐    |                | `estimate::`   | `30m` `2h` `1d` (`d` = 8h)              |
//!
//! Reading rules, the same for every attribute: U+FE0F variation selectors are stripped
//! first; a date is `YYYY-MM-DD` (an unpadded month or day accepted) and the first
//! spelling that carries a valid date wins; a marker whose token is not a date is plain
//! text (a decorative `⏳ waiting` is nothing to the parser); the space between an emoji
//! and its value is optional; a `[key:: value]` closes at its bracket; a repeat rule's
//! text runs until the next attribute, field, tag or anchor.

use crate::block::{BlockKind, Priority, TaskState};
use crate::parser::scan_fields;
use chrono::NaiveDate;

/// The spellings of each date attribute, in read order (the written form first).
pub const DUE_MARKERS: &[&str] = &["📅", "due::"];
pub const SCHEDULED_MARKERS: &[&str] = &["⏳", "⌛", "scheduled::"];
pub const START_MARKERS: &[&str] = &["⏩", "🛫", "start::"];
pub const CREATED_MARKERS: &[&str] = &["➕", "created::"];
pub const DONE_MARKERS: &[&str] = &["✅", "completion::"];
pub const CANCELLED_MARKERS: &[&str] = &["❌", "cancelled::"];

/// Every emoji that opens an attribute: where a repeat rule's text stops, and what
/// `taskText` strips for display.
pub const EMOJI_MARKERS: &[&str] = &[
    "📅", "⏳", "⌛", "⏩", "🛫", "➕", "✅", "❌", "❗", "❕", "🔺", "⏫", "🔼", "🔽", "⏬", "🔁", "🚮", "🏁", "🆔", "⛔", "🕐",
];

/// The `key::` names that are task attributes: a field chip is not drawn for them on a
/// task line, and `set_block_field` routes the dates to `reschedule_line`.
pub const ATTR_KEYS: &[&str] =
    &["due", "scheduled", "start", "created", "completion", "cancelled", "priority", "repeat", "oncompletion", "id", "dependson", "estimate"];

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

    /// The attributes of a task block; None for any other kind.
    pub fn of_kind(kind: &BlockKind) -> Option<(TaskState, TaskAttrs)> {
        match kind {
            BlockKind::Task { state, due, scheduled, start, created, done, cancelled, priority, repeat, delete_on_done, task_id, depends_on, estimate } => Some((
                *state,
                TaskAttrs {
                    due: *due,
                    scheduled: *scheduled,
                    start: *start,
                    created: *created,
                    done: *done,
                    cancelled: *cancelled,
                    priority: *priority,
                    repeat: repeat.clone(),
                    delete_on_done: *delete_on_done,
                    task_id: task_id.clone(),
                    depends_on: depends_on.clone(),
                    estimate: *estimate,
                },
            )),
            _ => None,
        }
    }
}

/// Without U+FE0F: an emoji typed with the variation selector reads as the bare one.
pub fn strip_vs16(s: &str) -> String {
    s.chars().filter(|&c| c != '\u{FE0F}').collect()
}

/// The date after the first of `markers` that carries one: the token after the marker
/// (up to whitespace or a closing bracket) as `%Y-%m-%d`. Emoji before `key::` is the
/// order of the tables above.
pub fn date_attr(text: &str, markers: &[&str]) -> Option<NaiveDate> {
    for marker in markers {
        if let Some(p) = text.find(marker) {
            let after = text[p + marker.len()..].trim_start();
            let token: String = after.chars().take_while(|c| !c.is_whitespace() && *c != ']' && *c != ')').collect();
            if let Ok(d) = NaiveDate::parse_from_str(&token, "%Y-%m-%d") {
                return Some(d);
            }
        }
    }
    None
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

/// `30m`, `2h`, `1.5h`, `2h30m`, `1d`, `1d2h`: minutes. None when the token is not a duration.
pub fn parse_estimate(s: &str) -> Option<u32> {
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

/// A token that ends a repeat rule's text: the next attribute, a field, a tag or an anchor.
fn is_stop(tok: &str) -> bool {
    EMOJI_MARKERS.iter().any(|m| tok.starts_with(m)) || tok.starts_with(['[', '(', '^', '#']) || tok.contains("::")
}

/// Read every attribute of a task line's text (the part after the checkbox).
pub fn parse_task_attrs(rest: &str) -> TaskAttrs {
    let text = strip_vs16(rest);
    let mut a = TaskAttrs {
        due: date_attr(&text, DUE_MARKERS),
        scheduled: date_attr(&text, SCHEDULED_MARKERS),
        start: date_attr(&text, START_MARKERS),
        created: date_attr(&text, CREATED_MARKERS),
        done: date_attr(&text, DONE_MARKERS),
        cancelled: date_attr(&text, CANCELLED_MARKERS),
        ..Default::default()
    };

    // The emoji spellings, token by token. A value glued to its emoji (`🕐45m`) is the
    // same as a spaced one; a marker's value is the rest of its token or the next token.
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let value_of = |i: usize, marker: &str| -> Option<(String, usize)> {
        let rem = tokens[i].strip_prefix(marker)?;
        if !rem.is_empty() {
            return Some((rem.to_string(), i + 1));
        }
        match tokens.get(i + 1) {
            Some(next) if !is_stop(next) => Some((next.to_string(), i + 2)),
            _ => Some((String::new(), i + 1)),
        }
    };
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        if a.priority == Priority::Normal {
            if let Some(p) = priority_token(tok) {
                a.priority = p;
                i += 1;
                continue;
            }
        }
        if a.repeat.is_none() && tok.starts_with("🔁") {
            let mut parts: Vec<&str> = Vec::new();
            let rem = &tok["🔁".len()..];
            if !rem.is_empty() {
                parts.push(rem);
            }
            let mut j = i + 1;
            while j < tokens.len() && !is_stop(tokens[j]) {
                parts.push(tokens[j]);
                j += 1;
            }
            let rule = parts.join(" ");
            if !rule.is_empty() {
                a.repeat = Some(rule);
            }
            i = j;
            continue;
        }
        if tok == "🚮" {
            a.delete_on_done = true;
            i += 1;
            continue;
        }
        if tok.starts_with("🏁") {
            let (v, next) = value_of(i, "🏁").unwrap_or_default();
            a.delete_on_done = !v.eq_ignore_ascii_case("keep");
            i = next.max(i + 1);
            continue;
        }
        if a.task_id.is_none() {
            if let Some((v, next)) = value_of(i, "🆔") {
                if !v.is_empty() {
                    a.task_id = Some(v);
                }
                i = next;
                continue;
            }
        }
        if a.depends_on.is_empty() {
            if let Some((v, mut next)) = value_of(i, "⛔") {
                let mut ids = split_ids(&v);
                let mut last = v;
                // `⛔ a, b`: a trailing comma carries on to the next token
                while last.ends_with(',') && next < tokens.len() && !is_stop(tokens[next]) {
                    last = tokens[next].to_string();
                    ids.extend(split_ids(&last));
                    next += 1;
                }
                a.depends_on = ids;
                i = next;
                continue;
            }
        }
        if a.estimate.is_none() {
            if let Some((v, next)) = value_of(i, "🕐") {
                a.estimate = parse_estimate(&v);
                i = next;
                continue;
            }
        }
        i += 1;
    }

    // The `key::` spellings fill what no emoji set (the dates were read both ways above)
    for f in scan_fields(&text, 0) {
        let value = f.value.trim();
        match f.key.to_ascii_lowercase().as_str() {
            "priority" if a.priority == Priority::Normal => {
                if let Some(p) = priority_word(value) {
                    a.priority = p;
                }
            }
            "repeat" if a.repeat.is_none() && !value.is_empty() => a.repeat = Some(value.to_string()),
            "oncompletion" if !a.delete_on_done => a.delete_on_done = value.eq_ignore_ascii_case("delete"),
            "id" if a.task_id.is_none() && !value.is_empty() => a.task_id = Some(value.to_string()),
            "dependson" if a.depends_on.is_empty() => a.depends_on = split_ids(value),
            "estimate" if a.estimate.is_none() => a.estimate = parse_estimate(value),
            _ => {}
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
    fn the_key_spellings_read_the_same() {
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
        // the bare spelling reads one token, and the unbracketed date reads too
        let b = parse_task_attrs("x due::2026-8-5 priority:: lowest");
        assert_eq!(b.due, d("2026-08-05"));
        assert_eq!(b.priority, Priority::Lowest);
    }

    #[test]
    fn priority_steps_and_the_forms_read_for_compatibility() {
        assert_eq!(parse_task_attrs("a ❗❗").priority, Priority::Highest);
        assert_eq!(parse_task_attrs("a ❕").priority, Priority::Low);
        assert_eq!(parse_task_attrs("a ❕❕").priority, Priority::Lowest);
        assert_eq!(parse_task_attrs("a ❗\u{FE0F}").priority, Priority::High);
        assert_eq!(parse_task_attrs("a 🔺").priority, Priority::Highest);
        assert_eq!(parse_task_attrs("a 🔼").priority, Priority::High);
        assert_eq!(parse_task_attrs("a ⏬").priority, Priority::Lowest);
        assert_eq!(parse_task_attrs("a [priority:: medium]").priority, Priority::High);
        assert_eq!(parse_task_attrs("a [priority:: 3]").priority, Priority::Normal);
        let legacy = parse_task_attrs("a 🛫 2026-09-10 ⌛ 2026-09-12 🏁 delete");
        assert_eq!(legacy.start, d("2026-09-10"));
        assert_eq!(legacy.scheduled, d("2026-09-12"));
        assert!(legacy.delete_on_done);
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
    fn estimates() {
        assert_eq!(parse_estimate("30m"), Some(30));
        assert_eq!(parse_estimate("2h"), Some(120));
        assert_eq!(parse_estimate("1.5h"), Some(90));
        assert_eq!(parse_estimate("2h30m"), Some(150));
        assert_eq!(parse_estimate("1d"), Some(480));
        assert_eq!(parse_estimate("1d2h"), Some(600));
        assert_eq!(parse_estimate("soon"), None);
        assert_eq!(parse_estimate("2"), None);
        assert_eq!(format_estimate(150), "2h30m");
        assert_eq!(format_estimate(480), "1d");
        assert_eq!(format_estimate(45), "45m");
        assert_eq!(parse_task_attrs("a 🕐45m").estimate, Some(45));
        assert_eq!(parse_task_attrs("a 🕐 later").estimate, None);
    }

    #[test]
    fn a_marker_without_a_date_is_text_and_the_first_valid_one_wins() {
        let a = parse_task_attrs("a ⏳ waiting for a reply 📅 2026-09-15 due:: 2026-09-16");
        assert!(a.scheduled.is_none());
        assert_eq!(a.due, d("2026-09-15"));
        assert!(parse_task_attrs("a 📅 2026-02-30").due.is_none());
        assert_eq!(parse_task_attrs("a ✅ 2026-09-14 ❌ 2026-09-13").done, d("2026-09-14"));
        assert_eq!(parse_task_attrs("a ✅ 2026-09-14 ❌ 2026-09-13").cancelled, d("2026-09-13"));
        assert_eq!(parse_task_attrs("a ➕ 2026-09-01").created, d("2026-09-01"));
    }
}
