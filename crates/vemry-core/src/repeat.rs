//! Recurring tasks: the rule after 🔁 / `repeat::`, and the next occurrence of a task
//! that has just been ticked.
//!
//! The rule is `every [N] day|week|month|year`, optionally followed by `when done`
//! (the unit may be plural). Without `when done` the next occurrence is counted from
//! the task's own date (its due date, else its scheduled date, else its start date);
//! with it, from the day the task was ticked. Every date the line carries moves by
//! the same amount, so a task "start Monday, due Friday, every week" keeps its shape.
//! The new line is the ticked line reopened: the checkbox back to `[ ]`, the ✅ / ❌
//! stamps gone, a ➕ created date refreshed, and no `^anchor` (an id must not be
//! duplicated). It is placed above the ticked line, which keeps its record.
//! The frontend mirrors this in `views/tasks.ts` (nextOccurrenceLine) — change both.

use crate::taskattrs::TaskAttrs;
use crate::taskedit::{reschedule_line, set_checkbox, DateField};
use crate::TaskState;
use chrono::{Days, Months, NaiveDate};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Day,
    Week,
    Month,
    Year,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Repeat {
    pub every: u32,
    pub unit: Unit,
    pub when_done: bool,
}

impl Repeat {
    /// The date one period after `from`. A month or year step lands on the same day of
    /// the month, clamped to the month's length.
    pub fn next(&self, from: NaiveDate) -> NaiveDate {
        let n = self.every.max(1);
        match self.unit {
            Unit::Day => from.checked_add_days(Days::new(n as u64)).unwrap_or(from),
            Unit::Week => from.checked_add_days(Days::new(7 * n as u64)).unwrap_or(from),
            Unit::Month => from.checked_add_months(Months::new(n)).unwrap_or(from),
            // saturating: a rule may name any number of years, and `12 * n` would overflow
            Unit::Year => from.checked_add_months(Months::new(n.saturating_mul(12))).unwrap_or(from),
        }
    }
}

/// `every 2 weeks when done` → a rule; None when the text is not one the MVP reads.
pub fn parse_repeat(rule: &str) -> Option<Repeat> {
    let lower = rule.trim().to_ascii_lowercase();
    let mut words: Vec<&str> = lower.split_whitespace().collect();
    let when_done = words.len() >= 2 && words[words.len() - 2..] == ["when", "done"];
    if when_done {
        words.truncate(words.len() - 2);
    }
    if words.first() != Some(&"every") {
        return None;
    }
    let (every, unit_word) = match words.len() {
        2 => (1, words[1]),
        3 => (words[1].parse::<u32>().ok().filter(|n| *n > 0)?, words[2]),
        _ => return None,
    };
    let unit = match unit_word.trim_end_matches('s') {
        "day" => Unit::Day,
        "week" => Unit::Week,
        "month" => Unit::Month,
        "year" => Unit::Year,
        _ => return None,
    };
    Some(Repeat { every, unit, when_done })
}

/// The line of the next occurrence of a task being ticked on `today`, from the line as
/// it is before the tick; None when the line has no readable rule.
pub fn next_occurrence(line: &str, attrs: &TaskAttrs, today: NaiveDate) -> Option<String> {
    let rule = parse_repeat(attrs.repeat.as_deref()?)?;
    let anchor = attrs.due.or(attrs.scheduled).or(attrs.start);
    let from = if rule.when_done { today } else { anchor.unwrap_or(today) };
    let target = rule.next(from);
    let shift = target - anchor.unwrap_or(from);
    let ymd = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
    let mut out = set_checkbox(line, TaskState::Todo);
    out = reschedule_line(&out, DateField::Done, None);
    out = reschedule_line(&out, DateField::Cancelled, None);
    for (field, date) in [(DateField::Due, attrs.due), (DateField::Scheduled, attrs.scheduled), (DateField::Start, attrs.start)] {
        if let Some(d) = date {
            out = reschedule_line(&out, field, Some(&ymd(d + shift)));
        }
    }
    if anchor.is_none() {
        // A rule with no date to count from: the next occurrence gets the target as its
        // scheduled day, or it would be a copy of the line with nothing to place it by
        out = reschedule_line(&out, DateField::Scheduled, Some(&ymd(target)));
    }
    if attrs.created.is_some() {
        out = reschedule_line(&out, DateField::Created, Some(&ymd(today)));
    }
    if let Some(at) = crate::blockid::anchor_start(&out) {
        out = out[..at].trim_end_matches([' ', '\t']).to_string();
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taskattrs::parse_task_attrs;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn rules_read_and_step() {
        assert_eq!(parse_repeat("every week"), Some(Repeat { every: 1, unit: Unit::Week, when_done: false }));
        assert_eq!(parse_repeat("Every 2 Weeks when done"), Some(Repeat { every: 2, unit: Unit::Week, when_done: true }));
        assert_eq!(parse_repeat("every day"), Some(Repeat { every: 1, unit: Unit::Day, when_done: false }));
        assert_eq!(parse_repeat("every 3 months"), Some(Repeat { every: 3, unit: Unit::Month, when_done: false }));
        assert_eq!(parse_repeat("every year"), Some(Repeat { every: 1, unit: Unit::Year, when_done: false }));
        assert!(parse_repeat("every month on the 1st").is_none());
        assert!(parse_repeat("weekly").is_none());
        assert!(parse_repeat("every 0 days").is_none());
        assert_eq!(parse_repeat("every month").unwrap().next(d("2026-01-31")), d("2026-02-28"));
        assert_eq!(parse_repeat("every 2 weeks").unwrap().next(d("2026-09-10")), d("2026-09-24"));
    }

    #[test]
    fn the_next_occurrence_moves_every_date_and_reopens() {
        let line = "- [ ] weekly report 🔁 every week ⏩ 2026-09-08 ⏳ 2026-09-10 📅 2026-09-12 ^w1";
        let next = next_occurrence(line, &parse_task_attrs(&line[6..]), d("2026-09-11")).unwrap();
        assert_eq!(next, "- [ ] weekly report 🔁 every week ⏩ 2026-09-15 ⏳ 2026-09-17 📅 2026-09-19");
        // when done: counted from the day of the tick, the other dates keeping their distance
        let line = "- [ ] water plants 🔁 every 3 days when done ⏳ 2026-09-01 📅 2026-09-02";
        let next = next_occurrence(line, &parse_task_attrs(&line[6..]), d("2026-09-11")).unwrap();
        assert_eq!(next, "- [ ] water plants 🔁 every 3 days when done ⏳ 2026-09-13 📅 2026-09-14");
        // no date at all: the target becomes the scheduled day
        let line = "  - [x] stretch 🔁 every day ✅ 2026-09-11";
        let next = next_occurrence(line, &parse_task_attrs(&line[8..]), d("2026-09-11")).unwrap();
        assert_eq!(next, "  - [ ] stretch 🔁 every day ⏳ 2026-09-12");
        // the created stamp is refreshed
        let line = "- [ ] a 🔁 every week ➕ 2026-09-01 📅 2026-09-12";
        let next = next_occurrence(line, &parse_task_attrs(&line[6..]), d("2026-09-11")).unwrap();
        assert_eq!(next, "- [ ] a 🔁 every week ➕ 2026-09-11 📅 2026-09-19");
        assert!(next_occurrence("- [ ] a 📅 2026-09-12", &parse_task_attrs("a 📅 2026-09-12"), d("2026-09-11")).is_none());
    }
}
