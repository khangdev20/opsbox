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
        ["audit", "schema_migrations", "settings"]
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
