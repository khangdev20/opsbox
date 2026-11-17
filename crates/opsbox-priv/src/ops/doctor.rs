//! Read-only, bounded diagnostics. No automatic fixes and no user-supplied commands.

use std::path::Path;
use std::time::Duration;

use CheckStatus::{Fail, Ok as Good, Skip, Warn};
use opsbox_proto::{CheckStatus, DoctorCheck, DoctorReport};

use super::exec::{find, run};

fn check(
    id: &str,
    status: CheckStatus,
    message: impl Into<String>,
    fix: Option<&str>,
) -> DoctorCheck {
    DoctorCheck {
        id: id.into(),
        status,
        message: message.into(),
        fix: fix.map(str::to_owned),
    }
}

pub(super) fn disk_check(path: &str, total: u64, available: u64) -> DoctorCheck {
    let used = if total == 0 {
        100.0
    } else {
        100.0 * (1.0 - available as f64 / total as f64)
    };
    let status = if total == 0 || available < 128 * 1024 * 1024 {
        Fail
    } else if used > 90.0 || available < 512 * 1024 * 1024 {
        Warn
    } else {
        Good
    };
    check(
        &format!("disk:{path}"),
        status,
        format!(
            "{path}: {:.1} GB available ({used:.1}% used)",
            available as f64 / (1u64 << 30) as f64
        ),
        Some("Review the Cleanup page or enlarge the server's disk."),
    )
}

pub(super) fn collect_with(master_key: &Path, dc: &opsbox_docker::DockerClient) -> DoctorReport {
    let mut checks = Vec::new();
    let systemctl = find(&["/usr/bin/systemctl", "/bin/systemctl"]);
    if let Some(ctl) = systemctl.filter(|_| Path::new("/run/systemd/system").is_dir()) {
        for unit in [
            "opsbox.service",
            "opsbox-priv.service",
            "opsbox-proxy.service",
            "opsbox-watchdog.service",
        ] {
            let active =
                run(&ctl, &["is-active", unit], Duration::from_secs(3)).is_some_and(|o| o.ok());
            checks.push(check(
                unit,
                if active { Good } else { Fail },
                if active {
                    "Service is running"
                } else {
                    "Service is not running"
                },
                Some(&format!(
                    "sudo systemctl status {unit}; sudo journalctl -u {unit} -n 50"
                )),
            ));
        }
    } else {
        checks.push(check(
            "systemd",
            Warn,
            "No running systemd instance was detected",
            Some("Install opsbox on a systemd Linux host."),
        ));
    }
    for path in ["/", "/var/lib/opsbox"] {
        match nix::sys::statvfs::statvfs(path) {
            #[allow(clippy::useless_conversion)]
            Ok(s) => checks.push(disk_check(
                path,
                u64::from(s.blocks()).saturating_mul(u64::from(s.fragment_size())),
                u64::from(s.blocks_available()).saturating_mul(u64::from(s.fragment_size())),
            )),
            Err(_) => checks.push(check(
                &format!("disk:{path}"),
                Warn,
                "Cannot read disk space at this path",
                Some("Check that install.sh has created the data directory."),
            )),
        }
    }
    let synced = find(&["/usr/bin/timedatectl", "/bin/timedatectl"])
        .and_then(|ctl| {
            run(
                &ctl,
                &["show", "-p", "NTPSynchronized", "--value"],
                Duration::from_secs(5),
            )
        })
        .filter(|o| o.ok())
        .map(|o| o.stdout.trim() == "yes");
    checks.push(check(
        "clock",
        if synced == Some(true) { Good } else { Warn },
        if synced == Some(true) {
            "System clock is synchronized"
        } else {
            "Clock synchronization could not be confirmed; TOTP depends on correct time"
        },
        Some("timedatectl status; sudo timedatectl set-ntp true"),
    ));
    let dns = find(&["/usr/bin/getent", "/bin/getent"])
        .and_then(|ctl| run(&ctl, &["ahosts", "github.com"], Duration::from_secs(5)))
        .is_some_and(|o| o.ok() && !o.stdout.trim().is_empty());
    checks.push(check(
        "dns",
        if dns { Good } else { Warn },
        if dns {
            "System resolver can resolve github.com"
        } else {
            "System resolver could not resolve github.com within five seconds"
        },
        Some("Inspect resolvectl status and /etc/resolv.conf; check outbound DNS access."),
    ));
    let docker = super::docker::status_with(dc);
    checks.push(if !docker.installed {
        check("docker", Skip, "Docker is not installed (optional)", None)
    } else if docker.running {
        let detail = docker
            .version
            .as_deref()
            .map(|v| format!("Docker daemon answers ({v})"))
            .unwrap_or_else(|| "Docker daemon answers".to_owned());
        check("docker", Good, detail, None)
    } else {
        check(
            "docker",
            Warn,
            "Docker is installed but the daemon did not answer",
            Some("sudo systemctl status docker"),
        )
    });
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let private = std::fs::symlink_metadata(master_key).is_ok_and(|m| {
        m.is_file()
            && m.len() == 32
            && m.permissions().mode() & 0o077 == 0
            && m.uid() == nix::unistd::geteuid().as_raw()
    });
    checks.push(check("master-key", if private { Good } else { Warn }, if private { "Secret encryption key has private ownership and permissions" } else { "Secret encryption key is missing or has unexpected ownership, mode or size" }, Some("Check the configured master_key path. After first enrolment it should be 32 bytes, root:root 0600. Do not replace an existing key.")));
    DoctorReport { checks }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn low_disk_space_is_reported_before_it_breaks_the_database() {
        assert_eq!(disk_check("/", 10 << 30, 8 << 30).status, Good);
        assert_eq!(disk_check("/", 10 << 30, 256 << 20).status, Warn);
        assert_eq!(disk_check("/", 10 << 30, 64 << 20).status, Fail);
        assert_eq!(disk_check("/", 0, 0).status, Fail);
    }
}
