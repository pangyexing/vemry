//! Line-level rewrites of a task or a block's first line: the checkbox state, a
//! date marker (`📅 2026-09-10` / `due:: 2026-09-10`, `⏳` / `scheduled::`) and an
//! inline `[key:: value]` field. The frontend's `views/tasks.ts` (rescheduleLine,
//! updateTask) and `fields.ts` (setFieldValue) implement the same rules for the
//! editor and the task views; this is the agent's copy, so a task the AI ticks or
//! reschedules lands on disk byte for byte as one the user dragged on the board.
//! Change the rules in both places together.

use crate::parser::{is_field_key, parse_list_marker, scan_fields};
use crate::TaskState;

/// The two date fields a task line carries, each with its emoji and `attr::`
/// spelling (emoji first: the parse priority of `parse_date_attr`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateField {
    Due,
    Scheduled,
}

impl DateField {
    pub fn markers(self) -> [&'static str; 2] {
        match self {
            DateField::Due => ["📅", "due::"],
            DateField::Scheduled => ["⏳", "scheduled::"],
        }
    }
    pub fn parse(key: &str) -> Option<Self> {
        match key {
            "due" => Some(DateField::Due),
            "scheduled" => Some(DateField::Scheduled),
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

/// The next whitespace-delimited token after `at` (skipping horizontal space):
/// (start, end) offsets, possibly empty.
fn next_token(line: &str, at: usize) -> (usize, usize) {
    let mut start = at;
    while line[start..].starts_with([' ', '\t']) {
        start += 1;
    }
    let end = line[start..].find(char::is_whitespace).map(|i| start + i).unwrap_or(line.len());
    (start, end)
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
                    out = cut(&out, start, te);
                    pos = start;
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

/// Set the checkbox of a task's first line (`[ ]` / `[/]` / `[x]`); a line with no
/// checkbox is returned unchanged.
pub fn set_checkbox(line: &str, state: TaskState) -> String {
    let ch = match state {
        TaskState::Done => 'x',
        TaskState::Doing => '/',
        TaskState::Todo => ' ',
    };
    let Some((_, _, at)) = parse_list_marker(line) else { return line.to_string() };
    let rest = &line[at..];
    let mut chars = rest.chars();
    match (chars.next(), chars.next(), chars.next()) {
        (Some('['), Some(' ' | '/' | 'x' | 'X'), Some(']')) => format!("{}[{ch}]{}", &line[..at], &rest[3..]),
        _ => line.to_string(),
    }
}

/// Set, replace or (`None`) remove one inline field on a line: the field with that
/// key is rewritten in place (its form kept), an empty placeholder `[key:: ]` is
/// filled, and a new field is appended as `[key:: value]` before any trailing
/// `^anchor`. The mirror of `setFieldValue`; `due`/`scheduled` on a task line are
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

    #[test]
    fn reschedule_replaces_inserts_and_removes_like_the_frontend() {
        assert_eq!(reschedule_line("- [ ] a 📅 2026-08-01", DateField::Due, Some("2026-09-10")), "- [ ] a 📅 2026-09-10");
        assert_eq!(reschedule_line("- [ ] a due::2026-8-1 ^k1", DateField::Due, Some("2026-09-10")), "- [ ] a due::2026-09-10 ^k1");
        assert_eq!(reschedule_line("- [ ] a ^k1", DateField::Due, Some("2026-09-10")), "- [ ] a 📅 2026-09-10 ^k1");
        assert_eq!(reschedule_line("- [ ] a", DateField::Scheduled, Some("2026-09-10")), "- [ ] a ⏳ 2026-09-10");
        // a bad token: the date goes after the marker and the token is demoted to text
        assert_eq!(reschedule_line("- [ ] a 📅 next Friday", DateField::Due, Some("2026-09-10")), "- [ ] a 📅 2026-09-10 next Friday");
        assert_eq!(reschedule_line("- [ ] a 📅 2026-08-01 due:: 2026-08-02 b", DateField::Due, None), "- [ ] a b");
        assert_eq!(reschedule_line("- [ ] a ⏳ 2026-08-01", DateField::Due, None), "- [ ] a ⏳ 2026-08-01");
        assert!(is_date_token("2026-9-5") && !is_date_token("2026-02-30") && !is_date_token("tomorrow"));
    }

    #[test]
    fn checkbox_and_fields_rewrite_in_place() {
        assert_eq!(set_checkbox("  - [ ] x", TaskState::Done), "  - [x] x");
        assert_eq!(set_checkbox("1. [x] x", TaskState::Todo), "1. [ ] x");
        assert_eq!(set_checkbox("- [/] x", TaskState::Doing), "- [/] x");
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
}
