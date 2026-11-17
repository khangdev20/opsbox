//! Read-only watchdog status reader for `opsbox-priv`.

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use opsbox_proto::{WatchdogCheck, WatchdogStatus};
use opsbox_update::SlotLayout;

use super::exec::{find, run};

const STATUS_PATH: &str = "/run/opsbox-watchdog/status.json";
const STALE_AFTER_SECS: i64 = 45;

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

fn systemd_watchdog_active() -> Option<bool> {
    if !Path::new("/run/systemd/system").is_dir() {
        return None;
    }
    let ctl = find(&["/usr/bin/systemctl", "/bin/systemctl"])?;
    Some(
        run(
            &ctl,
            &["is-active", "opsbox-watchdog.service"],
            Duration::from_secs(3),
        )
        .is_some_and(|o| o.stdout.trim() == "active"),
    )
}

pub(super) fn status() -> WatchdogStatus {
    status_from_path(
        Path::new(STATUS_PATH),
        systemd_watchdog_active(),
        now_secs(),
    )
}

pub fn status_from_path(path: &Path, systemd_active: Option<bool>, now: i64) -> WatchdogStatus {
    let parsed = std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<WatchdogStatus>(&s).ok());

    if let Some(mut st) = parsed {
        let fresh = now.saturating_sub(st.updated_at) <= STALE_AFTER_SECS;
        st.running = systemd_active.unwrap_or(true) && fresh;
        if !st.running {
            st.degraded = true;
            st.checks.insert(
                0,
                WatchdogCheck {
                    name: "opsbox-watchdog.service".into(),
                    status: "fail".into(),
                    state: if systemd_active == Some(false) {
                        "stopped".into()
                    } else {
                        "stale".into()
                    },
                    note: "Watchdog is not actively updating health status.".into(),
                },
            );
        }
        return st;
    }

    let layout = SlotLayout::default();
    let update_state = layout.read_state();
    let prev = update_state
        .previous_version
        .filter(|v| layout.slot_path(v).exists());
    let wd_active = systemd_active == Some(true);
    WatchdogStatus {
        running: wd_active,
        failed: false,
        degraded: !wd_active,
        updated_at: now,
        current_version: crate::VERSION.to_owned(),
        can_rollback: prev.is_some(),
        previous_version: prev,
        update_message: update_state.last_outcome.map(|o| o.message),
        checks: vec![WatchdogCheck {
            name: "opsbox-watchdog.service".into(),
            status: if wd_active { "ok" } else { "warn" }.into(),
            state: if wd_active {
                "active".into()
            } else {
                "not running".into()
            },
            note: if wd_active {
                "Waiting for first 10 s check".into()
            } else {
                "Start opsbox-watchdog.service for automatic restarts and safe mode (:8444).".into()
            },
        }],
        recent_events: Vec::new(),
        safe_mode_listen: None,
        ssh_open_until: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_status_reports_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let st = status_from_path(&dir.path().join("missing.json"), Some(false), 1000);
        assert!(!st.running);
        assert!(st.degraded);
        assert_eq!(st.checks[0].name, "opsbox-watchdog.service");
        assert_eq!(st.checks[0].status, "warn");
    }

    #[test]
    fn stale_status_is_flagged_as_not_running() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("status.json");
        let sample = WatchdogStatus {
            running: true,
            updated_at: 900,
            current_version: "0.1.0".into(),
            ..WatchdogStatus::default()
        };
        std::fs::write(&path, serde_json::to_vec(&sample).unwrap()).unwrap();
        let st = status_from_path(&path, Some(true), 1000);
        assert!(!st.running);
        assert!(st.degraded);
        assert_eq!(st.checks[0].state, "stale");
    }

    #[test]
    fn fresh_status_returns_running_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("status.json");
        let sample = WatchdogStatus {
            running: true,
            updated_at: 980,
            current_version: "0.1.0".into(),
            ..WatchdogStatus::default()
        };
        std::fs::write(&path, serde_json::to_vec(&sample).unwrap()).unwrap();
        let st = status_from_path(&path, Some(true), 1000);
        assert!(st.running);
        assert!(!st.degraded);
    }
}
