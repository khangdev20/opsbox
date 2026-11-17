//! Migration and persistence behaviour of the store, through its public API.

use opsbox_store::{LATEST_VERSION, Store};
use rusqlite::Connection;
use serde_json::json;

fn table_names(path: &std::path::Path) -> Vec<String> {
    let conn = Connection::open(path).unwrap();
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

#[test]
fn migrates_an_empty_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fresh.db");
    assert!(!path.exists());

    let store = Store::open(&path).unwrap();

    assert_eq!(store.schema_version().unwrap(), LATEST_VERSION);
    assert_eq!(store.journal_mode().unwrap(), "wal");
    assert_eq!(
        table_names(&path),
        [
            "audit",
            "login_failures",
            "metrics_hour",
            "metrics_minute",
            "schema_migrations",
            "sessions",
            "settings",
            "users"
        ]
    );
    store.ping().unwrap();
}

#[test]
fn reopening_is_idempotent_and_keeps_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("again.db");
    {
        let store = Store::open(&path).unwrap();
        store.set_setting("hostname", &json!("box-1")).unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), LATEST_VERSION);
    assert_eq!(store.get_setting("hostname").unwrap(), Some(json!("box-1")));
    let applied: i64 = Connection::open(&path)
        .unwrap()
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(applied, i64::from(LATEST_VERSION));
}

#[test]
fn refuses_a_database_from_a_newer_opsbox() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("future.db");
    drop(Store::open(&path).unwrap());
    Connection::open(&path)
        .unwrap()
        .execute(
            "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?1, 'from_the_future', 'x')",
            [LATEST_VERSION + 1],
        )
        .unwrap();
    let err = Store::open(&path).err().expect("must refuse").to_string();
    assert!(err.contains("newer database"), "{err}");
}

#[test]
fn settings_upsert_replaces_the_value() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.get_setting("missing").unwrap(), None);
    store
        .set_setting("panel", &json!({"enabled": false}))
        .unwrap();
    store
        .set_setting("panel", &json!({"enabled": true}))
        .unwrap();
    assert_eq!(
        store.get_setting("panel").unwrap(),
        Some(json!({"enabled": true}))
    );
}

#[cfg(unix)]
#[test]
fn database_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.db");
    let store = Store::open(&path).unwrap();
    store.set_setting("k", &json!(1)).unwrap();
    for f in ["private.db", "private.db-wal", "private.db-shm"] {
        let p = dir.path().join(f);
        if let Ok(meta) = std::fs::metadata(&p) {
            assert_eq!(
                meta.permissions().mode() & 0o077,
                0,
                "{f} is accessible to others"
            );
        }
    }
}

// ------------------------------------------------------------ accounts (migration 0002)

use opsbox_proto::Role;
use opsbox_store::SessionKind;

#[test]
fn migration_two_applies_on_top_of_an_existing_version_one_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("v1.db");
    // Build a database that stopped at version 1, as a Phase 0 install would be.
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../migrations/0001_initial.sql"))
            .unwrap();
        conn.execute_batch(
            "CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL, applied_at TEXT NOT NULL) STRICT;
             INSERT INTO schema_migrations VALUES (1, 'initial', 'x');
             INSERT INTO settings (key, value, created_at, updated_at) VALUES ('keep', '1', 'x', 'x');",
        )
        .unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), LATEST_VERSION);
    assert_eq!(store.get_setting("keep").unwrap(), Some(json!(1)));
    store.create_user("alice", Role::Admin, "hash", 1).unwrap();
}

#[test]
fn users_are_unique_case_insensitively_and_roles_round_trip() {
    let store = Store::open_in_memory().unwrap();
    let u = store.create_user("Alice", Role::Deployer, "h", 10).unwrap();
    assert_eq!(
        (u.role, u.totp_enabled, u.totp_secret.clone()),
        (Role::Deployer, false, None)
    );
    assert!(store.create_user("alice", Role::Viewer, "h", 11).is_err());
    assert_eq!(store.user_by_name("ALICE").unwrap().unwrap().id, u.id);
    assert!(store.user_by_name("bob").unwrap().is_none());
    assert_eq!(store.list_users().unwrap().len(), 1);
}

#[test]
fn setup_state_tracks_enrolled_admins_only() {
    let store = Store::open_in_memory().unwrap();
    assert_eq!(store.enrolled_admin_count().unwrap(), 0);
    let admin = store.create_user("admin", Role::Admin, "h", 1).unwrap();
    let viewer = store.create_user("v", Role::Viewer, "h", 1).unwrap();
    store.set_pending_totp(&admin.id, "enc:v1:a:b", 2).unwrap();
    assert_eq!(
        store.enrolled_admin_count().unwrap(),
        0,
        "pending enrolment is not an admin yet"
    );
    store.enable_totp(&viewer.id, 5, 3).unwrap(); // no secret: must not enable
    assert!(!store.user_by_id(&viewer.id).unwrap().unwrap().totp_enabled);
    store.enable_totp(&admin.id, 7, 3).unwrap();
    assert_eq!(store.enrolled_admin_count().unwrap(), 1);
    let u = store.user_by_id(&admin.id).unwrap().unwrap();
    assert!(u.totp_enabled && u.totp_last_step == 7);
}

#[test]
fn abandoned_setup_is_cleaned_up_without_touching_real_admins() {
    let store = Store::open_in_memory().unwrap();
    let pending = store.create_user("pending", Role::Admin, "h", 1).unwrap();
    store.create_user("viewer", Role::Viewer, "h", 1).unwrap();
    assert_eq!(store.delete_unenrolled_admins().unwrap(), 1);
    assert!(store.user_by_id(&pending.id).unwrap().is_none());
    assert!(store.user_by_name("viewer").unwrap().is_some());
}

#[test]
fn a_totp_step_can_only_be_consumed_once() {
    let store = Store::open_in_memory().unwrap();
    let u = store.create_user("a", Role::Admin, "h", 1).unwrap();
    assert!(store.advance_totp_step(&u.id, 100).unwrap());
    assert!(
        !store.advance_totp_step(&u.id, 100).unwrap(),
        "same step twice"
    );
    assert!(!store.advance_totp_step(&u.id, 99).unwrap(), "older step");
    assert!(store.advance_totp_step(&u.id, 101).unwrap());
}

#[test]
fn sessions_are_found_by_hash_and_cascade_with_the_user() {
    let store = Store::open_in_memory().unwrap();
    let u = store.create_user("a", Role::Admin, "h", 1).unwrap();
    let s = store
        .create_session(
            &u.id,
            "hash-1",
            SessionKind::Web,
            Some("203.0.113.5"),
            Some("curl"),
            100,
        )
        .unwrap();
    store
        .create_session(&u.id, "hash-2", SessionKind::Enroll, None, None, 100)
        .unwrap();
    let (found, user) = store.session_by_token_hash("hash-1").unwrap().unwrap();
    assert_eq!(
        (found.id.as_str(), found.kind, user.name.as_str()),
        (s.id.as_str(), SessionKind::Web, "a")
    );
    assert_eq!(
        store
            .session_by_token_hash("hash-2")
            .unwrap()
            .unwrap()
            .0
            .kind,
        SessionKind::Enroll
    );
    assert!(store.session_by_token_hash("nope").unwrap().is_none());

    store.touch_session(&s.id, 250).unwrap();
    assert_eq!(
        store
            .session_by_token_hash("hash-1")
            .unwrap()
            .unwrap()
            .0
            .last_seen,
        250
    );

    assert_eq!(store.delete_user_sessions(&u.id).unwrap(), 2);
    assert!(store.session_by_token_hash("hash-1").unwrap().is_none());
}

#[test]
fn expired_sessions_are_purged_by_either_limit() {
    let store = Store::open_in_memory().unwrap();
    let u = store.create_user("a", Role::Admin, "h", 1).unwrap();
    store
        .create_session(&u.id, "idle", SessionKind::Web, None, None, 1000)
        .unwrap();
    store
        .create_session(&u.id, "old", SessionKind::Web, None, None, 10)
        .unwrap();
    store
        .create_session(&u.id, "fresh", SessionKind::Web, None, None, 1000)
        .unwrap();
    let idle = store.session_by_token_hash("idle").unwrap().unwrap().0;
    store.touch_session(&idle.id, 100).unwrap(); // last seen long ago
    let old = store.session_by_token_hash("old").unwrap().unwrap().0;
    store.touch_session(&old.id, 2000).unwrap(); // active, but created before the absolute limit
    // idle limit: last_seen < 500; absolute limit: created_at < 500
    assert_eq!(store.purge_sessions(500, 500).unwrap(), 2);
    assert_eq!(store.session_count(&u.id).unwrap(), 1);
    assert!(store.session_by_token_hash("fresh").unwrap().is_some());
}

#[test]
fn failures_are_counted_per_user_and_per_address_inside_a_window() {
    let store = Store::open_in_memory().unwrap();
    for t in [100, 200, 300] {
        store.record_login_failure("Alice", "1.1.1.1", t).unwrap();
    }
    store.record_login_failure("bob", "1.1.1.1", 400).unwrap();
    store.record_login_failure("alice", "2.2.2.2", 500).unwrap();
    assert_eq!(store.failures_for_user("ALICE", 0).unwrap(), (4, Some(500)));
    assert_eq!(
        store.failures_for_user("alice", 250).unwrap(),
        (2, Some(500))
    );
    assert_eq!(store.failures_for_ip("1.1.1.1", 0).unwrap(), (4, Some(400)));
    assert_eq!(store.failures_for_ip("9.9.9.9", 0).unwrap(), (0, None));
    store.clear_failures_for_user("alice").unwrap();
    assert_eq!(store.failures_for_user("alice", 0).unwrap().0, 0);
    assert_eq!(store.prune_failures(1000).unwrap(), 1);
}

// ------------------------------------------------------------ metric rollups (migration 0003)

use opsbox_store::{Granularity, MetricRow};

fn row(ts: i64, cpu: f64) -> MetricRow {
    MetricRow {
        ts,
        cpu,
        cpu_max: cpu + 10.0,
        mem: 40.0,
        disk: r#"[{"mount":"/","used":1,"total":2}]"#.into(),
        net_in: 1.0,
        net_out: 2.0,
        load: 0.5,
    }
}

#[test]
fn rollups_round_trip_replace_and_come_back_in_order() {
    let store = Store::open_in_memory().unwrap();
    store
        .put_metric(Granularity::Minute, &row(120, 20.0))
        .unwrap();
    store
        .put_metric(Granularity::Minute, &row(60, 10.0))
        .unwrap();
    store
        .put_metric(Granularity::Minute, &row(60, 11.0))
        .unwrap(); // same period: replaced
    store.put_metric(Granularity::Hour, &row(0, 5.0)).unwrap();
    let rows = store.metrics_between(Granularity::Minute, 0, 1000).unwrap();
    assert_eq!(
        rows.iter().map(|r| (r.ts, r.cpu)).collect::<Vec<_>>(),
        [(60, 11.0), (120, 20.0)]
    );
    assert_eq!(
        store
            .metrics_between(Granularity::Minute, 100, 200)
            .unwrap()
            .len(),
        1,
        "half-open range"
    );
    assert_eq!(
        store
            .metrics_between(Granularity::Hour, 0, 1)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(rows[0].disk, r#"[{"mount":"/","used":1,"total":2}]"#);
}

#[test]
fn rollups_are_purged_by_their_own_retention() {
    let store = Store::open_in_memory().unwrap();
    let now = 100 * 86_400;
    store
        .put_metric(Granularity::Minute, &row(now - 8 * 86_400, 1.0))
        .unwrap(); // older than 7 d
    store
        .put_metric(Granularity::Minute, &row(now - 6 * 86_400, 2.0))
        .unwrap();
    store
        .put_metric(Granularity::Hour, &row(now - 80 * 86_400, 3.0))
        .unwrap(); // inside 90 d
    store
        .put_metric(Granularity::Hour, &row(now - 91 * 86_400, 4.0))
        .unwrap();
    assert_eq!(store.purge_metrics(now).unwrap(), 2);
    assert_eq!(
        store
            .metrics_between(Granularity::Minute, 0, now)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .metrics_between(Granularity::Hour, 0, now)
            .unwrap()
            .len(),
        1
    );
}
