//! Version slots (`/var/lib/opsbox/bin/opsbox-<version>`), symlink switching, and rollback state.

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::UpdateError;

pub const BIN_SLOTS_DIR: &str = "/var/lib/opsbox/bin";
pub const SYMLINK_PATH: &str = "/usr/local/bin/opsbox";
pub const UPDATE_STATE_PATH: &str = "/var/lib/opsbox/update-state.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotMeta {
    pub version: String,
    pub schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateOutcome {
    /// `"pending"`, `"updated"`, `"rolled_back"`, or `"failed"`.
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
pub struct SlotLayout {
    pub slots_dir: PathBuf,
    pub symlink: PathBuf,
    pub state_file: PathBuf,
}

impl Default for SlotLayout {
    fn default() -> Self {
        Self {
            slots_dir: PathBuf::from(BIN_SLOTS_DIR),
            symlink: PathBuf::from(SYMLINK_PATH),
            state_file: PathBuf::from(UPDATE_STATE_PATH),
        }
    }
}

impl SlotLayout {
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

    pub fn write_state(&self, state: &UpdateState) -> Result<(), UpdateError> {
        if let Some(parent) = self.state_file.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.state_file.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(state)?)?;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o644))?;
        fs::rename(&tmp, &self.state_file)?;
        Ok(())
    }

    pub fn write_slot_meta(&self, version: &str, schema_version: u32) -> Result<(), UpdateError> {
        let meta = SlotMeta {
            version: version.trim_start_matches('v').to_owned(),
            schema_version,
        };
        let path = self.meta_path(version);
        fs::write(&path, serde_json::to_vec(&meta)?)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))?;
        Ok(())
    }

    pub fn read_slot_meta(&self, version: &str) -> Option<SlotMeta> {
        fs::read_to_string(self.meta_path(version))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    }

    /// Detect the version currently pointed to by `self.symlink`.
    pub fn current_slot_version(&self) -> Option<String> {
        let target = fs::read_link(&self.symlink).ok()?;
        let name = target.file_name()?.to_str()?;
        name.strip_prefix("opsbox-").map(ToOwned::to_owned)
    }

    /// If `self.symlink` is a regular file (legacy single-binary install), copy it into
    /// `slots_dir/opsbox-<current_version>` and replace it with a symlink.
    pub fn migrate_single_binary(
        &self,
        current_version: &str,
        schema_version: u32,
    ) -> Result<Option<String>, UpdateError> {
        fs::create_dir_all(&self.slots_dir)?;
        fs::set_permissions(&self.slots_dir, fs::Permissions::from_mode(0o755))?;
        if let Some(ver) = self.current_slot_version() {
            if !self.meta_path(&ver).exists() {
                let _ = self.write_slot_meta(&ver, schema_version);
            }
            return Ok(Some(ver));
        }
        if let Ok(meta) = fs::symlink_metadata(&self.symlink)
            && meta.file_type().is_file()
        {
            let clean = current_version.trim_start_matches('v');
            let slot = self.slot_path(clean);
            fs::copy(&self.symlink, &slot)?;
            fs::set_permissions(&slot, fs::Permissions::from_mode(0o755))?;
            self.write_slot_meta(clean, schema_version)?;
            self.switch_symlink(&slot)?;
            return Ok(Some(clean.to_owned()));
        }
        Ok(None)
    }

    /// Atomically point `self.symlink` at `target`.
    pub fn switch_symlink(&self, target: &Path) -> Result<(), UpdateError> {
        if let Some(parent) = self.symlink.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.symlink.with_extension("tmp-link");
        let _ = fs::remove_file(&tmp);
        symlink(target, &tmp)?;
        fs::rename(&tmp, &self.symlink)?;
        Ok(())
    }

    /// Install `staged_bin` into `slots_dir/opsbox-<new_version>`, switch symlink,
    /// update `UpdateState`, and prune older slots (keeping current + previous).
    pub fn install_and_switch(
        &self,
        staged_bin: &Path,
        current_version: &str,
        current_schema: u32,
        new_version: &str,
        new_schema: u32,
    ) -> Result<Option<String>, UpdateError> {
        let prev = self.migrate_single_binary(current_version, current_schema)?;
        let clean_new = new_version.trim_start_matches('v').to_owned();
        let new_slot = self.slot_path(&clean_new);
        let tmp_slot = new_slot.with_extension("tmp");
        fs::copy(staged_bin, &tmp_slot)?;
        fs::set_permissions(&tmp_slot, fs::Permissions::from_mode(0o755))?;
        fs::rename(&tmp_slot, &new_slot)?;
        self.write_slot_meta(&clean_new, new_schema)?;
        self.switch_symlink(&new_slot)?;

        let prev_kept = prev.filter(|p| p != &clean_new);
        self.prune_old_slots(&clean_new, prev_kept.as_deref());

        let mut state = self.read_state();
        state.current_version = Some(clean_new);
        state.previous_version = prev_kept.clone();
        self.write_state(&state)?;

        Ok(prev_kept)
    }

    /// Roll back `self.symlink` to `to_version` (or `previous_version` from state),
    /// refusing if `current_db_schema` is newer than the target slot's schema version.
    pub fn rollback_slot(
        &self,
        to_version: Option<&str>,
        current_db_schema: u32,
    ) -> Result<String, UpdateError> {
        let state = self.read_state();
        let target_ver = to_version
            .map(|s| s.trim_start_matches('v').to_owned())
            .or_else(|| state.previous_version.clone())
            .ok_or(UpdateError::NoPreviousSlot)?;

        let slot = self.slot_path(&target_ver);
        if !slot.exists() {
            return Err(UpdateError::SlotMissing(target_ver));
        }
        if let Some(meta) = self.read_slot_meta(&target_ver)
            && current_db_schema > meta.schema_version
        {
            return Err(UpdateError::IncompatibleSchema {
                db_version: current_db_schema,
                slot_version: meta.schema_version,
                target: target_ver,
            });
        }

        self.switch_symlink(&slot)?;
        let mut next_state = state;
        next_state.current_version = Some(target_ver.clone());
        self.write_state(&next_state)?;
        Ok(target_ver)
    }

    /// Keep `keep_a` and `keep_b` slots; remove any older `opsbox-*` slots in `slots_dir`.
    pub fn prune_old_slots(&self, keep_a: &str, keep_b: Option<&str>) {
        let Ok(entries) = fs::read_dir(&self.slots_dir) else {
            return;
        };
        let keep_bin_a = format!("opsbox-{keep_a}");
        let keep_meta_a = format!("opsbox-{keep_a}.meta");
        let keep_bin_b = keep_b.map(|b| format!("opsbox-{b}"));
        let keep_meta_b = keep_b.map(|b| format!("opsbox-{b}.meta"));

        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("opsbox-") {
                continue;
            }
            let keep = name == keep_bin_a
                || name == keep_meta_a
                || keep_bin_b.as_deref() == Some(name.as_str())
                || keep_meta_b.as_deref() == Some(name.as_str());
            if !keep {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}
