//! Scheduled jobs as systemd timers (docs/BACKEND_SPEC.md 7.8): a pair of units
//! `opsbox-job-<id>.service` and `.timer` in `/etc/systemd/system`. The cron expression
//! is converted to `OnCalendar=` and the original kept in the unit for display.

use std::path::Path;
use std::time::Duration;

use opsbox_proto::{JobInfo, PrivError, PrivErrorCode};

use super::cron::to_on_calendar;
use super::exec::{find, run, tail};

const UNIT_DIR: &str = "/etc/systemd/system";
const PREFIX: &str = "opsbox-job-";
const MAX_COMMAND: usize = 1000;
const MAX_NAME: usize = 100;

fn err(code: PrivErrorCode, msg: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: msg.into(),
    }
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 8
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// systemd `ExecStart=` quoting: one argument in double quotes, with `\`, `"` escaped and
/// `$` and `%` doubled so neither systemd variable nor specifier expansion applies.
pub fn quote_arg(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    for c in arg.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("$$"),
            '%' => out.push_str("%%"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Names are shown in the UI and written into unit files: one line, printable, short.
pub fn check_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("Describe what the job does.".into());
    }
    if n.chars().count() > MAX_NAME || n.chars().any(char::is_control) {
        return Err(format!(
            "The description must be one line of at most {MAX_NAME} characters."
        ));
    }
    Ok(n.to_owned())
}

pub fn check_command(cmd: &str) -> Result<String, String> {
    let c = cmd.trim();
    if c.is_empty() {
        return Err("Enter the command to run.".into());
    }
    if c.len() > MAX_COMMAND || c.contains(['\n', '\r', '\0']) {
        return Err(format!(
            "The command must be a single line of at most {MAX_COMMAND} characters. Put longer scripts in a file and run that."
        ));
    }
    Ok(c.to_owned())
}

pub fn service_unit(name: &str, cron: &str, command: &str) -> String {
    format!(
        "[Unit]\nDescription=opsbox job: {name}\nX-Opsbox-Job-Name={name}\nX-Opsbox-Job-Cron={cron}\nX-Opsbox-Job-Command={cmd_comment}\n\n[Service]\nType=oneshot\nExecStart=/bin/sh -c {}\n",
        quote_arg(command),
        cmd_comment = command.replace('%', "%%"),
    )
}

pub fn timer_unit(name: &str, on_calendar: &str) -> String {
    format!(
        "[Unit]\nDescription=opsbox job timer: {name}\n\n[Timer]\nOnCalendar={on_calendar}\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n"
    )
}

/// Read back `(name, cron, command)` from a service unit this module wrote.
pub fn parse_service_unit(text: &str) -> Option<(String, String, String)> {
    let get = |key: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(key))
            .map(|v| v.replace("%%", "%"))
    };
    Some((
        get("X-Opsbox-Job-Name=")?,
        get("X-Opsbox-Job-Cron=")?,
        get("X-Opsbox-Job-Command=")?,
    ))
}

fn systemctl() -> Result<String, PrivError> {
    find(&["/usr/bin/systemctl", "/bin/systemctl"]).ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "systemd is not available on this machine.",
        )
    })
}

fn new_id() -> Result<String, PrivError> {
    let mut b = [0u8; 4];
    getrandom::fill(&mut b)
        .map_err(|_| err(PrivErrorCode::Internal, "random number generator failed"))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

pub(super) fn add(name: &str, cron: &str, command: &str) -> Result<String, PrivError> {
    add_with(name, cron, command, Path::new(UNIT_DIR), systemctl)
}

fn add_with(
    name: &str,
    cron: &str,
    command: &str,
    unit_dir: &Path,
    systemctl: impl FnOnce() -> Result<String, PrivError>,
) -> Result<String, PrivError> {
    let name = check_name(name).map_err(|m| err(PrivErrorCode::Invalid, m))?;
    let command = check_command(command).map_err(|m| err(PrivErrorCode::Invalid, m))?;
    let on_calendar = to_on_calendar(cron).map_err(|m| err(PrivErrorCode::Invalid, m))?;
    let cron = cron.trim().to_owned();
    let ctl = systemctl()?;
    let id = new_id()?;
    let write = |suffix: &str, content: &str| {
        super::network::atomic_write(
            &unit_dir.join(format!("{PREFIX}{id}.{suffix}")),
            content.as_bytes(),
            0o600,
        )
        .map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("cannot write the unit file: {}", e.message),
            )
        })
    };
    write("service", &service_unit(&name, &cron, &command))?;
    write("timer", &timer_unit(&name, &on_calendar))?;
    run(&ctl, &["daemon-reload"], Duration::from_secs(30));
    let o = run(
        &ctl,
        &["enable", "--now", &format!("{PREFIX}{id}.timer")],
        Duration::from_secs(30),
    );
    if !o.as_ref().is_some_and(|o| o.ok()) {
        let _ = remove_files_in(unit_dir, &id);
        run(&ctl, &["daemon-reload"], Duration::from_secs(30));
        let why = o.map(|o| tail(&o.stderr, 2).join(" ")).unwrap_or_default();
        return Err(err(
            PrivErrorCode::Invalid,
            format!("systemd did not accept that schedule. {why}"),
        ));
    }
    Ok(id)
}

fn remove_files(id: &str) -> std::io::Result<()> {
    remove_files_in(Path::new(UNIT_DIR), id)
}

fn remove_files_in(unit_dir: &Path, id: &str) -> std::io::Result<()> {
    for suffix in ["timer", "service"] {
        let p = unit_dir.join(format!("{PREFIX}{id}.{suffix}"));
        if Path::new(&p).exists() {
            std::fs::remove_file(p)?;
        }
    }
    Ok(())
}

pub(super) fn remove(id: &str) -> Result<(), PrivError> {
    if !valid_id(id) {
        return Err(err(PrivErrorCode::Invalid, "That is not a job id."));
    }
    if !Path::new(&format!("{UNIT_DIR}/{PREFIX}{id}.timer")).exists() {
        return Err(err(PrivErrorCode::NotFound, "There is no such job."));
    }
    let ctl = systemctl()?;
    run(
        &ctl,
        &["disable", "--now", &format!("{PREFIX}{id}.timer")],
        Duration::from_secs(30),
    );
    remove_files(id).map_err(|e| {
        err(
            PrivErrorCode::Internal,
            format!("cannot remove the unit files: {e}"),
        )
    })?;
    run(&ctl, &["daemon-reload"], Duration::from_secs(30));
    Ok(())
}

pub(super) fn run_now(id: &str) -> Result<(), PrivError> {
    if !valid_id(id) {
        return Err(err(PrivErrorCode::Invalid, "That is not a job id."));
    }
    if !Path::new(&format!("{UNIT_DIR}/{PREFIX}{id}.service")).exists() {
        return Err(err(PrivErrorCode::NotFound, "There is no such job."));
    }
    let ctl = systemctl()?;
    // A oneshot unit: `start` returns when the command has finished.
    let o = run(
        &ctl,
        &["start", &format!("{PREFIX}{id}.service")],
        Duration::from_secs(15 * 60),
    )
    .ok_or_else(|| {
        err(
            PrivErrorCode::Internal,
            "The job did not finish in 15 minutes.",
        )
    })?;
    if o.ok() {
        Ok(())
    } else {
        Err(err(
            PrivErrorCode::Internal,
            "The job ran but reported a failure. See Logs for its output.",
        ))
    }
}

pub(super) fn list() -> Result<Vec<JobInfo>, PrivError> {
    let ctl = systemctl().ok();
    let mut jobs = Vec::new();
    let Ok(dir) = std::fs::read_dir(UNIT_DIR) else {
        return Ok(jobs);
    };
    for entry in dir.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some(id) = file
            .strip_prefix(PREFIX)
            .and_then(|r| r.strip_suffix(".timer"))
        else {
            continue;
        };
        if !valid_id(id) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(format!("{UNIT_DIR}/{PREFIX}{id}.service")) else {
            continue;
        };
        let Some((name, cron, command)) = parse_service_unit(&text) else {
            continue;
        };
        let unit = format!("{PREFIX}{id}.service");
        // Result of the last run, and when it ended (from the journal's newest line for it).
        let ok = ctl
            .as_ref()
            .and_then(|c| {
                run(
                    c,
                    &["show", "-p", "Result", "--value", &unit],
                    Duration::from_secs(10),
                )
            })
            .is_none_or(|o| matches!(o.stdout.trim(), "success" | ""));
        let last = find(&["/usr/bin/journalctl", "/bin/journalctl"])
            .and_then(|j| {
                run(
                    &j,
                    &["-u", &unit, "-n", "1", "--no-pager", "-o", "json"],
                    Duration::from_secs(10),
                )
            })
            .and_then(|o| {
                super::journal::parse_entries(&o.stdout)
                    .first()
                    .map(|e| e.t / 1000)
            });
        jobs.push(JobInfo {
            id: id.to_owned(),
            cron,
            name,
            command,
            last_run: last,
            last_ok: ok,
            next_run: None,
        });
    }
    jobs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(jobs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_add_creates_private_unit_and_timer_files() {
        use std::os::unix::fs::PermissionsExt;

        // Change umask only in a child process; test threads share the parent's umask.
        if std::env::var_os("OPSBOX_TEST_JOB_UMASK").is_none() {
            let status = std::process::Command::new("/bin/sh")
                .args([
                    "-c",
                    "umask 000; exec \"$1\" --exact ops::jobs::tests::job_add_creates_private_unit_and_timer_files --nocapture",
                    "job-permissions-test",
                ])
                .arg(std::env::current_exe().unwrap())
                .env("OPSBOX_TEST_JOB_UMASK", "1")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let id = add_with(
            "Private backup",
            "0 2 * * *",
            "echo secret",
            dir.path(),
            || Ok("/usr/bin/true".into()),
        )
        .unwrap();
        let mut modes = Vec::new();
        for suffix in ["service", "timer"] {
            let path = dir.path().join(format!("{PREFIX}{id}.{suffix}"));
            modes.push(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777);
            let text = std::fs::read_to_string(path).unwrap();
            assert!(text.contains("Private backup"));
            if suffix == "service" {
                assert!(text.contains("ExecStart=/bin/sh -c \"echo secret\""));
            } else {
                assert!(text.contains("OnCalendar=*-*-* 02:00:00"));
            }
        }
        assert_eq!(modes, [0o600, 0o600]);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn root_helper_service_uses_private_umask() {
        assert!(
            include_str!("../../../../packaging/systemd/opsbox-priv.service")
                .lines()
                .any(|line| line == "UMask=0077")
        );
    }

    #[test]
    fn quoting_neutralises_systemd_and_shell_specials() {
        assert_eq!(quote_arg("echo hi"), "\"echo hi\"");
        assert_eq!(quote_arg("echo \"a\" \\n"), "\"echo \\\"a\\\" \\\\n\"");
        assert_eq!(quote_arg("echo $HOME 100%"), "\"echo $$HOME 100%%\"");
        assert_eq!(quote_arg(""), "\"\"");
    }

    #[test]
    fn the_unit_files_round_trip_and_hold_the_converted_schedule() {
        let svc = service_unit(
            "Back up the DB",
            "0 2 * * *",
            "pg_dump app | gzip > /var/backups/app-$(date +%F).sql.gz && echo \"100%\"",
        );
        assert!(svc.contains("ExecStart=/bin/sh -c \"pg_dump app | gzip > /var/backups/app-$$(date +%%F).sql.gz && echo \\\"100%%\\\"\"\n"), "{svc}");
        assert_eq!(
            parse_service_unit(&svc),
            Some((
                "Back up the DB".into(),
                "0 2 * * *".into(),
                "pg_dump app | gzip > /var/backups/app-$(date +%F).sql.gz && echo \"100%\"".into()
            ))
        );
        let timer = timer_unit("Back up the DB", &to_on_calendar("0 2 * * *").unwrap());
        assert!(
            timer.contains("OnCalendar=*-*-* 02:00:00\n")
                && timer.contains("Persistent=true")
                && timer.contains("WantedBy=timers.target")
        );
    }

    #[test]
    fn nothing_the_user_types_can_add_a_line_to_a_unit_file() {
        for hostile in ["a\nExecStart=/bin/evil", "a\r\nUser=root", "a\0b"] {
            assert!(check_command(hostile).is_err(), "{hostile:?}");
            assert!(check_name(hostile).is_err(), "{hostile:?}");
        }
        assert!(check_command(&"x".repeat(1001)).is_err());
        assert!(check_name(&"x".repeat(101)).is_err());
        assert!(check_name("   ").unwrap_err().contains("Describe"));
        assert_eq!(check_name("  Nightly backup  ").unwrap(), "Nightly backup");
        // A description with `%` cannot trigger a specifier either.
        assert!(
            service_unit("100% sure", "0 2 * * *", "true")
                .contains("Description=opsbox job: 100% sure")
        );
    }

    #[test]
    fn bad_schedules_are_refused_with_the_converters_sentence() {
        let e = add("x", "0 2 * *", "true").unwrap_err();
        assert_eq!(e.code, PrivErrorCode::Invalid);
        assert!(e.message.contains("5 fields"), "{}", e.message);
        assert!(
            add("x", "0 2 * * *", "")
                .unwrap_err()
                .message
                .contains("command")
        );
    }

    #[test]
    fn ids_are_eight_lower_case_hex_digits() {
        assert!(valid_id("0a1b2c3d"));
        for bad in [
            "",
            "0A1B2C3D",
            "0a1b2c3",
            "0a1b2c3d4",
            "../../etc",
            "0a1b2c3g",
        ] {
            assert!(!valid_id(bad), "{bad}");
        }
        assert_eq!(remove("../../x").unwrap_err().code, PrivErrorCode::Invalid);
        assert_eq!(run_now("nope").unwrap_err().code, PrivErrorCode::Invalid);
    }
}
