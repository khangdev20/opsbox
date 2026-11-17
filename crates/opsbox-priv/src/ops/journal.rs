//! Reading the journal (docs/BACKEND_SPEC.md 7.8): `journalctl --output=json`, filtered
//! by unit and priority, text filtering done here so no pattern reaches the tool.

use std::time::Duration;

use opsbox_proto::{JournalEntry, PrivError, PrivErrorCode};

use super::exec::{find, run};
use super::units::valid_service_name;

const MAX_LIMIT: u32 = 500;
/// Entries read before filtering by text.
const SCAN: u32 = 3000;

fn err(code: PrivErrorCode, msg: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: msg.into(),
    }
}

/// Journal cursors look like `s=...;i=1b3;b=...;m=...;t=...;x=...`.
pub fn valid_cursor(c: &str) -> bool {
    !c.is_empty()
        && c.len() <= 300
        && c.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'=' | b';' | b'-' | b'_'))
}

pub fn parse_entries(text: &str) -> Vec<JournalEntry> {
    text.lines()
        .filter_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            let micros: i64 = v["__REALTIME_TIMESTAMP"].as_str()?.parse().ok()?;
            let prio = v["PRIORITY"]
                .as_str()
                .and_then(|p| p.parse::<u8>().ok())
                .unwrap_or(6)
                .min(7);
            // MESSAGE is a string, or an array of bytes when it holds non-UTF-8 data.
            let msg = match &v["MESSAGE"] {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(a) => String::from_utf8_lossy(
                    &a.iter()
                        .filter_map(|b| b.as_u64().map(|n| n as u8))
                        .collect::<Vec<u8>>(),
                )
                .into_owned(),
                _ => String::new(),
            };
            let ident = v["SYSLOG_IDENTIFIER"]
                .as_str()
                .or_else(|| v["_COMM"].as_str())
                .unwrap_or("");
            let proc = match v["_PID"].as_str() {
                Some(pid) if !ident.is_empty() => format!("{ident}[{pid}]"),
                _ => ident.to_owned(),
            };
            Some(JournalEntry {
                t: micros / 1000,
                proc,
                prio,
                msg,
                cursor: v["__CURSOR"].as_str().unwrap_or("").to_owned(),
            })
        })
        .collect()
}

pub(super) fn query(
    unit: Option<&str>,
    max_priority: u8,
    text: Option<&str>,
    limit: u32,
    after: Option<&str>,
) -> Result<Vec<JournalEntry>, PrivError> {
    if max_priority > 7 {
        return Err(err(
            PrivErrorCode::Invalid,
            "The priority must be between 0 (emergency) and 7 (debug).",
        ));
    }
    let limit = limit.clamp(1, MAX_LIMIT);
    let unit_arg = match unit.filter(|u| !u.is_empty()) {
        Some(u) => {
            let full = if u.contains('.') {
                u.to_owned()
            } else {
                format!("{u}.service")
            };
            if !valid_service_name(&full) {
                return Err(err(PrivErrorCode::Invalid, "That is not a unit name."));
            }
            Some(full)
        }
        None => None,
    };
    if let Some(c) = after
        && !valid_cursor(c)
    {
        return Err(err(
            PrivErrorCode::Invalid,
            "That is not a journal position.",
        ));
    }
    let bin = find(&["/usr/bin/journalctl", "/bin/journalctl"]).ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "journalctl is not available on this machine.",
        )
    })?;

    let scan = if text.is_some_and(|t| !t.is_empty()) {
        SCAN
    } else {
        limit
    }
    .to_string();
    let prio = max_priority.to_string();
    let after_arg = after.map(|c| format!("--after-cursor={c}"));
    let mut args: Vec<&str> = vec!["--no-pager", "--output=json", "-p", &prio];
    if let Some(a) = &after_arg {
        // Following: everything after the cursor, oldest first (the caller polls often, so
        // this is small). `-n` would instead pick the newest lines and could skip some.
        args.push(a);
    } else {
        args.extend(["-n", &scan, "--reverse"]);
    }
    if let Some(u) = &unit_arg {
        args.extend(["-u", u]);
    }
    let o = run(&bin, &args, Duration::from_secs(20)).ok_or_else(|| {
        err(
            PrivErrorCode::Internal,
            "The journal did not answer in time.",
        )
    })?;
    if !o.ok() {
        return Err(err(
            PrivErrorCode::Internal,
            "The journal could not be read.",
        ));
    }
    let needle = text.map(str::to_lowercase).filter(|t| !t.is_empty());
    let mut entries: Vec<JournalEntry> = parse_entries(&o.stdout)
        .into_iter()
        .filter(|e| {
            needle.as_ref().is_none_or(|n| {
                e.proc.to_lowercase().contains(n) || e.msg.to_lowercase().contains(n)
            })
        })
        .collect();
    entries.truncate(limit as usize);
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = concat!(
        r#"{"__CURSOR":"s=abc;i=1;b=x;m=1;t=1;x=1","__REALTIME_TIMESTAMP":"1700000000123456","PRIORITY":"3","SYSLOG_IDENTIFIER":"systemd","_PID":"1","MESSAGE":"snapd.service: Failed with result 'exit-code'."}"#,
        "\n",
        r#"{"__CURSOR":"s=abc;i=2","__REALTIME_TIMESTAMP":"1700000001000000","PRIORITY":"6","_COMM":"sshd","MESSAGE":[72,105,32,226,128,148]}"#,
        "\n",
        "not json at all\n",
        r#"{"__REALTIME_TIMESTAMP":"nope","MESSAGE":"x"}"#,
        "\n",
        r#"{"__CURSOR":"s=abc;i=3","__REALTIME_TIMESTAMP":"1700000002000000","MESSAGE":"no priority, no ident"}"#,
        "\n",
    );

    #[test]
    fn json_lines_become_entries() {
        let e = parse_entries(SAMPLE);
        assert_eq!(e.len(), 3, "broken lines are skipped");
        assert_eq!(
            (e[0].t, e[0].prio, e[0].proc.as_str()),
            (1_700_000_000_123, 3, "systemd[1]")
        );
        assert_eq!(e[0].msg, "snapd.service: Failed with result 'exit-code'.");
        assert_eq!(e[1].proc, "sshd", "falls back to _COMM, no pid");
        assert_eq!(e[1].msg, "Hi —", "byte-array messages are decoded");
        assert_eq!(
            (e[2].prio, e[2].proc.as_str()),
            (6, ""),
            "missing priority is 'info'"
        );
        assert_eq!(e[0].cursor, "s=abc;i=1;b=x;m=1;t=1;x=1");
    }

    #[test]
    fn cursors_are_checked_before_use() {
        assert!(valid_cursor("s=0f6d;i=1b3;b=9c;m=5a0;t=611;x=2f"));
        for bad in ["", "a b", "x;rm -rf /", "$(id)", "a\nb", &"a".repeat(301)] {
            assert!(!valid_cursor(bad), "{bad:?}");
        }
    }

    #[test]
    fn bad_arguments_are_refused_before_the_tool_runs() {
        assert_eq!(
            query(None, 9, None, 10, None).unwrap_err().code,
            PrivErrorCode::Invalid
        );
        assert_eq!(
            query(Some("../x"), 6, None, 10, None).unwrap_err().code,
            PrivErrorCode::Invalid
        );
        assert_eq!(
            query(Some("ssh"), 6, None, 10, Some("bad cursor;"))
                .unwrap_err()
                .code,
            PrivErrorCode::Invalid
        );
    }
}
