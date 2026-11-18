//! Standalone version-slot and update-state manager for `opsbox-watchdog`.
//!
//! Mirrors the `/var/lib/opsbox/bin` and `/var/lib/opsbox/update-state.json`
//! layout without depending on `opsbox-update` (which `serve` shares).

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotMeta {
    pub version: String,
    pub schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateOutcome {
    pub status: String,
    pub from_version: String,
    pub to_version: String,
    pub message: String,
    pub at: i64,
    #[serde(default)]
    pub audited: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateState {
    pub current_version: Option<String>,
    pub previous_version: Option<String>,
    pub last_outcome: Option<UpdateOutcome>,
}

#[derive(Debug, Clone)]
pub struct SlotManager {
    pub slots_dir: PathBuf,
    pub symlink: PathBuf,
    pub state_file: PathBuf,
}

impl SlotManager {
    pub fn slot_path(&self, version: &str) -> PathBuf {
        self.slots_dir
            .join(format!("opsbox-{}", version.trim_start_matches('v')))
    }

    pub fn meta_path(&self, version: &str) -> PathBuf {
        self.slots_dir
            .join(format!("opsbox-{}.meta", version.trim_start_matches('v')))
    }

    pub fn read_state(&self) -> UpdateState {
        fs::read_to_string(&self.state_file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn write_state(&self, state: &UpdateState) -> Result<(), String> {
        if let Some(parent) = self.state_file.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let tmp = self.state_file.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
        fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644)).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &self.state_file).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn read_slot_meta(&self, version: &str) -> Option<SlotMeta> {
        fs::read_to_string(self.meta_path(version))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn current_slot_version(&self) -> Option<String> {
        let target = fs::read_link(&self.symlink).ok()?;
        let name = target.file_name()?.to_str()?;
        name.strip_prefix("opsbox-").map(ToOwned::to_owned)
    }

    /// Returns the previous slot version if its binary exists and differs from the active one.
    pub fn available_previous_slot(&self) -> Option<String> {
        let state = self.read_state();
        let prev = state.previous_version?;
        let active = self.current_slot_version().or(state.current_version);
        if active.as_deref() == Some(prev.as_str()) {
            return None;
        }
        self.slot_path(&prev).exists().then_some(prev)
    }

    pub fn mark_verified(&self, to_version: &str, from_version: &str, now: i64) {
        let mut state = self.read_state();
        state.last_outcome = Some(UpdateOutcome {
            status: "updated".to_owned(),
            from_version: from_version.to_owned(),
            to_version: to_version.to_owned(),
            message: format!("Updated to {to_version} and passed health check."),
            at: now,
            audited: false,
        });
        let _ = self.write_state(&state);
    }

    pub fn rollback_to_previous(&self, db_path: &Path, now: i64) -> Result<String, String> {
        let state = self.read_state();
        let target_ver = state
            .previous_version
            .clone()
            .ok_or_else(|| "No previous version slot is available for rollback.".to_owned())?;
        let slot = self.slot_path(&target_ver);
        if !slot.exists() {
            return Err(format!("Previous version slot {target_ver} is missing."));
        }
        let current_db_schema = if db_path.exists() {
            opsbox_store::db_schema_version(db_path).map_err(|e| e.to_string())?
        } else {
            0
        };
        if let Some(meta) = self.read_slot_meta(&target_ver)
            && current_db_schema > meta.schema_version
        {
            return Err(format!(
                "Cannot roll back to {target_ver}: database schema is v{current_db_schema} but {target_ver} only supports up to v{}.",
                meta.schema_version
            ));
        }

        let from_ver = self
            .current_slot_version()
            .or_else(|| state.current_version.clone())
            .unwrap_or_else(|| opsbox_version::VERSION.to_owned());

        let tmp = self.symlink.with_extension("tmp-link");
        let _ = fs::remove_file(&tmp);
        if let Some(parent) = self.symlink.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        symlink(&slot, &tmp).map_err(|e| format!("cannot stage rollback symlink: {e}"))?;
        fs::rename(&tmp, &self.symlink)
            .map_err(|e| format!("cannot switch rollback symlink: {e}"))?;

        let mut next = state;
        next.current_version = Some(target_ver.clone());
        next.last_outcome = Some(UpdateOutcome {
            status: "rolled_back".to_owned(),
            from_version: from_ver.clone(),
            to_version: target_ver.clone(),
            message: format!(
                "Rolled back from {from_ver} to {target_ver} after health checks failed."
            ),
            at: now,
            audited: false,
        });
        self.write_state(&next)?;
        Ok(target_ver)
    }

    /// Delete older release slots, always preserving the active and previous slots.
    pub fn prune_old_slots(&self) -> u64 {
        let Ok(rd) = fs::read_dir(&self.slots_dir) else {
            return 0;
        };
        let state = self.read_state();
        let active = self.current_slot_version().or(state.current_version);
        let prev = state.previous_version;

        let keep_names: Vec<String> = [active.as_deref(), prev.as_deref()]
            .into_iter()
            .flatten()
            .flat_map(|v| {
                let clean = v.trim_start_matches('v');
                [format!("opsbox-{clean}"), format!("opsbox-{clean}.meta")]
            })
            .collect();

        let mut freed = 0u64;
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("opsbox-") || keep_names.iter().any(|k| k == &name) {
                continue;
            }
            if let Ok(meta) = fs::symlink_metadata(entry.path())
                && meta.file_type().is_file()
                && fs::remove_file(entry.path()).is_ok()
            {
                freed = freed.saturating_add(meta.len());
            }
        }
        freed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_switches_symlink_and_respects_schema_compat() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SlotManager {
            slots_dir: dir.path().join("bin"),
            symlink: dir.path().join("opsbox"),
            state_file: dir.path().join("update-state.json"),
        };
        fs::create_dir_all(&mgr.slots_dir).unwrap();
        fs::write(mgr.slot_path("0.1.3"), b"old").unwrap();
        fs::write(
            mgr.meta_path("0.1.3"),
            serde_json::to_vec(&SlotMeta {
                version: "0.1.3".into(),
                schema_version: opsbox_store::LATEST_VERSION,
            })
            .unwrap(),
        )
        .unwrap();
        fs::write(mgr.slot_path("0.1.4"), b"new").unwrap();
        symlink(mgr.slot_path("0.1.4"), &mgr.symlink).unwrap();
        mgr.write_state(&UpdateState {
            current_version: Some("0.1.4".into()),
            previous_version: Some("0.1.3".into()),
            last_outcome: Some(UpdateOutcome {
                status: "pending".into(),
                from_version: "0.1.3".into(),
                to_version: "0.1.4".into(),
                message: "pending".into(),
                at: 100,
                audited: false,
            }),
        })
        .unwrap();

        assert_eq!(mgr.available_previous_slot().as_deref(), Some("0.1.3"));
        let db_path = dir.path().join("opsbox.db");
        {
            let _store = opsbox_store::Store::open(&db_path).unwrap();
        }
        let rolled = mgr.rollback_to_previous(&db_path, 200).unwrap();
        assert_eq!(rolled, "0.1.3");
        assert_eq!(fs::read(&mgr.symlink).unwrap(), b"old");
        let st = mgr.read_state();
        assert_eq!(st.last_outcome.unwrap().status, "rolled_back");
        assert_eq!(mgr.available_previous_slot(), None);
    }
}
