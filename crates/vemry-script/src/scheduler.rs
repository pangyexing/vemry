//! Cron timing logic, kept pure so it tests with synthetic clocks. Users
//! write standard POSIX 5-field expressions (minute hour day month weekday)
//! evaluated in the user's LOCAL timezone ("every day at 9" means 9 am on the
//! wall clock); the host
//! prepends a seconds field and translates the day-of-week numbering (POSIX
//! 0-7 with 0/7 = Sunday vs the `cron` crate's 1-7 with 1 = Sunday). Missed
//! fires (sleep, app closed) are skipped, not replayed — one `Missed` outcome
//! per skipped stretch for the audit log.

use chrono::{DateTime, Local, Utc};

/// A fire older than this when we notice it counts as missed (the app was
/// asleep or busy far beyond scheduling jitter).
pub const MISS_TOLERANCE: chrono::Duration = chrono::Duration::seconds(30);

/// Translate a user-facing POSIX 5-field cron line into the 6-field form the
/// `cron` crate parses. `None` when the field count is wrong; token-level
/// errors are left for the crate's parser to report.
pub fn to_crate_expr(expr: &str) -> Option<String> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    let [min, hour, dom, month, dow] = fields[..] else { return None };
    Some(format!("0 {min} {hour} {dom} {month} {}", translate_dow(dow)))
}

/// POSIX numeric days-of-week (0-7, 0 and 7 = Sunday) → crate ordinals
/// (1-7, 1 = Sunday). Name tokens (mon, fri…) already agree between the two
/// systems and pass through, as does `*` (both minimums mean Sunday, so
/// `*/n` steps land on the same days).
fn translate_dow(field: &str) -> String {
    field.split(',').map(translate_dow_part).collect::<Vec<_>>().join(",")
}

fn translate_dow_part(part: &str) -> String {
    let (base, step) = match part.split_once('/') {
        Some((b, s)) => (b, Some(s)),
        None => (part, None),
    };
    let posix = |s: &str| s.parse::<u8>().ok().filter(|d| *d <= 7);
    let mapped = |d: u8| (d % 7) + 1;
    if let Some((a, b)) = base.split_once('-') {
        if let (Some(a), Some(b)) = (posix(a), posix(b)) {
            // a mapped range can wrap (POSIX 5-7 → 6,7,1), so expand to a
            // day list, honoring any step
            let step_n = step.and_then(|s| s.parse::<usize>().ok()).unwrap_or(1).max(1);
            let days: Vec<String> = (a..=b).step_by(step_n).map(|d| mapped(d).to_string()).collect();
            if !days.is_empty() {
                return days.join(",");
            }
        }
    } else if let Some(d) = posix(base) {
        return match step {
            Some(s) => format!("{}/{s}", mapped(d)),
            None => mapped(d).to_string(),
        };
    }
    part.to_string()
}

pub struct CronEntry {
    pub expr: String,
    schedule: cron::Schedule,
    next: Option<DateTime<Utc>>,
}

#[derive(Debug, PartialEq)]
pub enum Due {
    /// Fire the job now (exactly once per check, no catch-up bursts).
    Fire,
    /// The scheduled time passed too long ago — skip and log.
    Missed { scheduled: DateTime<Utc> },
}

/// Next occurrence strictly after `now`, evaluated on the local wall clock
/// (the timezone users write schedules in) and returned as UTC for the
/// engine's arithmetic.
fn next_after(schedule: &cron::Schedule, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    schedule.after(&now.with_timezone(&Local)).next().map(|t| t.with_timezone(&Utc))
}

impl CronEntry {
    /// `expr` is the user's 5-field cron line (already validated at
    /// registration; a parse failure here still degrades to "never fires").
    pub fn new(expr: &str, now: DateTime<Utc>) -> Option<CronEntry> {
        let schedule: cron::Schedule = to_crate_expr(expr)?.parse().ok()?;
        let next = next_after(&schedule, now);
        Some(CronEntry { expr: expr.to_string(), schedule, next })
    }

    pub fn next_fire(&self) -> Option<DateTime<Utc>> {
        self.next
    }

    /// Advance past `now`, reporting what happened to the deadline we were
    /// holding. At most one `Fire`; a long gap collapses into one `Missed`.
    pub fn check(&mut self, now: DateTime<Utc>) -> Option<Due> {
        let scheduled = self.next?;
        if scheduled > now {
            return None;
        }
        self.next = next_after(&self.schedule, now);
        if now - scheduled > MISS_TOLERANCE {
            Some(Due::Missed { scheduled })
        } else {
            Some(Due::Fire)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, TimeZone, Timelike, Weekday};

    /// Parse a LOCAL wall-clock time and return it as the UTC instant the
    /// engine works with — expectations stay machine-independent because the
    /// schedule is evaluated on the same local clock.
    fn t(s: &str) -> DateTime<Utc> {
        let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap();
        Local.from_local_datetime(&naive).single().unwrap().with_timezone(&Utc)
    }

    #[test]
    fn five_field_exprs_fire_on_the_minute() {
        let now = t("2026-08-10 09:59:30");
        let mut e = CronEntry::new("0 10 * * *", now).unwrap();
        assert_eq!(e.next_fire(), Some(t("2026-08-10 10:00:00")));
        // not due yet
        assert_eq!(e.check(t("2026-08-10 09:59:59")), None);
        // due within tolerance → fire, next advances to tomorrow
        assert_eq!(e.check(t("2026-08-10 10:00:05")), Some(Due::Fire));
        assert_eq!(e.next_fire(), Some(t("2026-08-11 10:00:00")));
    }

    #[test]
    fn fires_on_local_wall_clock() {
        // "daily at 09:00" means 9 am on the user's clock, not 9 am UTC
        let now = t("2026-08-10 08:00:00");
        let e = CronEntry::new("0 9 * * *", now).unwrap();
        let local = e.next_fire().unwrap().with_timezone(&Local);
        assert_eq!((local.hour(), local.minute()), (9, 0));
        assert_eq!(local.day(), 10);
    }

    #[test]
    fn posix_day_of_week_numbering() {
        // 2026-08-10 is a Monday. POSIX 1 = Monday (the crate's raw 1 would
        // have meant Sunday), and both 0 and 7 mean Sunday.
        let now = t("2026-08-10 08:00:00");
        let monday = CronEntry::new("0 9 * * 1", now).unwrap();
        assert_eq!(monday.next_fire(), Some(t("2026-08-10 09:00:00")));
        for expr in ["0 9 * * 0", "0 9 * * 7"] {
            let sunday = CronEntry::new(expr, now).unwrap();
            let local = sunday.next_fire().unwrap().with_timezone(&Local);
            assert_eq!(local.weekday(), Weekday::Sun, "{expr}");
            assert_eq!(local.day(), 16, "{expr}");
        }
        // weekday range wraps over the mapped Sunday: Fri-Sun → 6,7,1
        assert_eq!(to_crate_expr("0 9 * * 5-7"), Some("0 0 9 * * 6,7,1".into()));
        assert_eq!(to_crate_expr("0 9 * * 1-5"), Some("0 0 9 * * 2,3,4,5,6".into()));
        // names and '*' pass through untouched
        assert_eq!(to_crate_expr("0 9 * * mon-fri"), Some("0 0 9 * * mon-fri".into()));
        assert_eq!(to_crate_expr("*/5 * * * *"), Some("0 */5 * * * *".into()));
    }

    #[test]
    fn late_wakeup_is_missed_not_replayed() {
        let now = t("2026-08-10 09:00:00");
        let mut e = CronEntry::new("*/5 * * * *", now).unwrap();
        assert_eq!(e.next_fire(), Some(t("2026-08-10 09:05:00")));
        // the machine slept for two hours: one Missed, no burst of fires
        let wake = t("2026-08-10 11:03:00");
        assert_eq!(e.check(wake), Some(Due::Missed { scheduled: t("2026-08-10 09:05:00") }));
        assert_eq!(e.next_fire(), Some(t("2026-08-10 11:05:00")));
        // and the next on-time check fires normally
        assert_eq!(e.check(t("2026-08-10 11:05:01")), Some(Due::Fire));
    }

    #[test]
    fn invalid_expr_never_fires() {
        assert!(CronEntry::new("not a cron", Utc::now()).is_none());
        assert!(CronEntry::new("* * * *", Utc::now()).is_none());
    }
}
