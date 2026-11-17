//! Standalone disk-space recovery for `opsbox-watchdog`.
//!
//! Frees opsbox's own space and safe system caches when disk fills up or when the
//! administrator clicks "Free disk space" in Safe Mode. Never deletes user apps,
//! running containers, or database files.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::exec::{find, run};
use crate::probes::human_bytes;
use crate::slots::SlotManager;

const TOOL_TIMEOUT: Duration = Duration::from_secs(60);
const TMP_MAX_AGE: Duration = Duration::from_secs(10 * 86400);
const XFER_MAX_AGE: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone, Default)]
pub struct CleanupReport {
    pub freed_bytes: u64,
    pub steps: Vec<String>,
}

impl CleanupReport {
    pub fn summary(&self) -> String {
        if self.steps.is_empty() {
            "No reclaimable opsbox caches or old slots were found.".to_owned()
        } else {
            format!(
                "Freed {} ({}).",
                human_bytes(self.freed_bytes),
                self.steps.join(", ")
            )
        }
    }
}

pub fn free_opsbox_space(
    slots: &SlotManager,
    xfer_dir: &Path,
    db_path: &Path,
    critical: bool,
) -> CleanupReport {
    let mut report = CleanupReport::default();
    let now = SystemTime::now();

    let slots_freed = slots.prune_old_slots();
    if slots_freed > 0 {
        report.freed_bytes = report.freed_bytes.saturating_add(slots_freed);
        report.steps.push("old opsbox versions".into());
    }

    let xfer_freed = clean_stale_xfer(xfer_dir, now);
    if xfer_freed > 0 {
        report.freed_bytes = report.freed_bytes.saturating_add(xfer_freed);
        report.steps.push("stale transfer files".into());
    }

    let bak_freed = clean_old_db_backups(db_path);
    if bak_freed > 0 {
        report.freed_bytes = report.freed_bytes.saturating_add(bak_freed);
        report.steps.push("older database migration backups".into());
    }

    let journal_freed = clean_journal(critical);
    if journal_freed > 0 {
        report.freed_bytes = report.freed_bytes.saturating_add(journal_freed);
        report.steps.push("old journal logs".into());
    }

    let pkg_freed = clean_pkg_cache();
    if pkg_freed > 0 {
        report.freed_bytes = report.freed_bytes.saturating_add(pkg_freed);
        report.steps.push("package cache".into());
    }

    let tmp_freed = clean_tmp_and_coredumps(now);
    if tmp_freed > 0 {
        report.freed_bytes = report.freed_bytes.saturating_add(tmp_freed);
        report.steps.push("old temp files and crash dumps".into());
    }

    let docker_freed = clean_docker_safe();
    if docker_freed > 0 {
        report.freed_bytes = report.freed_bytes.saturating_add(docker_freed);
        report
            .steps
            .push("Docker build cache and untagged images".into());
    }

    report
}

fn clean_stale_xfer(xfer_dir: &Path, now: SystemTime) -> u64 {
    let Ok(rd) = fs::read_dir(xfer_dir) else {
        return 0;
    };
    let mut freed = 0u64;
    for entry in rd.flatten() {
        let path = entry.path();
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.file_type().is_file() {
            continue;
        }
        let is_part = path.extension().is_some_and(|e| e == "part");
        let age = now
            .duration_since(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH))
            .unwrap_or_default();
        if ((is_part && age >= Duration::from_secs(60)) || age >= XFER_MAX_AGE)
            && fs::remove_file(&path).is_ok()
        {
            freed = freed.saturating_add(meta.len());
        }
    }
    freed
}

fn clean_old_db_backups(db_path: &Path) -> u64 {
    let Some(dir) = db_path.parent() else {
        return 0;
    };
    let prefix = format!(
        "{}.pre-v",
        db_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("opsbox.db")
    );
    let Ok(rd) = fs::read_dir(dir) else {
        return 0;
    };
    let mut backups: Vec<(PathBuf, u64, SystemTime)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&prefix) || !name.ends_with(".bak") {
                return None;
            }
            let meta = fs::symlink_metadata(e.path()).ok()?;
            meta.file_type().is_file().then(|| {
                (
                    e.path(),
                    meta.len(),
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                )
            })
        })
        .collect();
    // Always keep the newest migration backup; remove older ones.
    backups.sort_by_key(|(_, _, t)| std::cmp::Reverse(*t));
    let mut freed = 0u64;
    for (p, sz, _) in backups.into_iter().skip(1) {
        if fs::remove_file(p).is_ok() {
            freed = freed.saturating_add(sz);
        }
    }
    freed
}

fn journal_size_bytes() -> u64 {
    let mut total = 0u64;
    let Ok(rd) = fs::read_dir("/var/log/journal") else {
        return 0;
    };
    for sub in rd.flatten() {
        let Ok(inner) = fs::read_dir(sub.path()) else {
            continue;
        };
        for f in inner.flatten() {
            if let Ok(m) = fs::symlink_metadata(f.path())
                && m.file_type().is_file()
            {
                total = total.saturating_add(m.len());
            }
        }
    }
    total
}

fn clean_journal(critical: bool) -> u64 {
    let Some(ctl) = find(&["/usr/bin/journalctl", "/bin/journalctl"]) else {
        return 0;
    };
    let before = journal_size_bytes();
    let args: &[&str] = if critical {
        &["--vacuum-size=50M"]
    } else {
        &["--vacuum-time=7d"]
    };
    let _ = run(&ctl, args, TOOL_TIMEOUT);
    before.saturating_sub(journal_size_bytes())
}

fn dir_files_size(dir: &Path) -> u64 {
    let Ok(rd) = fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .filter_map(|e| fs::symlink_metadata(e.path()).ok())
        .filter(|m| m.file_type().is_file())
        .map(|m| m.len())
        .sum()
}

fn clean_pkg_cache() -> u64 {
    if let Some(apt) = find(&["/usr/bin/apt-get"]) {
        let dir = Path::new("/var/cache/apt/archives");
        let before = dir_files_size(dir);
        let _ = run(&apt, &["clean"], TOOL_TIMEOUT);
        before.saturating_sub(dir_files_size(dir))
    } else if let Some(dnf) = find(&["/usr/bin/dnf"]) {
        let _ = run(&dnf, &["clean", "packages"], TOOL_TIMEOUT);
        0
    } else {
        0
    }
}

fn clean_tmp_and_coredumps(now: SystemTime) -> u64 {
    let mut freed = 0u64;
    for dir in ["/var/lib/systemd/coredump", "/var/crash"] {
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                if let Ok(m) = fs::symlink_metadata(e.path())
                    && m.file_type().is_file()
                    && fs::remove_file(e.path()).is_ok()
                {
                    freed = freed.saturating_add(m.len());
                }
            }
        }
    }
    for root in ["/tmp", "/var/tmp"] {
        freed = freed.saturating_add(clean_old_tmp(Path::new(root), now, 0));
    }
    freed
}

fn tmp_skip(name: &str) -> bool {
    name.starts_with("systemd-private-")
        || name.starts_with("snap-private-tmp")
        || name.starts_with("tmux-")
        || name.starts_with(".X11")
        || name == ".ICE-unix"
        || name == "lost+found"
}

fn clean_old_tmp(dir: &Path, now: SystemTime, depth: usize) -> u64 {
    if depth > 6 {
        return 0;
    }
    let Ok(rd) = fs::read_dir(dir) else {
        return 0;
    };
    let mut freed = 0u64;
    for entry in rd.flatten() {
        let Ok(meta) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        let ft = meta.file_type();
        if ft.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !tmp_skip(&name) {
                freed = freed.saturating_add(clean_old_tmp(&entry.path(), now, depth + 1));
            }
        } else if ft.is_file() {
            let newest = [meta.modified().ok(), meta.accessed().ok()]
                .into_iter()
                .flatten()
                .max()
                .unwrap_or(SystemTime::UNIX_EPOCH);
            if now.duration_since(newest).unwrap_or_default() >= TMP_MAX_AGE
                && fs::remove_file(entry.path()).is_ok()
            {
                freed = freed.saturating_add(meta.len());
            }
        }
    }
    freed
}

fn clean_docker_safe() -> u64 {
    let Some(docker) = find(&["/usr/bin/docker", "/usr/local/bin/docker"]) else {
        return 0;
    };
    if !run(
        &docker,
        &["info", "--format", "{{.ServerVersion}}"],
        Duration::from_secs(5),
    )
    .is_some_and(|o| o.ok())
    {
        return 0;
    }
    let mut freed = 0u64;
    for args in [
        &["builder", "prune", "-f"][..],
        &["image", "prune", "-f"][..],
    ] {
        if let Some(o) = run(&docker, args, TOOL_TIMEOUT)
            && o.ok()
            && let Some(bytes) = o
                .stdout
                .lines()
                .find_map(|l| l.strip_prefix("Total reclaimed space:"))
                .and_then(|s| parse_docker_size(s.trim()))
        {
            freed = freed.saturating_add(bytes);
        }
    }
    freed
}

fn parse_docker_size(text: &str) -> Option<u64> {
    let split = text.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (num, unit) = text.split_at(split);
    let val: f64 = num.parse().ok()?;
    let mult = match unit.trim() {
        "B" | "" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "TB" => 1e12,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    if !val.is_finite() || val < 0.0 {
        return None;
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some((val * mult) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_old_slots_and_stale_xfer_files() {
        let dir = tempfile::tempdir().unwrap();
        let slots = SlotManager {
            slots_dir: dir.path().join("bin"),
            symlink: dir.path().join("opsbox"),
            state_file: dir.path().join("update-state.json"),
        };
        fs::create_dir_all(&slots.slots_dir).unwrap();
        fs::write(slots.slot_path("0.1.0"), vec![0u8; 1024]).unwrap();
        fs::write(slots.slot_path("0.1.1"), vec![0u8; 2048]).unwrap();
        fs::write(slots.slot_path("0.1.2"), vec![0u8; 4096]).unwrap();
        std::os::unix::fs::symlink(slots.slot_path("0.1.2"), &slots.symlink).unwrap();
        slots
            .write_state(&crate::slots::UpdateState {
                current_version: Some("0.1.2".into()),
                previous_version: Some("0.1.1".into()),
                last_outcome: None,
            })
            .unwrap();

        let freed_slots = slots.prune_old_slots();
        assert_eq!(freed_slots, 1024);
        assert!(!slots.slot_path("0.1.0").exists());
        assert!(slots.slot_path("0.1.1").exists());
        assert!(slots.slot_path("0.1.2").exists());
    }
}
