//! `SystemFacts`: the cheap checks behind the "needs attention" list.
//!
//! Replaced by proper package and systemd operations in M4; until then these use the
//! standard tools with absolute paths, no shell, a clean environment and a timeout.

use std::path::Path;
use std::time::Duration;
#[cfg(test)]
use std::time::Instant;

use opsbox_docker::DockerClient;
use opsbox_proto::SystemFacts;

pub(super) fn collect_with(dc: &DockerClient) -> SystemFacts {
    // Reuse the real apt/dnf implementation instead of depending on Ubuntu's
    // optional update-notifier/apt-check package being installed.
    let packages = super::pkg::list().ok();
    let counts = packages
        .as_ref()
        .map(|p| {
            (
                u32::try_from(p.updates.len()).unwrap_or(u32::MAX),
                u32::try_from(p.updates.iter().filter(|u| u.security).count()).unwrap_or(u32::MAX),
            )
        })
        .or_else(apt_updates);
    let (updates_total, updates_security) = counts.unzip();
    SystemFacts {
        reboot_required: ["/run/reboot-required", "/var/run/reboot-required"]
            .iter()
            .any(|p| Path::new(p).exists())
            || packages.is_some_and(|p| p.reboot_required),
        failed_units: failed_units(),
        updates_total,
        updates_security,
        restarting_containers: super::docker::restarting_containers_with(dc),
    }
}

/// Run `program args...` and return its combined output, or `None` on any failure.
fn run(program: &str, args: &[&str], limit: Duration) -> Option<String> {
    super::exec::run(program, args, limit)
        .filter(|o| o.ok())
        .map(|o| o.combined())
}

pub fn parse_failed_units(output: &str) -> Vec<String> {
    output
        .lines()
        .filter_map(|l| {
            // "  foo.service  loaded failed failed Foo daemon" (a leading "●" on some versions)
            let name = l.trim().trim_start_matches('●').split_whitespace().next()?;
            (name.contains('.') && !name.starts_with('.')).then(|| name.to_owned())
        })
        .collect()
}

fn failed_units() -> Vec<String> {
    let systemctl = ["/usr/bin/systemctl", "/bin/systemctl"]
        .into_iter()
        .find(|p| Path::new(p).exists());
    systemctl
        .and_then(|bin| {
            run(
                bin,
                &[
                    "list-units",
                    "--state=failed",
                    "--no-legend",
                    "--plain",
                    "--no-pager",
                ],
                Duration::from_secs(5),
            )
        })
        .map(|o| parse_failed_units(&o))
        .unwrap_or_default()
}

/// Ubuntu and Debian derivatives ship `apt-check`, which prints "updates;security".
pub fn parse_apt_check(output: &str) -> Option<(u32, u32)> {
    let line = output.lines().map(str::trim).find(|l| l.contains(';'))?;
    let (all, sec) = line.split_once(';')?;
    Some((all.parse().ok()?, sec.parse().ok()?))
}

fn apt_updates() -> Option<(u32, u32)> {
    run(
        "/usr/lib/update-notifier/apt-check",
        &[],
        Duration::from_secs(30),
    )
    .and_then(|o| parse_apt_check(&o))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_units_are_read_from_systemctl_output() {
        let out = "  snapd.service            loaded failed failed Snap Daemon\n● foo.timer loaded failed failed Foo\n\n";
        assert_eq!(parse_failed_units(out), ["snapd.service", "foo.timer"]);
        assert!(parse_failed_units("").is_empty());
        assert!(parse_failed_units("0 loaded units listed.\n").is_empty());
    }

    #[test]
    fn apt_check_output() {
        assert_eq!(parse_apt_check("14;3\n"), Some((14, 3)));
        assert_eq!(parse_apt_check("Reading...\n0;0"), Some((0, 0)));
        assert_eq!(parse_apt_check("garbage"), None);
        assert_eq!(parse_apt_check("a;b"), None);
    }

    #[test]
    fn commands_that_do_not_exist_or_hang_are_not_fatal() {
        assert_eq!(run("/nonexistent/tool", &[], Duration::from_secs(1)), None);
        #[cfg(unix)]
        {
            let t = Instant::now();
            assert_eq!(run("/bin/sleep", &["5"], Duration::from_millis(200)), None);
            assert!(
                t.elapsed() < Duration::from_secs(2),
                "the hung command was killed"
            );
        }
    }
}
