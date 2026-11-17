//! Package updates for apt and dnf (docs/BACKEND_SPEC.md 7.8).
//!
//! The caller never names packages: `priv` works out what is pending itself, so a
//! request cannot ask for an arbitrary install. Output of the package tools is parsed
//! by pure functions that are tested against real-looking samples.

use std::path::Path;
use std::time::Duration;

use opsbox_proto::{CommandOutcome, PkgSelect, PkgState, PkgUpdate, PrivError, PrivErrorCode};

use super::exec::{find, run, run_with, tail};

fn err(code: PrivErrorCode, msg: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: msg.into(),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Manager {
    Apt,
    Dnf,
}

fn manager() -> Option<Manager> {
    if find(&["/usr/bin/apt-get"]).is_some() {
        Some(Manager::Apt)
    } else if find(&["/usr/bin/dnf"]).is_some() {
        Some(Manager::Dnf)
    } else {
        None
    }
}

fn is_kernel(name: &str) -> bool {
    name.starts_with("linux-image")
        || name.starts_with("linux-generic")
        || name == "kernel"
        || name.starts_with("kernel-core")
}

/// Package names as the tools print them; anything else is never passed on.
pub fn valid_package_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'.' | b'-' | b':' | b'_'))
}

// ------------------------------------------------------------------ apt

/// `apt list --upgradable`:
/// `openssl/noble-updates,noble-security 3.0.13-0ubuntu3.5 amd64 [upgradable from: 3.0.13-0ubuntu3.4]`
pub fn parse_apt_upgradable(text: &str) -> Vec<PkgUpdate> {
    text.lines()
        .filter_map(|line| {
            let (name_suites, rest) = line.split_once(' ')?;
            let (name, suites) = name_suites.split_once('/')?;
            let available = rest.split_whitespace().next()?;
            let current = rest
                .split("upgradable from:")
                .nth(1)
                .map(|c| c.trim().trim_end_matches(']').to_owned())
                .unwrap_or_default();
            if !valid_package_name(name) {
                return None;
            }
            // Ubuntu: noble-security; Debian: bookworm-security; also "stable-security".
            let security = suites
                .split(',')
                .any(|s| s.ends_with("-security") || s == "security");
            Some(PkgUpdate {
                name: name.to_owned(),
                current,
                available: available.to_owned(),
                security,
                kernel: is_kernel(name),
            })
        })
        .collect()
}

// ------------------------------------------------------------------ dnf

/// `dnf check-update -q`: `openssl.x86_64   1:3.0.7-27.el9   baseos`; a blank line may
/// separate the obsoletes section, which is ignored.
pub fn parse_dnf_check_update(text: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            break; // the "Obsoleting Packages" block follows the first blank line
        }
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 3 {
            continue;
        }
        let Some((name, arch)) = f[0].rsplit_once('.') else {
            continue;
        };
        if valid_package_name(name) {
            out.push((name.to_owned(), arch.to_owned(), f[1].to_owned()));
        }
    }
    out
}

/// `dnf updateinfo list --security -q`: `RHSA-2024:1234 Important/Sec. openssl-3.0.7-27.el9.x86_64`.
/// Returns the NVRAs of packages that fix a security advisory.
pub fn parse_dnf_security(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.split_whitespace().nth(2).map(str::to_owned))
        .collect()
}

/// Build `name-version-release.arch` from check-update fields, dropping any epoch.
fn nvra(name: &str, arch: &str, evr: &str) -> String {
    let vr = evr.split_once(':').map_or(evr, |(_, v)| v);
    format!("{name}-{vr}.{arch}")
}

// ------------------------------------------------------------------ list

fn running_kernel() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

fn mtime_secs(path: &str) -> Option<i64> {
    let m = std::fs::metadata(path).ok()?.modified().ok()?;
    i64::try_from(m.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs()).ok()
}

fn list_updates(m: Manager) -> Result<Vec<PkgUpdate>, PrivError> {
    match m {
        Manager::Apt => {
            let o = run(
                "/usr/bin/apt",
                &["list", "--upgradable"],
                Duration::from_secs(60),
            )
            .ok_or_else(|| err(PrivErrorCode::Internal, "apt did not answer"))?;
            Ok(parse_apt_upgradable(&o.stdout))
        }
        Manager::Dnf => {
            // Exit status 100 means "updates are available"; 0 means none; anything else is an error.
            let o = run(
                "/usr/bin/dnf",
                &["-q", "check-update"],
                Duration::from_secs(120),
            )
            .ok_or_else(|| err(PrivErrorCode::Internal, "dnf did not answer"))?;
            if !matches!(o.code, Some(0 | 100)) {
                return Err(err(
                    PrivErrorCode::Internal,
                    format!("dnf check-update failed: {}", tail(&o.stderr, 2).join(" ")),
                ));
            }
            let pending = parse_dnf_check_update(&o.stdout);
            let sec: Vec<String> = run(
                "/usr/bin/dnf",
                &["-q", "updateinfo", "list", "--security"],
                Duration::from_secs(120),
            )
            .map(|o| parse_dnf_security(&o.stdout))
            .unwrap_or_default();
            Ok(pending
                .into_iter()
                .map(|(name, arch, evr)| {
                    let id = nvra(&name, &arch, &evr);
                    PkgUpdate {
                        security: sec.contains(&id),
                        kernel: is_kernel(&name),
                        current: String::new(),
                        available: evr,
                        name,
                    }
                })
                .collect())
        }
    }
}

pub(super) fn list() -> Result<PkgState, PrivError> {
    let Some(m) = manager() else {
        return Ok(PkgState {
            manager: String::new(),
            updates: vec![],
            reboot_required: reboot_required(),
            reboot_at: reboot_at(),
            auto_security: None,
            running_kernel: running_kernel(),
            index_time: None,
        });
    };
    let updates = list_updates(m)?;
    Ok(PkgState {
        manager: match m {
            Manager::Apt => "apt",
            Manager::Dnf => "dnf",
        }
        .into(),
        updates,
        reboot_required: reboot_required(),
        reboot_at: reboot_at(),
        auto_security: auto_security(m),
        running_kernel: running_kernel(),
        index_time: mtime_secs("/var/lib/apt/periodic/update-success-stamp")
            .or_else(|| mtime_secs("/var/lib/apt/lists")),
    })
}

/// `/run/systemd/shutdown/scheduled` holds `USEC=<microseconds since the epoch>` while a
/// delayed shutdown or reboot is waiting.
pub fn parse_scheduled(text: &str) -> Option<i64> {
    let usec: i64 = text
        .lines()
        .find_map(|l| l.strip_prefix("USEC="))?
        .trim()
        .parse()
        .ok()?;
    let mode = text
        .lines()
        .find_map(|l| l.strip_prefix("MODE="))
        .unwrap_or("");
    (mode == "reboot" || mode.is_empty()).then_some(usec / 1_000_000)
}

fn reboot_at() -> Option<i64> {
    parse_scheduled(&std::fs::read_to_string("/run/systemd/shutdown/scheduled").ok()?)
}

fn reboot_required() -> bool {
    ["/run/reboot-required", "/var/run/reboot-required"]
        .iter()
        .any(|p| Path::new(p).exists())
}

// ------------------------------------------------------------------ install

pub(super) fn install(which: PkgSelect) -> Result<CommandOutcome, PrivError> {
    install_inner(which, &[])
}

pub(super) fn install_selected(names: &[String]) -> Result<CommandOutcome, PrivError> {
    if names.is_empty() || names.len() > 200 || names.iter().any(|n| !valid_package_name(n)) {
        return Err(err(
            PrivErrorCode::Invalid,
            "Select 1 to 200 valid pending package names.",
        ));
    }
    install_inner(PkgSelect::All, names)
}

fn selected_pending(pending: &[PkgUpdate], names: &[String]) -> Result<(), PrivError> {
    if let Some(n) = names
        .iter()
        .find(|n| !pending.iter().any(|p| &p.name == *n))
    {
        return Err(err(
            PrivErrorCode::Invalid,
            format!("{n} is no longer a pending update. Refresh the list and choose again."),
        ));
    }
    Ok(())
}

fn install_inner(which: PkgSelect, selected: &[String]) -> Result<CommandOutcome, PrivError> {
    let m = manager().ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "No supported package manager (apt or dnf) was found on this machine.",
        )
    })?;
    let env = [
        ("DEBIAN_FRONTEND", "noninteractive"),
        ("NEEDRESTART_MODE", "a"),
    ];
    let limit = Duration::from_secs(30 * 60);
    match m {
        Manager::Apt => {
            // Refresh the index first so "pending" is what is really pending.
            let upd = run_with(
                "/usr/bin/apt-get",
                &["-q", "update"],
                Duration::from_secs(300),
                None,
                &env,
            );
            if !upd.as_ref().is_some_and(|o| o.ok()) {
                return Err(err(
                    PrivErrorCode::Internal,
                    "Could not refresh the package lists. Check the network and try again.",
                ));
            }
            let pending = list_updates(Manager::Apt)?;
            selected_pending(&pending, selected)?;
            let names: Vec<&str> = pending
                .iter()
                .filter(|u| which == PkgSelect::All || u.security)
                .filter(|u| selected.is_empty() || selected.contains(&u.name))
                .map(|u| u.name.as_str())
                .collect();
            if names.is_empty() {
                return Ok(CommandOutcome {
                    ok: true,
                    tail: vec!["Nothing to install.".into()],
                    count: 0,
                });
            }
            let mut args = vec![
                "-q",
                "-y",
                "-o",
                "Dpkg::Options::=--force-confold",
                "-o",
                "Dpkg::Options::=--force-confdef",
                "install",
                "--only-upgrade",
            ];
            args.extend(names.iter().copied());
            let o = run_with("/usr/bin/apt-get", &args, limit, None, &env).ok_or_else(|| {
                err(
                    PrivErrorCode::Internal,
                    "The update did not finish in time.",
                )
            })?;
            Ok(CommandOutcome {
                ok: o.ok(),
                tail: tail(&o.combined(), 20),
                count: u32::try_from(names.len()).unwrap_or(u32::MAX),
            })
        }
        Manager::Dnf => {
            let pending = list_updates(Manager::Dnf)?;
            selected_pending(&pending, selected)?;
            let n = pending
                .iter()
                .filter(|u| which == PkgSelect::All || u.security)
                .filter(|u| selected.is_empty() || selected.contains(&u.name))
                .count();
            if n == 0 {
                return Ok(CommandOutcome {
                    ok: true,
                    tail: vec!["Nothing to install.".into()],
                    count: 0,
                });
            }
            let mut args: Vec<&str> = if which == PkgSelect::Security {
                vec!["-q", "-y", "upgrade", "--security"]
            } else {
                vec!["-q", "-y", "upgrade"]
            };
            args.extend(selected.iter().map(String::as_str));
            let o = run_with("/usr/bin/dnf", &args, limit, None, &[]).ok_or_else(|| {
                err(
                    PrivErrorCode::Internal,
                    "The update did not finish in time.",
                )
            })?;
            Ok(CommandOutcome {
                ok: o.ok(),
                tail: tail(&o.combined(), 20),
                count: u32::try_from(n).unwrap_or(u32::MAX),
            })
        }
    }
}

// ------------------------------------------------------- automatic updates

const APT_AUTO: &str = "/etc/apt/apt.conf.d/20auto-upgrades";
const DNF_AUTO_CONF: &str = "/etc/dnf/automatic.conf";

pub fn apt_auto_conf(on: bool) -> String {
    let v = if on { "1" } else { "0" };
    format!(
        "APT::Periodic::Update-Package-Lists \"1\";\nAPT::Periodic::Unattended-Upgrade \"{v}\";\n"
    )
}

/// Is `APT::Periodic::Unattended-Upgrade` set to something other than 0?
pub fn parse_apt_auto(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim();
        l.starts_with("APT::Periodic::Unattended-Upgrade")
            && l.split('"').nth(1).is_some_and(|v| v != "0")
    })
}

fn auto_security(m: Manager) -> Option<bool> {
    match m {
        Manager::Apt => Some(std::fs::read_to_string(APT_AUTO).is_ok_and(|t| parse_apt_auto(&t))),
        Manager::Dnf => {
            let o = run(
                "/usr/bin/systemctl",
                &["is-enabled", "dnf-automatic-install.timer"],
                Duration::from_secs(10),
            )
            .or_else(|| {
                run(
                    "/bin/systemctl",
                    &["is-enabled", "dnf-automatic-install.timer"],
                    Duration::from_secs(10),
                )
            })?;
            Some(o.stdout.trim() == "enabled")
        }
    }
}

/// Rewrite `key = value` lines of an ini-style file, appending keys that are missing.
pub fn set_ini_keys(text: &str, section: &str, pairs: &[(&str, &str)]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut in_section = false;
    let mut seen = vec![false; pairs.len()];
    let flush_missing = |out: &mut Vec<String>, seen: &mut Vec<bool>| {
        for (i, (k, v)) in pairs.iter().enumerate() {
            if !seen[i] {
                out.push(format!("{k} = {v}"));
                seen[i] = true;
            }
        }
    };
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            if in_section {
                flush_missing(&mut out, &mut seen);
            }
            in_section = &t[1..t.len() - 1] == section;
            out.push(line.to_owned());
            continue;
        }
        if in_section
            && let Some((k, _)) = t.split_once('=')
            && let Some(i) = pairs.iter().position(|(pk, _)| *pk == k.trim())
        {
            out.push(format!("{} = {}", pairs[i].0, pairs[i].1));
            seen[i] = true;
            continue;
        }
        out.push(line.to_owned());
    }
    if in_section {
        flush_missing(&mut out, &mut seen);
    } else if seen.iter().any(|s| !s) {
        out.push(format!("[{section}]"));
        flush_missing(&mut out, &mut seen);
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

pub(super) fn set_auto(on: bool) -> Result<(), PrivError> {
    let m = manager().ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "No supported package manager (apt or dnf) was found on this machine.",
        )
    })?;
    let env = [("DEBIAN_FRONTEND", "noninteractive")];
    match m {
        Manager::Apt => {
            if on && !Path::new("/usr/bin/unattended-upgrade").exists() {
                let o = run_with(
                    "/usr/bin/apt-get",
                    &["-q", "-y", "install", "unattended-upgrades"],
                    Duration::from_secs(600),
                    None,
                    &env,
                );
                if !o.is_some_and(|o| o.ok()) {
                    return Err(err(
                        PrivErrorCode::Internal,
                        "Could not install unattended-upgrades. Check the network and try again.",
                    ));
                }
            }
            write_atomic(APT_AUTO, &apt_auto_conf(on))
        }
        Manager::Dnf => {
            let systemctl = find(&["/usr/bin/systemctl", "/bin/systemctl"]).ok_or_else(|| {
                err(
                    PrivErrorCode::Unsupported,
                    "systemd is required for automatic updates.",
                )
            })?;
            if on {
                let have = run(
                    "/usr/bin/rpm",
                    &["-q", "dnf-automatic"],
                    Duration::from_secs(20),
                )
                .is_some_and(|o| o.ok());
                if !have
                    && !run_with(
                        "/usr/bin/dnf",
                        &["-q", "-y", "install", "dnf-automatic"],
                        Duration::from_secs(600),
                        None,
                        &[],
                    )
                    .is_some_and(|o| o.ok())
                {
                    return Err(err(
                        PrivErrorCode::Internal,
                        "Could not install dnf-automatic. Check the network and try again.",
                    ));
                }
                let old = std::fs::read_to_string(DNF_AUTO_CONF).unwrap_or_default();
                write_atomic(
                    DNF_AUTO_CONF,
                    &set_ini_keys(
                        &old,
                        "commands",
                        &[("upgrade_type", "security"), ("apply_updates", "yes")],
                    ),
                )?;
                run(
                    &systemctl,
                    &["enable", "--now", "dnf-automatic.timer"],
                    Duration::from_secs(60),
                );
            } else {
                run(
                    &systemctl,
                    &["disable", "--now", "dnf-automatic.timer"],
                    Duration::from_secs(60),
                );
            }
            Ok(())
        }
    }
}

/// Write `path` through a temporary file in the same directory and rename it over.
pub(super) fn write_atomic(path: &str, content: &str) -> Result<(), PrivError> {
    let tmp = format!("{path}.opsbox-tmp");
    let io = |e: std::io::Error| err(PrivErrorCode::Internal, format!("cannot write {path}: {e}"));
    std::fs::write(&tmp, content).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_packages_must_be_valid_and_currently_pending() {
        for names in [
            vec![],
            vec!["--reinstall".into()],
            vec!["a;b".into()],
            vec!["../bad".into()],
        ] {
            assert_eq!(
                install_selected(&names).unwrap_err().code,
                PrivErrorCode::Invalid
            );
        }
        let pending = parse_apt_upgradable("curl/noble-security 2 arm64 [upgradable from: 1]\n");
        assert!(selected_pending(&pending, &["curl".into()]).is_ok());
        assert!(selected_pending(&pending, &["not-pending".into()]).is_err());
    }

    const APT: &str = "Listing...\nopenssl/noble-updates,noble-security 3.0.13-0ubuntu3.5 amd64 [upgradable from: 3.0.13-0ubuntu3.4]\nlibc6/noble-updates,noble-security 2.39-0ubuntu8.3 amd64 [upgradable from: 2.39-0ubuntu8.2]\nlinux-image-generic/noble-updates,noble-security 6.8.0-52.53 amd64 [upgradable from: 6.8.0-48.48]\ncurl/noble-updates 8.5.0-2ubuntu10.5 amd64 [upgradable from: 8.5.0-2ubuntu10.4]\nbash/bookworm-security 5.2.15-2+deb12u2 amd64 [upgradable from: 5.2.15-2+deb12u1]\n";

    #[test]
    fn apt_upgradable_with_security_and_kernel_flags() {
        let u = parse_apt_upgradable(APT);
        assert_eq!(u.len(), 5);
        let by = |n: &str| u.iter().find(|p| p.name == n).unwrap();
        assert_eq!(
            (
                by("openssl").current.as_str(),
                by("openssl").available.as_str(),
                by("openssl").security
            ),
            ("3.0.13-0ubuntu3.4", "3.0.13-0ubuntu3.5", true)
        );
        assert!(!by("curl").security, "only the -updates pocket");
        assert!(by("bash").security, "Debian's bookworm-security");
        assert!(by("linux-image-generic").kernel && by("linux-image-generic").security);
        assert!(!by("libc6").kernel);
        assert_eq!(by("bash").available, "5.2.15-2+deb12u2");
    }

    #[test]
    fn apt_output_noise_is_ignored() {
        assert!(
            parse_apt_upgradable(
                "Listing... Done\n\nWARNING: apt does not have a stable CLI interface.\n"
            )
            .is_empty()
        );
        assert!(
            parse_apt_upgradable("evil;rm/noble 1 amd64\n").is_empty(),
            "odd names are dropped"
        );
    }

    #[test]
    fn dnf_check_update_and_security_matching() {
        let check = "openssl.x86_64                1:3.0.7-27.el9      baseos\ncurl.x86_64    7.76.1-29.el9_4    baseos\nkernel-core.x86_64  5.14.0-427.el9  baseos\n\nObsoleting Packages\nold.x86_64 1.0 repo\n";
        let pending = parse_dnf_check_update(check);
        assert_eq!(
            pending.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(),
            ["openssl", "curl", "kernel-core"]
        );
        let sec = parse_dnf_security(
            "RHSA-2024:1234 Important/Sec. openssl-3.0.7-27.el9.x86_64\nRHSA-2024:5678 Moderate/Sec. kernel-core-5.14.0-427.el9.x86_64\n",
        );
        assert_eq!(sec.len(), 2);
        let (n, a, e) = &pending[0];
        assert!(
            sec.contains(&nvra(n, a, e)),
            "epoch is dropped when matching: {}",
            nvra(n, a, e)
        );
        let (n, a, e) = &pending[1];
        assert!(!sec.contains(&nvra(n, a, e)));
    }

    #[test]
    fn package_names_are_checked_before_use() {
        for ok in [
            "openssl",
            "libc6",
            "g++-13",
            "python3.12",
            "linux-image-6.8.0-52-generic",
            "libstdc++6:amd64",
        ] {
            assert!(valid_package_name(ok), "{ok}");
        }
        for bad in [
            "",
            "-rf",
            "a b",
            "a;b",
            "a$(x)",
            "../x",
            &"a".repeat(129),
            "a\nb",
        ] {
            assert!(!valid_package_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_scheduled_reboot_is_read_from_systemd() {
        assert_eq!(
            parse_scheduled("USEC=1700000060000000\nWARN_WALL=1\nMODE=reboot\nWALL_MESSAGE=x\n"),
            Some(1_700_000_060)
        );
        assert_eq!(
            parse_scheduled("USEC=1700000060000000\nMODE=poweroff\n"),
            None,
            "a power-off is not our reboot"
        );
        assert_eq!(parse_scheduled("garbage"), None);
        assert_eq!(parse_scheduled("USEC=abc\n"), None);
    }

    #[test]
    fn apt_auto_upgrade_file() {
        assert!(parse_apt_auto(&apt_auto_conf(true)));
        assert!(!parse_apt_auto(&apt_auto_conf(false)));
        assert!(!parse_apt_auto(""));
        assert!(parse_apt_auto(
            "// comment\nAPT::Periodic::Unattended-Upgrade \"1\";\n"
        ));
        assert!(apt_auto_conf(true).contains("Update-Package-Lists \"1\""));
    }

    #[test]
    fn ini_edit_replaces_adds_and_leaves_the_rest() {
        let src = "[base]\nfoo = 1\n\n[commands]\n# comment\nupgrade_type = default\napply_updates = no\nrandom_sleep = 360\n\n[email]\nto = root\n";
        let out = set_ini_keys(
            src,
            "commands",
            &[("upgrade_type", "security"), ("apply_updates", "yes")],
        );
        assert!(out.contains("upgrade_type = security") && out.contains("apply_updates = yes"));
        assert!(
            out.contains("random_sleep = 360")
                && out.contains("[email]\nto = root")
                && out.contains("[base]\nfoo = 1")
        );
        assert!(!out.contains("= default") && !out.contains("= no"));
        // Missing keys are appended to the section; a missing section is created.
        let added = set_ini_keys(
            "[commands]\nupgrade_type = default\n[email]\nto = x\n",
            "commands",
            &[("upgrade_type", "security"), ("apply_updates", "yes")],
        );
        assert!(added.contains("apply_updates = yes\n[email]"), "{added}");
        let fresh = set_ini_keys(
            "[email]\nto = x\n",
            "commands",
            &[("upgrade_type", "security")],
        );
        assert!(
            fresh.ends_with("[commands]\nupgrade_type = security\n"),
            "{fresh}"
        );
    }
}
