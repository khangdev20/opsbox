//! Self-update, signature verification and version slots (Phase 2).

pub mod release;
pub mod slots;

pub use release::{
    Channel, CheckResult, DEFAULT_REPO, ReleaseInfo, check_latest, current_target,
    download_and_verify, download_and_verify_for_target, select_release, verify_minisign,
    verify_sha256,
};
pub use slots::{
    BIN_SLOTS_DIR, SYMLINK_PATH, SlotLayout, SlotMeta, UPDATE_STATE_PATH, UpdateOutcome,
    UpdateState,
};

/// Minisign public key compiled into the binary (matches `install.sh`).
pub const MINISIGN_PUBKEY: &str = "RWRCUyMVksFGTIOeCqnTqQkIyfCLb5O1NPxAYB0tNyxfPl0whpWvfWJn";

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("unsupported CPU architecture: {0}")]
    UnsupportedArch(String),
    #[error("invalid version string: {0}")]
    InvalidVersion(String),
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("checksum verification failed: {0}")]
    Checksum(String),
    #[error("signature verification failed: {0}")]
    Signature(String),
    #[error("archive error: {0}")]
    Archive(String),
    #[error("new binary failed self-test: {0}")]
    Selftest(String),
    #[error("no previous version slot is available for rollback")]
    NoPreviousSlot,
    #[error("previous version slot {0} is missing")]
    SlotMissing(String),
    #[error(
        "cannot roll back to {target}: database schema is v{db_version} but {target} only supports up to v{slot_version}"
    )]
    IncompatibleSchema {
        db_version: u32,
        slot_version: u32,
        target: String,
    },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn semver_selection_respects_channel_and_drafts() {
        let json = r#"[
            {"tag_name": "v0.3.0", "draft": true, "prerelease": false, "html_url": "https://example.com/3"},
            {"tag_name": "v0.2.0-beta.1", "draft": false, "prerelease": true, "html_url": "https://example.com/2b"},
            {"tag_name": "v0.1.5", "draft": false, "prerelease": false, "html_url": "https://example.com/15"},
            {"tag_name": "v0.1.0", "draft": false, "prerelease": false, "html_url": "https://example.com/10"}
        ]"#;

        let stable = select_release("0.1.0", Channel::Stable, json, 100).unwrap();
        assert!(stable.update_available);
        assert_eq!(stable.latest.as_ref().unwrap().version, "0.1.5");

        let beta = select_release("0.1.5", Channel::Beta, json, 100).unwrap();
        assert!(beta.update_available);
        assert_eq!(beta.latest.as_ref().unwrap().version, "0.2.0-beta.1");

        let up_to_date = select_release("0.2.0-beta.1", Channel::Beta, json, 100).unwrap();
        assert!(!up_to_date.update_available);
    }

    #[test]
    fn sha256_verification_catches_tampering() {
        let data = b"hello opsbox";
        let digest: String = Sha256::digest(data)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        verify_sha256(data, &format!("{digest}  opsbox.tar.gz\n")).unwrap();
        assert!(verify_sha256(b"hello opsb0x", &format!("{digest}  opsbox.tar.gz")).is_err());
        assert!(verify_sha256(data, "not-a-hash").is_err());
    }

    #[test]
    fn minisign_verification_rejects_bad_signature_or_key() {
        // Invalid signature format or wrong key must always fail.
        assert!(verify_minisign(b"payload", "garbage", MINISIGN_PUBKEY).is_err());
        assert!(
            verify_minisign(
                b"payload",
                "untrusted comment: x\nRWQ=\ntrusted comment: y\nAA==\n",
                MINISIGN_PUBKEY
            )
            .is_err()
        );
    }

    #[test]
    fn single_binary_migrates_to_slots_and_rolls_back_safely() {
        let dir = tempdir().unwrap();
        let layout = SlotLayout {
            slots_dir: dir.path().join("bin"),
            symlink: dir.path().join("opsbox"),
            state_file: dir.path().join("update-state.json"),
        };

        // Simulate legacy single-binary install at symlink path.
        fs::write(&layout.symlink, b"v0.1.0-bin").unwrap();
        let staged = dir.path().join("staged.bin");
        fs::write(&staged, b"v0.2.0-bin").unwrap();

        let prev = layout
            .install_and_switch(&staged, "0.1.0", 4, "0.2.0", 4)
            .unwrap();
        assert_eq!(prev.as_deref(), Some("0.1.0"));
        assert_eq!(layout.current_slot_version().as_deref(), Some("0.2.0"));
        assert_eq!(fs::read(&layout.symlink).unwrap(), b"v0.2.0-bin");
        assert!(layout.slot_path("0.1.0").exists());

        // Install v0.3.0 -> v0.1.0 should be pruned, keeping v0.2.0 and v0.3.0.
        let staged3 = dir.path().join("staged3.bin");
        fs::write(&staged3, b"v0.3.0-bin").unwrap();
        let prev3 = layout
            .install_and_switch(&staged3, "0.2.0", 4, "0.3.0", 5)
            .unwrap();
        assert_eq!(prev3.as_deref(), Some("0.2.0"));
        assert!(!layout.slot_path("0.1.0").exists());
        assert!(layout.slot_path("0.2.0").exists());
        assert!(layout.slot_path("0.3.0").exists());

        // Refuse rollback if DB schema is already v5 and v0.2.0 only knows v4.
        let err = layout.rollback_slot(None, 5).unwrap_err();
        assert!(matches!(err, UpdateError::IncompatibleSchema { .. }));

        // Rollback succeeds when DB schema is still v4.
        let rolled = layout.rollback_slot(None, 4).unwrap();
        assert_eq!(rolled, "0.2.0");
        assert_eq!(fs::read(&layout.symlink).unwrap(), b"v0.2.0-bin");
    }

    #[test]
    fn archive_extraction_helper_validates_sha256_before_signature() {
        let dir = tempdir().unwrap();
        let tar_gz_path = dir
            .path()
            .join("opsbox-0.1.0-x86_64-unknown-linux-musl.tar.gz");
        {
            let file = fs::File::create(&tar_gz_path).unwrap();
            let enc = GzEncoder::new(file, Compression::default());
            let mut tar = tar::Builder::new(enc);
            let payload = b"binary";
            let mut header = tar::Header::new_gnu();
            header.set_size(payload.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(
                &mut header,
                Path::new("opsbox-0.1.0-x86_64-unknown-linux-musl/opsbox"),
                &payload[..],
            )
            .unwrap();
            tar.finish().unwrap();
        }
        let bytes = fs::read(&tar_gz_path).unwrap();
        let sha: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        fs::write(
            dir.path()
                .join("opsbox-0.1.0-x86_64-unknown-linux-musl.tar.gz.sha256"),
            format!("{sha}  opsbox-0.1.0-x86_64-unknown-linux-musl.tar.gz\n"),
        )
        .unwrap();
        fs::write(
            dir.path()
                .join("opsbox-0.1.0-x86_64-unknown-linux-musl.tar.gz.minisig"),
            "invalid",
        )
        .unwrap();

        // Point OPSBOX_RELEASE_BASE to file://dir -> fails at signature verification, never extracts.
        let base = format!("file://{}", dir.path().display());
        let res = std::panic::catch_unwind(|| {
            verify_sha256(&bytes, &format!("{sha}  x")).unwrap();
            verify_minisign(&bytes, "invalid", MINISIGN_PUBKEY)
        });
        assert!(res.unwrap().is_err());
        let _ = base;
    }
}
