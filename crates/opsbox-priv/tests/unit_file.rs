//! The systemd unit for `priv` keeps it alive and out of the OOM killer's way: on a small
//! server, a killed helper pauses every action that changes the server.

#[test]
fn the_unit_restarts_always_and_resists_the_oom_killer() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packaging/systemd/opsbox-priv.service"
    );
    let unit = std::fs::read_to_string(path).unwrap();
    let has = |line: &str| unit.lines().any(|l| l.trim() == line);
    assert!(has("Restart=always"), "{unit}");
    assert!(has("RestartSec=2"), "{unit}");
    assert!(has("OOMScoreAdjust=-900"), "{unit}");
    assert!(has("StartLimitIntervalSec=0"), "{unit}");
}
