//! Line-level rewrites of a task or a block's first line: the checkbox state (with the
//! ✅ / ❌ stamp a closing state leaves), a date marker (`📅 2026-09-10` /
//! `due:: 2026-09-10`, and every other date of `taskattrs.rs`) and an inline
//! `[key:: value]` field. The frontend's `views/tasks.ts` (rescheduleLine, setStateLine,
//! completeTaskSlice, updateTask) and `fields.ts` (setFieldValue) implement the same
//! rules for the editor and the task views; this is the agent's copy, so a task the AI
//! ticks or reschedules lands on disk byte for byte as one the user dragged on the
//! board. Change the rules in both places together.

use crate::parser::{is_field_key, parse_list_marker, parse_task_line, scan_fields};
use crate::repeat::next_occurrence;
use crate::taskattrs::{self, parse_task_attrs};
use crate::TaskState;
use chrono::NaiveDate;

/// The date attributes a task line carries, each with its spellings (emoji first: the
/// parse priority of `taskattrs::date_attr`; the first spelling is the written one).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateField {
    Due,
    Scheduled,
    Start,
    Created,
    Done,
    Cancelled,
}

impl DateField {
    pub fn markers(self) -> &'static [&'static str] {
        match self {
            DateField::Due => taskattrs::DUE_MARKERS,
            DateField::Scheduled => taskattrs::SCHEDULED_MARKERS,
            DateField::Start => taskattrs::START_MARKERS,
            DateField::Created => taskattrs::CREATED_MARKERS,
            DateField::Done => taskattrs::DONE_MARKERS,
            DateField::Cancelled => taskattrs::CANCELLED_MARKERS,
        }
    }
    /// The `key::` name of the field (`completion` for the done date: the spelling other
    /// tools read).
    pub fn parse(key: &str) -> Option<Self> {
        match key.to_ascii_lowercase().as_str() {
            "due" => Some(DateField::Due),
            "scheduled" => Some(DateField::Scheduled),
            "start" => Some(DateField::Start),
            "created" => Some(DateField::Created),
            "completion" => Some(DateField::Done),
            "cancelled" => Some(DateField::Cancelled),
            _ => None,
        }
    }
}

/// `YYYY-M-D` with an unpadded month or day accepted (chrono's `%m`/`%d` do), but a
/// real calendar date.
pub fn is_date_token(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 || parts[0].len() != 4 || parts[1].is_empty() || parts[1].len() > 2 || parts[2].is_empty() || parts[2].len() > 2 {
        return false;
    }
    if !parts.iter().all(|p| p.chars().all(|c| c.is_ascii_digit())) {
        return false;
    }
    let (y, m, d) = (parts[0].parse::<i32>().ok(), parts[1].parse::<u32>().ok(), parts[2].parse::<u32>().ok());
    matches!((y, m, d), (Some(y), Some(m), Some(d)) if chrono::NaiveDate::from_ymd_opt(y, m, d).is_some())
}

/// Append `piece` (with its own leading space) to a line before any trailing anchor.
fn append_before_anchor(line: &str, piece: &str) -> String {
    match crate::blockid::anchor_start(line) {
        Some(at) => format!("{}{piece}{}", line[..at].trim_end_matches([' ', '\t']), &line[at..]),
        None => format!("{}{piece}", line.trim_end_matches([' ', '\t'])),
    }
}

/// Remove `[from, to)` from a line without leaving a double space behind.
fn cut(line: &str, from: usize, to: usize) -> String {
    let mut before = &line[..from];
    let after = &line[to..];
    if before.ends_with([' ', '\t']) && (after.is_empty() || after.starts_with([' ', '\t'])) {
        before = before.trim_end_matches([' ', '\t']);
    }
    format!("{before}{after}")
}

/// The next token after `at` (skipping horizontal space), ending at whitespace or at a
/// closing bracket (a `[key:: value]`'s): (start, end) offsets, possibly empty.
fn next_token(line: &str, at: usize) -> (usize, usize) {
    let mut start = at;
    while line[start..].starts_with([' ', '\t']) {
        start += 1;
    }
    let end = line[start..].find(|c: char| c.is_whitespace() || c == ']' || c == ')').map(|i| start + i).unwrap_or(line.len());
    (start, end)
}

/// The bracket pair around a `[key:: value]` occurrence, when the marker sits just
/// inside one: the widened (from, to) to cut.
fn widen_to_brackets(line: &str, from: usize, to: usize) -> (usize, usize) {
    for (open, close) in [('[', ']'), ('(', ')')] {
        if line[..from].ends_with(open) && line[to..].starts_with(close) {
            return (from - 1, to + 1);
        }
    }
    (from, to)
}

/// Set, replace or (`None`) remove a task line's date marker, keeping the marker
/// spelling the user wrote (emoji or `attr::`); a fresh marker is inserted
/// emoji-style before any trailing `^anchor`. The mirror of `rescheduleLine`.
pub fn reschedule_line(line: &str, field: DateField, date: Option<&str>) -> String {
    let Some(date) = date else {
        // Both spellings may be present (the parser falls back from emoji to attr::):
        // every one carrying a date is removed, or the date would silently move
        let mut out = line.to_string();
        for marker in field.markers() {
            let mut pos = 0;
            while let Some(at) = out[pos..].find(marker) {
                let start = pos + at;
                let (ts, te) = next_token(&out, start + marker.len());
                if is_date_token(&out[ts..te]) {
                    let (from, to) = if marker.ends_with("::") { widen_to_brackets(&out, start, te) } else { (start, te) };
                    out = cut(&out, from, to);
                    // the cut may have trimmed the space before it, leaving the line shorter than `from`
                    pos = from.min(out.len());
                } else {
                    pos = start + marker.len();
                }
            }
        }
        return out;
    };
    // The rewrite lands on the token the parser reads (each marker's first occurrence,
    // emoji first): a date-shaped token is replaced in place; a bad token gets the
    // date inserted after the marker and is demoted to ordinary text
    for marker in field.markers() {
        let Some(start) = line.find(marker) else { continue };
        let after_marker = start + marker.len();
        let (ts, te) = next_token(line, after_marker);
        if is_date_token(&line[ts..te]) {
            return format!("{}{date}{}", &line[..ts], &line[te..]);
        }
        let sep = if line[after_marker..ts].is_empty() { " " } else { "" };
        let trail = if ts < te { " " } else { "" };
        return format!("{}{sep}{date}{trail}{}", &line[..ts], &line[ts..]);
    }
    append_before_anchor(line, &format!(" {} {date}", field.markers()[0]))
}

/// Set the checkbox of a task's first line (`[ ]` / `[/]` / `[x]` / `[-]`); a line with
/// no checkbox is returned unchanged.
pub fn set_checkbox(line: &str, state: TaskState) -> String {
    let ch = match state {
        TaskState::Done => 'x',
        TaskState::Doing => '/',
        TaskState::Todo => ' ',
        TaskState::Cancelled => '-',
    };
    let Some((_, _, at)) = parse_list_marker(line) else { return line.to_string() };
    let rest = &line[at..];
    let mut chars = rest.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some('['), Some(' ' | '/' | 'x' | 'X' | '-'), Some(']')) => format!("{}[{ch}]{}", &line[..at], &rest[3..]),
        _ => line.to_string(),
    }
}

/// Set a task line's state with the stamp the state leaves: done writes ✅ today (and
/// drops any ❌), cancelled writes ❌ today (and drops any ✅), reopening drops both.
/// The mirror of `setStateLine`.
pub fn set_state(line: &str, state: TaskState, today: NaiveDate) -> String {
    let today = today.format("%Y-%m-%d").to_string();
    let mut out = set_checkbox(line, state);
    match state {
        TaskState::Done => {
            out = reschedule_line(&out, DateField::Cancelled, None);
            out = reschedule_line(&out, DateField::Done, Some(&today));
        }
        TaskState::Cancelled => {
            out = reschedule_line(&out, DateField::Done, None);
            out = reschedule_line(&out, DateField::Cancelled, Some(&today));
        }
        TaskState::Todo | TaskState::Doing => {
            out = reschedule_line(&out, DateField::Done, None);
            out = reschedule_line(&out, DateField::Cancelled, None);
        }
    }
    out
}

/// The whole rewrite of a task block's slice for a state change, as the views make it:
/// the first line gets `set_state`; ticking a repeating task puts its next occurrence
/// above it; ticking a 🚮 task removes it — with a repeat the next occurrence stands in
/// its place, without one the result is empty and the caller deletes the block (its
/// line break included). The mirror of `completeTaskSlice`.
pub fn complete_task_slice(slice: &str, state: TaskState, today: NaiveDate) -> String {
    let (head, tail) = match slice.find('\n') {
        Some(nl) => {
            let cr = usize::from(slice[..nl].ends_with('\r'));
            (&slice[..nl - cr], &slice[nl - cr..])
        }
        None => (slice, ""),
    };
    let new_head = set_state(head, state, today);
    if state == TaskState::Done {
        if let Some((_, rest)) = parse_task_line(head) {
            let attrs = parse_task_attrs(rest);
            if let Some(next) = next_occurrence(head, &attrs, today) {
                return if attrs.delete_on_done { format!("{next}{tail}") } else { format!("{next}\n{new_head}{tail}") };
            }
            if attrs.delete_on_done {
                return String::new();
            }
        }
    }
    format!("{new_head}{tail}")
}

/// Set, replace or (`None`) remove one inline field on a line: the field with that
/// key is rewritten in place (its form kept), an empty placeholder `[key:: ]` is
/// filled, and a new field is appended as `[key:: value]` before any trailing
/// `^anchor`. The mirror of `setFieldValue`; the dates of a task line are
/// `reschedule_line`'s business. None = the key is not a field key.
pub fn set_field_value(line: &str, key: &str, value: Option<&str>) -> Option<String> {
    if !is_field_key(key) {
        return None;
    }
    let lower = key.to_lowercase();
    if let Some(f) = scan_fields(line, 0).into_iter().find(|f| f.key.to_lowercase() == lower) {
        let Some(value) = value else { return Some(cut(line, f.span.start, f.span.end)) };
        let occ = &line[f.span.clone()];
        let colons = occ.find("::")? + 2;
        let vpos = occ[colons..].find(&f.value).map(|i| colons + i)?;
        let vfrom = f.span.start + vpos;
        let vto = vfrom + f.value.len();
        return Some(format!("{}{value}{}", &line[..vfrom], &line[vto..]));
    }
    // An empty placeholder the index does not hold: `[key:: ]`, `[key::]`, `(key:: )`
    for (open, close) in [('[', ']'), ('(', ')')] {
        let needle = format!("{open}{key}::");
        if let Some(at) = line.find(&needle) {
            let after = at + needle.len();
            let inner_end = line[after..].find(close).map(|i| after + i);
            if let Some(end) = inner_end {
                if line[after..end].trim().is_empty() {
                    return Some(match value {
                        Some(v) => format!("{}{open}{key}:: {v}{close}{}", &line[..at], &line[end + 1..]),
                        None => cut(line, at, end + 1),
                    });
                }
            }
        }
    }
    Some(match value {
        None => line.to_string(),
        Some(v) => append_before_anchor(line, &format!(" [{key}:: {v}]")),
    })
}

/// Apply a rewrite to the first line of a block slice, its other lines kept verbatim
/// (markers and fields live on a block's first line by convention).
pub fn rewrite_first_line(slice: &str, f: impl FnOnce(&str) -> String) -> String {
    match slice.find('\n') {
        Some(nl) => {
            let head = slice[..nl].trim_end_matches('\r');
            let eol = if slice[..nl].ends_with('\r') { "\r" } else { "" };
            format!("{}{eol}{}", f(head), &slice[nl..])
        }
        None => f(slice),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn reschedule_replaces_inserts_and_removes_like_the_frontend() {
        assert_eq!(reschedule_line("- [ ] a 📅 2026-08-01", DateField::Due, Some("2026-09-10")), "- [ ] a 📅 2026-09-10");
        assert_eq!(reschedule_line("- [ ] a due::2026-8-1 ^k1", DateField::Due, Some("2026-09-10")), "- [ ] a due::2026-09-10 ^k1");
        assert_eq!(reschedule_line("- [ ] a ^k1", DateField::Due, Some("2026-09-10")), "- [ ] a 📅 2026-09-10 ^k1");
        assert_eq!(reschedule_line("- [ ] a", DateField::Scheduled, Some("2026-09-10")), "- [ ] a ⏳ 2026-09-10");
        assert_eq!(reschedule_line("- [ ] a", DateField::Start, Some("2026-09-10")), "- [ ] a ⏩ 2026-09-10");
        // a bad token: the date goes after the marker and the token is demoted to text
        assert_eq!(reschedule_line("- [ ] a 📅 next Friday", DateField::Due, Some("2026-09-10")), "- [ ] a 📅 2026-09-10 next Friday");
        assert_eq!(reschedule_line("- [ ] a 📅 2026-08-01 due:: 2026-08-02 b", DateField::Due, None), "- [ ] a b");
        assert_eq!(reschedule_line("- [ ] a ⏳ 2026-08-01", DateField::Due, None), "- [ ] a ⏳ 2026-08-01");
        // a bracketed field: replaced inside its brackets, removed with them
        assert_eq!(reschedule_line("- [ ] a [due:: 2026-08-01] b", DateField::Due, Some("2026-09-10")), "- [ ] a [due:: 2026-09-10] b");
        assert_eq!(reschedule_line("- [ ] a [due:: 2026-08-01] b", DateField::Due, None), "- [ ] a b");
        // the spellings read for compatibility are rewritten in place too
        assert_eq!(reschedule_line("- [ ] a 🛫 2026-08-01", DateField::Start, Some("2026-09-10")), "- [ ] a 🛫 2026-09-10");
        assert!(is_date_token("2026-9-5") && !is_date_token("2026-02-30") && !is_date_token("tomorrow"));
    }

    #[test]
    fn checkbox_and_fields_rewrite_in_place() {
        assert_eq!(set_checkbox("  - [ ] x", TaskState::Done), "  - [x] x");
        assert_eq!(set_checkbox("1. [x] x", TaskState::Todo), "1. [ ] x");
        assert_eq!(set_checkbox("- [/] x", TaskState::Doing), "- [/] x");
        assert_eq!(set_checkbox("- [ ] x", TaskState::Cancelled), "- [-] x");
        assert_eq!(set_checkbox("- [-] x", TaskState::Todo), "- [ ] x");
        assert_eq!(set_checkbox("- plain", TaskState::Done), "- plain");
        assert_eq!(set_field_value("- [ ] a [owner:: 王] ^k1", "owner", Some("李")).unwrap(), "- [ ] a [owner:: 李] ^k1");
        assert_eq!(set_field_value("- [ ] a [owner:: 王] b", "owner", None).unwrap(), "- [ ] a b");
        assert_eq!(set_field_value("- [ ] a ^k1", "project", Some("网关")).unwrap(), "- [ ] a [project:: 网关] ^k1");
        assert_eq!(set_field_value("- [ ] a", "project", Some("网关")).unwrap(), "- [ ] a [project:: 网关]");
        assert_eq!(set_field_value("- [ ] a [status:: ] b", "status", Some("open")).unwrap(), "- [ ] a [status:: open] b");
        assert_eq!(set_field_value("- a priority:: 3", "priority", Some("1")).unwrap(), "- a priority:: 1");
        assert!(set_field_value("- a", "bad key", Some("x")).is_none());
        assert_eq!(rewrite_first_line("- [ ] a\n  sub\n", |l| set_checkbox(l, TaskState::Done)), "- [x] a\n  sub\n");
    }

    #[test]
    fn a_state_change_leaves_its_stamp() {
        let t = day("2026-09-11");
        assert_eq!(set_state("- [ ] a 📅 2026-09-12 ^k1", TaskState::Done, t), "- [x] a 📅 2026-09-12 ✅ 2026-09-11 ^k1");
        assert_eq!(set_state("- [ ] a", TaskState::Cancelled, t), "- [-] a ❌ 2026-09-11");
        assert_eq!(set_state("- [x] a ✅ 2026-09-10", TaskState::Todo, t), "- [ ] a");
        assert_eq!(set_state("- [-] a ❌ 2026-09-10", TaskState::Doing, t), "- [/] a");
        assert_eq!(set_state("- [-] a ❌ 2026-09-10", TaskState::Done, t), "- [x] a ✅ 2026-09-11");
        assert_eq!(set_state("- [x] a [completion:: 2026-09-10]", TaskState::Done, t), "- [x] a [completion:: 2026-09-11]");
    }

    #[test]
    fn completing_a_slice_repeats_and_deletes() {
        let t = day("2026-09-11");
        assert_eq!(complete_task_slice("- [ ] a 📅 2026-09-12\n  sub\n", TaskState::Done, t), "- [x] a 📅 2026-09-12 ✅ 2026-09-11\n  sub\n");
        assert_eq!(
            complete_task_slice("- [ ] a 🔁 every week 📅 2026-09-12", TaskState::Done, t),
            "- [ ] a 🔁 every week 📅 2026-09-19\n- [x] a 🔁 every week 📅 2026-09-12 ✅ 2026-09-11"
        );
        assert_eq!(complete_task_slice("- [ ] a 🔁 every week 🚮 📅 2026-09-12", TaskState::Done, t), "- [ ] a 🔁 every week 🚮 📅 2026-09-19");
        assert_eq!(complete_task_slice("- [ ] a 🚮", TaskState::Done, t), "");
        // any other state change is the stamp alone
        assert_eq!(complete_task_slice("- [ ] a 🔁 every week 🚮", TaskState::Cancelled, t), "- [-] a 🔁 every week 🚮 ❌ 2026-09-11");
        assert_eq!(complete_task_slice("- [ ] a\r\n  sub", TaskState::Done, t), "- [x] a ✅ 2026-09-11\r\n  sub");
    }
}
