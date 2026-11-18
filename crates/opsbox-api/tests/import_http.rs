//! `POST /api/v1/config/import/preview` and `…/apply`: round-trip with export, wrong
//! passphrase, unknown format, and the non-overwrite guarantee.

mod common;

use axum::http::StatusCode;
use common::{Browser, Harness};
use opsbox_proto::{Role, Secret};
use opsbox_store::{NewService, ServiceKind, SourceKind};
use serde_json::{Value, json};
use std::time::Duration;

const PASSWORD: &str = "TestAdminPass123!";
const PASSPHRASE: &str = "correct horse passphrase";
const SECRET_VAR: &str = "VAR-SECRET-7f3a9c";
const PUBLIC_VAR: &str = "plain-value-visible";
const DEST_SECRET: &str = "DEST-SECRET-key-91bd";

struct Fixture {
    h: Harness,
    admin: Browser,
}

async fn seal(h: &Harness, ctx: &str, plain: &str) -> String {
    h.state
        .priv_client
        .secret_seal("test", Some("test"), ctx, Secret::new(plain))
        .await
        .unwrap()
}

async fn fixture() -> Fixture {
    let h = Harness::new(true);
    let admin = h.signed_in("alice", Role::Admin);
    let user = h.store.user_by_name("alice").unwrap().unwrap();
    let hash = opsbox_auth::password::Hasher::new(opsbox_auth::password::HashParams {
        memory_kib: 8,
        iterations: 1,
        parallelism: 1,
    })
    .hash(PASSWORD)
    .unwrap();
    h.store.set_password_hash(&user.id, &hash, 0).unwrap();

    let (proj, env) = h.store.ensure_default_project(1).unwrap();
    let base = NewService {
        project_id: &proj.id,
        env_id: &env.id,
        name: "web",
        kind: ServiceKind::App,
        owner_id: Some(&user.id),
        source_kind: SourceKind::GitUrl,
        repo: "https://git.example.test/acme/web.git",
        branch: "main",
        build_method: "dockerfile",
        dockerfile_path: Some("Dockerfile"),
        start_cmd: None,
        port: 3000,
        health_path: "/healthz",
        domain: Some("web.example.test"),
        image_ref: None,
        deploy_key_id: None,
        auto_deploy: false,
        memory_limit_mb: Some(512),
        cpu_limit_millis: None,
        graph_x: 0,
        graph_y: 0,
        note: Some("front door"),
        now: 2,
    };
    let web = h.store.create_service(base.clone()).unwrap();
    let db = h
        .store
        .create_service_with_exposure(
            NewService {
                name: "main-db",
                kind: ServiceKind::Db,
                source_kind: SourceKind::DockerImage,
                branch: "16",
                owner_id: None,
                domain: None,
                repo: "",
                build_method: "Docker image",
                image_ref: Some("postgres:16"),
                ..base
            },
            None,
            Some("postgres"),
            true,
            false,
        )
        .unwrap();
    for (svc, key, value, secret) in [
        (&web, "SESSION_KEY", SECRET_VAR, true),
        (&web, "PUBLIC_URL", PUBLIC_VAR, false),
        (&db, "POSTGRES_PASSWORD", SECRET_VAR, true),
    ] {
        let enc = seal(&h, &format!("deploy:var:{}:{key}", svc.id), value).await;
        h.store
            .upsert_service_var(&svc.id, key, &enc, secret, 3)
            .unwrap();
    }
    h.store
        .upsert_compose_stack(opsbox_store::UpsertComposeStack {
            project_id: &proj.id,
            env_id: &env.id,
            name: "blog",
            owner_id: Some(&user.id),
            compose_yaml: "services:\n  blog:\n    image: ghost\n",
            project_name: "blog",
            service_names: &["blog".to_owned()],
            depends_edges: &[],
            confirmed: true,
            now: 5,
        })
        .unwrap();
    let dest = seal(&h, "backup:dest:s3-main", DEST_SECRET).await;
    h.store
        .create_backup_destination(
            "s3-main",
            "s3",
            "Offsite",
            &json!({"bucket": "acme-backups"}),
            Some(&dest),
            None,
            6,
        )
        .unwrap();
    h.store
        .upsert_backup_job(&db.id, Some("postgres"), "dump", "0 3 * * *", true, 7)
        .unwrap();
    Fixture { h, admin }
}

async fn export_doc(f: &mut Fixture, passphrase: Option<&str>) -> Value {
    let mut body = json!({ "password": PASSWORD });
    if let Some(p) = passphrase {
        body["passphrase"] = json!(p);
    }
    let r = f.admin.post("/api/v1/config/export", body).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    r.json()
}

async fn wait_task(admin: &mut Browser, id: &str) -> Value {
    for _ in 0..200 {
        let r = admin.get(&format!("/api/v1/tasks/{id}")).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text());
        let t = r.json();
        match t["status"].as_str() {
            Some("ok") | Some("failed") => return t,
            _ => tokio::time::sleep(Duration::from_millis(25)).await,
        }
    }
    panic!("task {id} did not finish");
}

/// Strip ids, times and host metadata so two exports can be compared for configuration.
fn normalize(doc: &Value) -> Value {
    let mut out = doc.clone();
    for key in ["exportedAt", "exportedBy", "opsboxVersion", "host"] {
        out.as_object_mut().unwrap().remove(key);
    }
    if let Some(services) = out.get_mut("services").and_then(Value::as_array_mut) {
        for s in services.iter_mut() {
            let o = s.as_object_mut().unwrap();
            o.remove("id");
            // Owner is matched by name on import; a fresh server may not have the same people.
            o.insert("owner".into(), Value::Null);
            // Deploy keys are never imported.
            if let Some(src) = o.get_mut("source").and_then(Value::as_object_mut) {
                src.insert("deployKeyId".into(), Value::Null);
            }
            if let Some(vars) = s.get_mut("vars").and_then(Value::as_array_mut) {
                vars.sort_by(|a, b| {
                    a["key"]
                        .as_str()
                        .unwrap_or("")
                        .cmp(b["key"].as_str().unwrap_or(""))
                });
            }
            if let Some(domains) = s.get_mut("domains").and_then(Value::as_array_mut) {
                domains.sort_by(|a, b| {
                    a["domain"]
                        .as_str()
                        .unwrap_or("")
                        .cmp(b["domain"].as_str().unwrap_or(""))
                });
            }
        }
        services.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });
    }
    if let Some(compose) = out.get_mut("composeProjects").and_then(Value::as_array_mut) {
        for c in compose.iter_mut() {
            c.as_object_mut()
                .unwrap()
                .insert("owner".into(), Value::Null);
        }
        compose.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });
    }
    if let Some(dbs) = out.get_mut("databases").and_then(Value::as_array_mut) {
        dbs.sort_by(|a, b| {
            a["service"]
                .as_str()
                .unwrap_or("")
                .cmp(b["service"].as_str().unwrap_or(""))
        });
    }
    if let Some(dests) = out
        .pointer_mut("/backups/destinations")
        .and_then(Value::as_array_mut)
    {
        for d in dests.iter_mut() {
            // Destination id is kept as the import key; compare by name + provider + config.
            d.as_object_mut().unwrap().remove("id");
        }
        dests.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });
    }
    if let Some(jobs) = out
        .pointer_mut("/backups/jobs")
        .and_then(Value::as_array_mut)
    {
        for j in jobs.iter_mut() {
            j.as_object_mut().unwrap().remove("id");
        }
        jobs.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or("")
                .cmp(b["name"].as_str().unwrap_or(""))
        });
    }
    if let Some(rules) = out
        .pointer_mut("/alerts/rules")
        .and_then(Value::as_array_mut)
    {
        rules.sort_by(|a, b| {
            a["id"]
                .as_str()
                .unwrap_or("")
                .cmp(b["id"].as_str().unwrap_or(""))
        });
    }
    // Secrets ciphertext / salt / nonce differ per seal; compare included flag and needsReentry kinds only.
    if let Some(sec) = out.get_mut("secrets").and_then(Value::as_object_mut) {
        sec.remove("ciphertext");
        sec.remove("encryption");
        if let Some(nr) = sec.get_mut("needsReentry").and_then(Value::as_array_mut) {
            nr.sort_by(|a, b| {
                let ka = format!(
                    "{}:{}:{}",
                    a["kind"].as_str().unwrap_or(""),
                    a["service"].as_str().unwrap_or(""),
                    a["key"].as_str().or(a["id"].as_str()).or(a["name"].as_str()).unwrap_or("")
                );
                let kb = format!(
                    "{}:{}:{}",
                    b["kind"].as_str().unwrap_or(""),
                    b["service"].as_str().unwrap_or(""),
                    b["key"].as_str().or(b["id"].as_str()).or(b["name"].as_str()).unwrap_or("")
                );
                ka.cmp(&kb)
            });
        }
    }
    out.as_object_mut().unwrap().remove("unavailable");
    out
}

#[tokio::test]
async fn unknown_format_is_refused() {
    let mut f = fixture().await;
    let mut doc = export_doc(&mut f, None).await;
    doc["format"] = json!("opsbox-config/999");
    let r = f
        .admin
        .post(
            "/api/v1/config/import/preview",
            json!({ "password": PASSWORD, "document": doc }),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text());
    let body = r.json();
    assert_eq!(body["error"]["field"], "format");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("opsbox-config/1"),
        "{}",
        body
    );
}

#[tokio::test]
async fn wrong_passphrase_fails_cleanly() {
    let mut f = fixture().await;
    let doc = export_doc(&mut f, Some(PASSPHRASE)).await;
    assert_eq!(doc["secrets"]["included"], true);

    let r = f
        .admin
        .post(
            "/api/v1/config/import/preview",
            json!({
                "password": PASSWORD,
                "passphrase": "wrong passphrase here",
                "document": doc,
            }),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
    let msg = r.json()["error"]["message"].as_str().unwrap().to_owned();
    assert!(
        msg.contains("passphrase") || msg.contains("changed"),
        "{msg}"
    );
    // Store untouched: both services still present, nothing half-applied.
    assert_eq!(f.h.store.list_all_services().unwrap().len(), 2);
}

#[tokio::test]
async fn existing_objects_are_never_overwritten() {
    let mut f = fixture().await;
    let doc = export_doc(&mut f, Some(PASSPHRASE)).await;

    // Preview on the same server: everything should be skipped.
    let r = f
        .admin
        .post(
            "/api/v1/config/import/preview",
            json!({
                "password": PASSWORD,
                "passphrase": PASSPHRASE,
                "document": doc,
            }),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let plan = r.json();
    assert!(
        plan["willCreate"]
            .as_object()
            .map(|m| m.values().all(|v| v.as_array().map_or(true, Vec::is_empty)))
            .unwrap_or(true)
            || plan["skipped"].as_array().map(|a| !a.is_empty()).unwrap_or(false),
        "expected skips for existing objects: {plan}"
    );
    let skipped = plan["skipped"].as_array().unwrap();
    assert!(
        skipped.iter().any(|e| e["kind"] == "service" && e["name"] == "web"),
        "{skipped:?}"
    );

    // Apply must not change the note on the existing service.
    let before = f.h.store.get_service("web").unwrap().unwrap();
    let r = f
        .admin
        .post(
            "/api/v1/config/import/apply",
            json!({
                "password": PASSWORD,
                "passphrase": PASSPHRASE,
                "document": doc,
            }),
        )
        .await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text());
    let task_id = r.json()["task"].as_str().unwrap().to_owned();
    let task = wait_task(&mut f.admin, &task_id).await;
    assert_eq!(task["status"], "ok", "{task}");
    let after = f.h.store.get_service("web").unwrap().unwrap();
    assert_eq!(before.id, after.id);
    assert_eq!(before.note, after.note);
    assert_eq!(before.repo, after.repo);
    assert_eq!(f.h.store.list_all_services().unwrap().len(), 2);
}

#[tokio::test]
async fn round_trip_export_import_export() {
    let mut src = fixture().await;
    let doc = export_doc(&mut src, Some(PASSPHRASE)).await;

    // Fresh empty store (admin only).
    let dst = Harness::new(true);
    let mut admin = dst.signed_in("bob", Role::Admin);
    let user = dst.store.user_by_name("bob").unwrap().unwrap();
    let hash = opsbox_auth::password::Hasher::new(opsbox_auth::password::HashParams {
        memory_kib: 8,
        iterations: 1,
        parallelism: 1,
    })
    .hash(PASSWORD)
    .unwrap();
    dst.store.set_password_hash(&user.id, &hash, 0).unwrap();
    assert!(dst.store.list_all_services().unwrap().is_empty());

    let preview = admin
        .post(
            "/api/v1/config/import/preview",
            json!({
                "password": PASSWORD,
                "passphrase": PASSPHRASE,
                "document": doc,
            }),
        )
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{}", preview.text());
    let plan = preview.json();
    assert!(
        plan["willCreate"]["services"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == "web"),
        "{plan}"
    );
    assert_eq!(plan["secretsDecrypted"], true);

    let apply = admin
        .post(
            "/api/v1/config/import/apply",
            json!({
                "password": PASSWORD,
                "passphrase": PASSPHRASE,
                "document": doc,
            }),
        )
        .await;
    assert_eq!(apply.status, StatusCode::ACCEPTED, "{}", apply.text());
    let task = wait_task(&mut admin, apply.json()["task"].as_str().unwrap()).await;
    assert_eq!(task["status"], "ok", "{task}");

    let names: Vec<String> = dst
        .store
        .list_all_services()
        .unwrap()
        .into_iter()
        .map(|s| s.name)
        .collect();
    assert!(names.contains(&"web".into()), "{names:?}");
    assert!(names.contains(&"main-db".into()), "{names:?}");
    assert!(dst.store.get_compose_stack("blog").unwrap().is_some());
    assert!(
        dst.store
            .get_backup_destination("s3-main")
            .unwrap()
            .is_some()
    );

    // Re-export from the destination and compare configuration.
    let mut dst_fix = Fixture { h: dst, admin };
    let again = export_doc(&mut dst_fix, Some(PASSPHRASE)).await;
    let a = normalize(&doc);
    let b = normalize(&again);
    assert_eq!(
        a["services"], b["services"],
        "services differ\nleft: {}\nright: {}",
        serde_json::to_string_pretty(&a["services"]).unwrap(),
        serde_json::to_string_pretty(&b["services"]).unwrap()
    );
    assert_eq!(a["composeProjects"], b["composeProjects"]);
    assert_eq!(a["databases"], b["databases"]);
    assert_eq!(
        a["backups"]["destinations"],
        b["backups"]["destinations"]
    );
}
