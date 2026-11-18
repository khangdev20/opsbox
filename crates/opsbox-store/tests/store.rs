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
            "api_tokens",
            "audit",
            "backup_destinations",
            "backup_jobs",
            "backup_snapshots",
            "compose_stacks",
            "deploy_keys",
            "deploy_logs",
            "deploys",
            "environments",
            "github_app",
            "github_manifest_states",
            "github_webhook_deliveries",
            "invites",
            "login_failures",
            "metrics_hour",
            "metrics_minute",
            "projects",
            "schema_migrations",
            "service_domains",
            "service_var_refs",
            "service_vars",
            "services",
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

#[test]
fn invitations_belong_to_a_user_and_go_with_them() {
    use opsbox_proto::Role;
    let store = Store::open_in_memory().unwrap();
    let u = store
        .create_user("newcomer", Role::Viewer, "", 100)
        .unwrap();
    store
        .put_invite(&u.id, "hash-1", 1000, "admin", 100)
        .unwrap();
    assert!(store.has_invite(&u.id).unwrap());
    let (inv, user) = store.invite_by_token_hash("hash-1").unwrap().unwrap();
    assert_eq!(
        (
            inv.expires_at,
            user.name.as_str(),
            user.password_hash.as_str()
        ),
        (1000, "newcomer", "")
    );
    // A new invitation replaces the old one: the old link stops working.
    store
        .put_invite(&u.id, "hash-2", 2000, "admin", 150)
        .unwrap();
    assert!(store.invite_by_token_hash("hash-1").unwrap().is_none());
    assert!(store.invite_by_token_hash("hash-2").unwrap().is_some());
    // Purging removes expired invitations and the accounts that never got past them,
    // but not an account that has a password.
    let kept = store
        .create_user("kept", Role::Viewer, "$argon2id$x", 100)
        .unwrap();
    store
        .put_invite(&kept.id, "hash-3", 500, "admin", 100)
        .unwrap();
    store
        .put_invite(&u.id, "hash-4", 500, "admin", 100)
        .unwrap();
    assert_eq!(store.purge_invites(600).unwrap(), 1);
    assert!(store.user_by_name("newcomer").unwrap().is_none());
    assert!(store.user_by_name("kept").unwrap().is_some());
    assert!(!store.has_invite(&kept.id).unwrap());
    // Deleting a user takes the invitation with it.
    let again = store.create_user("again", Role::Viewer, "", 100).unwrap();
    store
        .put_invite(&again.id, "hash-5", 9999, "admin", 100)
        .unwrap();
    assert!(store.delete_user(&again.id).unwrap());
    assert!(store.invite_by_token_hash("hash-5").unwrap().is_none());
}

#[test]
fn api_tokens_are_stored_by_hash_with_a_unique_name() {
    use opsbox_store::TokenScope;
    let store = Store::open_in_memory().unwrap();
    let t = store
        .create_api_token("ci", "h1", TokenScope::Deploy, "admin", 100)
        .unwrap();
    assert!(
        store
            .create_api_token("CI", "h2", TokenScope::Read, "admin", 100)
            .is_err(),
        "names are unique, ignoring case"
    );
    assert_eq!(
        store.api_token_by_hash("h1").unwrap().unwrap().scope,
        TokenScope::Deploy
    );
    assert!(
        store.api_token_by_hash("ci").unwrap().is_none(),
        "the token itself is never a key"
    );
    store.touch_api_token(&t.id, 200).unwrap();
    store.touch_api_token(&t.id, 230).unwrap(); // within a minute: not rewritten
    assert_eq!(store.list_api_tokens().unwrap()[0].last_used_at, Some(200));
    assert_eq!(store.delete_api_token(&t.id).unwrap().unwrap().name, "ci");
    assert!(store.list_api_tokens().unwrap().is_empty());
}

#[test]
fn read_only_store_reads_users_settings_and_checks_integrity() {
    use opsbox_proto::Role;
    use opsbox_store::ReadOnlyStore;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("ro.db");
    {
        let store = Store::open(&path).unwrap();
        store
            .create_user("Alice", Role::Admin, "$argon2id$hash", 100)
            .unwrap();
        store
            .set_setting("opsbox.panel.domain", &json!("panel.example.com"))
            .unwrap();
    }

    let ro = ReadOnlyStore::open(&path).unwrap();
    ro.check_integrity().unwrap();
    assert_eq!(ro.schema_version().unwrap(), LATEST_VERSION);
    let user = ro.user_by_name("alice").unwrap().unwrap();
    assert_eq!(user.name, "Alice");
    assert_eq!(user.role, Role::Admin);
    assert_eq!(
        ro.get_setting("opsbox.panel.domain").unwrap(),
        Some(json!("panel.example.com"))
    );
    // Does not create -shm/-wal sidecars when closed cleanly.
    assert!(!dir.path().join("ro.db-shm").exists());
}

#[test]
fn read_only_store_detects_corruption_and_missing_files() {
    use opsbox_store::ReadOnlyStore;

    let dir = tempfile::tempdir().unwrap();
    assert!(ReadOnlyStore::open(&dir.path().join("missing.db")).is_err());

    let corrupt = dir.path().join("corrupt.db");
    std::fs::write(&corrupt, b"not a sqlite database at all").unwrap();
    let check = ReadOnlyStore::open(&corrupt).and_then(|ro| {
        ro.check_integrity()
            .map_err(|e| opsbox_store::StoreError::Io(std::io::Error::other(e)))
    });
    assert!(check.is_err());
}

#[test]
fn github_app_manifest_states_deploy_keys_and_webhooks_round_trip() {
    use opsbox_store::{NewDeployKey, NewGithubApp, NewWebhookDelivery};

    let store = Store::open_in_memory().unwrap();
    store
        .save_github_manifest_state("st-1", Some("acme-org"), "admin", 500, 100)
        .unwrap();
    // Expired state returns None.
    assert!(
        store
            .consume_github_manifest_state("st-1", 600)
            .unwrap()
            .is_none()
    );
    // Live state consumes atomically once.
    store
        .save_github_manifest_state("st-2", None, "admin", 500, 100)
        .unwrap();
    let consumed = store
        .consume_github_manifest_state("st-2", 200)
        .unwrap()
        .unwrap();
    assert_eq!(consumed.created_by, "admin");
    assert!(consumed.org.is_none());
    assert!(
        store
            .consume_github_manifest_state("st-2", 200)
            .unwrap()
            .is_none()
    );

    store
        .upsert_github_app(&NewGithubApp {
            app_id: 4242,
            slug: "opsbox-srv".into(),
            name: "opsbox srv".into(),
            owner_login: "acme".into(),
            owner_type: "Organization".into(),
            html_url: "https://github.com/organizations/acme/settings/apps/opsbox-srv".into(),
            private_key_enc: "enc:v1:key".into(),
            webhook_secret_enc: "enc:v1:wh".into(),
            webhook_url: Some("https://panel.example.com/api/v1/github/webhook".into()),
            webhook_active: true,
            installation_id: None,
            installation_account: None,
            installation_html_url: None,
            repository_selection: None,
            created_by: "admin".into(),
            now: 100,
        })
        .unwrap();

    let app = store.get_github_app().unwrap().unwrap();
    assert_eq!(app.app_id, 4242);
    assert!(app.webhook_active);
    let dbg = format!("{app:?}");
    assert!(!dbg.contains("enc:v1:key") && !dbg.contains("enc:v1:wh"));

    assert!(
        store
            .update_github_installation(
                Some(999),
                Some("acme"),
                Some("https://github.com/organizations/acme/settings/installations/999"),
                Some("selected"),
                200
            )
            .unwrap()
    );
    let id = store
        .record_github_webhook_delivery(&NewWebhookDelivery {
            delivery_id: Some("deliv-1".into()),
            event: "push".into(),
            repo: Some("acme/shop".into()),
            git_ref: Some("refs/heads/main".into()),
            commit_sha: Some("abc1234".into()),
            pusher: Some("alice".into()),
            received_at: 300,
        })
        .unwrap();
    assert!(!id.is_empty());
    let app = store.get_github_app().unwrap().unwrap();
    assert_eq!(app.installation_id, Some(999));
    assert_eq!(app.last_webhook_at, Some(300));
    assert_eq!(app.last_webhook_event.as_deref(), Some("push"));
    assert_eq!(store.list_github_webhook_deliveries(10).unwrap().len(), 1);

    store
        .create_deploy_key(&NewDeployKey {
            id: "dk-1".into(),
            repo: "acme/secret".into(),
            ssh_url: "git@github.com:acme/secret.git".into(),
            public_key: "ssh-ed25519 AAAA test".into(),
            fingerprint: "SHA256:abc".into(),
            private_key_enc: "enc:v1:priv".into(),
            created_by: "admin".into(),
            created_at: 300,
        })
        .unwrap();
    assert_eq!(store.list_deploy_keys().unwrap().len(), 1);
    assert_eq!(
        store
            .get_deploy_key("dk-1")
            .unwrap()
            .unwrap()
            .private_key_enc,
        "enc:v1:priv"
    );
    assert!(store.delete_deploy_key("dk-1").unwrap());
    assert!(store.list_deploy_keys().unwrap().is_empty());
    assert!(store.delete_github_app().unwrap());
    assert!(store.get_github_app().unwrap().is_none());
}

#[test]
fn deploy_pipeline_tables_switch_fail_rollback_and_prune() {
    use opsbox_proto::Role;
    use opsbox_store::{
        DeployStatus, DeployTrigger, NewService, ServiceKind, ServiceState, SourceKind, TokenScope,
        UpdateService,
    };

    let store = Store::open_in_memory().unwrap();
    let owner = store
        .create_user("deployer1", Role::Deployer, "", 10)
        .unwrap();
    let (proj, env) = store.resolve_project("default", 100).unwrap().unwrap();
    assert_eq!(proj.name, "default");
    assert_eq!(env.name, "production");

    // Per-service token scope round-trip works after migration 0006.
    assert!(TokenScope::parse("deploy:Bad!").is_none());
    assert!(TokenScope::parse("deploy:").is_none());
    let tok = store
        .create_api_token(
            "ci-web",
            "hash-web",
            TokenScope::DeployService("web".into()),
            "admin",
            100,
        )
        .unwrap();
    assert_eq!(
        store.api_token_by_hash("hash-web").unwrap().unwrap().scope,
        TokenScope::DeployService("web".into())
    );
    assert_eq!(tok.scope.service_name(), Some("web"));

    let svc = store
        .create_service(NewService {
            project_id: &proj.id,
            env_id: &env.id,
            name: "web",
            kind: ServiceKind::App,
            owner_id: Some(&owner.id),
            source_kind: SourceKind::Github,
            repo: "acme/web",
            branch: "main",
            build_method: "auto",
            dockerfile_path: None,
            start_cmd: None,
            port: 3000,
            health_path: "/healthz",
            domain: Some("web.example.com"),
            image_ref: None,
            deploy_key_id: None,
            auto_deploy: true,
            memory_limit_mb: Some(512),
            cpu_limit_millis: Some(1000),
            graph_x: 10,
            graph_y: 20,
            note: None,
            now: 100,
        })
        .unwrap();
    assert_eq!(svc.state, ServiceState::Idle);
    assert_eq!(store.get_service("web").unwrap().unwrap().id, svc.id);
    assert_eq!(store.list_services_by_owner(&owner.id).unwrap().len(), 1);

    let updated = store
        .update_service(
            &svc.id,
            UpdateService {
                port: Some(8080),
                auto_deploy: Some(false),
                now: 105,
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(updated.port, 8080);
    assert!(!updated.auto_deploy);

    store
        .upsert_service_var(&svc.id, "DATABASE_URL", "enc:v1:abc", true, 110)
        .unwrap();
    store
        .upsert_service_var(&svc.id, "DATABASE_URL", "enc:v1:xyz", true, 111)
        .unwrap();
    let vars = store.list_service_vars(&svc.id).unwrap();
    assert_eq!(vars.len(), 1);
    assert_eq!(vars[0].value_enc, "enc:v1:xyz");
    assert!(vars[0].is_secret);

    // Run 6 successful deploys and verify atomic switch + retention (keep = 5).
    for i in 1..=6_i64 {
        let dep = store
            .create_deploy(
                &svc.id,
                Some(&format!("deadbeef{i}")),
                &format!("deploy {i}"),
                DeployTrigger::Manual,
                "deployer1",
                200 + i,
            )
            .unwrap();
        store
            .append_deploy_logs(&dep.id, &[(200 + i, format!("step {i}"))])
            .unwrap();
        let prev = store
            .complete_deploy_switch(
                &svc.id,
                &dep.id,
                &format!("opsbox/web:{i}"),
                &format!("cid-{i}"),
                u16::try_from(40000 + i).unwrap(),
                Some("dockerfile"),
                205 + i,
            )
            .unwrap();
        if i == 1 {
            assert_eq!(prev, None);
        } else {
            assert_eq!(prev.as_deref(), Some(format!("cid-{}", i - 1)).as_deref());
        }
    }

    let svc_live = store.get_service(&svc.id).unwrap().unwrap();
    assert_eq!(svc_live.state, ServiceState::Active);
    assert_eq!(svc_live.active_container.as_deref(), Some("cid-6"));
    assert_eq!(svc_live.active_host_port, Some(40006));
    assert_eq!(svc_live.build_method, "dockerfile");
    assert_eq!(
        store.list_active_service_domains().unwrap(),
        vec![("web.example.com".into(), 40006)]
    );

    let deploys = store.list_deploys(&svc.id, 20).unwrap();
    assert_eq!(deploys.len(), 6);
    assert_eq!(deploys[0].status, DeployStatus::Live);
    assert!(
        deploys[1..]
            .iter()
            .all(|d| d.status == DeployStatus::Replaced)
    );
    assert_eq!(store.list_deploy_logs(&deploys[0].id, 10).unwrap().len(), 1);

    let prune = store.prune_old_deploy_image_refs(&svc.id, 5).unwrap();
    assert_eq!(prune, vec!["opsbox/web:1".to_owned()]);

    // A 7th deploy fails before switch -> old container cid-6 stays active & state remains Active!
    let bad = store
        .create_deploy(
            &svc.id,
            None,
            "bad build",
            DeployTrigger::Manual,
            "deployer1",
            300,
        )
        .unwrap();
    store
        .fail_deploy(&svc.id, &bad.id, "Docker build failed.", 305)
        .unwrap();
    let after_fail = store.get_service(&svc.id).unwrap().unwrap();
    assert_eq!(after_fail.state, ServiceState::Active);
    assert_eq!(after_fail.active_container.as_deref(), Some("cid-6"));
    let failed_latest = store.list_failed_latest_deploys().unwrap();
    assert_eq!(failed_latest.len(), 1);
    assert_eq!(failed_latest[0].1.id, bad.id);

    assert!(store.delete_service_var(&svc.id, "DATABASE_URL").unwrap());
    assert!(store.delete_service(&svc.id).unwrap().is_some());
    assert!(store.get_service("web").unwrap().is_none());
}

#[test]
fn service_domains_crud_primary_and_routes() {
    use opsbox_store::{
        NewService, NewServiceDomain, ServiceKind, SourceKind, UpdateServiceDomain,
    };

    let store = Store::open_in_memory().unwrap();
    let (proj, env) = store.ensure_default_project(100).unwrap();

    let svc = store
        .create_service(NewService {
            project_id: &proj.id,
            env_id: &env.id,
            name: "shop",
            kind: ServiceKind::App,
            owner_id: None,
            source_kind: SourceKind::DockerImage,
            repo: "nginx:1.27-alpine",
            branch: "main",
            build_method: "Docker image",
            dockerfile_path: None,
            start_cmd: None,
            port: 80,
            health_path: "/",
            domain: Some("Shop.Example.COM"),
            image_ref: Some("nginx:1.27-alpine"),
            deploy_key_id: None,
            auto_deploy: false,
            memory_limit_mb: None,
            cpu_limit_millis: None,
            graph_x: 0,
            graph_y: 0,
            note: None,
            now: 100,
        })
        .unwrap();
    assert_eq!(svc.domain.as_deref(), Some("shop.example.com"));

    let initial = store.list_service_domains(&svc.id).unwrap();
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].domain, "shop.example.com");
    assert!(initial[0].is_primary);
    assert!(initial[0].force_https);
    assert!(initial[0].hsts);

    // Add a www redirect and a secondary custom domain.
    store
        .add_service_domains(
            &svc.id,
            &[
                NewServiceDomain {
                    domain: "www.shop.example.com",
                    is_primary: false,
                    redirect_to: Some("shop.example.com"),
                    force_https: true,
                    hsts: true,
                },
                NewServiceDomain {
                    domain: "shop.acme.co",
                    is_primary: false,
                    redirect_to: None,
                    force_https: false,
                    hsts: false,
                },
            ],
            200,
        )
        .unwrap();

    // Redirecting domain cannot be promoted to primary.
    assert!(
        store
            .make_primary_service_domain(&svc.id, "www.shop.example.com", 210)
            .unwrap()
            .is_none()
    );

    // Promoting shop.acme.co updates primary flags and services.domain.
    let promoted = store
        .make_primary_service_domain(&svc.id, "shop.acme.co", 220)
        .unwrap()
        .unwrap();
    assert!(promoted.is_primary);
    assert_eq!(
        store
            .get_service(&svc.id)
            .unwrap()
            .unwrap()
            .domain
            .as_deref(),
        Some("shop.acme.co")
    );

    // Update DNS status and toggles.
    let updated = store
        .update_service_domain(
            "shop.acme.co",
            UpdateServiceDomain {
                dns_status: Some("ok"),
                dns_checked_at: Some(Some(230)),
                force_https: Some(true),
                hsts: Some(true),
                now: 230,
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(updated.dns_status, "ok");
    assert!(updated.force_https);
    assert!(updated.hsts);

    // Make container active and verify active routes include all 3 domains.
    let dep = store
        .create_deploy(
            &svc.id,
            None,
            "init",
            opsbox_store::DeployTrigger::Manual,
            "boss",
            240,
        )
        .unwrap();
    store
        .complete_deploy_switch(
            &svc.id,
            &dep.id,
            "nginx:1.27-alpine",
            "c1",
            40123,
            None,
            245,
        )
        .unwrap();

    let routes = store.list_active_service_routes().unwrap();
    assert_eq!(routes.len(), 3);
    assert_eq!(routes[0].domain, "shop.acme.co");
    assert_eq!(routes[0].host_port, Some(40123));

    // Removing primary auto-promotes remaining non-redirect domain (shop.example.com).
    let (removed, next_prim) = store
        .remove_service_domain("shop.acme.co", 250)
        .unwrap()
        .unwrap();
    assert_eq!(removed.domain, "shop.acme.co");
    assert_eq!(next_prim.as_deref(), Some("shop.example.com"));
    assert_eq!(
        store
            .get_service(&svc.id)
            .unwrap()
            .unwrap()
            .domain
            .as_deref(),
        Some("shop.example.com")
    );
}

#[test]
fn compose_stacks_and_backup_jobs_round_trip() {
    use opsbox_store::{NewService, ServiceKind, SourceKind, UpsertComposeStack};

    let store = Store::open_in_memory().unwrap();
    let (proj, env) = store.ensure_default_project(10).unwrap();

    let stack = store
        .upsert_compose_stack(UpsertComposeStack {
            project_id: &proj.id,
            env_id: &env.id,
            name: "shop-stack",
            owner_id: None,
            compose_yaml: "services:\n  web:\n    image: nginx:1.27-alpine\n",
            project_name: "opsbox-shop-stack",
            service_names: &["web".into(), "cache".into()],
            depends_edges: &[("web".into(), "cache".into())],
            confirmed: false,
            now: 20,
        })
        .unwrap();
    assert_eq!(stack.project_name, "opsbox-shop-stack");
    assert_eq!(stack.depends_edges, vec![("web".into(), "cache".into())]);
    assert!(!stack.confirmed);

    let db = store
        .create_service_with_meta(
            NewService {
                project_id: &proj.id,
                env_id: &env.id,
                name: "postgres-main",
                kind: ServiceKind::Db,
                owner_id: None,
                source_kind: SourceKind::DockerImage,
                repo: "PostgreSQL 16.4 · volume 1 GB",
                branch: "main",
                build_method: "template:postgres",
                dockerfile_path: None,
                start_cmd: None,
                port: 5432,
                health_path: "tcp",
                domain: None,
                image_ref: Some("postgres:16.4"),
                deploy_key_id: None,
                auto_deploy: false,
                memory_limit_mb: None,
                cpu_limit_millis: None,
                graph_x: 0,
                graph_y: 20,
                note: Some("backup daily 02:00"),
                now: 30,
            },
            None,
            Some("postgres"),
            true,
        )
        .unwrap();
    assert_eq!(db.template_id.as_deref(), Some("postgres"));
    assert!(db.internal_only);

    let job = store
        .upsert_backup_job(&db.id, Some("postgres"), "pg_dump", "0 2 * * *", true, 30)
        .unwrap();
    assert_eq!(job.method, "pg_dump");
    assert_eq!(job.cron, "0 2 * * *");
    assert!(job.enabled);
    assert_eq!(
        store
            .get_backup_job_for_service(&db.id)
            .unwrap()
            .unwrap()
            .id,
        job.id
    );

    let web = store
        .create_service_with_meta(
            NewService {
                project_id: &proj.id,
                env_id: &env.id,
                name: "web",
                kind: ServiceKind::Compose,
                owner_id: None,
                source_kind: SourceKind::DockerImage,
                repo: "compose · shop-stack",
                branch: "main",
                build_method: "docker compose",
                dockerfile_path: None,
                start_cmd: None,
                port: 80,
                health_path: "/",
                domain: None,
                image_ref: Some("nginx:1.27-alpine"),
                deploy_key_id: None,
                auto_deploy: false,
                memory_limit_mb: None,
                cpu_limit_millis: None,
                graph_x: 290,
                graph_y: 20,
                note: Some("stack shop-stack"),
                now: 40,
            },
            Some(&stack.id),
            None,
            false,
        )
        .unwrap();
    assert_eq!(web.compose_stack_id.as_deref(), Some(stack.id.as_str()));
    assert_eq!(
        store
            .list_services_by_compose_stack(&stack.id)
            .unwrap()
            .len(),
        1
    );

    store.delete_compose_stack(&stack.id).unwrap();
    assert!(store.get_compose_stack(&stack.id).unwrap().is_none());
    assert!(store.get_service(&web.id).unwrap().is_none());
}

#[test]
fn var_ref_tracking_rotation_staleness_and_exposure_round_trip() {
    use opsbox_store::{DeployTrigger, NewService, ServiceKind, SourceKind};
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("refs.db")).unwrap();

    let (proj, env) = store.ensure_default_project(10).unwrap();

    let db = store
        .create_service_with_exposure(
            NewService {
                project_id: &proj.id,
                env_id: &env.id,
                name: "pg-main",
                kind: ServiceKind::Db,
                owner_id: None,
                source_kind: SourceKind::DockerImage,
                repo: "template · postgres",
                branch: "16",
                build_method: "postgres template",
                dockerfile_path: None,
                start_cmd: None,
                port: 5432,
                health_path: "/",
                domain: None,
                image_ref: Some("postgres:16"),
                deploy_key_id: None,
                auto_deploy: false,
                memory_limit_mb: None,
                cpu_limit_millis: None,
                graph_x: 0,
                graph_y: 0,
                note: None,
                now: 20,
            },
            None,
            Some("postgres"),
            true,
            false,
        )
        .unwrap();
    assert!(db.internal_only);
    assert!(!db.public_port);
    assert!(db.credentials_rotated_at.is_none());

    let updated_db = store
        .set_service_exposure(&db.id, false, true, 25)
        .unwrap()
        .unwrap();
    assert!(!updated_db.internal_only);
    assert!(updated_db.public_port);

    let app = store
        .create_service(NewService {
            project_id: &proj.id,
            env_id: &env.id,
            name: "api",
            kind: ServiceKind::App,
            owner_id: None,
            source_kind: SourceKind::GitUrl,
            repo: "https://example.com/api.git",
            branch: "main",
            build_method: "dockerfile",
            dockerfile_path: None,
            start_cmd: None,
            port: 3000,
            health_path: "/healthz",
            domain: None,
            image_ref: None,
            deploy_key_id: None,
            auto_deploy: true,
            memory_limit_mb: None,
            cpu_limit_millis: None,
            graph_x: 290,
            graph_y: 0,
            note: None,
            now: 30,
        })
        .unwrap();

    let v = store
        .upsert_service_var_with_refs(
            &app.id,
            "DATABASE_URL",
            "sealed-ref",
            true,
            Some("${{ pg-main.DATABASE_URL }}"),
            &[(db.id.clone(), "DATABASE_URL".into())],
            35,
        )
        .unwrap();
    assert_eq!(v.ref_expr.as_deref(), Some("${{ pg-main.DATABASE_URL }}"));

    assert_eq!(
        store.list_project_var_ref_edges(&proj.id, &env.id).unwrap(),
        vec![("api".to_owned(), "pg-main".to_owned())]
    );

    // Not yet deployed -> rotation keeps needs_redeploy = false.
    let refs_before_deploy = store
        .mark_credentials_rotated(&db.id, &["DATABASE_URL", "PGPASSWORD"], 40)
        .unwrap();
    assert_eq!(refs_before_deploy.len(), 1);
    assert!(!refs_before_deploy[0].needs_redeploy);

    // Deploy app -> completes cleanly.
    let dep1 = store
        .create_deploy(&app.id, None, "deploy", DeployTrigger::Manual, "admin", 50)
        .unwrap();
    let seq1 = store.capture_service_ref_seq(&app.id).unwrap();
    store
        .complete_deploy_switch_with_refs(
            &app.id,
            &dep1.id,
            "opsbox/api:1",
            "cid-1",
            3001,
            None,
            Some(seq1),
            55,
        )
        .unwrap();

    // Rotate while app is live -> app becomes stale (`needs_redeploy = true`).
    let refs_after_rot = store
        .mark_credentials_rotated(&db.id, &["DATABASE_URL", "PGPASSWORD"], 60)
        .unwrap();
    assert_eq!(refs_after_rot.len(), 1);
    assert!(refs_after_rot[0].needs_redeploy);
    assert_eq!(refs_after_rot[0].var_keys, vec!["DATABASE_URL"]);
    assert_eq!(
        store
            .get_service(&db.id)
            .unwrap()
            .unwrap()
            .credentials_rotated_at,
        Some(60)
    );

    // Simulate a mid-deploy rotation race: snapshot seq BEFORE second rotation, then complete deploy.
    let seq_mid = store.capture_service_ref_seq(&app.id).unwrap();
    let dep2 = store
        .create_deploy(
            &app.id,
            None,
            "redeploy",
            DeployTrigger::Manual,
            "admin",
            65,
        )
        .unwrap();
    store
        .mark_credentials_rotated(&db.id, &["DATABASE_URL"], 66)
        .unwrap();
    store
        .complete_deploy_switch_with_refs(
            &app.id,
            &dep2.id,
            "opsbox/api:2",
            "cid-2",
            3002,
            None,
            Some(seq_mid),
            67,
        )
        .unwrap();
    assert!(
        store.list_referencing_services(&db.id).unwrap()[0].needs_redeploy,
        "mid-deploy rotation must stay stale"
    );

    // Final redeploy clears staleness.
    let seq_final = store.capture_service_ref_seq(&app.id).unwrap();
    let dep3 = store
        .create_deploy(
            &app.id,
            None,
            "redeploy 2",
            DeployTrigger::Manual,
            "admin",
            70,
        )
        .unwrap();
    store
        .complete_deploy_switch_with_refs(
            &app.id,
            &dep3.id,
            "opsbox/api:3",
            "cid-3",
            3003,
            None,
            Some(seq_final),
            71,
        )
        .unwrap();
    assert!(!store.list_referencing_services(&db.id).unwrap()[0].needs_redeploy);

    // Disconnect removes only vars pointing to `db`.
    let removed = store
        .delete_service_vars_referencing(&app.id, &db.id)
        .unwrap();
    assert_eq!(removed, vec!["DATABASE_URL"]);
    assert!(store.list_service_vars(&app.id).unwrap().is_empty());
    assert!(store.list_referencing_services(&db.id).unwrap().is_empty());
    assert!(
        store
            .list_project_var_ref_edges(&proj.id, &env.id)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn backup_jobs_snapshots_and_online_hot_backup() {
    use opsbox_proto::{BackupCheckState, BackupTargetKind};
    use opsbox_store::{
        LOCAL_DESTINATION_ID, NewBackupJob, OPSBOX_SELF_JOB_ID, ReadOnlyStore, SyncBackupSnapshot,
        UpdateBackupJob,
    };

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("opsbox.db");
    let store = Store::open(&db_path).unwrap();

    // Seeded local destination & opsbox self-backup job exist right after migration.
    let dests = store.list_backup_destinations().unwrap();
    assert_eq!(dests.len(), 1);
    assert_eq!(dests[0].id, LOCAL_DESTINATION_ID);
    let self_job = store.get_backup_job(OPSBOX_SELF_JOB_ID).unwrap().unwrap();
    assert_eq!(self_job.kind, BackupTargetKind::Opsbox);
    assert!(self_job.enabled);

    // Create path & volume jobs.
    let path_job = store
        .create_backup_job(NewBackupJob {
            name: "app-files",
            kind: BackupTargetKind::Path,
            what: &json!({ "paths": ["/var/www/app"] }),
            service_id: None,
            template_id: None,
            method: "path",
            cron: "0 3 * * *",
            retain_daily: 7,
            retain_weekly: 4,
            destination_id: LOCAL_DESTINATION_ID,
            enabled: true,
            next_run_at: Some(100),
            now: 50,
        })
        .unwrap();
    assert_eq!(store.list_due_backup_jobs(150).unwrap().len(), 1);

    let updated = store
        .update_backup_job(
            &path_job.id,
            UpdateBackupJob {
                retain_daily: Some(10),
                cron: Some("30 3 * * *"),
                next_run_at: Some(Some(500)),
                now: 160,
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(updated.retain_daily, 10);
    assert_eq!(updated.cron, "30 3 * * *");

    // Record run & sync snapshots with check state.
    store
        .record_backup_job_result(
            &path_job.id,
            true,
            200,
            Some("snap-1"),
            Some(4096),
            Some(120),
            None,
            Some(900),
        )
        .unwrap();
    let tags = vec![format!("opsbox-job:{}", path_job.id)];
    let paths = vec!["/var/www/app".to_owned()];
    store
        .sync_backup_snapshots(
            LOCAL_DESTINATION_ID,
            &[
                SyncBackupSnapshot {
                    job_id: Some(&path_job.id),
                    restic_id: "1111111111111111",
                    short_id: "11111111",
                    what: "app-files",
                    kind: BackupTargetKind::Path,
                    taken_at: 200,
                    size_bytes: 4096,
                    tags: &tags,
                    paths: &paths,
                },
                SyncBackupSnapshot {
                    job_id: Some(&path_job.id),
                    restic_id: "2222222222222222",
                    short_id: "22222222",
                    what: "app-files",
                    kind: BackupTargetKind::Path,
                    taken_at: 300,
                    size_bytes: 8192,
                    tags: &tags,
                    paths: &paths,
                },
            ],
            Some((BackupCheckState::ChecksumOk, 310)),
            310,
        )
        .unwrap();
    let snaps = store.list_backup_snapshots(50).unwrap();
    assert_eq!(snaps.len(), 2);
    assert_eq!(snaps[0].check_state, BackupCheckState::ChecksumOk);

    // Prune sync removes deleted snapshots while keeping survivors.
    store
        .sync_backup_snapshots(
            LOCAL_DESTINATION_ID,
            &[SyncBackupSnapshot {
                job_id: Some(&path_job.id),
                restic_id: "2222222222222222",
                short_id: "22222222",
                what: "app-files",
                kind: BackupTargetKind::Path,
                taken_at: 300,
                size_bytes: 8192,
                tags: &tags,
                paths: &paths,
            }],
            None,
            320,
        )
        .unwrap();
    let after_prune = store.list_backup_snapshots(50).unwrap();
    assert_eq!(after_prune.len(), 1);
    assert_eq!(after_prune[0].short_id, "22222222");

    // Hot SQLite online backup while WAL store is open and writing.
    store
        .set_setting("backup.marker", &json!("alive-wal"))
        .unwrap();
    let bak_path = dir.path().join("stage").join("opsbox-hot.db");
    let ro = ReadOnlyStore::open(&db_path).unwrap();
    ro.online_backup(&bak_path).unwrap();

    let copied = ReadOnlyStore::open(&bak_path).unwrap();
    copied.check_integrity().unwrap();
    assert_eq!(
        copied.get_setting("backup.marker").unwrap(),
        Some(json!("alive-wal"))
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&bak_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
