//! Line-level rewrites of a task or a block's first line: the checkbox state (with the
//! ✅ / ❌ stamp a closing state leaves), a date attribute (`📅 2026-09-10` /
//! `[due:: 2026-09-10]`, and every other date of `taskattrs.rs`) and an inline
//! `[key:: value]` field. The frontend's `views/tasks.ts` (rescheduleLine, insertAttr,
//! setStateLine, completeTaskSlice, updateTask) and `fields.ts` (setFieldValue)
//! implement the same rules for the editor and the task views; this is the agent's
//! copy, so a task the AI ticks or reschedules lands on disk byte for byte as one the
//! user dragged on the board. Change the rules in both places together.

use crate::parser::{is_field_key, parse_list_marker, parse_task_line, scan_fields};
use crate::repeat::next_occurrence;
use crate::taskattrs::{parse_date, parse_task_attrs, scan_task_attrs, AttrKind};
use crate::TaskState;
use chrono::NaiveDate;

/// The date attributes a task line carries.
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
    pub fn kind(self) -> AttrKind {
        match self {
            DateField::Due => AttrKind::Due,
            DateField::Scheduled => AttrKind::Scheduled,
            DateField::Start => AttrKind::Start,
            DateField::Created => AttrKind::Created,
            DateField::Done => AttrKind::Done,
            DateField::Cancelled => AttrKind::Cancelled,
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

/// `YYYY-MM-DD` exactly, and a real calendar date (the one date spelling a task attribute takes).
pub fn is_date_token(s: &str) -> bool {
    parse_date(s).is_some()
}

/// Append `piece` (with its own leading space) to a line before any trailing anchor.
fn append_before_anchor(line: &str, piece: &str) -> String {
    match crate::blockid::anchor_start(line) {
        // The anchor's own separator is written back as one space: `anchor_start` points at
        // the first of however many stood there, and keeping them would leave a run of
        // spaces the frontend's appendBeforeAnchor does not write
        Some(at) => format!("{}{piece} {}", line[..at].trim_end_matches([' ', '\t']), line[at..].trim_start_matches([' ', '\t'])),
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

/// Where the scanner's text starts on a line: after the checkbox of a task line, at the
/// start of any other.
fn rest_offset(line: &str) -> usize {
    parse_task_line(line).map(|(_, rest)| line.len() - rest.len()).unwrap_or(0)
}

/// Insert a fresh attribute (`piece`, e.g. `📅 2026-09-10`) into the trailing run at
/// its place in the written order: before the first attribute that comes later in the
/// order, else at the end of the line before any `^anchor`. The mirror of `insertAttr`.
pub fn insert_attr(line: &str, kind: AttrKind, piece: &str) -> String {
    let at0 = rest_offset(line);
    let marks = scan_task_attrs(&line[at0..]);
    if let Some(m) = marks.iter().find(|m| m.tail && m.kind > kind) {
        let at = at0 + m.span.start;
        return format!("{}{piece} {}", &line[..at], &line[at..]);
    }
    append_before_anchor(line, &format!(" {piece}"))
}

/// A marker of `kind` typed at the end of the line with nothing after it (an `@`
/// palette pick, `📅` awaiting its date): where its value goes.
fn bare_marker_end(line: &str, kind: AttrKind) -> Option<usize> {
    let end = crate::blockid::anchor_start(line).unwrap_or(line.len());
    let body = line[..end].trim_end_matches([' ', '\t']);
    let stripped = body.trim_end_matches('\u{FE0F}');
    kind.emoji().iter().any(|e| stripped.ends_with(e)).then_some(body.len())
}

/// Set, replace or (`None`) remove a task line's date attribute. The attribute the
/// parser reads (the last of its kind in the trailing run) is rewritten in place, its
/// spelling kept; a bare marker left at the end of the line is given the date; a
/// fresh attribute is written emoji-style at its place in the order. Removing takes
/// every occurrence in the run (a duplicate would otherwise become the date); a marker
/// in the description is a word and is left alone. The mirror of `rescheduleLine`.
pub fn reschedule_line(line: &str, field: DateField, date: Option<&str>) -> String {
    let kind = field.kind();
    let at0 = rest_offset(line);
    let marks = scan_task_attrs(&line[at0..]);
    let Some(date) = date else {
        let mut out = line.to_string();
        for m in marks.iter().rev().filter(|m| m.tail && m.kind == kind) {
            out = cut(&out, at0 + m.span.start, at0 + m.span.end);
        }
        return out;
    };
    if let Some(m) = marks.iter().find(|m| m.active && m.kind == kind) {
        let (s, e) = (at0 + m.span.start, at0 + m.span.end);
        let at = s + line[s..e].rfind(&m.value).expect("a date mark carries its value");
        return format!("{}{date}{}", &line[..at], &line[at + m.value.len()..]);
    }
    if let Some(at) = bare_marker_end(line, kind) {
        return format!("{} {date}{}", &line[..at], &line[at..]);
    }
    insert_attr(line, kind, &format!("{} {date}", kind.emoji()[0]))
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
        assert_eq!(reschedule_line("- [ ] a [due:: 2026-08-01] ^k1", DateField::Due, Some("2026-09-10")), "- [ ] a [due:: 2026-09-10] ^k1");
        assert_eq!(reschedule_line("- [ ] a ^k1", DateField::Due, Some("2026-09-10")), "- [ ] a 📅 2026-09-10 ^k1");
        assert_eq!(reschedule_line("- [ ] a", DateField::Scheduled, Some("2026-09-10")), "- [ ] a ⏳ 2026-09-10");
        assert_eq!(reschedule_line("- [ ] a", DateField::Start, Some("2026-09-10")), "- [ ] a ⏩ 2026-09-10");
        // a marker in the description is a word: the date is written fresh, at the end
        assert_eq!(reschedule_line("- [ ] a 📅 next Friday", DateField::Due, Some("2026-09-10")), "- [ ] a 📅 next Friday 📅 2026-09-10");
        assert_eq!(reschedule_line("- [ ] call 📅 2026-08-01 about it", DateField::Due, None), "- [ ] call 📅 2026-08-01 about it");
        // removing takes every occurrence in the run
        assert_eq!(reschedule_line("- [ ] a 📅 2026-08-01 [due:: 2026-08-02]", DateField::Due, None), "- [ ] a");
        assert_eq!(reschedule_line("- [ ] a ⏳ 2026-08-01", DateField::Due, None), "- [ ] a ⏳ 2026-08-01");
        // a bracketed field: replaced inside its brackets, removed with them
        assert_eq!(reschedule_line("- [ ] a [due:: 2026-08-01] #b", DateField::Due, Some("2026-09-10")), "- [ ] a [due:: 2026-09-10] #b");
        assert_eq!(reschedule_line("- [ ] a [due:: 2026-08-01] #b", DateField::Due, None), "- [ ] a #b");
        // the other spellings are rewritten in place too
        assert_eq!(reschedule_line("- [ ] a 🛫 2026-08-01", DateField::Start, Some("2026-09-10")), "- [ ] a 🛫 2026-09-10");
        // a bare marker at the end gets its date
        assert_eq!(reschedule_line("- [ ] a 📅", DateField::Due, Some("2026-09-10")), "- [ ] a 📅 2026-09-10");
        assert_eq!(reschedule_line("- [ ] a ⏳ ^k1", DateField::Scheduled, Some("2026-09-10")), "- [ ] a ⏳ 2026-09-10 ^k1");
        assert!(is_date_token("2026-09-05") && !is_date_token("2026-9-5") && !is_date_token("2026-02-30") && !is_date_token("tomorrow"));
    }

    #[test]
    fn a_fresh_attribute_lands_in_the_written_order() {
        assert_eq!(reschedule_line("- [ ] a 📅 2026-09-15 ^k1", DateField::Start, Some("2026-09-10")), "- [ ] a ⏩ 2026-09-10 📅 2026-09-15 ^k1");
        assert_eq!(reschedule_line("- [ ] a #t 🔁 every week 📅 2026-09-15", DateField::Scheduled, Some("2026-09-12")), "- [ ] a #t 🔁 every week ⏳ 2026-09-12 📅 2026-09-15");
        assert_eq!(reschedule_line("- [ ] a ⏩ 2026-09-10 📅 2026-09-15", DateField::Created, Some("2026-09-01")), "- [ ] a ➕ 2026-09-01 ⏩ 2026-09-10 📅 2026-09-15");
        assert_eq!(reschedule_line("- [ ] a ❗ 📅 2026-09-15 #t", DateField::Done, Some("2026-09-16")), "- [ ] a ❗ 📅 2026-09-15 #t ✅ 2026-09-16");
        assert_eq!(insert_attr("- [ ] a ❗ 📅 2026-09-15", AttrKind::Id, "🆔 k3x9pa"), "- [ ] a 🆔 k3x9pa ❗ 📅 2026-09-15");
        assert_eq!(insert_attr("- [ ] 买菜📅 2026-09-15", AttrKind::Estimate, "🕐 30m"), "- [ ] 买菜🕐 30m 📅 2026-09-15");
        assert_eq!(insert_attr("- [ ] a ^k1", AttrKind::Depends, "⛔ k3x9pa"), "- [ ] a ⛔ k3x9pa ^k1");
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
