use opsbox_alerts::{Engine, Observation, Quiet, REPEAT_SECS, State};
use std::collections::BTreeSet;
const NOON: i64 = 12 * 3600;
fn disk(target: &str, critical: bool) -> Observation {
    Observation::new(
        "disk",
        target,
        "Disk needs space.".into(),
        "#/cleanup",
        critical,
    )
}
fn known() -> BTreeSet<String> {
    BTreeSet::from(["disk".into()])
}
fn fire(e: &mut Engine) {
    e.evaluate(vec![disk("/", false)], &known(), NOON);
    e.evaluate(vec![disk("/", false)], &known(), NOON + 60);
}
#[test]
fn duration_cancellation_dedupe_repeat_and_restart() {
    let mut e = Engine::default();
    e.evaluate(vec![disk("/", false)], &known(), NOON);
    assert_eq!(e.incidents["disk:/"].state, State::Pending);
    assert!(!e.due("disk:/", "c1", NOON));
    e.evaluate(vec![], &known(), NOON + 30);
    assert_eq!(e.incidents["disk:/"].state, State::Ok);
    fire(&mut e);
    assert!(e.due("disk:/", "c1", NOON + 60));
    e.delivered("disk:/", "c1", NOON + 60);
    assert!(!e.due("disk:/", "c1", NOON + 120));
    let e: Engine = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
    assert!(!e.due("disk:/", "c1", NOON + 60 + REPEAT_SECS - 1));
    assert!(e.due("disk:/", "c1", NOON + 60 + REPEAT_SECS));
    assert!(e.due("disk:/", "c2", NOON + 120));
}
#[test]
fn unknown_sources_never_announce_recovery() {
    let mut e = Engine::default();
    fire(&mut e);
    e.evaluate(vec![], &BTreeSet::new(), NOON + 100);
    assert_eq!(e.incidents["disk:/"].state, State::Firing);
    e.evaluate(vec![], &known(), NOON + 120);
    assert_eq!(e.incidents["disk:/"].state, State::Resolved);
    assert!(!e.due("disk:/", "c1", NOON + 120)); // no unmatched recovery
}
#[test]
fn recovery_only_to_channels_that_received_firing_and_retry_is_independent() {
    let mut e = Engine::default();
    fire(&mut e);
    e.delivered("disk:/", "c1", NOON + 60);
    e.attempted("disk:/", "c2", NOON + 60);
    assert!(!e.due("disk:/", "c2", NOON + 119));
    assert!(e.due("disk:/", "c2", NOON + 120));
    assert!(!e.due("disk:/", "c1", NOON + 120));
    e.evaluate(vec![], &known(), NOON + 121);
    assert!(e.due("disk:/", "c1", NOON + 121));
    assert!(!e.due("disk:/", "c2", NOON + 121));
    e.delivered("disk:/", "c1", NOON + 121);
    assert!(!e.due("disk:/", "c1", NOON + 500));
}
#[test]
fn warning_waits_critical_worsening_bypasses_quiet_and_silence_expires() {
    let mut e = Engine {
        quiet: Quiet {
            from: "00:00".into(),
            to: "23:59".into(),
        },
        ..Engine::default()
    };
    fire(&mut e);
    assert!(!e.due("disk:/", "c1", NOON + 60));
    e.evaluate(vec![disk("/", true)], &known(), NOON + 75);
    assert!(e.due("disk:/", "c1", NOON + 75));
    e.incidents.get_mut("disk:/").unwrap().silence_until = NOON + 500;
    assert!(!e.due("disk:/", "c1", NOON + 499));
    assert!(e.due("disk:/", "c1", NOON + 500));
    e.delivered("disk:/", "c1", NOON + 500);
    assert!(!e.due("disk:/", "c1", NOON + 560));
}
#[test]
fn warning_flushes_when_quiet_ends_without_unmatched_recovery() {
    let mut e = Engine {
        quiet: Quiet {
            from: "11:00".into(),
            to: "13:00".into(),
        },
        ..Engine::default()
    };
    fire(&mut e);
    assert!(!e.due("disk:/", "c1", NOON + 60));
    assert!(e.due("disk:/", "c1", 13 * 3600));
    e.evaluate(vec![], &known(), NOON + 120);
    assert!(!e.due("disk:/", "c1", 13 * 3600));
}
#[test]
fn worsening_sends_before_repeat_and_targets_are_independent() {
    let mut e = Engine::default();
    fire(&mut e);
    e.delivered("disk:/", "c1", NOON + 60);
    e.evaluate(
        vec![disk("/", true), disk("/data", false)],
        &known(),
        NOON + 120,
    );
    assert!(e.due("disk:/", "c1", NOON + 120));
    assert_eq!(e.incidents["disk:/data"].state, State::Pending);
    e.evaluate(vec![disk("/data", false)], &known(), NOON + 180);
    assert_eq!(e.incidents["disk:/data"].state, State::Firing);
    assert_eq!(e.incidents["disk:/"].state, State::Resolved);
}
#[test]
fn disabling_rule_recovers_and_history_is_bounded() {
    let mut e = Engine::default();
    fire(&mut e);
    e.delivered("disk:/", "c1", NOON + 60);
    e.rules.iter_mut().find(|r| r.id == "disk").unwrap().on = false;
    e.evaluate(vec![disk("/", true)], &known(), NOON + 120);
    assert_eq!(e.incidents["disk:/"].state, State::Resolved);
    assert!(e.due("disk:/", "c1", NOON + 120));
    assert_eq!(e.rules.len(), 13);
}
#[test]
fn quiet_hours_cross_midnight_and_equal_disables() {
    let q = Quiet::default();
    assert!(q.contains(23 * 3600));
    assert!(q.contains(6 * 3600));
    assert!(!q.contains(7 * 3600));
    assert!(
        !Quiet {
            from: "00:00".into(),
            to: "00:00".into()
        }
        .contains(0)
    );
    for s in ["24:00", "-1:00", "00:60", "9:00", "00:0", "xx:yy"] {
        assert!(Quiet::minute(s).is_none(), "{s}");
    }
}
