//! Pure, deterministic watchdog decision engine (spec 7.15, 7.16).
//!
//! Tracks failures in a 10-minute rolling window per unit, triggers automatic disk
//! cleanup when disk exhaustion causes repeated failures, takes over post-update
//! verification + automatic rollback, and surfaces safe-mode alert state.

use std::collections::VecDeque;

use crate::slots::UpdateState;

pub const FAILURE_WINDOW_SECS: i64 = 10 * 60;
pub const FAILURE_THRESHOLD: usize = 3;
pub const RESTART_COOLDOWN_SECS: i64 = 10;
pub const DEGRADED_RESTART_COOLDOWN_SECS: i64 = 60;
pub const CLEANUP_COOLDOWN_SECS: i64 = 10 * 60;
/// Wait briefly right after `self_update_install` triggers `opsbox-restart`.
pub const UPDATE_RESTART_GRACE_SECS: i64 = 15;
/// Confirm an update healthy after 2 minutes (`settings.js` / prompt 03).
pub const UPDATE_VERIFY_SECS: i64 = 120;
/// Still attribute failures to a recent update during the 5-minute window (spec 7.15).
pub const POST_UPDATE_ROLLBACK_WINDOW_SECS: i64 = 300;
const MAX_EVENTS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitId {
    Priv,
    Proxy,
    Serve,
}

impl UnitId {
    pub fn unit_name(self) -> &'static str {
        match self {
            Self::Priv => "opsbox-priv.service",
            Self::Proxy => "opsbox-proxy.service",
            Self::Serve => "opsbox.service",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Priv => "priv",
            Self::Proxy => "proxy",
            Self::Serve => "serve",
        }
    }
}

#[derive(Debug, Clone)]
pub struct TickInput {
    pub now: i64,
    pub serve_ok: bool,
    pub priv_ok: bool,
    pub proxy_ok: bool,
    pub proxy_port_conflict: bool,
    pub disk_critical: bool,
    pub update_state: UpdateState,
    pub previous_slot_available: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyAction {
    RestartUnit(UnitId),
    FreeSpaceAndRestart,
    RollbackToPrevious {
        to_version: String,
    },
    MarkUpdateVerified {
        to_version: String,
        from_version: String,
    },
}

#[derive(Debug, Default)]
struct UnitTracker {
    failures: VecDeque<i64>,
    last_restart_at: Option<i64>,
}

impl UnitTracker {
    fn prune(&mut self, now: i64) {
        let cutoff = now - FAILURE_WINDOW_SECS;
        while self.failures.front().is_some_and(|t| *t < cutoff) {
            self.failures.pop_front();
        }
    }

    fn record_failure(&mut self, now: i64) -> usize {
        self.prune(now);
        self.failures.push_back(now);
        self.failures.len()
    }

    fn clear(&mut self) {
        self.failures.clear();
        self.last_restart_at = None;
    }

    fn can_restart(&self, now: i64, cooldown: i64) -> bool {
        self.last_restart_at
            .is_none_or(|t| now.saturating_sub(t) >= cooldown)
    }
}

#[derive(Debug, Default)]
pub struct WatchdogPolicy {
    proxy_conflict_active: bool,
    priv_unit: UnitTracker,
    proxy_unit: UnitTracker,
    serve_unit: UnitTracker,
    last_cleanup_at: Option<i64>,
    last_rollback_from: Option<String>,
    pub safe_mode_active: bool,
    pub recent_events: VecDeque<String>,
}

impl WatchdogPolicy {
    pub fn push_event(&mut self, msg: impl Into<String>) {
        if self.recent_events.len() >= MAX_EVENTS {
            self.recent_events.pop_front();
        }
        self.recent_events.push_back(msg.into());
    }

    pub fn clear_failures(&mut self) {
        self.priv_unit.clear();
        self.proxy_unit.clear();
        self.serve_unit.clear();
        self.safe_mode_active = false;
    }

    pub fn events_vec(&self) -> Vec<String> {
        self.recent_events.iter().cloned().collect()
    }

    pub fn evaluate(&mut self, input: &TickInput) -> Vec<PolicyAction> {
        let now = input.now;
        let proxy_ok = input.proxy_ok || input.proxy_port_conflict;
        if input.proxy_port_conflict {
            self.proxy_unit.clear();
        }
        if input.proxy_port_conflict && !self.proxy_conflict_active {
            self.push_event(
                "Proxy port conflict: serving on other ports. Open Settings > Network.",
            );
        }
        self.proxy_conflict_active = input.proxy_port_conflict;
        let all_ok = input.serve_ok && input.priv_ok && proxy_ok;

        if let Some(ref outcome) = input.update_state.last_outcome
            && outcome.status == "pending"
        {
            let age = now.saturating_sub(outcome.at);
            if age < UPDATE_RESTART_GRACE_SECS {
                // Let `opsbox-restart` finish swapping units before counting failures.
                return Vec::new();
            }
            if all_ok && age >= UPDATE_VERIFY_SECS {
                self.clear_failures();
                self.push_event(format!(
                    "Update to {} passed post-update health checks.",
                    outcome.to_version
                ));
                return vec![PolicyAction::MarkUpdateVerified {
                    to_version: outcome.to_version.clone(),
                    from_version: outcome.from_version.clone(),
                }];
            }
        }

        if all_ok {
            if self.safe_mode_active {
                self.push_event("All opsbox services recovered; leaving safe mode alert state.");
            }
            self.safe_mode_active = false;
            self.priv_unit.prune(now);
            self.proxy_unit.prune(now);
            self.serve_unit.prune(now);
            return Vec::new();
        }

        let mut actions = Vec::new();
        let mut threshold_reached = false;
        let mut units_to_restart = Vec::new();

        // Check in dependency order: priv -> proxy -> serve.
        let checks = [
            (UnitId::Priv, input.priv_ok),
            (UnitId::Proxy, proxy_ok),
            (UnitId::Serve, input.serve_ok),
        ];

        for (unit, ok) in checks {
            let tracker = match unit {
                UnitId::Priv => &mut self.priv_unit,
                UnitId::Proxy => &mut self.proxy_unit,
                UnitId::Serve => &mut self.serve_unit,
            };
            if ok {
                tracker.prune(now);
                continue;
            }
            let count = tracker.record_failure(now);
            if count >= FAILURE_THRESHOLD {
                threshold_reached = true;
            }
            units_to_restart.push((unit, count));
        }

        if threshold_reached {
            let can_cleanup = input.disk_critical
                && self
                    .last_cleanup_at
                    .is_none_or(|t| now.saturating_sub(t) >= CLEANUP_COOLDOWN_SECS);
            if can_cleanup {
                self.last_cleanup_at = Some(now);
                self.priv_unit.last_restart_at = Some(now);
                self.proxy_unit.last_restart_at = Some(now);
                self.serve_unit.last_restart_at = Some(now);
                self.push_event(
                    "3 failures in 10 min with critical disk space: freeing opsbox space and restarting.",
                );
                return vec![PolicyAction::FreeSpaceAndRestart];
            }

            if let Some(target_ver) = self.should_rollback(input) {
                self.last_rollback_from = input.update_state.current_version.clone();
                self.clear_failures();
                self.push_event(format!(
                    "3 failures in 10 min after update: rolling back to {target_ver}."
                ));
                return vec![PolicyAction::RollbackToPrevious {
                    to_version: target_ver,
                }];
            }

            if !self.safe_mode_active {
                self.safe_mode_active = true;
                self.push_event(
                    "Unit failed 3 times in 10 min; entering safe mode alert state (:8444).",
                );
            }
        }

        let cooldown = if self.safe_mode_active {
            DEGRADED_RESTART_COOLDOWN_SECS
        } else {
            RESTART_COOLDOWN_SECS
        };

        for (unit, count) in units_to_restart {
            let tracker = match unit {
                UnitId::Priv => &mut self.priv_unit,
                UnitId::Proxy => &mut self.proxy_unit,
                UnitId::Serve => &mut self.serve_unit,
            };
            if tracker.can_restart(now, cooldown) {
                tracker.last_restart_at = Some(now);
                self.push_event(format!(
                    "{} health check failed ({count}/3 in 10 min): restarting {}.",
                    unit.label(),
                    unit.unit_name()
                ));
                actions.push(PolicyAction::RestartUnit(unit));
            }
        }

        actions
    }

    fn should_rollback(&self, input: &TickInput) -> Option<String> {
        let prev = input.previous_slot_available.as_ref()?;
        if let Some(ref cur) = input.update_state.current_version
            && self.last_rollback_from.as_deref() == Some(cur.as_str())
        {
            return None;
        }
        let outcome = input.update_state.last_outcome.as_ref()?;
        let age = input.now.saturating_sub(outcome.at);
        let recent_update = outcome.status == "pending"
            || (outcome.status == "updated" && age <= POST_UPDATE_ROLLBACK_WINDOW_SECS);
        recent_update.then(|| prev.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slots::UpdateOutcome;

    fn base_input(now: i64) -> TickInput {
        TickInput {
            now,
            serve_ok: true,
            priv_ok: true,
            proxy_ok: true,
            proxy_port_conflict: false,
            disk_critical: false,
            update_state: UpdateState::default(),
            previous_slot_available: None,
        }
    }

    #[test]
    fn restarts_failed_unit_and_enters_safe_mode_after_three_failures() {
        let mut policy = WatchdogPolicy::default();

        // Failure 1 -> restart serve
        let a1 = policy.evaluate(&TickInput {
            serve_ok: false,
            ..base_input(100)
        });
        assert_eq!(a1, vec![PolicyAction::RestartUnit(UnitId::Serve)]);
        assert!(!policy.safe_mode_active);

        // Failure 2 -> restart serve
        let a2 = policy.evaluate(&TickInput {
            serve_ok: false,
            ..base_input(110)
        });
        assert_eq!(a2, vec![PolicyAction::RestartUnit(UnitId::Serve)]);
        assert!(!policy.safe_mode_active);

        // Failure 3 -> enters safe mode alert state (no disk full, no recent update)
        let a3 = policy.evaluate(&TickInput {
            serve_ok: false,
            ..base_input(120)
        });
        assert!(policy.safe_mode_active);
        // Backoff prevents immediate restart on tick 3 because last restart was 10s ago (< 60s).
        assert!(a3.is_empty());

        // After 60s backoff, it tries once more while staying in safe mode.
        let a4 = policy.evaluate(&TickInput {
            serve_ok: false,
            ..base_input(180)
        });
        assert_eq!(a4, vec![PolicyAction::RestartUnit(UnitId::Serve)]);
        assert!(policy.safe_mode_active);

        // Recovery clears safe_mode_active.
        let ok = policy.evaluate(&base_input(190));
        assert!(ok.is_empty());
        assert!(!policy.safe_mode_active);
    }

    #[test]
    fn frees_opsbox_space_first_when_disk_is_critical() {
        let mut policy = WatchdogPolicy::default();
        for t in [100, 110] {
            let _ = policy.evaluate(&TickInput {
                serve_ok: false,
                disk_critical: true,
                ..base_input(t)
            });
        }
        let a3 = policy.evaluate(&TickInput {
            serve_ok: false,
            disk_critical: true,
            previous_slot_available: Some("0.1.3".into()),
            update_state: UpdateState {
                current_version: Some("0.1.4".into()),
                previous_version: Some("0.1.3".into()),
                last_outcome: Some(UpdateOutcome {
                    status: "pending".into(),
                    from_version: "0.1.3".into(),
                    to_version: "0.1.4".into(),
                    message: "pending".into(),
                    at: 10,
                    audited: false,
                }),
            },
            ..base_input(120)
        });
        assert_eq!(a3, vec![PolicyAction::FreeSpaceAndRestart]);
    }

    #[test]
    fn rolls_back_when_three_failures_begin_after_update() {
        let mut policy = WatchdogPolicy::default();
        let state = UpdateState {
            current_version: Some("0.1.4".into()),
            previous_version: Some("0.1.3".into()),
            last_outcome: Some(UpdateOutcome {
                status: "pending".into(),
                from_version: "0.1.3".into(),
                to_version: "0.1.4".into(),
                message: "pending".into(),
                at: 50,
                audited: false,
            }),
        };

        // Inside 15s grace period -> waits without counting failure.
        assert!(
            policy
                .evaluate(&TickInput {
                    serve_ok: false,
                    update_state: state.clone(),
                    previous_slot_available: Some("0.1.3".into()),
                    ..base_input(60)
                })
                .is_empty()
        );

        for t in [70, 80] {
            let acts = policy.evaluate(&TickInput {
                serve_ok: false,
                update_state: state.clone(),
                previous_slot_available: Some("0.1.3".into()),
                ..base_input(t)
            });
            assert_eq!(acts, vec![PolicyAction::RestartUnit(UnitId::Serve)]);
        }

        let a3 = policy.evaluate(&TickInput {
            serve_ok: false,
            update_state: state,
            previous_slot_available: Some("0.1.3".into()),
            ..base_input(90)
        });
        assert_eq!(
            a3,
            vec![PolicyAction::RollbackToPrevious {
                to_version: "0.1.3".into()
            }]
        );
    }

    #[test]
    fn marks_pending_update_verified_after_two_minutes_healthy() {
        let mut policy = WatchdogPolicy::default();
        let state = UpdateState {
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
        };

        assert!(
            policy
                .evaluate(&TickInput {
                    update_state: state.clone(),
                    ..base_input(200)
                })
                .is_empty()
        );

        let acts = policy.evaluate(&TickInput {
            update_state: state,
            ..base_input(220)
        });
        assert_eq!(
            acts,
            vec![PolicyAction::MarkUpdateVerified {
                to_version: "0.1.4".into(),
                from_version: "0.1.3".into()
            }]
        );
    }
}

#[cfg(test)]
mod conflict_tests {
    use super::*;
    #[test]
    fn conflict_is_healthy_even_during_update_verification() {
        let mut policy = WatchdogPolicy::default();
        let mut input = TickInput {
            now: 1,
            serve_ok: true,
            priv_ok: true,
            proxy_ok: false,
            proxy_port_conflict: true,
            disk_critical: false,
            update_state: UpdateState::default(),
            previous_slot_available: Some("old".into()),
        };
        for at in 1..1000 {
            input.now = at;
            assert!(policy.evaluate(&input).is_empty());
            assert!(policy.proxy_unit.failures.is_empty());
            assert!(!policy.safe_mode_active);
        }
        assert_eq!(policy.events_vec().len(), 1);
        input.update_state.last_outcome = Some(crate::slots::UpdateOutcome {
            status: "pending".into(),
            from_version: "old".into(),
            to_version: "new".into(),
            message: String::new(),
            at: 0,
            audited: false,
        });
        assert!(matches!(
            policy.evaluate(&input).as_slice(),
            [PolicyAction::MarkUpdateVerified { .. }]
        ));
        input.update_state = UpdateState::default();
        input.proxy_port_conflict = false;
        for at in [1000, 1011, 1022] {
            input.now = at;
            policy.evaluate(&input);
        }
        assert!(policy.safe_mode_active);
    }
}
