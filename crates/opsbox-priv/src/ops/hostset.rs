//! Reboot, swap, hostname and time zone (docs/BACKEND_SPEC.md 7.8).

use std::path::Path;
use std::time::Duration;

use opsbox_proto::{HostSettings, PrivError, PrivErrorCode};

use super::exec::{find, run, tail};

fn err(code: PrivErrorCode, msg: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: msg.into(),
    }
}

fn needed(candidates: &[&str], what: &str) -> Result<String, PrivError> {
    find(candidates).ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            format!("{what} is not available on this machine."),
        )
    })
}

fn ok_or(o: Option<super::exec::Output>, what: &str) -> Result<(), PrivError> {
    match o {
        Some(o) if o.ok() => Ok(()),
        Some(o) => Err(err(
            PrivErrorCode::Internal,
            format!("{what} failed: {}", tail(&o.combined(), 2).join(" ")),
        )),
        None => Err(err(
            PrivErrorCode::Internal,
            format!("{what} did not finish in time"),
        )),
    }
}

// ------------------------------------------------------------------ reboot

pub(super) fn reboot(delay_min: u32) -> Result<(), PrivError> {
    if !(1..=60).contains(&delay_min) {
        return Err(err(
            PrivErrorCode::Invalid,
            "The reboot delay must be between 1 and 60 minutes.",
        ));
    }
    let shutdown = needed(
        &["/usr/sbin/shutdown", "/sbin/shutdown", "/usr/bin/shutdown"],
        "shutdown",
    )?;
    let when = format!("+{delay_min}");
    ok_or(
        run(
            &shutdown,
            &["-r", &when, "opsbox: reboot requested from the panel"],
            Duration::from_secs(20),
        ),
        "Scheduling the reboot",
    )
}

pub(super) fn reboot_cancel() -> Result<(), PrivError> {
    let shutdown = needed(
        &["/usr/sbin/shutdown", "/sbin/shutdown", "/usr/bin/shutdown"],
        "shutdown",
    )?;
    ok_or(
        run(&shutdown, &["-c"], Duration::from_secs(20)),
        "Cancelling the reboot",
    )
}

// ---------------------------------------------------------------- hostname

/// RFC 1123 host name: dot-separated labels of letters, digits and hyphens.
pub fn check_hostname(name: &str) -> Result<(), String> {
    let bad = "A host name uses lower-case letters, digits and hyphens (dots separate parts), up to 63 characters per part and 253 in all.";
    if name.is_empty() || name.len() > 253 {
        return Err(bad.into());
    }
    for label in name.split('.') {
        let ok = !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !ok {
            return Err(bad.into());
        }
    }
    Ok(())
}

pub(super) fn set_hostname(name: &str) -> Result<(), PrivError> {
    check_hostname(name).map_err(|m| err(PrivErrorCode::Invalid, m))?;
    let ctl = needed(&["/usr/bin/hostnamectl", "/bin/hostnamectl"], "hostnamectl")?;
    ok_or(
        run(&ctl, &["set-hostname", name], Duration::from_secs(20)),
        "Changing the host name",
    )
}

// ---------------------------------------------------------------- time zone

/// `Region/City`, `UTC`, `Etc/GMT+3`: no `..`, no leading slash, only name characters.
pub fn check_timezone(zone: &str) -> Result<(), String> {
    let ok = !zone.is_empty()
        && zone.len() <= 64
        && !zone.starts_with('/')
        && !zone.contains("..")
        && zone
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'+' | b'-'));
    if ok {
        Ok(())
    } else {
        Err("That is not a time zone name. Use a name like Europe/Berlin or UTC.".into())
    }
}

pub(super) fn set_timezone(zone: &str) -> Result<(), PrivError> {
    check_timezone(zone).map_err(|m| err(PrivErrorCode::Invalid, m))?;
    if !Path::new("/usr/share/zoneinfo").join(zone).is_file() {
        return Err(err(
            PrivErrorCode::Invalid,
            "This machine does not know that time zone. Try a name like Europe/Berlin or UTC.",
        ));
    }
    let ctl = needed(&["/usr/bin/timedatectl", "/bin/timedatectl"], "timedatectl")?;
    ok_or(
        run(&ctl, &["set-timezone", zone], Duration::from_secs(20)),
        "Changing the time zone",
    )
}

// -------------------------------------------------------------------- swap

const SWAPFILE: &str = "/swapfile";
const MIN_SWAP_MB: u64 = 128;
const FSTAB_LINE: &str = "/swapfile none swap sw 0 0";

/// Add the swap entry to fstab text if it is missing, or remove it.
pub fn fstab_with_swap(fstab: &str, present: bool) -> String {
    let is_ours = |l: &str| {
        let f: Vec<&str> = l.split_whitespace().collect();
        f.len() >= 3 && f[0] == SWAPFILE && f[2] == "swap" && !l.trim_start().starts_with('#')
    };
    let mut lines: Vec<&str> = fstab.lines().filter(|l| !is_ours(l)).collect();
    if present {
        lines.push(FSTAB_LINE);
    }
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

/// Is a swap size acceptable given the free space? Never uses more than half of it.
pub fn check_swap_size(size_mb: u64, free_bytes: u64) -> Result<(), String> {
    if size_mb == 0 {
        return Ok(());
    }
    if size_mb < MIN_SWAP_MB {
        return Err(format!(
            "Swap smaller than {MIN_SWAP_MB} MB is not useful. Use 0 to turn swap off."
        ));
    }
    if size_mb.saturating_mul(1 << 20) > free_bytes / 2 {
        return Err(format!(
            "That would use more than half of the free disk space ({} MB free). Choose a smaller size.",
            free_bytes >> 20
        ));
    }
    Ok(())
}

fn free_bytes(path: &str) -> u64 {
    nix::sys::statvfs::statvfs(path).map_or(0, |s| {
        #[allow(clippy::useless_conversion)]
        {
            u64::from(s.blocks_available()) * u64::from(s.fragment_size())
        }
    })
}

pub(super) fn set_swap(size_mb: u64) -> Result<(), PrivError> {
    let existing = Path::new(SWAPFILE).exists();
    // The file's own space comes back when it is replaced.
    let free = free_bytes("/")
        + if existing {
            std::fs::metadata(SWAPFILE).map_or(0, |m| m.len())
        } else {
            0
        };
    check_swap_size(size_mb, free).map_err(|m| err(PrivErrorCode::Invalid, m))?;
    let swapoff = needed(
        &["/usr/sbin/swapoff", "/sbin/swapoff", "/usr/bin/swapoff"],
        "swapoff",
    )?;
    let t = Duration::from_secs(300);
    if existing {
        ok_or(run(&swapoff, &[SWAPFILE], t), "Turning the old swap off")?;
        std::fs::remove_file(SWAPFILE).map_err(|e| {
            err(
                PrivErrorCode::Internal,
                format!("cannot remove the old swap file: {e}"),
            )
        })?;
    }
    let fstab = std::fs::read_to_string("/etc/fstab").unwrap_or_default();
    if size_mb == 0 {
        super::pkg::write_atomic("/etc/fstab", &fstab_with_swap(&fstab, false))?;
        return Ok(());
    }
    let size = format!("{size_mb}M");
    let fallocate = find(&["/usr/bin/fallocate", "/bin/fallocate"]);
    let made =
        fallocate.is_some_and(|f| run(&f, &["-l", &size, SWAPFILE], t).is_some_and(|o| o.ok()));
    if !made {
        // Filesystems without fallocate support: write the zeroes.
        let dd = needed(&["/usr/bin/dd", "/bin/dd"], "dd")?;
        ok_or(
            run(
                &dd,
                &[
                    "if=/dev/zero",
                    &format!("of={SWAPFILE}"),
                    "bs=1M",
                    &format!("count={size_mb}"),
                ],
                Duration::from_secs(900),
            ),
            "Creating the swap file",
        )?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(SWAPFILE, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| err(PrivErrorCode::Internal, e.to_string()))?;
    }
    let mkswap = needed(
        &["/usr/sbin/mkswap", "/sbin/mkswap", "/usr/bin/mkswap"],
        "mkswap",
    )?;
    let swapon = needed(
        &["/usr/sbin/swapon", "/sbin/swapon", "/usr/bin/swapon"],
        "swapon",
    )?;
    let cleanup = |e: PrivError| {
        let _ = std::fs::remove_file(SWAPFILE);
        e
    };
    ok_or(run(&mkswap, &[SWAPFILE], t), "Preparing the swap file").map_err(cleanup)?;
    ok_or(run(&swapon, &[SWAPFILE], t), "Turning swap on").map_err(cleanup)?;
    super::pkg::write_atomic("/etc/fstab", &fstab_with_swap(&fstab, true))
}

// ------------------------------------------------------------------ reading settings

/// `SwapTotal` from `/proc/meminfo`, in megabytes.
pub fn parse_swap_mb(meminfo: &str) -> u64 {
    meminfo
        .lines()
        .find_map(|l| l.strip_prefix("SwapTotal:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|kb| kb.parse::<u64>().ok())
        .map_or(0, |kb| kb / 1024)
}

/// The zone name from the `/etc/localtime` link target (`.../zoneinfo/Europe/Paris`).
pub fn zone_from_link(target: &str) -> Option<String> {
    target
        .split_once("zoneinfo/")
        .map(|(_, z)| z.trim().to_owned())
        .filter(|z| !z.is_empty())
}

pub(crate) fn settings() -> HostSettings {
    let timedatectl = find(&["/usr/bin/timedatectl", "/bin/timedatectl"]);
    let timezone = timedatectl
        .as_deref()
        .and_then(|t| {
            run(
                t,
                &["show", "-p", "Timezone", "--value"],
                Duration::from_secs(5),
            )
        })
        .filter(super::exec::Output::ok)
        .map(|o| o.stdout.trim().to_owned())
        .filter(|z| !z.is_empty())
        .or_else(|| {
            std::fs::read_link("/etc/localtime")
                .ok()
                .and_then(|p| zone_from_link(&p.to_string_lossy()))
        })
        .unwrap_or_else(|| "UTC".into());
    let timezones = timedatectl
        .as_deref()
        .and_then(|t| run(t, &["list-timezones"], Duration::from_secs(5)))
        .filter(super::exec::Output::ok)
        .map(|o| {
            o.stdout
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let swap_mb = std::fs::read_to_string("/proc/meminfo")
        .map(|t| parse_swap_mb(&t))
        .unwrap_or(0);
    HostSettings {
        timezone,
        timezones,
        swap_mb,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_names() {
        for ok in ["srv-sgn-01", "a", "web.example.com", "x1"] {
            assert!(check_hostname(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "Srv",
            "-a",
            "a-",
            "a..b",
            ".a",
            "a b",
            "a_b",
            "a;b",
            &"a".repeat(64),
            &format!("{}.com", "a.".repeat(130)),
        ] {
            assert!(check_hostname(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn time_zone_names() {
        for ok in [
            "UTC",
            "Europe/Berlin",
            "America/Argentina/Buenos_Aires",
            "Etc/GMT+3",
        ] {
            assert!(check_timezone(ok).is_ok(), "{ok}");
        }
        for bad in [
            "",
            "/etc/passwd",
            "../../etc/shadow",
            "Europe/../x",
            "a b",
            "a;b",
            "UTC\n",
            &"a".repeat(65),
        ] {
            assert!(check_timezone(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            set_timezone("Not/AZone").unwrap_err().code,
            PrivErrorCode::Invalid
        );
    }

    #[test]
    fn fstab_edits_are_exact_and_idempotent() {
        let base = "UUID=abc / ext4 defaults 0 1\n# /swapfile none swap sw 0 0\n";
        let on = fstab_with_swap(base, true);
        assert!(on.ends_with("/swapfile none swap sw 0 0\n") && on.contains("UUID=abc / ext4"));
        assert_eq!(
            fstab_with_swap(&on, true),
            on,
            "adding twice changes nothing"
        );
        let off = fstab_with_swap(&on, false);
        assert!(
            !off.lines().any(|l| l == FSTAB_LINE) && off.contains("# /swapfile none swap sw 0 0"),
            "comments are left alone: {off}"
        );
        assert_eq!(fstab_with_swap("", false), "\n");
    }

    #[test]
    fn swap_sizes_respect_the_free_space() {
        assert!(check_swap_size(0, 0).is_ok(), "0 means off");
        assert!(
            check_swap_size(64, 1 << 40)
                .unwrap_err()
                .contains("not useful")
        );
        assert!(check_swap_size(2048, 10 << 30).is_ok());
        assert!(
            check_swap_size(6000, 10 << 30)
                .unwrap_err()
                .contains("half of the free disk space")
        );
        assert!(check_swap_size(u64::MAX, 10 << 30).is_err(), "no overflow");
    }

    #[test]
    fn reboot_delay_is_bounded() {
        assert_eq!(reboot(0).unwrap_err().code, PrivErrorCode::Invalid);
        assert_eq!(reboot(61).unwrap_err().code, PrivErrorCode::Invalid);
    }

    #[test]
    fn swap_and_zone_are_read_from_system_text() {
        assert_eq!(
            parse_swap_mb("MemTotal: 1 kB\nSwapTotal:       2097148 kB\nSwapFree: 1 kB\n"),
            2047
        );
        assert_eq!(parse_swap_mb("SwapTotal: 0 kB\n"), 0);
        assert_eq!(parse_swap_mb("nothing here"), 0);
        assert_eq!(
            zone_from_link("/usr/share/zoneinfo/Asia/Ho_Chi_Minh").as_deref(),
            Some("Asia/Ho_Chi_Minh")
        );
        assert_eq!(
            zone_from_link("../usr/share/zoneinfo/UTC").as_deref(),
            Some("UTC")
        );
        assert_eq!(zone_from_link("/etc/other"), None);
        assert_eq!(zone_from_link("/usr/share/zoneinfo/"), None);
    }
}
