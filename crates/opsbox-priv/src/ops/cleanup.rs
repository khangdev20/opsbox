//! Cleanup (docs/BACKEND_SPEC.md 7.11): scanners that estimate what can be freed, and the
//! runs that free it.
//!
//! Rules every scanner follows: it never touches anything a running app is using; it removes
//! only regular files it finds itself (never follows symbolic links, never removes folders
//! it did not create); Docker scanners exist only when Docker is installed and answers.
//! Removing packages is not cleanup, so `autoremove` is deliberately not offered.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use opsbox_proto::{CleanupFreed, CleanupItem, CleanupScan, PrivError, PrivErrorCode, PrivReply};

use super::exec::{find, run};

const TOOL: Duration = Duration::from_secs(120);
const QUICK: Duration = Duration::from_secs(10);
/// How old a file in `/tmp` must be (days since it was last used or changed).
const TMP_DAYS: u64 = 10;
/// Journals older than this are removed.
const JOURNAL_DAYS: u64 = 7;
const JOURNAL_DIRS: &[&str] = &["/var/log/journal"];
const TMP_ROOTS: &[&str] = &["/tmp", "/var/tmp"];
const COREDUMP_DIRS: &[&str] = &["/var/lib/systemd/coredump", "/var/crash"];
pub(crate) const SLOT_DIR: &str = "/var/lib/opsbox/bin";
const ACTIVE_LINK: &str = "/usr/local/bin/opsbox";
const TIMER: &str = "/etc/systemd/system/opsbox-cleanup.timer";
const SERVICE: &str = "/etc/systemd/system/opsbox-cleanup.service";

/// The order scanners run in, and the only ids a request may name.
pub const ALL_IDS: &[&str] = &[
    "journal",
    "pkg-cache",
    "slots",
    "tmp",
    "coredumps",
    "docker-images",
    "docker-buildcache",
    "docker-containers",
    "docker-volumes",
];

fn err(code: PrivErrorCode, message: impl Into<String>) -> PrivError {
    PrivError {
        code,
        message: message.into(),
    }
}

// ------------------------------------------------------------------ pure helpers

/// `1.2G`, `512M`, `3.5GB`, `0B`, `45.2kB`: a size printed by journalctl or Docker.
/// Both binary (journalctl) and decimal (Docker) spellings are read as the tool means them.
pub fn parse_size(text: &str) -> Option<u64> {
    let t = text.trim();
    let split = t.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (num, unit) = t.split_at(split);
    let value: f64 = num.parse().ok()?;
    let unit = unit.trim();
    let mult: f64 = match unit {
        "B" | "" => 1.0,
        // journalctl: binary units written with one letter
        "K" => 1024.0,
        "M" => 1024.0 * 1024.0,
        "G" => 1024.0 * 1024.0 * 1024.0,
        "T" => 1024.0_f64.powi(4),
        // docker: decimal units
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        // docker also uses these for some outputs
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    if !value.is_finite() || value < 0.0 {
        return None;
    }
    // Saturating float-to-int conversion is what is wanted here.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some((value * mult) as u64)
}

/// `journalctl --disk-usage`: "Archived and active journals take up 1.2G in the file system."
pub fn parse_journal_usage(text: &str) -> Option<u64> {
    let after = text
        .split("take up ")
        .nth(1)
        .or_else(|| text.split("takes up ").nth(1))?;
    parse_size(after.split_whitespace().next()?)
}

/// One line of `docker system df --format '{{json .}}'`.
#[derive(Debug, PartialEq, Eq)]
pub struct DockerDf {
    pub kind: String,
    pub reclaimable: u64,
}

/// `docker system df --format '{{json .}}'`: one JSON object per line, with a
/// `Reclaimable` such as `"1.2GB (50%)"`.
pub fn parse_docker_df(text: &str) -> Vec<DockerDf> {
    text.lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter_map(|v| {
            let kind = v.get("Type")?.as_str()?.to_owned();
            let rec = v.get("Reclaimable")?.as_str()?;
            let size = rec.split_whitespace().next()?;
            Some(DockerDf {
                kind,
                reclaimable: parse_size(size).unwrap_or(0),
            })
        })
        .collect()
}

fn age_of(meta: &fs::Metadata, now: SystemTime) -> Duration {
    let newest = [meta.modified().ok(), meta.accessed().ok()]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(SystemTime::UNIX_EPOCH);
    now.duration_since(newest).unwrap_or_default()
}

/// Regular files under `root` (no links followed) older than `min_age`, skipping any
/// directory for which `skip_dir` says yes. Returns path and size.
pub fn stale_files(
    root: &Path,
    min_age: Duration,
    now: SystemTime,
    skip_dir: &dyn Fn(&str) -> bool,
) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let Ok(meta) = fs::symlink_metadata(entry.path()) else {
                continue;
            };
            let ft = meta.file_type();
            if ft.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !skip_dir(&name) {
                    stack.push(entry.path());
                }
            } else if ft.is_file() && age_of(&meta, now) >= min_age {
                out.push((entry.path(), meta.len()));
            }
        }
    }
    out.sort();
    out
}

/// Folders in `/tmp` that belong to running services or sessions and must never be touched.
fn tmp_skip(name: &str) -> bool {
    name.starts_with("systemd-private-")
        || name.starts_with("snap-private-tmp")
        || name.starts_with("tmux-")
        || name.starts_with(".X11")
        || name == ".ICE-unix"
        || name == "lost+found"
}

/// All regular files in a folder (not recursive), as path and size.
fn files_in(dir: &Path) -> Vec<(PathBuf, u64)> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut v: Vec<_> = rd
        .flatten()
        .filter_map(|e| {
            let m = fs::symlink_metadata(e.path()).ok()?;
            m.file_type().is_file().then(|| (e.path(), m.len()))
        })
        .collect();
    v.sort();
    v
}

/// Release slots to remove: everything in `dir` except the active one and the newest
/// other one ("keep current and previous", spec section 4).
pub fn old_slots(dir: &Path, active: Option<&Path>) -> Vec<(PathBuf, u64)> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut slots: Vec<(PathBuf, u64, SystemTime)> = rd
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("opsbox-"))
        .filter_map(|e| {
            let m = fs::symlink_metadata(e.path()).ok()?;
            m.file_type().is_file().then(|| {
                (
                    e.path(),
                    m.len(),
                    m.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                )
            })
        })
        .filter(|(p, _, _)| active.is_none_or(|a| a != p.as_path()))
        .collect();
    // Newest first; the first is the previous release and is kept.
    slots.sort_by_key(|(_, _, t)| std::cmp::Reverse(*t));
    slots.into_iter().skip(1).map(|(p, s, _)| (p, s)).collect()
}

fn sum(files: &[(PathBuf, u64)]) -> u64 {
    files.iter().map(|(_, s)| s).sum()
}

/// Delete regular files from a list; returns the bytes actually freed. A file that is
/// already gone, or is no longer a regular file, is skipped.
fn delete_files(files: &[(PathBuf, u64)]) -> u64 {
    let mut freed = 0;
    for (path, size) in files {
        let still_regular = fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file());
        if still_regular && fs::remove_file(path).is_ok() {
            freed += size;
        }
    }
    freed
}

fn humanize(bytes: u64) -> String {
    // The numbers are for people to read; precision beyond a decimal is noise.
    #[allow(clippy::cast_precision_loss)]
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.0} MB", b / 1e6)
    } else {
        format!("{:.0} kB", b / 1e3)
    }
}

// ------------------------------------------------------------------ scanners

fn journal_files(now: SystemTime) -> Vec<(PathBuf, u64)> {
    let mut all = Vec::new();
    for base in JOURNAL_DIRS {
        let Ok(rd) = fs::read_dir(base) else { continue };
        for machine in rd.flatten() {
            // Archived journals have `@` in their name; the one being written does not.
            // `skip_dir` is always true: only the files directly in the machine's folder count.
            let files: Vec<_> = stale_files(
                &machine.path(),
                Duration::from_secs(JOURNAL_DAYS * 86400),
                now,
                &|_| true,
            )
            .into_iter()
            .filter(|(p, _)| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains('@') && n.ends_with(".journal"))
            })
            .collect();
            all.extend(files);
        }
    }
    all.sort();
    all
}

fn scan_journal(now: SystemTime) -> Option<CleanupItem> {
    let old = journal_files(now);
    let usage = run("/usr/bin/journalctl", &["--disk-usage"], QUICK)
        .and_then(|o| parse_journal_usage(&o.combined()));
    // Before a vacuum nothing says how much of the total is old: the archived files do.
    let bytes = sum(&old);
    (usage.is_some() || bytes > 0).then(|| CleanupItem {
        id: "journal".into(),
        title: format!("System journal older than {JOURNAL_DAYS} days"),
        desc: format!(
            "{} · keeps the last {JOURNAL_DAYS} days{}",
            "/var/log/journal",
            usage.map_or(String::new(), |u| format!(
                " · whole journal is {}",
                humanize(u)
            ))
        ),
        bytes,
        safe: true,
        warn: false,
    })
}

fn pkg_cache_files() -> (Vec<(PathBuf, u64)>, &'static str) {
    if find(&["/usr/bin/apt-get"]).is_some() {
        let mut v = files_in(Path::new("/var/cache/apt/archives"));
        v.extend(files_in(Path::new("/var/cache/apt/archives/partial")));
        (v, "apt")
    } else if find(&["/usr/bin/dnf"]).is_some() {
        let rpms = stale_files(
            Path::new("/var/cache/dnf"),
            Duration::ZERO,
            SystemTime::now(),
            &|_| false,
        )
        .into_iter()
        .filter(|(p, _)| p.extension().is_some_and(|e| e == "rpm"))
        .collect();
        (rpms, "dnf")
    } else {
        (Vec::new(), "")
    }
}

fn scan_pkg_cache() -> Option<CleanupItem> {
    let (files, mgr) = pkg_cache_files();
    if mgr.is_empty() {
        return None;
    }
    Some(CleanupItem {
        id: "pkg-cache".into(),
        title: "Downloaded package files".into(),
        desc: format!(
            "{mgr} cache · {} files · installed packages are not touched",
            files.len()
        ),
        bytes: sum(&files),
        safe: true,
        warn: false,
    })
}

fn active_slot() -> Option<PathBuf> {
    fs::canonicalize(ACTIVE_LINK).ok()
}

fn scan_slots() -> Option<CleanupItem> {
    let old = old_slots(Path::new(SLOT_DIR), active_slot().as_deref());
    (!old.is_empty()).then(|| CleanupItem {
        id: "slots".into(),
        title: "Old opsbox versions".into(),
        desc: format!(
            "{} older release{} · the running and the previous version stay",
            old.len(),
            if old.len() == 1 { "" } else { "s" }
        ),
        bytes: sum(&old),
        safe: true,
        warn: false,
    })
}

fn tmp_files(now: SystemTime) -> Vec<(PathBuf, u64)> {
    let mut all = Vec::new();
    for root in TMP_ROOTS {
        all.extend(stale_files(
            Path::new(root),
            Duration::from_secs(TMP_DAYS * 86400),
            now,
            &tmp_skip,
        ));
    }
    all
}

fn scan_tmp(now: SystemTime) -> Option<CleanupItem> {
    let files = tmp_files(now);
    (!files.is_empty()).then(|| CleanupItem {
        id: "tmp".into(),
        title: "Old files in /tmp and /var/tmp".into(),
        desc: format!("{} files not used for {TMP_DAYS}+ days", files.len()),
        bytes: sum(&files),
        safe: true,
        warn: false,
    })
}

fn coredump_files() -> Vec<(PathBuf, u64)> {
    COREDUMP_DIRS
        .iter()
        .flat_map(|d| files_in(Path::new(d)))
        .collect()
}

fn scan_coredumps() -> Option<CleanupItem> {
    let files = coredump_files();
    (!files.is_empty()).then(|| CleanupItem {
        id: "coredumps".into(),
        title: "Crash dumps".into(),
        desc: format!(
            "{} core dump{}",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        ),
        bytes: sum(&files),
        safe: true,
        warn: false,
    })
}

// ------------------------------------------------------------------ docker

fn docker_path() -> Option<String> {
    find(&["/usr/bin/docker", "/usr/local/bin/docker"])
}

/// Docker is there and its daemon answers.
fn docker_ready() -> Option<String> {
    let d = docker_path()?;
    run(&d, &["info", "--format", "{{.ServerVersion}}"], QUICK)
        .filter(super::exec::Output::ok)
        .map(|_| d)
}

fn docker_df(d: &str) -> Vec<DockerDf> {
    run(d, &["system", "df", "--format", "{{json .}}"], TOOL)
        .filter(super::exec::Output::ok)
        .map(|o| parse_docker_df(&o.stdout))
        .unwrap_or_default()
}

/// Total size of dangling images (untagged leftovers of builds).
fn docker_dangling_images(d: &str) -> (u64, usize) {
    let out = run(
        d,
        &[
            "images",
            "--filter",
            "dangling=true",
            "--format",
            "{{.Size}}",
        ],
        TOOL,
    );
    let sizes: Vec<u64> = out
        .filter(super::exec::Output::ok)
        .map(|o| o.stdout.lines().filter_map(parse_size).collect())
        .unwrap_or_default();
    (sizes.iter().sum(), sizes.len())
}

fn scan_docker(d: &str) -> Vec<CleanupItem> {
    let df = docker_df(d);
    let reclaim = |kind: &str| {
        df.iter()
            .find(|r| r.kind == kind)
            .map_or(0, |r| r.reclaimable)
    };
    let (img_bytes, img_n) = docker_dangling_images(d);
    let mut items = Vec::new();
    if img_n > 0 {
        items.push(CleanupItem {
            id: "docker-images".into(),
            title: "Docker: unused image layers".into(),
            desc: format!(
                "{img_n} untagged image{} · running containers and tagged images are kept",
                if img_n == 1 { "" } else { "s" }
            ),
            bytes: img_bytes,
            safe: true,
            warn: false,
        });
    }
    let cache = reclaim("Build Cache");
    if cache > 0 {
        items.push(CleanupItem {
            id: "docker-buildcache".into(),
            title: "Docker: build cache".into(),
            desc: "Layers kept to speed up builds · the next build may take longer".into(),
            bytes: cache,
            safe: true,
            warn: false,
        });
    }
    let stopped = reclaim("Containers");
    if stopped > 0 {
        items.push(CleanupItem {
            id: "docker-containers".into(),
            title: "Docker: stopped containers".into(),
            desc: "Containers that are not running · their writable layers are deleted".into(),
            bytes: stopped,
            safe: false,
            warn: false,
        });
    }
    let volumes = reclaim("Local Volumes");
    if volumes > 0 {
        items.push(CleanupItem {
            id: "docker-volumes".into(),
            title: "Docker volumes not used by any container".into(),
            desc: "▲ May contain data (databases, uploads) · unchecked by default".into(),
            bytes: volumes,
            safe: false,
            warn: true,
        });
    }
    items
}

// ------------------------------------------------------------------ scan and run

fn system_bytes() -> u64 {
    run("/usr/bin/du", &["-sxk", "/usr"], Duration::from_secs(20))
        .filter(super::exec::Output::ok)
        .and_then(|o| {
            o.stdout
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<u64>().ok())
        })
        .map_or(0, |k| k.saturating_mul(1024))
}

pub(crate) fn auto_enabled() -> bool {
    run(
        "/usr/bin/systemctl",
        &["is-enabled", "opsbox-cleanup.timer"],
        QUICK,
    )
    .is_some_and(|o| o.stdout.trim() == "enabled")
}

pub(crate) fn scan() -> PrivReply {
    let now = SystemTime::now();
    let mut items: Vec<CleanupItem> = [
        scan_journal(now),
        scan_pkg_cache(),
        scan_slots(),
        scan_tmp(now),
        scan_coredumps(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if let Some(d) = docker_ready() {
        items.extend(scan_docker(&d));
    }
    PrivReply::Cleanup(CleanupScan {
        items,
        system_bytes: system_bytes(),
        auto: auto_enabled(),
    })
}

/// Run one scanner; the bytes it freed.
fn run_one(id: &str) -> Result<u64, String> {
    let now = SystemTime::now();
    match id {
        "journal" => {
            let before = sum(&journal_files(now));
            let out = run(
                "/usr/bin/journalctl",
                &["--vacuum-time", &format!("{JOURNAL_DAYS}d")],
                TOOL,
            )
            .ok_or("journalctl is not available")?;
            if !out.ok() {
                return Err(out
                    .combined()
                    .trim()
                    .lines()
                    .last()
                    .unwrap_or("failed")
                    .to_owned());
            }
            Ok(before.saturating_sub(sum(&journal_files(SystemTime::now()))))
        }
        "pkg-cache" => {
            let (files, mgr) = pkg_cache_files();
            let before = sum(&files);
            let out = match mgr {
                "apt" => run("/usr/bin/apt-get", &["clean"], TOOL),
                "dnf" => run("/usr/bin/dnf", &["clean", "packages"], TOOL),
                _ => return Err("no supported package manager".into()),
            }
            .ok_or("the package manager did not run")?;
            if !out.ok() {
                return Err(out
                    .combined()
                    .trim()
                    .lines()
                    .last()
                    .unwrap_or("failed")
                    .to_owned());
            }
            Ok(before.saturating_sub(sum(&pkg_cache_files().0)))
        }
        "slots" => Ok(delete_files(&old_slots(
            Path::new(SLOT_DIR),
            active_slot().as_deref(),
        ))),
        "tmp" => Ok(delete_files(&tmp_files(now))),
        "coredumps" => Ok(delete_files(&coredump_files())),
        "docker-images" | "docker-buildcache" | "docker-containers" | "docker-volumes" => {
            let d = docker_ready().ok_or("Docker is not running")?;
            let before = docker_df(&d);
            let (args, kind): (&[&str], &str) = match id {
                "docker-images" => (&["image", "prune", "-f"], "Images"),
                "docker-buildcache" => (&["builder", "prune", "-f"], "Build Cache"),
                "docker-containers" => (&["container", "prune", "-f"], "Containers"),
                _ => (&["volume", "prune", "-f"], "Local Volumes"),
            };
            let out = run(&d, args, TOOL).ok_or("docker did not run")?;
            if !out.ok() {
                return Err(out
                    .combined()
                    .trim()
                    .lines()
                    .last()
                    .unwrap_or("failed")
                    .to_owned());
            }
            let rec = |df: &[DockerDf]| {
                df.iter()
                    .find(|r| r.kind == kind)
                    .map_or(0, |r| r.reclaimable)
            };
            // "Total reclaimed space: 1.2GB" is the tool's own answer.
            let said = out
                .stdout
                .lines()
                .find_map(|l| l.strip_prefix("Total reclaimed space:"))
                .and_then(|s| parse_size(s.trim()));
            Ok(said.unwrap_or_else(|| rec(&before).saturating_sub(rec(&docker_df(&d)))))
        }
        _ => Err("unknown".into()),
    }
}

pub fn run_ids(ids: &[String]) -> Result<PrivReply, PrivError> {
    if ids.is_empty() {
        return Err(err(PrivErrorCode::Invalid, "Choose what to clean."));
    }
    if let Some(bad) = ids.iter().find(|i| !ALL_IDS.contains(&i.as_str())) {
        return Err(err(
            PrivErrorCode::Invalid,
            format!("“{bad}” is not something that can be cleaned."),
        ));
    }
    // Fixed order, each at most once, whatever the request said.
    let freed = ALL_IDS
        .iter()
        .filter(|a| ids.iter().any(|i| i == **a))
        .map(|id| match run_one(id) {
            Ok(bytes) => CleanupFreed {
                id: (*id).into(),
                bytes,
                error: None,
            },
            Err(e) => CleanupFreed {
                id: (*id).into(),
                bytes: 0,
                error: Some(e),
            },
        })
        .collect();
    Ok(PrivReply::CleanupFreed(freed))
}

/// The scanners that are part of the automatic weekly clean.
pub fn safe_ids() -> Vec<String> {
    [
        "journal",
        "pkg-cache",
        "slots",
        "tmp",
        "coredumps",
        "docker-images",
        "docker-buildcache",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect()
}

// ------------------------------------------------------------------ automatic clean

fn opsbox_binary() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(str::to_owned))
        .filter(|p| !p.contains(" (deleted)"))
        .unwrap_or_else(|| "/usr/local/bin/opsbox".into())
}

fn unit_texts(binary: &str) -> (String, String) {
    let service = format!(
        "[Unit]\nDescription=opsbox weekly cleanup of old files (safe set only)\n\n[Service]\nType=oneshot\nExecStart={binary} cleanup --safe\nNice=19\nIOSchedulingClass=idle\n"
    );
    let timer = "[Unit]\nDescription=opsbox weekly cleanup\n\n[Timer]\nOnCalendar=weekly\nPersistent=true\nRandomizedDelaySec=1h\n\n[Install]\nWantedBy=timers.target\n".to_owned();
    (service, timer)
}

pub(crate) fn set_auto(on: bool) -> Result<(), PrivError> {
    let systemctl = find(&["/usr/bin/systemctl", "/bin/systemctl"]).ok_or_else(|| {
        err(
            PrivErrorCode::Unsupported,
            "This machine does not use systemd, so a weekly timer cannot be set up.",
        )
    })?;
    if on {
        let (service, timer) = unit_texts(&opsbox_binary());
        fs::write(SERVICE, service)
            .and_then(|()| fs::write(TIMER, timer))
            .map_err(|e| {
                err(
                    PrivErrorCode::Internal,
                    format!("Could not write the timer: {e}"),
                )
            })?;
        let _ = run(&systemctl, &["daemon-reload"], QUICK);
        let out = run(
            &systemctl,
            &["enable", "--now", "opsbox-cleanup.timer"],
            QUICK,
        )
        .ok_or_else(|| err(PrivErrorCode::Internal, "systemctl did not run."))?;
        if !out.ok() {
            return Err(err(
                PrivErrorCode::Internal,
                format!("Could not start the timer: {}", out.combined().trim()),
            ));
        }
    } else {
        let _ = run(
            &systemctl,
            &["disable", "--now", "opsbox-cleanup.timer"],
            QUICK,
        );
        let _ = fs::remove_file(TIMER);
        let _ = fs::remove_file(SERVICE);
        let _ = run(&systemctl, &["daemon-reload"], QUICK);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    #[test]
    fn sizes_in_both_spellings() {
        assert_eq!(parse_size("1.5G"), Some(1_610_612_736));
        assert_eq!(parse_size("512M"), Some(536_870_912));
        assert_eq!(parse_size("0B"), Some(0));
        assert_eq!(parse_size("45.2kB"), Some(45_200));
        assert_eq!(parse_size("2.5GB"), Some(2_500_000_000));
        assert_eq!(parse_size("128MB"), Some(128_000_000));
        assert_eq!(parse_size("10"), None, "a bare number has no unit boundary");
        assert_eq!(parse_size("1.2 parsecs"), None);
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("-3MB"), None);
    }

    #[test]
    fn journal_usage_line() {
        assert_eq!(
            parse_journal_usage("Archived and active journals take up 1.2G in the file system.\n"),
            Some(1_288_490_188)
        );
        assert_eq!(
            parse_journal_usage("Journals take up 8.0M in the file system."),
            Some(8_388_608)
        );
        assert_eq!(parse_journal_usage("No journal files were found."), None);
    }

    #[test]
    fn docker_df_lines() {
        let text = r#"{"Active":"2","Reclaimable":"1.2GB (60%)","Size":"2GB","TotalCount":"5","Type":"Images"}
{"Active":"1","Reclaimable":"0B (0%)","Size":"12kB","TotalCount":"1","Type":"Containers"}
{"Active":"0","Reclaimable":"300MB","Size":"300MB","TotalCount":"2","Type":"Local Volumes"}
{"Active":"0","Reclaimable":"45.2kB","Size":"45kB","TotalCount":"3","Type":"Build Cache"}
not json
{"Type":"Images"}"#;
        let df = parse_docker_df(text);
        assert_eq!(df.len(), 4);
        assert_eq!(
            df[0],
            DockerDf {
                kind: "Images".into(),
                reclaimable: 1_200_000_000
            }
        );
        assert_eq!(df[2].reclaimable, 300_000_000);
        assert_eq!(df[3].reclaimable, 45_200);
    }

    fn touch(path: &Path, size: usize, age_days: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![0u8; size]).unwrap();
        let t = SystemTime::now() - Duration::from_secs(age_days * 86400);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(t).set_accessed(t))
            .unwrap();
    }

    #[test]
    fn only_old_regular_files_outside_protected_folders_are_listed() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        touch(&r.join("old.txt"), 100, 20);
        touch(&r.join("new.txt"), 100, 1);
        touch(&r.join("sub/older.log"), 50, 30);
        touch(&r.join("systemd-private-abc/service/data"), 999, 40);
        touch(&r.join("tmux-1000/default"), 999, 40);
        std::os::unix::fs::symlink("/etc/passwd", r.join("link")).unwrap();
        let list = stale_files(
            r,
            Duration::from_secs(10 * 86400),
            SystemTime::now(),
            &tmp_skip,
        );
        let names: Vec<_> = list
            .iter()
            .map(|(p, s)| (p.strip_prefix(r).unwrap().to_str().unwrap().to_owned(), *s))
            .collect();
        assert_eq!(
            names,
            [
                ("old.txt".to_owned(), 100),
                ("sub/older.log".to_owned(), 50)
            ]
        );
        // And deleting works only on what was listed.
        assert_eq!(delete_files(&list), 150);
        assert!(r.join("new.txt").exists() && r.join("systemd-private-abc/service/data").exists());
        assert!(
            r.join("link").exists(),
            "a link is never followed or removed"
        );
        assert!(Path::new("/etc/passwd").exists());
    }

    #[test]
    fn a_listed_file_that_changed_meanwhile_is_not_removed() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("x");
        touch(&f, 10, 30);
        let list = stale_files(
            d.path(),
            Duration::from_secs(86400),
            SystemTime::now(),
            &|_| false,
        );
        fs::remove_file(&f).unwrap();
        fs::create_dir(&f).unwrap();
        assert_eq!(delete_files(&list), 0, "now a folder, so left alone");
        assert!(f.is_dir());
    }

    #[test]
    fn old_slots_keep_the_active_and_the_previous() {
        let d = tempfile::tempdir().unwrap();
        for (name, age) in [
            ("opsbox-0.1.0", 50),
            ("opsbox-0.2.0", 40),
            ("opsbox-0.3.0", 30),
            ("opsbox-0.4.0", 5),
        ] {
            touch(&d.path().join(name), 10, age);
        }
        touch(&d.path().join("unrelated"), 99, 90);
        let active = d.path().join("opsbox-0.4.0");
        let old = old_slots(d.path(), Some(&active));
        let names: Vec<_> = old
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_str().unwrap())
            .collect();
        // Active 0.4.0 stays, newest other (0.3.0) stays as the previous one.
        assert_eq!(names.len(), 2);
        assert!(
            names.contains(&"opsbox-0.1.0") && names.contains(&"opsbox-0.2.0"),
            "{names:?}"
        );
        assert!(
            old_slots(d.path(), None).len() == 3,
            "without an active link only the newest is kept"
        );
        assert!(old_slots(&d.path().join("missing"), None).is_empty());
        let _ = UNIX_EPOCH;
    }

    #[test]
    fn requests_are_validated_before_anything_runs() {
        assert_eq!(run_ids(&[]).unwrap_err().code, PrivErrorCode::Invalid);
        let e = run_ids(&["tmp".into(), "rm -rf".into()]).unwrap_err();
        assert_eq!(e.code, PrivErrorCode::Invalid);
        assert!(e.message.contains("rm -rf"));
    }

    #[test]
    fn the_automatic_clean_runs_only_the_safe_set() {
        let safe = safe_ids();
        for risky in ["docker-volumes", "docker-containers"] {
            assert!(!safe.iter().any(|s| s == risky));
        }
        assert!(safe.iter().all(|s| ALL_IDS.contains(&s.as_str())));
        let (service, timer) = unit_texts("/usr/local/bin/opsbox");
        assert!(service.contains("ExecStart=/usr/local/bin/opsbox cleanup --safe"));
        assert!(timer.contains("OnCalendar=weekly") && timer.contains("Persistent=true"));
    }
}
