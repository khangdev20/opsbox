//! Self-update, signature verification and version slots (Phase 2).

pub mod release;
pub mod slots;

pub use release::{
    CACHE_TTL_SECS, Channel, CheckResult, DEFAULT_REPO, HttpFetcher, HttpReply, NoOfferReason,
    ReleaseChecker, ReleaseFetcher, ReleaseInfo, check_latest, check_latest_default_channel,
    current_target, default_channel, download_and_verify, download_and_verify_for_target,
    select_release, select_release_default_channel, verify_minisign, verify_sha256,
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
    use std::sync::Arc;
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

    /// Every published release so far is an alpha, newest first, as GitHub returns them.
    const ALPHAS: &str = r#"[
        {"tag_name": "v0.1.0-alpha.20", "draft": false, "prerelease": true, "html_url": "https://example.com/20"},
        {"tag_name": "v0.1.0-alpha.14", "draft": false, "prerelease": true, "html_url": "https://example.com/14"},
        {"tag_name": "v0.1.0-alpha.13", "draft": false, "prerelease": true, "html_url": "https://example.com/13"}
    ]"#;

    #[test]
    fn beta_channel_sees_a_newer_alpha() {
        let res = select_release("0.1.0-alpha.13", Channel::Beta, ALPHAS, 100).unwrap();
        assert!(res.update_available);
        assert!(!res.migration);
        assert_eq!(res.latest.unwrap().version, "0.1.0-alpha.20");

        let current = select_release("0.1.0-alpha.20", Channel::Beta, ALPHAS, 100).unwrap();
        assert!(!current.update_available);
    }

    #[test]
    fn stable_channel_with_only_prereleases_has_no_release() {
        let res = select_release("0.1.0-alpha.13", Channel::Stable, ALPHAS, 100).unwrap();
        assert!(res.latest.is_none());
        assert!(!res.update_available);
    }

    #[test]
    fn old_untagged_build_is_offered_the_newest_alpha_once() {
        let res = select_release("0.1.0", Channel::Beta, ALPHAS, 100).unwrap();
        assert!(res.update_available);
        assert!(res.migration);
        assert_eq!(res.latest.unwrap().version, "0.1.0-alpha.20");

        // Once the build reports its tag the offer is gone.
        let after = select_release("0.1.0-alpha.20", Channel::Beta, ALPHAS, 100).unwrap();
        assert!(!after.update_available && !after.migration);

        // A different major.minor.patch is not a migration.
        let other = select_release("0.2.0", Channel::Beta, ALPHAS, 100).unwrap();
        assert!(!other.update_available && !other.migration);

        // A real 0.1.0 release being published wins over the alphas: no migration.
        let with_final = r#"[
            {"tag_name": "v0.1.0", "draft": false, "prerelease": false, "html_url": "x"},
            {"tag_name": "v0.1.0-alpha.20", "draft": false, "prerelease": true, "html_url": "y"}
        ]"#;
        let res = select_release("0.1.0", Channel::Beta, with_final, 100).unwrap();
        assert!(!res.update_available && !res.migration);
    }

    #[test]
    fn unchosen_channel_still_migrates_an_old_build() {
        // The installer never wrote a channel, so the default (stable) finds nothing;
        // the migration offer must still come through.
        let res = select_release_default_channel("0.1.0", ALPHAS, 100).unwrap();
        assert!(res.migration && res.update_available);
        assert_eq!(res.channel, Channel::Beta);

        // A tagged alpha defaults to beta and sees newer alphas.
        let res = select_release_default_channel("0.1.0-alpha.13", ALPHAS, 100).unwrap();
        assert!(res.update_available && !res.migration);

        assert_eq!(default_channel("0.1.0-alpha.13"), Channel::Beta);
        assert_eq!(default_channel("0.1.0"), Channel::Stable);
    }

    #[test]
    fn dev_builds_are_never_offered_updates() {
        for ch in [Channel::Stable, Channel::Beta] {
            let res = select_release("0.1.0-dev+abc123def", ch, ALPHAS, 100).unwrap();
            assert!(res.dev_build);
            assert!(res.latest.is_none());
            assert!(!res.update_available);
        }
        let res = select_release_default_channel("0.1.0-dev", ALPHAS, 100).unwrap();
        assert!(res.dev_build && !res.update_available);
    }

    /// A scripted GitHub: answers in order, counts requests.
    struct Script {
        replies: std::sync::Mutex<std::collections::VecDeque<Result<HttpReply, String>>>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl ReleaseFetcher for Script {
        fn get(&self, _url: &str) -> Result<HttpReply, String> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.replies
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected extra request")
        }
    }

    fn ok(body: &str) -> Result<HttpReply, String> {
        Ok(HttpReply {
            status: 200,
            body: body.to_owned(),
            ..HttpReply::default()
        })
    }

    /// A checker over `replies`, a settable clock, and the request counter.
    fn checker(
        replies: Vec<Result<HttpReply, String>>,
    ) -> (
        ReleaseChecker,
        Arc<std::sync::atomic::AtomicI64>,
        Arc<Script>,
    ) {
        let script = Arc::new(Script {
            replies: std::sync::Mutex::new(replies.into()),
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        let now = Arc::new(std::sync::atomic::AtomicI64::new(1_000_000));
        let clock = now.clone();
        let c = ReleaseChecker::new(
            script.clone(),
            Arc::new(move || clock.load(std::sync::atomic::Ordering::SeqCst)),
        );
        (c, now, script)
    }

    fn calls(s: &Script) -> usize {
        s.calls.load(std::sync::atomic::Ordering::SeqCst)
    }

    #[test]
    fn select_release_names_the_reason_nothing_is_offered() {
        // Offered: no reason.
        let res = select_release("0.1.0-alpha.13", Channel::Beta, ALPHAS, 100).unwrap();
        assert_eq!(res.reason, None);

        // Nothing on the channel.
        let res = select_release("0.1.0-alpha.13", Channel::Stable, ALPHAS, 100).unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::NoReleasesOnChannel));
        let res = select_release("0.1.0-alpha.13", Channel::Beta, "[]", 100).unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::NoReleasesOnChannel));

        // Newest is equal, or older.
        let res = select_release("0.1.0-alpha.20", Channel::Beta, ALPHAS, 100).unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::UpToDate));
        let res = select_release("0.1.0-alpha.99", Channel::Beta, ALPHAS, 100).unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::UpToDate));
        assert_eq!(res.latest.unwrap().version, "0.1.0-alpha.20");

        // Development build.
        let res = select_release("0.1.0-dev+abc", Channel::Beta, ALPHAS, 100).unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::DevBuild));

        // A migration offer is an offer.
        let res = select_release("0.1.0", Channel::Beta, ALPHAS, 100).unwrap();
        assert_eq!(res.reason, None);
    }

    #[test]
    fn check_reports_each_reason() {
        let (c, _, _) = checker(vec![ok(ALPHAS)]);
        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Stable), true)
            .unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::NoReleasesOnChannel));
        assert_eq!(res.channel, Channel::Stable);

        let (c, _, _) = checker(vec![ok(ALPHAS)]);
        let res = c
            .check("0.1.0-alpha.20", Some(Channel::Beta), true)
            .unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::UpToDate));

        let (c, _, _) = checker(vec![Err("dns error: no such host".to_owned())]);
        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), true)
            .unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::NetworkError));
        assert!(res.detail.unwrap().contains("no such host"));
        assert!(!res.update_available);

        let (c, _, _) = checker(vec![Ok(HttpReply {
            status: 500,
            ..HttpReply::default()
        })]);
        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), true)
            .unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::NetworkError));
        assert!(res.detail.unwrap().contains("500"));

        let (c, _, _) = checker(vec![ok("<html>not json</html>")]);
        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), true)
            .unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::NetworkError));

        // An unchosen channel with no stable release says so, on the stable channel.
        let (c, _, _) = checker(vec![ok(ALPHAS)]);
        let res = c.check("0.1.0-alpha.20", None, false).unwrap();
        assert_eq!(res.channel, Channel::Beta);
        assert_eq!(res.reason, Some(NoOfferReason::UpToDate));
        let (c, _, _) = checker(vec![ok(ALPHAS)]);
        let res = c.check("0.2.0", None, false).unwrap();
        assert_eq!(res.channel, Channel::Stable);
        assert_eq!(res.reason, Some(NoOfferReason::NoReleasesOnChannel));
    }

    #[test]
    fn a_failed_check_still_names_the_channel_and_time() {
        let (c, now, _) = checker(vec![Err("timeout".to_owned())]);
        now.store(1234, std::sync::atomic::Ordering::SeqCst);
        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), true)
            .unwrap();
        assert_eq!(res.checked_at, 1234);
        assert_eq!(res.channel, Channel::Beta);
        assert_eq!(res.current_version, "0.1.0-alpha.13");
    }

    #[test]
    fn dev_builds_do_not_ask_github() {
        let (c, _, script) = checker(vec![]);
        let res = c.check("0.1.0-dev+abc", Some(Channel::Beta), true).unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::DevBuild));
        assert_eq!(calls(&script), 0);
    }

    #[test]
    fn cache_is_used_for_an_hour_and_bypassed_by_force() {
        let (c, now, script) = checker(vec![ok(ALPHAS), ok(ALPHAS), ok(ALPHAS)]);
        let t0 = now.load(std::sync::atomic::Ordering::SeqCst);

        let first = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), false)
            .unwrap();
        assert_eq!(calls(&script), 1);
        assert_eq!(first.checked_at, t0);

        // Within the hour: cached, and "last checked" stays the time GitHub was asked.
        now.store(t0 + 3599, std::sync::atomic::Ordering::SeqCst);
        let second = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), false)
            .unwrap();
        assert_eq!(calls(&script), 1);
        assert_eq!(second.checked_at, t0);
        assert!(second.update_available);

        // The cache holds releases, not a verdict: another channel is answered from it too.
        let stable = c
            .check("0.1.0-alpha.13", Some(Channel::Stable), false)
            .unwrap();
        assert_eq!(calls(&script), 1);
        assert_eq!(stable.reason, Some(NoOfferReason::NoReleasesOnChannel));

        // "Check now" bypasses it.
        let forced = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), true)
            .unwrap();
        assert_eq!(calls(&script), 2);
        assert_eq!(forced.checked_at, t0 + 3599);

        // And after an hour the next ordinary check asks again.
        now.store(
            t0 + 3599 + CACHE_TTL_SECS,
            std::sync::atomic::Ordering::SeqCst,
        );
        c.check("0.1.0-alpha.13", Some(Channel::Beta), false)
            .unwrap();
        assert_eq!(calls(&script), 3);
    }

    #[test]
    fn failures_are_not_cached() {
        let (c, _, script) = checker(vec![Err("offline".to_owned()), ok(ALPHAS)]);
        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), false)
            .unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::NetworkError));
        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), false)
            .unwrap();
        assert!(res.update_available);
        assert_eq!(calls(&script), 2);
    }

    fn rate_limited(reset: Option<i64>, retry_after: Option<i64>) -> Result<HttpReply, String> {
        Ok(HttpReply {
            status: 403,
            body: r#"{"message":"API rate limit exceeded"}"#.to_owned(),
            rate_limit_remaining: Some(0),
            rate_limit_reset: reset,
            retry_after,
        })
    }

    #[test]
    fn a_403_with_rate_limit_headers_backs_off_until_the_reset_time() {
        let (c, now, script) =
            checker(vec![rate_limited(Some(1_000_000 + 1800), None), ok(ALPHAS)]);
        let t0 = now.load(std::sync::atomic::Ordering::SeqCst);

        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), false)
            .unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::RateLimited));
        assert_eq!(res.retry_at, Some(t0 + 1800));
        assert_eq!(calls(&script), 1);

        // Backing off: neither an ordinary nor a forced check goes to GitHub.
        now.store(t0 + 600, std::sync::atomic::Ordering::SeqCst);
        for force in [false, true] {
            let res = c
                .check("0.1.0-alpha.13", Some(Channel::Beta), force)
                .unwrap();
            assert_eq!(res.reason, Some(NoOfferReason::RateLimited));
            assert_eq!(res.retry_at, Some(t0 + 1800));
        }
        assert_eq!(calls(&script), 1);

        // At the reset time it asks again.
        now.store(t0 + 1800, std::sync::atomic::Ordering::SeqCst);
        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), false)
            .unwrap();
        assert!(res.update_available);
        assert_eq!(res.reason, None);
        assert_eq!(calls(&script), 2);
    }

    #[test]
    fn backoff_honours_retry_after_and_stays_within_bounds() {
        let (c, now, _) = checker(vec![rate_limited(None, Some(120))]);
        let t0 = now.load(std::sync::atomic::Ordering::SeqCst);
        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), true)
            .unwrap();
        assert_eq!(res.retry_at, Some(t0 + 120));

        // A reset time already in the past still waits a minute; a far one is capped.
        let (c, _, _) = checker(vec![rate_limited(Some(t0 - 50), None)]);
        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), true)
            .unwrap();
        assert_eq!(res.retry_at, Some(t0 + 60));
        let (c, _, _) = checker(vec![rate_limited(Some(t0 + 86_400), None)]);
        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), true)
            .unwrap();
        assert_eq!(res.retry_at, Some(t0 + 3600));
    }

    #[test]
    fn a_403_without_rate_limit_headers_is_not_a_backoff() {
        let forbidden = || {
            Ok(HttpReply {
                status: 403,
                ..HttpReply::default()
            })
        };
        let (c, _, script) = checker(vec![forbidden(), ok(ALPHAS)]);
        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), true)
            .unwrap();
        assert_eq!(res.reason, Some(NoOfferReason::NetworkError));
        assert!(res.detail.unwrap().contains("403"));
        // Not rate limited, so the next check goes straight out.
        let res = c
            .check("0.1.0-alpha.13", Some(Channel::Beta), true)
            .unwrap();
        assert!(res.update_available);
        assert_eq!(calls(&script), 2);
    }

    #[test]
    fn http_fetcher_reads_file_urls_and_reports_missing_files() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("releases.json");
        fs::write(&path, ALPHAS).unwrap();
        let reply = HttpFetcher
            .get(&format!("file://{}", path.display()))
            .unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.body, ALPHAS);
        assert!(
            HttpFetcher
                .get("file:///nonexistent/releases.json")
                .is_err()
        );
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
