//! Cleanup (docs/BACKEND_SPEC.md 7.11): scanners that estimate what can be freed, and the
//! runs that free it.
//!
//! Rules every scanner follows: it never touches anything a running app is using; it removes
//! only regular files it finds itself (never follows symbolic links, never removes folders
//! it did not create); Docker scanners exist only when Docker is installed and answers.
//! Removing packages is not cleanup, so `autoremove` is deliberately not offered.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use opsbox_proto::{CleanupFreed, CleanupItem, CleanupScan, PrivError, PrivErrorCode, PrivReply};

use super::exec::{find, run};

const TOOL: Duration = Duration::from_secs(120);
const QUICK: Duration = Duration::from_secs(10);
const SCAN_CACHE_TTL: Duration = Duration::from_secs(60);
const API_WAIT_DEADLINE: Duration = Duration::from_millis(250);
const SYSTEM_BYTES_TTL_SECS: i64 = 24 * 3600;
const SYSTEM_BYTES_CACHE: &str = "/var/lib/opsbox/cleanup-system-bytes.json";
/// How old a file in `/tmp` must be (days since it was last used or changed).
const TMP_DAYS: u64 = 10;
const TMP_MAX_DEPTH: usize = 8;
const TMP_MAX_ENTRIES: usize = 50_000;
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

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
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
    pub total_count: usize,
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
            let total_count = v
                .get("TotalCount")
                .and_then(|c| c.as_str())
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            Some(DockerDf {
                kind,
                reclaimable: parse_size(size).unwrap_or(0),
                total_count,
            })
        })
        .collect()
}

/// Parse `docker builder du` output (JSON lines or summary `Total:` / `Reclaimable:` line).
pub fn parse_builder_du(text: &str) -> u64 {
    let mut json_sum = 0u64;
    let mut saw_json = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('{')
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed)
        {
            saw_json = true;
            let rec = v
                .get("Reclaimable")
                .or_else(|| v.get("Size"))
                .and_then(|s| s.as_str())
                .and_then(|s| s.split_whitespace().next())
                .and_then(parse_size)
                .unwrap_or(0);
            json_sum = json_sum.saturating_add(rec);
        }
    }
    if saw_json {
        return json_sum;
    }
    for line in text.lines().rev() {
        let t = line.trim();
        if let Some(rest) = t
            .strip_prefix("Reclaimable:")
            .or_else(|| t.strip_prefix("Total:"))
            && let Some(sz) = rest.split_whitespace().next().and_then(parse_size)
        {
            return sz;
        }
    }
    0
}

/// Parse `docker ps -a -s --format '{{.Size}}'` lines like `"12.3kB (virtual 45.6MB)"`.
pub fn parse_container_sizes(text: &str) -> (u64, usize) {
    let mut total = 0u64;
    let mut count = 0usize;
    for line in text.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        count += 1;
        if let Some(first) = t.split_whitespace().next().and_then(parse_size) {
            total = total.saturating_add(first);
        }
    }
    (total, count)
}

fn age_of(meta: &fs::Metadata, now: SystemTime) -> Duration {
    let newest = [meta.modified().ok(), meta.accessed().ok()]
        .into_iter()
        .flatten()
        .max()
        .unwrap_or(SystemTime::UNIX_EPOCH);
    now.duration_since(newest).unwrap_or_default()
}

#[derive(Debug, Default)]
pub struct WalkResult {
    pub files: Vec<(PathBuf, u64)>,
    pub truncated: bool,
}

/// Bounded recursive walk for regular files older than `min_age` (no links followed).
pub fn stale_files_bounded(
    root: &Path,
    min_age: Duration,
    now: SystemTime,
    skip_dir: &dyn Fn(&str) -> bool,
    max_depth: usize,
    max_entries: usize,
) -> WalkResult {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    let mut truncated = false;

    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.flatten() {
            if seen >= max_entries {
                truncated = true;
                break;
            }
            seen += 1;
            // On Unix `DirEntry::metadata` uses `lstat` (does not follow symlinks).
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let ft = meta.file_type();
            if ft.is_dir() {
                if depth < max_depth {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !skip_dir(&name) {
                        stack.push((entry.path(), depth + 1));
                    }
                } else {
                    truncated = true;
                }
            } else if ft.is_file() && age_of(&meta, now) >= min_age {
                out.push((entry.path(), meta.len()));
            }
        }
        if truncated {
            break;
        }
    }
    out.sort();
    WalkResult {
        files: out,
        truncated,
    }
}

/// Regular files under `root` (no links followed) older than `min_age`, skipping any
/// directory for which `skip_dir` says yes. Returns path and size.
pub fn stale_files(
    root: &Path,
    min_age: Duration,
    now: SystemTime,
    skip_dir: &dyn Fn(&str) -> bool,
) -> Vec<(PathBuf, u64)> {
    stale_files_bounded(root, min_age, now, skip_dir, usize::MAX, usize::MAX).files
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
            let m = e.metadata().ok()?;
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
            let m = e.metadata().ok()?;
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
        let Ok(rd) = fs::read_dir(base) else {
            continue;
        };
        for machine in rd.flatten() {
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
        pending: false,
        duration_ms: None,
    })
}

fn pkg_cache_files() -> (Vec<(PathBuf, u64)>, &'static str) {
    if find(&["/usr/bin/apt-get"]).is_some() {
        let mut v = files_in(Path::new("/var/cache/apt/archives"));
        v.extend(files_in(Path::new("/var/cache/apt/archives/partial")));
        (v, "apt")
    } else if find(&["/usr/bin/dnf"]).is_some() {
        let rpms = stale_files_bounded(
            Path::new("/var/cache/dnf"),
            Duration::ZERO,
            SystemTime::now(),
            &|_| false,
            6,
            20_000,
        )
        .files
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
        pending: false,
        duration_ms: None,
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
        pending: false,
        duration_ms: None,
    })
}

fn tmp_files_bounded(now: SystemTime) -> WalkResult {
    let mut all = WalkResult::default();
    for root in TMP_ROOTS {
        let res = stale_files_bounded(
            Path::new(root),
            Duration::from_secs(TMP_DAYS * 86400),
            now,
            &tmp_skip,
            TMP_MAX_DEPTH,
            TMP_MAX_ENTRIES,
        );
        all.files.extend(res.files);
        all.truncated |= res.truncated;
    }
    all
}

fn scan_tmp(now: SystemTime) -> Option<CleanupItem> {
    let res = tmp_files_bounded(now);
    (!res.files.is_empty()).then(|| CleanupItem {
        id: "tmp".into(),
        title: "Old files in /tmp and /var/tmp".into(),
        desc: if res.truncated {
            format!(
                "at least {} files not used for {TMP_DAYS}+ days",
                res.files.len()
            )
        } else {
            format!("{} files not used for {TMP_DAYS}+ days", res.files.len())
        },
        bytes: sum(&res.files),
        safe: true,
        warn: false,
        pending: false,
        duration_ms: None,
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
        pending: false,
        duration_ms: None,
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
        QUICK,
    );
    let sizes: Vec<u64> = out
        .filter(super::exec::Output::ok)
        .map(|o| o.stdout.lines().filter_map(parse_size).collect())
        .unwrap_or_default();
    (sizes.iter().sum(), sizes.len())
}

fn docker_buildcache(d: &str) -> u64 {
    run(d, &["builder", "du", "--format", "{{json .}}"], QUICK)
        .filter(super::exec::Output::ok)
        .map(|o| parse_builder_du(&o.stdout))
        .unwrap_or(0)
}

fn docker_stopped_containers(d: &str) -> (u64, usize) {
    let out = run(
        d,
        &[
            "ps",
            "-a",
            "-s",
            "--filter",
            "status=exited",
            "--filter",
            "status=created",
            "--format",
            "{{.Size}}",
        ],
        QUICK,
    );
    out.filter(super::exec::Output::ok)
        .map(|o| parse_container_sizes(&o.stdout))
        .unwrap_or((0, 0))
}

fn docker_unused_volumes_count(d: &str) -> usize {
    run(
        d,
        &["volume", "ls", "-q", "--filter", "dangling=true"],
        QUICK,
    )
    .filter(super::exec::Output::ok)
    .map_or(0, |o| {
        o.stdout.lines().filter(|l| !l.trim().is_empty()).count()
    })
}

fn scan_docker(d: &str) -> Vec<CleanupItem> {
    let mut items = Vec::new();
    let (img_bytes, img_n) = docker_dangling_images(d);
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
            pending: false,
            duration_ms: None,
        });
    }
    let cache = docker_buildcache(d);
    if cache > 0 {
        items.push(CleanupItem {
            id: "docker-buildcache".into(),
            title: "Docker: build cache".into(),
            desc: "Layers kept to speed up builds · the next build may take longer".into(),
            bytes: cache,
            safe: true,
            warn: false,
            pending: false,
            duration_ms: None,
        });
    }
    let (stopped_bytes, stopped_n) = docker_stopped_containers(d);
    if stopped_n > 0 {
        items.push(CleanupItem {
            id: "docker-containers".into(),
            title: "Docker: stopped containers".into(),
            desc: format!(
                "{stopped_n} stopped container{} · their writable layers are deleted",
                if stopped_n == 1 { "" } else { "s" }
            ),
            bytes: stopped_bytes,
            safe: false,
            warn: false,
            pending: false,
            duration_ms: None,
        });
    }
    let vol_n = docker_unused_volumes_count(d);
    if vol_n > 0 {
        items.push(CleanupItem {
            id: "docker-volumes".into(),
            title: "Docker volumes not used by any container".into(),
            desc: format!(
                "▲ {vol_n} unused volume{} · may contain data · unchecked by default",
                if vol_n == 1 { "" } else { "s" }
            ),
            bytes: 0,
            safe: false,
            warn: true,
            pending: false,
            duration_ms: None,
        });
    }
    items
}

pub(crate) fn volume_sizes() -> Result<(u64, usize), PrivError> {
    let Some(d) = docker_ready() else {
        return Ok((0, 0));
    };
    let df = docker_df(&d);
    let vol = df.iter().find(|r| r.kind == "Local Volumes");
    let bytes = vol.map_or(0, |v| v.reclaimable);
    let count = vol.map_or(0, |v| v.total_count);
    if let Ok(mut cache) = scan_cache().mu.lock()
        && let Some(entry) = cache
            .items
            .get_mut("docker-volumes")
            .and_then(|o| o.as_mut())
    {
        entry.bytes = bytes;
    }
    Ok((bytes, count))
}

// ------------------------------------------------------------------ system bytes cache

#[derive(serde::Serialize, serde::Deserialize)]
struct SystemBytesFile {
    bytes: u64,
    updated_at: i64,
}

static SYSTEM_BYTES_RUNNING: AtomicBool = AtomicBool::new(false);

fn cached_system_bytes() -> u64 {
    let path = Path::new(SYSTEM_BYTES_CACHE);
    let existing = fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<SystemBytesFile>(&s).ok());
    let now = now_secs();
    let stale = existing
        .as_ref()
        .is_none_or(|e| now - e.updated_at >= SYSTEM_BYTES_TTL_SECS);
    if stale
        && std::env::var_os("OPSBOX_NO_BG_DU").is_none()
        && SYSTEM_BYTES_RUNNING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    {
        std::thread::spawn(move || {
            let bytes = compute_system_bytes_low_io();
            let payload = SystemBytesFile {
                bytes,
                updated_at: now_secs(),
            };
            if let Ok(json) = serde_json::to_vec(&payload) {
                let tmp = path.with_extension("json.tmp");
                if fs::write(&tmp, json).is_ok() {
                    let _ = fs::rename(&tmp, path);
                }
            }
            SYSTEM_BYTES_RUNNING.store(false, Ordering::SeqCst);
        });
    }
    existing.map_or(0, |e| e.bytes)
}

fn compute_system_bytes_low_io() -> u64 {
    let out = if find(&["/usr/bin/ionice"]).is_some() && find(&["/usr/bin/nice"]).is_some() {
        run(
            "/usr/bin/nice",
            &[
                "-n",
                "19",
                "/usr/bin/ionice",
                "-c3",
                "/usr/bin/du",
                "-sxk",
                "/usr",
            ],
            Duration::from_secs(60),
        )
    } else {
        run("/usr/bin/du", &["-sxk", "/usr"], Duration::from_secs(60))
    };
    out.filter(super::exec::Output::ok)
        .and_then(|o| {
            o.stdout
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<u64>().ok())
        })
        .map_or(0, |k| k.saturating_mul(1024))
}

// ------------------------------------------------------------------ concurrent scan & 60s cache

const SCANNER_GROUPS: &[&str] = &[
    "journal",
    "pkg-cache",
    "slots",
    "tmp",
    "coredumps",
    "docker",
    "auto",
];

#[derive(Default)]
struct CacheState {
    started_at: Option<Instant>,
    scanned_at: i64,
    in_flight: HashSet<String>,
    items: BTreeMap<String, Option<CleanupItem>>,
    auto: bool,
    timings_ms: BTreeMap<String, u64>,
}

struct SharedCache {
    mu: Mutex<CacheState>,
    cv: Condvar,
}

fn scan_cache() -> &'static SharedCache {
    static CACHE: OnceLock<SharedCache> = OnceLock::new();
    CACHE.get_or_init(|| SharedCache {
        mu: Mutex::new(CacheState::default()),
        cv: Condvar::new(),
    })
}

pub fn invalidate_cache() {
    if let Ok(mut g) = scan_cache().mu.lock() {
        g.started_at = None;
        g.items.clear();
        g.in_flight.clear();
        g.timings_ms.clear();
    }
}

fn pending_placeholder(id: &str) -> CleanupItem {
    let (title, safe, warn) = match id {
        "journal" => ("System journal", true, false),
        "pkg-cache" => ("Downloaded package files", true, false),
        "slots" => ("Old opsbox versions", true, false),
        "tmp" => ("Old files in /tmp and /var/tmp", true, false),
        "coredumps" => ("Crash dumps", true, false),
        "docker" => ("Docker", true, false),
        _ => (id, false, false),
    };
    CleanupItem {
        id: id.to_owned(),
        title: title.to_owned(),
        desc: "Scanning…".to_owned(),
        bytes: 0,
        safe,
        warn,
        pending: true,
        duration_ms: None,
    }
}

fn build_scan_snapshot(state: &CacheState) -> CleanupScan {
    let mut items = Vec::new();
    for id in ALL_IDS {
        if let Some(Some(item)) = state.items.get(*id) {
            items.push(item.clone());
        } else if state.in_flight.contains(*id) {
            items.push(pending_placeholder(id));
        }
    }
    if state.in_flight.contains("docker") && !items.iter().any(|i| i.id.starts_with("docker-")) {
        items.push(pending_placeholder("docker"));
    }
    CleanupScan {
        items,
        system_bytes: cached_system_bytes(),
        auto: state.auto,
        scanned_at: state.scanned_at,
    }
}

pub(crate) fn auto_enabled() -> bool {
    run(
        "/usr/bin/systemctl",
        &["is-enabled", "opsbox-cleanup.timer"],
        QUICK,
    )
    .is_some_and(|o| o.stdout.trim() == "enabled")
}

fn spawn_scanner_group(group: &'static str, now: SystemTime) {
    std::thread::spawn(move || {
        let span = tracing::info_span!("cleanup.scan", scanner = group);
        let _enter = span.enter();
        let t0 = Instant::now();
        let mut results: Vec<(String, Option<CleanupItem>)> = Vec::new();
        let mut auto_val: Option<bool> = None;

        match group {
            "journal" => results.push(("journal".into(), scan_journal(now))),
            "pkg-cache" => results.push(("pkg-cache".into(), scan_pkg_cache())),
            "slots" => results.push(("slots".into(), scan_slots())),
            "tmp" => results.push(("tmp".into(), scan_tmp(now))),
            "coredumps" => results.push(("coredumps".into(), scan_coredumps())),
            "docker" => {
                if let Some(d) = docker_ready() {
                    for item in scan_docker(&d) {
                        results.push((item.id.clone(), Some(item)));
                    }
                }
            }
            "auto" => auto_val = Some(auto_enabled()),
            _ => {}
        }

        let elapsed_ms = u64::try_from(t0.elapsed().as_millis()).unwrap_or(u64::MAX);
        tracing::info!(scanner = group, elapsed_ms, "cleanup scanner finished");

        let cache = scan_cache();
        if let Ok(mut g) = cache.mu.lock() {
            g.in_flight.remove(group);
            g.timings_ms.insert(group.to_owned(), elapsed_ms);
            if let Some(a) = auto_val {
                g.auto = a;
            }
            for (id, mut item) in results {
                if let Some(ref mut it) = item {
                    it.duration_ms = Some(elapsed_ms);
                }
                g.items.insert(id, item);
            }
            g.scanned_at = now_secs();
            cache.cv.notify_all();
        }
    });
}

pub(crate) fn scan_with_wait(fresh: bool, wait: Duration) -> CleanupScan {
    let cache = scan_cache();
    let mut g = cache.mu.lock().unwrap_or_else(|e| e.into_inner());
    let valid = !fresh && g.started_at.is_some_and(|t| t.elapsed() < SCAN_CACHE_TTL);

    if !valid && g.in_flight.is_empty() {
        g.started_at = Some(Instant::now());
        g.scanned_at = now_secs();
        g.items.clear();
        g.timings_ms.clear();
        for group in SCANNER_GROUPS {
            g.in_flight.insert((*group).to_owned());
        }
        let now = SystemTime::now();
        for group in SCANNER_GROUPS {
            spawn_scanner_group(group, now);
        }
    }

    let deadline = Instant::now() + wait;
    while !g.in_flight.is_empty() {
        let Some(rem) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        if rem.is_zero() {
            break;
        }
        let (next, _) = cache
            .cv
            .wait_timeout(g, rem)
            .unwrap_or_else(|e| e.into_inner());
        g = next;
    }

    build_scan_snapshot(&g)
}

pub(crate) fn scan(fresh: bool) -> PrivReply {
    PrivReply::Cleanup(scan_with_wait(fresh, API_WAIT_DEADLINE))
}

/// CLI helper: run a fresh scan waiting up to `LOCAL_DEADLINE` (or `TOOL` for Docker)
/// and return `(CleanupScan, timings_ms)`.
pub fn scan_with_timings() -> (CleanupScan, BTreeMap<String, u64>) {
    let _ = scan_with_wait(true, Duration::from_secs(10));
    let g = scan_cache().mu.lock().unwrap_or_else(|e| e.into_inner());
    (build_scan_snapshot(&g), g.timings_ms.clone())
}

#[cfg(test)]
type ScannerFn = Box<dyn FnOnce() -> Option<CleanupItem> + Send + 'static>;

/// Testable concurrent runner with per-scanner deadline: any scanner still running after
/// `deadline` is emitted with `pending: true` without delaying the others.
#[cfg(test)]
fn run_scanners_with_deadline(
    scanners: Vec<(&'static str, ScannerFn)>,
    deadline: Duration,
) -> Vec<CleanupItem> {
    let (tx, rx) = std::sync::mpsc::channel::<(&'static str, Option<CleanupItem>)>();
    let ids: Vec<&'static str> = scanners.iter().map(|(id, _)| *id).collect();
    let mut done: BTreeMap<&'static str, Option<CleanupItem>> = BTreeMap::new();

    for (id, f) in scanners {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let res = f();
            let _ = tx.send((id, res));
        });
    }
    drop(tx);
    let until = Instant::now() + deadline;
    while done.len() < ids.len() {
        let Some(rem) = until.checked_duration_since(Instant::now()) else {
            break;
        };
        match rx.recv_timeout(rem) {
            Ok((id, item)) => {
                done.insert(id, item);
            }
            Err(_) => break,
        }
    }
    let mut out = Vec::new();
    for id in ids {
        match done.get(id) {
            Some(Some(item)) => out.push(item.clone()),
            Some(None) => {}
            None => out.push(pending_placeholder(id)),
        }
    }
    out
}

// ------------------------------------------------------------------ run

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
        "tmp" => Ok(delete_files(&tmp_files_bounded(now).files)),
        "coredumps" => Ok(delete_files(&coredump_files())),
        "docker-images" | "docker-buildcache" | "docker-containers" | "docker-volumes" => {
            let d = docker_ready().ok_or("Docker is not running")?;
            let args: &[&str] = match id {
                "docker-images" => &["image", "prune", "-f"],
                "docker-buildcache" => &["builder", "prune", "-f"],
                "docker-containers" => &["container", "prune", "-f"],
                _ => &["volume", "prune", "-f"],
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
            // "Total reclaimed space: 1.2GB" is the tool's own answer.
            let said = out
                .stdout
                .lines()
                .find_map(|l| l.strip_prefix("Total reclaimed space:"))
                .and_then(|s| parse_size(s.trim()));
            Ok(said.unwrap_or(0))
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
    invalidate_cache();
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
    invalidate_cache();
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
                reclaimable: 1_200_000_000,
                total_count: 5,
            }
        );
        assert_eq!(df[2].reclaimable, 300_000_000);
        assert_eq!(df[2].total_count, 2);
        assert_eq!(df[3].reclaimable, 45_200);
    }

    #[test]
    fn docker_builder_and_container_parsers() {
        let json_du = r#"{"ID":"a","Reclaimable":"120MB","Size":"200MB"}
{"ID":"b","Reclaimable":"30.5kB","Size":"50kB"}"#;
        assert_eq!(parse_builder_du(json_du), 120_030_500);

        let text_du = "ID\tRECLAIMABLE\nTotal: 42.5MB\n";
        assert_eq!(parse_builder_du(text_du), 42_500_000);

        let ps = "12.3kB (virtual 45.6MB)\n0B (virtual 10MB)\n2MB (virtual 20MB)\n";
        assert_eq!(parse_container_sizes(ps), (2_012_300, 3));
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
        assert_eq!(delete_files(&list), 150);
        assert!(r.join("new.txt").exists() && r.join("systemd-private-abc/service/data").exists());
        assert!(
            r.join("link").exists(),
            "a link is never followed or removed"
        );
        assert!(Path::new("/etc/passwd").exists());
    }

    #[test]
    fn bounded_tmp_walk_stops_at_depth_and_entry_limits() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        for i in 0..10 {
            touch(&r.join(format!("f{i}.tmp")), 10, 20);
        }
        let limited = stale_files_bounded(
            r,
            Duration::from_secs(10 * 86400),
            SystemTime::now(),
            &tmp_skip,
            8,
            4,
        );
        assert!(limited.truncated);
        assert_eq!(limited.files.len(), 4);

        touch(&r.join("a/b/c/deep.tmp"), 10, 20);
        let depth_limited = stale_files_bounded(
            r,
            Duration::from_secs(10 * 86400),
            SystemTime::now(),
            &tmp_skip,
            1,
            1000,
        );
        assert!(depth_limited.truncated);
        assert!(
            !depth_limited
                .files
                .iter()
                .any(|(p, _)| p.ends_with("deep.tmp"))
        );
    }

    #[test]
    fn slow_scanner_is_reported_pending_without_delaying_others() {
        let fast_item = CleanupItem {
            id: "journal".into(),
            title: "Journal".into(),
            desc: "ok".into(),
            bytes: 123,
            safe: true,
            warn: false,
            pending: false,
            duration_ms: None,
        };
        let t0 = Instant::now();
        let items = run_scanners_with_deadline(
            vec![
                ("journal", Box::new(move || Some(fast_item))),
                (
                    "tmp",
                    Box::new(|| {
                        std::thread::sleep(Duration::from_millis(300));
                        None
                    }),
                ),
            ],
            Duration::from_millis(60),
        );
        assert!(t0.elapsed() < Duration::from_millis(200));
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "journal");
        assert!(!items[0].pending);
        assert_eq!(items[1].id, "tmp");
        assert!(items[1].pending);
    }

    #[test]
    fn cache_is_invalidated_by_run() {
        {
            let mut g = scan_cache().mu.lock().unwrap();
            g.started_at = Some(Instant::now());
            g.items.insert(
                "tmp".into(),
                Some(CleanupItem {
                    id: "tmp".into(),
                    title: "Tmp".into(),
                    desc: "cached".into(),
                    bytes: 42,
                    safe: true,
                    warn: false,
                    pending: false,
                    duration_ms: Some(1),
                }),
            );
        }
        let cached = scan_with_wait(false, Duration::from_millis(10));
        assert_eq!(cached.items.len(), 1);
        assert_eq!(cached.items[0].bytes, 42);

        let _ = run_ids(&["slots".into()]);
        let g = scan_cache().mu.lock().unwrap();
        assert!(g.started_at.is_none());
        assert!(g.items.is_empty());
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
