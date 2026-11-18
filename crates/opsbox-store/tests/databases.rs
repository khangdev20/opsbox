//! Per-app database logins and settings (migration 0011).

use opsbox_store::{NewService, ServiceKind, ServiceRecord, SourceKind, Store};

fn service(store: &Store, name: &str, kind: ServiceKind) -> ServiceRecord {
    let (proj, env) = store.resolve_project("default", 1).unwrap().unwrap();
    store
        .create_service(NewService {
            project_id: &proj.id,
            env_id: &env.id,
            name,
            kind,
            owner_id: None,
            source_kind: SourceKind::DockerImage,
            repo: "postgres:16.4",
            branch: "16.4",
            build_method: "PostgreSQL",
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
            graph_y: 0,
            note: None,
            now: 1,
        })
        .unwrap()
}

#[test]
fn removing_one_app_never_touches_another_apps_variables() {
    let store = Store::open_in_memory().unwrap();
    let db = service(&store, "pg", ServiceKind::Db);
    let web = service(&store, "web", ServiceKind::App);
    let web_api = service(&store, "web-api", ServiceKind::App);
    // `OPSBOX_APP_WEB_` is a text prefix of `OPSBOX_APP_WEB_API_`.
    for (app, prefix) in [(&web, "OPSBOX_APP_WEB_"), (&web_api, "OPSBOX_APP_WEB_API_")] {
        store
            .upsert_database_app(&db.id, &app.id, "app_x", "app_x", prefix, false, 2)
            .unwrap();
        for (suffix, _, secret, _) in opsbox_proto::APP_CONNECTION_VARS {
            store
                .upsert_service_var(&db.id, &format!("{prefix}{suffix}"), "enc", *secret, 2)
                .unwrap();
        }
    }
    store
        .upsert_service_var(&db.id, "POSTGRES_PASSWORD", "enc", true, 2)
        .unwrap();

    store.remove_database_app(&db.id, &web.id).unwrap();

    let keys: Vec<String> = store
        .list_service_vars(&db.id)
        .unwrap()
        .into_iter()
        .map(|v| v.key)
        .collect();
    assert!(
        !keys
            .iter()
            .any(|k| k.starts_with("OPSBOX_APP_WEB_") && !k.starts_with("OPSBOX_APP_WEB_API_"))
    );
    assert_eq!(
        keys.iter()
            .filter(|k| k.starts_with("OPSBOX_APP_WEB_API_"))
            .count(),
        opsbox_proto::APP_CONNECTION_VARS.len()
    );
    assert!(keys.contains(&"POSTGRES_PASSWORD".to_owned()));
    let left = store.list_database_apps(&db.id).unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].app_service_id, web_api.id);
}

#[test]
fn settings_default_then_persist_pooler_and_upgrade_volume() {
    let store = Store::open_in_memory().unwrap();
    let db = service(&store, "pg", ServiceKind::Db);
    let d = store.get_database_settings(&db.id).unwrap();
    assert!(!d.pooler_enabled && d.data_volume.is_none() && d.pooler_mode == "transaction");
    store
        .set_database_pooler(&db.id, true, Some("cid-1"), "session", 3)
        .unwrap();
    store
        .complete_database_upgrade(
            &db.id,
            "17.2",
            "postgres:17.2",
            "cid-2",
            0,
            "opsbox-pg-data-v17",
            4,
        )
        .unwrap();
    let d = store.get_database_settings(&db.id).unwrap();
    assert!(d.pooler_enabled);
    assert_eq!(d.pooler_container.as_deref(), Some("cid-1"));
    assert_eq!(d.pooler_mode, "session");
    assert_eq!(d.data_volume.as_deref(), Some("opsbox-pg-data-v17"));
    let s = store.get_service(&db.id).unwrap().unwrap();
    assert_eq!(s.branch, "17.2");
    assert_eq!(s.image_ref.as_deref(), Some("postgres:17.2"));
    assert_eq!(s.active_container.as_deref(), Some("cid-2"));
    assert_eq!(s.active_host_port, None);
}
