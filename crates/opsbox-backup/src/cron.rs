//! Pure 5-field cron parser, validator, human label formatter, and UTC next/previous
//! schedule evaluator for backup jobs (docs/BACKEND_SPEC.md 7.12).

use time::{Date, Duration, Month, OffsetDateTime, Time, Weekday};

use crate::BackupError;

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const DAY_LABELS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldSpec {
    label: &'static str,
    min: u8,
    max: u8,
    names: &'static [&'static str],
    name_base: u8,
}

const MINUTE: FieldSpec = FieldSpec {
    label: "minute",
    min: 0,
    max: 59,
    names: &[],
    name_base: 0,
};
const HOUR: FieldSpec = FieldSpec {
    label: "hour",
    min: 0,
    max: 23,
    names: &[],
    name_base: 0,
};
const DOM: FieldSpec = FieldSpec {
    label: "day of month",
    min: 1,
    max: 31,
    names: &[],
    name_base: 0,
};
const MONTH: FieldSpec = FieldSpec {
    label: "month",
    min: 1,
    max: 12,
    names: &MONTHS,
    name_base: 1,
};
const DOW: FieldSpec = FieldSpec {
    label: "day of week",
    min: 0,
    max: 7,
    names: &DAYS,
    name_base: 0,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldSet {
    any: bool,
    bits: u64,
}

impl FieldSet {
    fn contains(&self, val: u8) -> bool {
        val < 64 && (self.bits & (1u64 << val)) != 0
    }
}

fn parse_num(f: &FieldSpec, token: &str) -> Result<u8, BackupError> {
    let lower = token.to_ascii_lowercase();
    let n = match f.names.iter().position(|n| *n == lower) {
        Some(idx) => {
            let idx_u8 = u8::try_from(idx).unwrap_or(u8::MAX);
            idx_u8.saturating_add(f.name_base)
        }
        None => token.parse::<u8>().map_err(|_| {
            BackupError::Validation(format!(
                "\"{token}\" is not a valid {} (expected a number{}).",
                f.label,
                if f.names.is_empty() { "" } else { " or a name" }
            ))
        })?,
    };
    if (f.min..=f.max).contains(&n) {
        Ok(n)
    } else {
        Err(BackupError::Validation(format!(
            "The {} must be between {} and {}, not {n}.",
            f.label, f.min, f.max
        )))
    }
}

fn parse_field(f: &FieldSpec, raw: &str) -> Result<FieldSet, BackupError> {
    if raw == "*" {
        let mut bits = 0u64;
        for v in f.min..=f.max {
            bits |= 1u64 << v;
        }
        return Ok(FieldSet { any: true, bits });
    }
    let mut bits = 0u64;
    for part in raw.split(',') {
        if part.is_empty() {
            return Err(BackupError::Validation(format!(
                "The {} field has an empty entry.",
                f.label
            )));
        }
        let (base, step) = match part.split_once('/') {
            Some((b, s)) => {
                let st: u8 = s.parse().map_err(|_| {
                    BackupError::Validation(format!(
                        "The step \"{s}\" in the {} field is not a number.",
                        f.label
                    ))
                })?;
                if st == 0 {
                    return Err(BackupError::Validation(format!(
                        "A step of 0 in the {} field would never run.",
                        f.label
                    )));
                }
                (b, st)
            }
            None => (part, 1),
        };
        let (lo, hi) = if base == "*" {
            (f.min, f.max)
        } else if let Some((a, b)) = base.split_once('-') {
            let lo = parse_num(f, a)?;
            let hi = parse_num(f, b)?;
            if lo > hi {
                return Err(BackupError::Validation(format!(
                    "The range {base} in the {} field runs backwards.",
                    f.label
                )));
            }
            (lo, hi)
        } else {
            let n = parse_num(f, base)?;
            if part.contains('/') {
                (n, f.max)
            } else {
                (n, n)
            }
        };
        let mut v = lo;
        while v <= hi {
            bits |= 1u64 << v;
            match v.checked_add(step) {
                Some(next) => v = next,
                None => break,
            }
        }
    }
    if bits == 0 {
        return Err(BackupError::Validation(format!(
            "The {} field matched no values.",
            f.label
        )));
    }
    Ok(FieldSet { any: false, bits })
}

fn normalize_dow(mut fs: FieldSet) -> FieldSet {
    // Both 0 and 7 represent Sunday in standard cron.
    if fs.contains(7) {
        fs.bits |= 1u64 << 0;
        fs.bits &= !(1u64 << 7);
    }
    fs
}

/// Parsed 5-field cron schedule evaluated in UTC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSchedule {
    normalized: String,
    minute: FieldSet,
    hour: FieldSet,
    dom: FieldSet,
    month: FieldSet,
    dow: FieldSet,
}

impl CronSchedule {
    pub fn parse(expr: &str) -> Result<Self, BackupError> {
        let trimmed = expr.trim();
        let expanded = if let Some(alias) = trimmed.strip_prefix('@') {
            match alias.to_ascii_lowercase().as_str() {
                "hourly" => "0 * * * *",
                "daily" | "midnight" => "0 0 * * *",
                "weekly" => "0 0 * * 0",
                "monthly" => "0 0 1 * *",
                "yearly" | "annually" => "0 0 1 1 *",
                other => {
                    return Err(BackupError::Validation(format!(
                        "\"@{other}\" is not a supported schedule alias. Use @hourly, @daily, @weekly, @monthly, or five cron fields."
                    )));
                }
            }
        } else {
            trimmed
        };
        let fields: Vec<&str> = expanded.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(BackupError::Validation(format!(
                "A schedule needs 5 fields (minute hour day-of-month month day-of-week), for example \"0 2 * * *\" for 02:00 UTC every day. Found {} fields.",
                fields.len()
            )));
        }
        let minute = parse_field(&MINUTE, fields[0])?;
        let hour = parse_field(&HOUR, fields[1])?;
        let dom = parse_field(&DOM, fields[2])?;
        let month = parse_field(&MONTH, fields[3])?;
        let dow = normalize_dow(parse_field(&DOW, fields[4])?);
        Ok(Self {
            normalized: fields.join(" "),
            minute,
            hour,
            dom,
            month,
            dow,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.normalized
    }

    fn matches_date(&self, date: Date) -> bool {
        let m = month_num(date.month());
        if !self.month.contains(m) {
            return false;
        }
        let d = date.day();
        let w = weekday_num(date.weekday());
        match (self.dom.any, self.dow.any) {
            (true, true) => true,
            (false, true) => self.dom.contains(d),
            (true, false) => self.dow.contains(w),
            (false, false) => self.dom.contains(d) || self.dow.contains(w),
        }
    }

    /// Next UTC unix timestamp strictly after `after_unix` matching this schedule.
    pub fn next_run_after(&self, after_unix: i64) -> Option<i64> {
        let start_ts = (after_unix.div_euclid(60) + 1).saturating_mul(60);
        let start_dt = OffsetDateTime::from_unix_timestamp(start_ts).ok()?;
        let max_days = 366 * 4;
        let mut date = start_dt.date();

        for day_idx in 0..=max_days {
            if self.matches_date(date) {
                let start_hour = if day_idx == 0 { start_dt.hour() } else { 0 };
                for h in start_hour..24 {
                    if !self.hour.contains(h) {
                        continue;
                    }
                    let start_min = if day_idx == 0 && h == start_dt.hour() {
                        start_dt.minute()
                    } else {
                        0
                    };
                    for m in start_min..60 {
                        if self.minute.contains(m) {
                            let t = Time::from_hms(h, m, 0).ok()?;
                            return Some(date.with_time(t).assume_utc().unix_timestamp());
                        }
                    }
                }
            }
            date = date.next_day()?;
        }
        None
    }

    /// Most recent UTC unix timestamp at or before `at_or_before_unix` matching this schedule.
    pub fn prev_run_at_or_before(&self, at_or_before_unix: i64) -> Option<i64> {
        let end_ts = at_or_before_unix.div_euclid(60).saturating_mul(60);
        let end_dt = OffsetDateTime::from_unix_timestamp(end_ts).ok()?;
        let max_days = 366 * 4;
        let mut date = end_dt.date();

        for day_idx in 0..=max_days {
            if self.matches_date(date) {
                let max_hour = if day_idx == 0 { end_dt.hour() } else { 23 };
                for h in (0..=max_hour).rev() {
                    if !self.hour.contains(h) {
                        continue;
                    }
                    let max_min = if day_idx == 0 && h == end_dt.hour() {
                        end_dt.minute()
                    } else {
                        59
                    };
                    for m in (0..=max_min).rev() {
                        if self.minute.contains(m) {
                            let t = Time::from_hms(h, m, 0).ok()?;
                            return Some(date.with_time(t).assume_utc().unix_timestamp());
                        }
                    }
                }
            }
            date = date.previous_day()?;
        }
        None
    }
}

/// Validate a cron string and return its canonical whitespace-collapsed form (or alias).
pub fn validate_cron(expr: &str) -> Result<String, BackupError> {
    let trimmed = expr.trim();
    let _ = CronSchedule::parse(trimmed)?;
    Ok(trimmed.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Format a cron expression cleanly for UI tables (e.g., `"daily 02:00"`).
pub fn human_label(expr: &str) -> String {
    let Ok(parsed) = CronSchedule::parse(expr) else {
        return expr.trim().to_owned();
    };
    let parts: Vec<&str> = parsed.as_str().split_whitespace().collect();
    if parts.len() != 5 {
        return expr.trim().to_owned();
    }
    if let (Ok(min), Ok(hr)) = (parts[0].parse::<u8>(), parts[1].parse::<u8>()) {
        if parts[2] == "*" && parts[3] == "*" && parts[4] == "*" {
            return format!("daily {hr:02}:{min:02}");
        }
        if parts[2] == "*"
            && parts[3] == "*"
            && let Ok(dow) = parts[4].parse::<u8>()
            && dow <= 7
        {
            return format!(
                "weekly {} {hr:02}:{min:02}",
                DAY_LABELS[usize::from(dow % 7)]
            );
        }
        if parts[3] == "*"
            && parts[4] == "*"
            && let Ok(dom) = parts[2].parse::<u8>()
        {
            return format!("monthly day {dom} {hr:02}:{min:02}");
        }
    }
    if parts[1] == "*"
        && parts[2] == "*"
        && parts[3] == "*"
        && parts[4] == "*"
        && let Ok(min) = parts[0].parse::<u8>()
    {
        return format!("hourly :{min:02}");
    }
    expr.trim().to_owned()
}

/// Format unix timestamp as UTC `YYYY-MM-DD HH:MM`.
pub fn format_utc_short(ts: i64) -> String {
    let Ok(dt) = OffsetDateTime::from_unix_timestamp(ts) else {
        return "unknown".to_owned();
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        dt.year(),
        month_num(dt.month()),
        dt.day(),
        dt.hour(),
        dt.minute()
    )
}

/// Compute `Duration` step helper when checking missed runs (returns true if `last_run_at`
/// missed the most recent scheduled time by more than `grace_secs`).
pub fn is_run_missed(
    expr: &str,
    last_run_at: Option<i64>,
    created_at: i64,
    now_unix: i64,
    grace_secs: i64,
) -> bool {
    let Ok(sched) = CronSchedule::parse(expr) else {
        return false;
    };
    let check_at = now_unix.saturating_sub(grace_secs.max(0));
    let Some(expected) = sched.prev_run_at_or_before(check_at) else {
        return false;
    };
    let baseline = last_run_at.unwrap_or(created_at);
    // Avoid flagging a brand-new job created after `expected` — wait until a run boundary
    // strictly after `created_at` was missed.
    expected > created_at && baseline < expected
}

fn month_num(m: Month) -> u8 {
    match m {
        Month::January => 1,
        Month::February => 2,
        Month::March => 3,
        Month::April => 4,
        Month::May => 5,
        Month::June => 6,
        Month::July => 7,
        Month::August => 8,
        Month::September => 9,
        Month::October => 10,
        Month::November => 11,
        Month::December => 12,
    }
}

fn weekday_num(w: Weekday) -> u8 {
    match w {
        Weekday::Sunday => 0,
        Weekday::Monday => 1,
        Weekday::Tuesday => 2,
        Weekday::Wednesday => 3,
        Weekday::Thursday => 4,
        Weekday::Friday => 5,
        Weekday::Saturday => 6,
    }
}

#[allow(dead_code)]
fn _keep_duration_used() -> Duration {
    Duration::ZERO
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(y: i32, m: u8, d: u8, h: u8, min: u8) -> i64 {
        let month = Month::try_from(m).unwrap();
        Date::from_calendar_date(y, month, d)
            .unwrap()
            .with_hms(h, min, 0)
            .unwrap()
            .assume_utc()
            .unix_timestamp()
    }

    #[test]
    fn parses_standard_expressions_and_aliases() {
        for expr in [
            "0 2 * * *",
            "*/15 * * * *",
            "0 8-18/2 * * 1-5",
            "0 9 * jan,jul mon,fri",
            "0 3 * * 7",
            "@hourly",
            "@daily",
            "@weekly",
            "@monthly",
            "@yearly",
        ] {
            assert!(CronSchedule::parse(expr).is_ok(), "{expr}");
        }
        for bad in [
            "",
            "* * *",
            "* * * * * *",
            "60 * * * *",
            "* 24 * * *",
            "0 0 32 * *",
            "0 0 * 13 *",
            "0 0 * * 8",
            "*/0 * * * *",
            "5-1 * * * *",
            "1,,2 * * * *",
            "@reboot",
        ] {
            assert!(CronSchedule::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn next_and_prev_run_evaluate_accurately() {
        let daily = CronSchedule::parse("0 2 * * *").unwrap();
        let before = ts(2026, 11, 18, 1, 59);
        assert_eq!(daily.next_run_after(before), Some(ts(2026, 11, 18, 2, 0)));
        let at_exact = ts(2026, 11, 18, 2, 0);
        assert_eq!(daily.next_run_after(at_exact), Some(ts(2026, 11, 19, 2, 0)));
        assert_eq!(
            daily.prev_run_at_or_before(ts(2026, 11, 18, 3, 0)),
            Some(ts(2026, 11, 18, 2, 0))
        );

        // Leap year handling for Feb 29.
        let leap = CronSchedule::parse("30 4 29 2 *").unwrap();
        let next_leap = leap.next_run_after(ts(2026, 1, 1, 0, 0)).unwrap();
        assert_eq!(next_leap, ts(2028, 2, 29, 4, 30));

        // Sunday as 7 matches Sunday (2026-11-22 is a Sunday).
        let sun = CronSchedule::parse("15 3 * * 7").unwrap();
        assert_eq!(
            sun.next_run_after(ts(2026, 11, 18, 0, 0)),
            Some(ts(2026, 11, 22, 3, 15))
        );
    }

    #[test]
    fn missed_run_detection_respects_grace_and_creation_time() {
        let created = ts(2026, 11, 18, 1, 0);
        let run_at = ts(2026, 11, 18, 2, 0);
        // Before 02:00 + 300s grace -> not missed.
        assert!(!is_run_missed(
            "0 2 * * *",
            None,
            created,
            run_at + 120,
            300
        ));
        // After 02:00 + 300s grace with no run -> missed.
        assert!(is_run_missed("0 2 * * *", None, created, run_at + 600, 300));
        // With successful run at 02:01 -> not missed.
        assert!(!is_run_missed(
            "0 2 * * *",
            Some(run_at + 60),
            created,
            run_at + 600,
            300
        ));
        // Created AFTER 02:00 -> not missed for yesterday's slot.
        assert!(!is_run_missed(
            "0 2 * * *",
            None,
            ts(2026, 11, 18, 3, 0),
            ts(2026, 11, 18, 4, 0),
            300
        ));
    }

    #[test]
    fn human_labels_match_ui_style() {
        assert_eq!(human_label("0 2 * * *"), "daily 02:00");
        assert_eq!(human_label("@daily"), "daily 00:00");
        assert_eq!(human_label("0 3 * * 0"), "weekly Sun 03:00");
        assert_eq!(human_label("15 * * * *"), "hourly :15");
    }
}
