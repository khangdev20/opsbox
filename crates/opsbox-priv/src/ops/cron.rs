//! Cron expressions to systemd `OnCalendar=` (docs/BACKEND_SPEC.md 7.8), with errors
//! written for people.
//!
//! Supported: five fields (minute hour day-of-month month day-of-week), each `*`, a
//! number, a list `a,b`, a range `a-b` or a step `*/n` / `a-b/n`; month and weekday
//! names; and `@hourly`, `@daily`, `@weekly`, `@monthly`, `@yearly`. systemd cannot
//! express cron's "day of month OR day of week" rule, so setting both is refused.

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
const DAY_NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

struct Field {
    label: &'static str,
    min: u32,
    max: u32,
    names: &'static [&'static str],
    /// Number of the first name (months start at 1, weekdays at 0).
    name_base: u32,
}

const MINUTE: Field = Field {
    label: "minute",
    min: 0,
    max: 59,
    names: &[],
    name_base: 0,
};
const HOUR: Field = Field {
    label: "hour",
    min: 0,
    max: 23,
    names: &[],
    name_base: 0,
};
const DOM: Field = Field {
    label: "day of month",
    min: 1,
    max: 31,
    names: &[],
    name_base: 0,
};
const MONTH: Field = Field {
    label: "month",
    min: 1,
    max: 12,
    names: &MONTHS,
    name_base: 1,
};
// 0 and 7 are both Sunday.
const DOW: Field = Field {
    label: "day of week",
    min: 0,
    max: 7,
    names: &DAYS,
    name_base: 0,
};

fn number(f: &Field, token: &str) -> Result<u32, String> {
    let lower = token.to_ascii_lowercase();
    let n = match f.names.iter().position(|n| *n == lower) {
        Some(i) => i as u32 + f.name_base,
        None => token.parse::<u32>().map_err(|_| {
            format!(
                "\"{token}\" is not a valid {} (expected a number{}).",
                f.label,
                if f.names.is_empty() { "" } else { " or a name" }
            )
        })?,
    };
    if (f.min..=f.max).contains(&n) {
        Ok(n)
    } else {
        Err(format!(
            "The {} must be between {} and {}, not {n}.",
            f.label, f.min, f.max
        ))
    }
}

/// One field to the pieces systemd wants. `Any` means `*`.
#[derive(Debug, PartialEq, Eq)]
enum Parsed {
    Any,
    Items(Vec<Item>),
}

#[derive(Debug, PartialEq, Eq)]
enum Item {
    One(u32),
    Range(u32, u32),
    /// start, optional end, step
    Step(u32, Option<u32>, u32),
}

fn parse_field(f: &Field, text: &str) -> Result<Parsed, String> {
    if text == "*" {
        return Ok(Parsed::Any);
    }
    let mut items = Vec::new();
    for part in text.split(',') {
        if part.is_empty() {
            return Err(format!(
                "The {} field has an empty entry (a stray comma?).",
                f.label
            ));
        }
        let (base, step) = match part.split_once('/') {
            Some((b, s)) => {
                let step: u32 = s.parse().map_err(|_| {
                    format!("The step \"{s}\" in the {} field is not a number.", f.label)
                })?;
                if step == 0 {
                    return Err(format!(
                        "A step of 0 in the {} field would never run.",
                        f.label
                    ));
                }
                (b, Some(step))
            }
            None => (part, None),
        };
        let item = match (base, step) {
            ("*", Some(step)) => Item::Step(f.min, None, step),
            ("*", None) => {
                return Err(format!(
                    "Write * on its own for the {} field, not inside a list.",
                    f.label
                ));
            }
            (b, step) => {
                if let Some((lo, hi)) = b.split_once('-') {
                    let (lo, hi) = (number(f, lo)?, number(f, hi)?);
                    if lo > hi {
                        return Err(format!(
                            "The range {b} in the {} field runs backwards.",
                            f.label
                        ));
                    }
                    match step {
                        Some(s) => Item::Step(lo, Some(hi), s),
                        None => Item::Range(lo, hi),
                    }
                } else {
                    let n = number(f, b)?;
                    match step {
                        Some(s) => Item::Step(n, Some(f.max), s),
                        None => Item::One(n),
                    }
                }
            }
        };
        items.push(item);
    }
    Ok(Parsed::Items(items))
}

fn render(p: &Parsed, pad: bool, name: impl Fn(u32) -> String) -> String {
    let num = |n: u32| if pad { format!("{n:02}") } else { name(n) };
    match p {
        Parsed::Any => "*".into(),
        Parsed::Items(items) => items
            .iter()
            .map(|i| match i {
                Item::One(n) => num(*n),
                Item::Range(a, b) => format!("{}..{}", num(*a), num(*b)),
                // systemd: `start/step` repeats from start; with an end, expand the range first.
                Item::Step(a, None, s) => format!("{}/{s}", num(*a)),
                Item::Step(a, Some(b), s) => {
                    let all: Vec<String> = (*a..=*b).step_by(*s as usize).map(&num).collect();
                    all.join(",")
                }
            })
            .collect::<Vec<_>>()
            .join(","),
    }
}

fn normalize_dow(p: Parsed) -> Parsed {
    // 7 is Sunday too; systemd names make that a non-issue, but ranges like 5-7 need care.
    match p {
        Parsed::Items(items) => Parsed::Items(
            items
                .into_iter()
                .flat_map(|i| match i {
                    Item::One(7) => vec![Item::One(0)],
                    Item::Range(a, 7) if a < 7 => vec![Item::Range(a, 6), Item::One(0)],
                    Item::Step(a, Some(7), s) => {
                        let mut out = Vec::new();
                        for n in (a..=7).step_by(s as usize) {
                            out.push(Item::One(if n == 7 { 0 } else { n }));
                        }
                        out
                    }
                    other => vec![other],
                })
                .collect(),
        ),
        any => any,
    }
}

/// Convert a cron expression to an `OnCalendar=` value.
pub fn to_on_calendar(expr: &str) -> Result<String, String> {
    let expr = expr.trim();
    if let Some(alias) = expr.strip_prefix('@') {
        return match alias {
            "hourly" => Ok("hourly".into()),
            "daily" | "midnight" => Ok("daily".into()),
            "weekly" => Ok("weekly".into()),
            "monthly" => Ok("monthly".into()),
            "yearly" | "annually" => Ok("yearly".into()),
            other => Err(format!(
                "\"@{other}\" is not a schedule opsbox knows. Use @hourly, @daily, @weekly, @monthly or five fields."
            )),
        };
    }
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "A schedule needs 5 fields (minute hour day-of-month month day-of-week), for example \"0 2 * * *\" for 02:00 every day. This one has {}.",
            fields.len()
        ));
    }
    let minute = parse_field(&MINUTE, fields[0])?;
    let hour = parse_field(&HOUR, fields[1])?;
    let dom = parse_field(&DOM, fields[2])?;
    let month = parse_field(&MONTH, fields[3])?;
    let dow = normalize_dow(parse_field(&DOW, fields[4])?);
    if dom != Parsed::Any && dow != Parsed::Any {
        return Err("Set either the day of the month or the day of the week, not both: systemd timers cannot express cron's \"either\" rule.".into());
    }
    let day_part = match &dow {
        Parsed::Any => String::new(),
        d => format!(
            "{} ",
            render(d, false, |n| DAY_NAMES[(n % 7) as usize].to_owned())
        ),
    };
    Ok(format!(
        "{day_part}*-{}-{} {}:{}:00",
        render(&month, true, |n| n.to_string()),
        render(&dom, true, |n| n.to_string()),
        render(&hour, true, |n| n.to_string()),
        render(&minute, true, |n| n.to_string()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(expr: &str) -> String {
        to_on_calendar(expr).unwrap_or_else(|e| panic!("{expr}: {e}"))
    }
    fn err(expr: &str) -> String {
        to_on_calendar(expr).expect_err(expr)
    }

    #[test]
    fn common_schedules() {
        assert_eq!(ok("0 2 * * *"), "*-*-* 02:00:00");
        assert_eq!(ok("30 4 1 * *"), "*-*-01 04:30:00");
        assert_eq!(ok("0 3 * * 0"), "Sun *-*-* 03:00:00");
        assert_eq!(ok("15 10 * * 1-5"), "Mon..Fri *-*-* 10:15:00");
        assert_eq!(ok("0 0 1 1 *"), "*-01-01 00:00:00");
        assert_eq!(ok("* * * * *"), "*-*-* *:*:00");
    }

    #[test]
    fn steps_lists_and_ranges() {
        assert_eq!(ok("*/5 * * * *"), "*-*-* *:00/5:00");
        assert_eq!(ok("17 */12 * * *"), "*-*-* 00/12:17:00");
        assert_eq!(ok("0,30 8-17 * * *"), "*-*-* 08..17:00,30:00");
        assert_eq!(ok("0 8-18/2 * * *"), "*-*-* 08,10,12,14,16,18:00:00");
        assert_eq!(ok("10-20/5 * * * *"), "*-*-* *:10,15,20:00");
        assert_eq!(ok("5/15 * * * *"), "*-*-* *:05,20,35,50:00");
        assert_eq!(
            ok("0 0 */10 * *"),
            "*-*-1/10 00:00:00".replace("1/10", "01/10")
        );
    }

    #[test]
    fn names_and_sunday_seven() {
        assert_eq!(ok("0 9 * * mon,wed,fri"), "Mon,Wed,Fri *-*-* 09:00:00");
        assert_eq!(ok("0 9 * jan,jul *"), "*-01,07-* 09:00:00");
        assert_eq!(ok("0 9 * * 7"), "Sun *-*-* 09:00:00");
        assert_eq!(ok("0 9 * * 5-7"), "Fri..Sat,Sun *-*-* 09:00:00");
        assert_eq!(ok("0 9 * * MON-FRI"), "Mon..Fri *-*-* 09:00:00");
    }

    #[test]
    fn aliases() {
        for (a, b) in [
            ("@hourly", "hourly"),
            ("@daily", "daily"),
            ("@weekly", "weekly"),
            ("@monthly", "monthly"),
            ("@yearly", "yearly"),
        ] {
            assert_eq!(ok(a), b);
        }
        assert!(err("@reboot").contains("@reboot"));
    }

    #[test]
    fn whitespace_is_forgiven() {
        assert_eq!(ok("  0   2  *   *  * "), "*-*-* 02:00:00");
    }

    #[test]
    fn errors_say_what_is_wrong() {
        assert!(err("").contains("5 fields") && err("").contains("has 0"));
        assert!(err("* * * *").contains("has 4"));
        assert!(err("* * * * * *").contains("has 6"));
        assert!(err("60 * * * *").contains("minute must be between 0 and 59, not 60"));
        assert!(err("* 24 * * *").contains("hour must be between 0 and 23"));
        assert!(err("* * 0 * *").contains("day of month must be between 1 and 31"));
        assert!(err("* * * 13 *").contains("month must be between 1 and 12"));
        assert!(err("* * * * 8").contains("day of week must be between 0 and 7"));
        assert!(err("a * * * *").contains("\"a\" is not a valid minute"));
        assert!(err("*/0 * * * *").contains("step of 0"));
        assert!(err("*/x * * * *").contains("not a number"));
        assert!(err("5-1 * * * *").contains("runs backwards"));
        assert!(err("1,,2 * * * *").contains("empty entry"));
        assert!(
            err("* * * foo *").contains("not a valid month")
                && err("* * * foo *").contains("or a name")
        );
        assert!(err("0 0 1 * 1").contains("either the day of the month or the day of the week"));
        assert!(err("1,* * * * *").contains("on its own"));
    }

    #[test]
    fn nothing_hostile_survives_into_the_output() {
        for hostile in [
            "0 0 * * *\nExecStart=/bin/evil",
            "0 0 * * * ; rm -rf /",
            "$(id) * * * *",
            "0 0 * * *%n",
        ] {
            // Either rejected, or a clean schedule; never anything with a newline or shell syntax.
            if let Ok(s) = to_on_calendar(hostile) {
                assert!(!s.contains(['\n', ';', '$', '%']), "{hostile} -> {s}");
            }
        }
    }
}
