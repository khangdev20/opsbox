//! Input validation helpers shared between `serve` (`opsbox-api`) and `priv` (`opsbox-priv`).

use std::path::{Component, Path};

use opsbox_proto::valid_docker_ref;

use crate::BackupError;

pub const MAX_JOB_NAME_LEN: usize = 80;
pub const MAX_PATHS_PER_JOB: usize = 32;
pub const MAX_VOLUMES_PER_JOB: usize = 32;
pub const MAX_RETAIN_DAILY: u32 = 3650;
pub const MAX_RETAIN_WEEKLY: u32 = 520;

/// Validate a human-friendly backup job name.
pub fn validate_job_name(raw: &str) -> Result<String, BackupError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(BackupError::Validation(
            "Give the backup schedule a name.".into(),
        ));
    }
    if trimmed.chars().count() > MAX_JOB_NAME_LEN || trimmed.chars().any(char::is_control) {
        return Err(BackupError::Validation(format!(
            "The backup name must be one line of at most {MAX_JOB_NAME_LEN} characters."
        )));
    }
    Ok(trimmed.to_owned())
}

/// Validate retention counts (`keep_daily`, `keep_weekly`).
pub fn validate_retention(daily: u32, weekly: u32) -> Result<(u32, u32), BackupError> {
    if daily == 0 && weekly == 0 {
        return Err(BackupError::Validation(
            "Keep at least one daily or one weekly backup.".into(),
        ));
    }
    if daily > MAX_RETAIN_DAILY {
        return Err(BackupError::Validation(format!(
            "Daily retention cannot exceed {MAX_RETAIN_DAILY} snapshots."
        )));
    }
    if weekly > MAX_RETAIN_WEEKLY {
        return Err(BackupError::Validation(format!(
            "Weekly retention cannot exceed {MAX_RETAIN_WEEKLY} snapshots."
        )));
    }
    Ok((daily, weekly))
}

/// Validate a single absolute path before sending to `priv` (and re-validated inside `priv`).
pub fn validate_abs_path(raw: &str) -> Result<String, BackupError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 1024 || trimmed.contains('\0') {
        return Err(BackupError::Validation(
            "Backup path must be non-empty, under 1024 bytes, and free of NUL bytes.".into(),
        ));
    }
    let p = Path::new(trimmed);
    if !p.is_absolute() {
        return Err(BackupError::Validation(format!(
            "Backup path \"{trimmed}\" must be an absolute path (starting with /)."
        )));
    }
    if p.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(BackupError::Validation(format!(
            "Backup path \"{trimmed}\" must not contain '..' segments."
        )));
    }
    if matches!(
        trimmed,
        "/" | "/proc" | "/sys" | "/dev" | "/etc/opsbox/master.key"
    ) || trimmed.ends_with("/master.key")
        || trimmed.starts_with("/proc/")
        || trimmed.starts_with("/sys/")
        || trimmed.starts_with("/dev/")
    {
        return Err(BackupError::Validation(format!(
            "Path \"{trimmed}\" cannot be backed up directly."
        )));
    }
    Ok(trimmed.to_owned())
}

/// Validate a non-empty deduplicated list of paths for a `path` backup job.
pub fn validate_path_list(paths: &[String]) -> Result<Vec<String>, BackupError> {
    if paths.is_empty() || paths.len() > MAX_PATHS_PER_JOB {
        return Err(BackupError::Validation(format!(
            "Specify between 1 and {MAX_PATHS_PER_JOB} directories or files to back up."
        )));
    }
    let mut out = Vec::with_capacity(paths.len());
    for p in paths {
        let clean = validate_abs_path(p)?;
        if !out.contains(&clean) {
            out.push(clean);
        }
    }
    Ok(out)
}

/// Validate a non-empty deduplicated list of Docker volume names.
pub fn validate_volume_list(volumes: &[String]) -> Result<Vec<String>, BackupError> {
    if volumes.is_empty() || volumes.len() > MAX_VOLUMES_PER_JOB {
        return Err(BackupError::Validation(format!(
            "Specify between 1 and {MAX_VOLUMES_PER_JOB} Docker volumes to back up."
        )));
    }
    let mut out = Vec::with_capacity(volumes.len());
    for v in volumes {
        let trimmed = v.trim();
        if !valid_docker_ref(trimmed) {
            return Err(BackupError::Validation(format!(
                "\"{trimmed}\" is not a valid Docker volume name."
            )));
        }
        if !out.iter().any(|x| x == trimmed) {
            out.push(trimmed.to_owned());
        }
    }
    Ok(out)
}

/// Validate `--read-data-subset` argument syntax for `restic check`.
pub fn validate_read_data_subset(raw: &str) -> Result<String, BackupError> {
    let s = raw.trim();
    if let Some(pct) = s.strip_suffix('%') {
        let n: u32 = pct.parse().map_err(|_| {
            BackupError::Validation(
                "Subset percentage must be a number between 1% and 100%.".into(),
            )
        })?;
        if (1..=100).contains(&n) {
            return Ok(format!("{n}%"));
        }
        return Err(BackupError::Validation(
            "Subset percentage must be between 1% and 100%.".into(),
        ));
    }
    if let Some((a, b)) = s.split_once('/') {
        let na: u32 = a.parse().map_err(|_| {
            BackupError::Validation("Subset fraction must look like n/t (e.g. 1/10).".into())
        })?;
        let nb: u32 = b.parse().map_err(|_| {
            BackupError::Validation("Subset fraction must look like n/t (e.g. 1/10).".into())
        })?;
        if nb > 0 && (1..=nb).contains(&na) {
            return Ok(format!("{na}/{nb}"));
        }
        return Err(BackupError::Validation(
            "Subset fraction n/t requires 1 <= n <= t.".into(),
        ));
    }
    if s.len() >= 2 {
        let last = s.chars().last().unwrap_or_default();
        let num = &s[..s.len() - 1];
        if matches!(last, 'k' | 'K' | 'm' | 'M' | 'g' | 'G' | 't' | 'T')
            && let Ok(n) = num.parse::<u64>()
            && n > 0
        {
            return Ok(s.to_owned());
        }
    }
    Err(BackupError::Validation(
        "Invalid read_data_subset value (use e.g. 10%, 1/10, or 100M).".into(),
    ))
}

/// Build the default copy-restore path `<path>.restored-<suffix>` for a directory/file snapshot.
pub fn default_restored_path(original_path: &str, suffix: &str) -> Result<String, BackupError> {
    let clean = validate_abs_path(original_path)?;
    let trimmed = clean.trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(BackupError::Validation(
            "Cannot derive copy restore target for root path.".into(),
        ));
    }
    let candidate = format!("{trimmed}.restored-{suffix}");
    validate_abs_path(&candidate)
}

/// Build the default copy-restore volume `<vol>-restored-<suffix>` for a Docker volume snapshot.
pub fn default_restored_volume(original_vol: &str, suffix: &str) -> Result<String, BackupError> {
    let clean = original_vol.trim();
    if !valid_docker_ref(clean) {
        return Err(BackupError::Validation(format!(
            "\"{clean}\" is not a valid Docker volume name."
        )));
    }
    let candidate = format!("{clean}-restored-{suffix}");
    if !valid_docker_ref(&candidate) {
        return Err(BackupError::Validation(format!(
            "Derived volume name \"{candidate}\" is not valid."
        )));
    }
    Ok(candidate)
}

/// Validate a path restore destination. `replace = true` also forbids overwriting `/var/lib/opsbox`.
pub fn validate_restore_path(raw: &str, replace: bool) -> Result<String, BackupError> {
    let clean = validate_abs_path(raw)?;
    if replace && (clean == "/var/lib/opsbox" || clean.starts_with("/var/lib/opsbox/")) {
        return Err(BackupError::Validation(
            "Replacing the live opsbox data directory in place is not allowed; restore to a copy instead.".into(),
        ));
    }
    Ok(clean)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_paths_volumes_retention_and_subsets() {
        assert!(validate_abs_path("/var/www/app").is_ok());
        assert!(validate_abs_path("relative/path").is_err());
        assert!(validate_abs_path("/var/../etc/passwd").is_err());
        assert!(validate_abs_path("/proc/1").is_err());
        assert!(validate_abs_path("/etc/opsbox/master.key").is_err());
        assert!(validate_abs_path("/").is_err());

        assert!(validate_volume_list(&["pgdata".into(), "redisdata".into()]).is_ok());
        assert!(validate_volume_list(&[]).is_err());
        assert!(validate_volume_list(&["../bad".into()]).is_err());

        assert_eq!(validate_retention(14, 8).unwrap(), (14, 8));
        assert!(validate_retention(0, 0).is_err());
        assert!(validate_retention(9999, 0).is_err());

        assert_eq!(validate_read_data_subset("10%").unwrap(), "10%");
        assert_eq!(validate_read_data_subset("1/10").unwrap(), "1/10");
        assert_eq!(validate_read_data_subset("500M").unwrap(), "500M");
        assert!(validate_read_data_subset("0%").is_err());
        assert!(validate_read_data_subset("101%").is_err());
        assert!(validate_read_data_subset("3/2").is_err());
        assert!(validate_read_data_subset("; rm -rf /").is_err());

        assert_eq!(
            default_restored_path("/var/www/app/", "2026-11-18").unwrap(),
            "/var/www/app.restored-2026-11-18"
        );
        assert_eq!(
            default_restored_volume("pgdata", "2026-11-18").unwrap(),
            "pgdata-restored-2026-11-18"
        );
        assert!(validate_restore_path("/var/lib/opsbox/opsbox.db", false).is_ok());
        assert!(validate_restore_path("/var/lib/opsbox/opsbox.db", true).is_err());
        assert!(validate_restore_path("/var/lib/opsbox", true).is_err());
    }
}
