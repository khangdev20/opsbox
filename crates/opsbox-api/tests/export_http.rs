//! `POST /api/v1/config/export`: the document, its schema, the secrets rules and the audit
//! trail. A real router and a real `priv` (so sealed secrets really are sealed).

mod common;

use axum::http::StatusCode;
use common::{Browser, Harness};
use opsbox_api::export::{DecryptError, Kdf, decrypt_secrets, seal_secrets};
use opsbox_proto::{Role, Secret};
use opsbox_store::{NewService, ServiceKind, SourceKind};
use serde_json::{Value, json};

const PASSWORD: &str = "TestAdminPass123!";
const PASSPHRASE: &str = "correct horse passphrase";
const SECRET_VAR: &str = "VAR-SECRET-7f3a9c";
const PUBLIC_VAR: &str = "plain-value-visible";
const DEST_SECRET: &str = "DEST-SECRET-key-91bd";
const DEPLOY_KEY_ENC: &str = "DEPLOY-PRIVATE-KEY-ENC-5521";
const CHANNEL_SECRET: &str = "https://hooks.example.test/CHANNEL-SECRET-44aa";

// ------------------------------------------------------------------------- JSON schema

/// The part of JSON Schema that `docs/schemas/config-export-v1.json` uses: `type` (one or
/// several), `const`, `enum`, `required`, `properties`, `additionalProperties: false`,
/// `items` and local `$ref`. Anything else in the schema file fails the test, so the file
/// cannot quietly start relying on keywords nobody checks.
fn validate(schema: &Value, root: &Value, v: &Value, at: &str, errs: &mut Vec<String>) {
    let Some(obj) = schema.as_object() else {
        errs.push(format!("{at}: schema is not an object"));
        return;
    };
    for key in obj.keys() {
        let known = [
            "$schema",
            "$id",
            "title",
            "description",
            "$ref",
            "$defs",
            "type",
            "const",
            "enum",
            "required",
            "properties",
            "additionalProperties",
            "items",
        ];
        if !known.contains(&key.as_str()) {
            errs.push(format!("{at}: the test validator does not know `{key}`"));
        }
    }
    if let Some(r) = obj.get("$ref").and_then(Value::as_str) {
        let target = r
            .strip_prefix("#/")
            .and_then(|p| p.split('/').try_fold(root, |n, k| n.get(k)));
        match target {
            Some(t) => validate(t, root, v, at, errs),
            None => errs.push(format!("{at}: unresolved {r}")),
        }
        return;
    }
    if let Some(t) = obj.get("type") {
        let names: Vec<&str> = match t {
            Value::String(s) => vec![s.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        let ok = names.iter().any(|n| match *n {
            "object" => v.is_object(),
            "array" => v.is_array(),
            "string" => v.is_string(),
            "boolean" => v.is_boolean(),
            "null" => v.is_null(),
            "integer" => v.is_i64() || v.is_u64(),
            "number" => v.is_number(),
            _ => false,
        });
        if !ok {
            errs.push(format!("{at}: expected {names:?}, found {v}"));
            return;
        }
    }
    if let Some(c) = obj.get("const")
        && c != v
    {
        errs.push(format!("{at}: expected {c}, found {v}"));
    }
    if let Some(Value::Array(options)) = obj.get("enum")
        && !options.contains(v)
    {
        errs.push(format!("{at}: {v} is not one of {options:?}"));
    }
    if let Some(m) = v.as_object() {
        for r in obj
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if !m.contains_key(r) {
                errs.push(format!("{at}: missing `{r}`"));
            }
        }
        let props = obj.get("properties").and_then(Value::as_object);
        for (k, child) in m {
            match props.and_then(|p| p.get(k)) {
                Some(s) => validate(s, root, child, &format!("{at}.{k}"), errs),
                None if obj.get("additionalProperties") == Some(&Value::Bool(false)) => {
                    errs.push(format!("{at}: unexpected `{k}`"));
                }
                None => {}
            }
        }
    }
    if let (Some(items), Some(a)) = (obj.get("items"), v.as_array()) {
        for (i, child) in a.iter().enumerate() {
            validate(items, root, child, &format!("{at}[{i}]"), errs);
        }
    }
}

fn schema() -> Value {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/schemas/config-export-v1.json"
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("the schema is checked in"))
        .expect("the schema is JSON")
}

fn assert_valid(doc: &Value) {
    let s = schema();
    let mut errs = Vec::new();
    validate(&s, &s, doc, "$", &mut errs);
    assert!(
        errs.is_empty(),
        "document does not match the schema:\n{}",
        errs.join("\n")
    );
}

#[test]
fn the_validator_itself_rejects_what_it_should() {
    let s = json!({
        "type": "object", "additionalProperties": false, "required": ["a"],
        "properties": { "a": { "type": ["string", "null"] }, "b": { "enum": ["x"] } }
    });
    let check = |v: Value| {
        let mut e = Vec::new();
        validate(&s, &s, &v, "$", &mut e);
        e
    };
    assert!(check(json!({"a": null})).is_empty());
    assert!(!check(json!({})).is_empty(), "missing required");
    assert!(!check(json!({"a": 1})).is_empty(), "wrong type");
    assert!(!check(json!({"a": "x", "c": 1})).is_empty(), "extra key");
    assert!(!check(json!({"a": "x", "b": "y"})).is_empty(), "enum");
    let mut e = Vec::new();
    validate(
        &json!({"pattern": "x"}),
        &json!({}),
        &json!("y"),
        "$",
        &mut e,
    );
    assert!(!e.is_empty(), "unknown keyword is reported");
}

// --------------------------------------------------------------------------- the fixture

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

/// An admin with a real password, one of each kind of thing the export carries, and a
/// secret in every place a secret can live.
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
    h.store
        .set_pending_totp(&user.id, "TOTP-SEALED-SECRET", 0)
        .unwrap();
    h.store.enable_totp(&user.id, 0, 0).unwrap();

    h.store
        .create_deploy_key(&opsbox_store::NewDeployKey {
            id: "key-1".into(),
            repo: "acme/web".into(),
            ssh_url: "git@git.example.test:acme/web.git".into(),
            public_key: "ssh-ed25519 AAAApublic".into(),
            fingerprint: "SHA256:abc".into(),
            private_key_enc: DEPLOY_KEY_ENC.into(),
            created_by: "alice".into(),
            created_at: 1,
        })
        .unwrap();
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
        dockerfile_path: Some("deploy/Dockerfile"),
        start_cmd: Some("./run --prod"),
        port: 3000,
        health_path: "/healthz",
        domain: Some("web.example.test"),
        image_ref: Some("registry.example.test/web:1"),
        deploy_key_id: Some("key-1"),
        auto_deploy: true,
        memory_limit_mb: Some(512),
        cpu_limit_millis: Some(1500),
        graph_x: 40,
        graph_y: 80,
        note: Some("front door"),
        now: 2,
    };
    let web = h
        .store
        .create_service_with_exposure(base.clone(), None, Some("n8n"), true, true)
        .unwrap();
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
        .add_service_domains(
            &web.id,
            &[opsbox_store::NewServiceDomain {
                domain: "www.example.test",
                is_primary: false,
                redirect_to: Some("web.example.test"),
                force_https: true,
                hsts: true,
            }],
            4,
        )
        .unwrap();
    h.store
        .upsert_compose_stack(opsbox_store::UpsertComposeStack {
            project_id: &proj.id,
            env_id: &env.id,
            name: "blog",
            owner_id: Some(&user.id),
            compose_yaml: "services:\n  blog:\n    image: ghost\n",
            project_name: "blog",
            service_names: &["blog".to_owned()],
            depends_edges: &[("blog".to_owned(), "blog-db".to_owned())],
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
            &json!({"bucket": "acme-backups", "region": "eu-west-1"}),
            Some(&dest),
            None,
            6,
        )
        .unwrap();
    h.store
        .upsert_backup_job(&db.id, Some("postgres"), "dump", "0 3 * * *", true, 7)
        .unwrap();

    let mut engine = opsbox_alerts::Engine::default();
    engine.channels.insert(
        "c1".into(),
        opsbox_alerts::Channel {
            id: "c1".into(),
            sealed: seal(&h, "alerts:channel:c1", CHANNEL_SECRET).await,
            verified: true,
            error: None,
        },
    );
    h.store
        .set_setting("alerts.engine.v1", &serde_json::to_value(&engine).unwrap())
        .unwrap();
    h.store
        .set_setting("security.allow_ips", &json!(["203.0.113.0/24"]))
        .unwrap();
    h.store
        .upsert_database_app(&db.id, &web.id, "web", "web", "", false, 8)
        .unwrap();
    Fixture { h, admin }
}

async fn export(f: &mut Fixture, body: Value) -> common::Reply {
    f.admin.post("/api/v1/config/export", body).await
}

fn audit_rows(h: &Harness) -> Vec<opsbox_store::AuditEntry> {
    h.store
        .recent_audit(200)
        .unwrap()
        .into_iter()
        .filter(|e| e.action == "config.export")
        .collect()
}

// ------------------------------------------------------------------------------ the tests

#[tokio::test]
async fn the_default_export_matches_the_schema_and_holds_no_secret() {
    let mut f = fixture().await;
    let r = export(&mut f, json!({"password": PASSWORD})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let doc = r.json();
    assert_valid(&doc);
    assert_eq!(doc["format"], "opsbox-config/1");
    assert_eq!(doc["secrets"]["included"], false);
    assert!(doc["secrets"].get("ciphertext").is_none());

    // Nothing secret anywhere in the bytes, not even in an unexpected field.
    let text = r.text();
    for secret in [
        SECRET_VAR,
        DEST_SECRET,
        CHANNEL_SECRET,
        "TOTP-SEALED-SECRET",
        "$argon2",
        PASSWORD,
    ] {
        assert!(!text.contains(secret), "the file contains {secret:?}");
    }
    assert!(
        !text.contains("enc:v1:"),
        "sealed values must not leak either"
    );
    // A value the owner marked as not secret is configuration and is carried.
    assert!(text.contains(PUBLIC_VAR));

    // What has to be typed again is listed, by name.
    let again: Vec<(String, String)> = doc["secrets"]["needsReentry"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["kind"].as_str().unwrap().to_owned(),
                e["key"]
                    .as_str()
                    .or(e["id"].as_str())
                    .or(e["name"].as_str())
                    .unwrap_or("")
                    .to_owned(),
            )
        })
        .collect();
    for want in [
        ("serviceVar", "SESSION_KEY"),
        ("serviceVar", "POSTGRES_PASSWORD"),
        ("backupDestination", "s3-main"),
        ("alertChannel", "c1"),
        ("user", "alice"),
        ("apiTokens", ""),
        ("deployKey", "key-1"),
    ] {
        assert!(
            again.contains(&(want.0.to_owned(), want.1.to_owned())),
            "needsReentry lacks {want:?}: {again:?}"
        );
    }

    // The things asked for are all there.
    let services = doc["services"].as_array().unwrap();
    let web = services.iter().find(|s| s["name"] == "web").unwrap();
    assert_eq!(
        web["source"]["repo"],
        "https://git.example.test/acme/web.git"
    );
    assert_eq!(web["limits"]["memoryMb"], 512);
    assert_eq!(web["owner"], "alice");
    assert!(
        web["domains"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["domain"] == "www.example.test" && d["forceHttps"] == true)
    );
    assert_eq!(doc["databases"][0]["service"], "main-db");
    assert_eq!(doc["databases"][0]["apps"][0]["app"], "web");
    assert_eq!(doc["databases"][0]["engine"], "PostgreSQL");
    assert_eq!(doc["composeProjects"][0]["name"], "blog");
    assert_eq!(doc["templates"][0]["service"], "web");
    assert_eq!(
        doc["backups"]["destinations"][0]["config"]["bucket"],
        "acme-backups"
    );
    assert_eq!(doc["backups"]["jobs"][0]["cron"], "0 3 * * *");
    assert_eq!(doc["alerts"]["channels"][0]["id"], "c1");
    assert!(doc["alerts"]["rules"].as_array().unwrap().len() > 2);
    assert_eq!(doc["network"]["allowedIps"][0], "203.0.113.0/24");
    assert!(
        doc["users"]
            .as_array()
            .unwrap()
            .contains(&json!({"name": "alice", "role": "admin", "twoFactorEnabled": true}))
    );
    f.h.shutdown().await;
}

#[tokio::test]
async fn the_file_name_has_the_host_and_the_date() {
    let mut f = fixture().await;
    let r = export(&mut f, json!({"password": PASSWORD})).await;
    let doc = r.json();
    let disposition = r.header("content-disposition").to_owned();
    let name = disposition
        .strip_prefix("attachment; filename=\"")
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or_else(|| panic!("{disposition}"));
    let host = opsbox_api::export::filename_host(doc["host"].as_str().unwrap());
    let date = &doc["exportedAt"].as_str().unwrap()[..10];
    assert_eq!(name, format!("opsbox-config-{host}-{date}.json"));
    assert_eq!(r.header("cache-control"), "no-store");
    f.h.shutdown().await;
}

#[tokio::test]
async fn with_a_passphrase_the_secrets_come_back_only_sealed() {
    let mut f = fixture().await;
    let r = export(
        &mut f,
        json!({"password": PASSWORD, "passphrase": PASSPHRASE}),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let doc = r.json();
    assert_valid(&doc);
    assert_eq!(doc["secrets"]["included"], true);
    let text = r.text();
    for secret in [
        SECRET_VAR,
        DEST_SECRET,
        CHANNEL_SECRET,
        PASSPHRASE,
        PASSWORD,
        "TOTP-SEALED-SECRET",
        "$argon2",
    ] {
        assert!(!text.contains(secret), "the file contains {secret:?}");
    }

    let opened = decrypt_secrets(&doc, PASSPHRASE).unwrap();
    let var = |svc: &str, key: &str| {
        opened["serviceVars"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["service"] == svc && v["key"] == key)
            .map(|v| v["value"].as_str().unwrap().to_owned())
    };
    assert_eq!(var("web", "SESSION_KEY").as_deref(), Some(SECRET_VAR));
    assert_eq!(
        var("main-db", "POSTGRES_PASSWORD").as_deref(),
        Some(SECRET_VAR)
    );
    assert_eq!(
        var("web", "PUBLIC_URL"),
        None,
        "not secret: it is in the open part"
    );
    assert_eq!(opened["backupDestinations"][0]["id"], "s3-main");
    assert_eq!(opened["backupDestinations"][0]["secrets"], DEST_SECRET);
    assert_eq!(opened["alertChannels"][0]["config"], CHANNEL_SECRET);

    // What was sealed no longer has to be typed again; what cannot be exported still does.
    let kinds: Vec<&str> = doc["secrets"]["needsReentry"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert!(
        !kinds.contains(&"serviceVar")
            && !kinds.contains(&"backupDestination")
            && !kinds.contains(&"alertChannel")
    );
    assert!(kinds.contains(&"user") && kinds.contains(&"apiTokens"));
    f.h.shutdown().await;
}

#[tokio::test]
async fn a_wrong_passphrase_or_a_changed_file_fails_cleanly() {
    let mut f = fixture().await;
    let doc = export(
        &mut f,
        json!({"password": PASSWORD, "passphrase": PASSPHRASE}),
    )
    .await
    .json();
    assert_eq!(
        decrypt_secrets(&doc, "not the passphrase"),
        Err(DecryptError::WrongPassphrase)
    );
    assert_eq!(
        decrypt_secrets(&doc, ""),
        Err(DecryptError::WrongPassphrase)
    );

    let mut changed = doc.clone();
    let ct = changed["secrets"]["ciphertext"]
        .as_str()
        .unwrap()
        .to_owned();
    let flipped = if ct.starts_with('A') { "B" } else { "A" };
    changed["secrets"]["ciphertext"] = json!(format!("{flipped}{}", &ct[1..]));
    assert_eq!(
        decrypt_secrets(&changed, PASSPHRASE),
        Err(DecryptError::WrongPassphrase)
    );

    let mut damaged = doc.clone();
    damaged["secrets"]["encryption"]["nonce"] = json!("not base64 !!");
    assert_eq!(
        decrypt_secrets(&damaged, PASSPHRASE),
        Err(DecryptError::Malformed)
    );
    let mut hungry = doc.clone();
    hungry["secrets"]["encryption"]["memoryKib"] = json!(u32::MAX);
    assert_eq!(
        decrypt_secrets(&hungry, PASSPHRASE),
        Err(DecryptError::Malformed)
    );
    assert_eq!(
        decrypt_secrets(&json!({"secrets": {}}), PASSPHRASE),
        Err(DecryptError::NotEncrypted)
    );
    assert_eq!(
        decrypt_secrets(&json!({}), PASSPHRASE),
        Err(DecryptError::Malformed)
    );

    let plain = export(&mut f, json!({"password": PASSWORD})).await.json();
    assert_eq!(
        decrypt_secrets(&plain, PASSPHRASE),
        Err(DecryptError::NotEncrypted)
    );
    f.h.shutdown().await;
}

#[tokio::test]
async fn both_kinds_of_export_are_audited_without_the_passphrase() {
    let mut f = fixture().await;
    assert_eq!(
        export(&mut f, json!({"password": PASSWORD})).await.status,
        StatusCode::OK
    );
    assert_eq!(
        export(
            &mut f,
            json!({"password": PASSWORD, "passphrase": PASSPHRASE})
        )
        .await
        .status,
        StatusCode::OK
    );
    let rows = audit_rows(&f.h);
    assert_eq!(rows.len(), 2, "{rows:?}");
    // Newest first.
    let (with, without) = (&rows[0], &rows[1]);
    for e in [with, without] {
        assert_eq!(e.actor, "alice");
        assert_eq!(e.result, "ok");
        let d: Value = serde_json::from_str(&e.detail).unwrap();
        assert!(
            d["file"].as_str().unwrap().starts_with("opsbox-config-"),
            "{d}"
        );
        assert!(!e.detail.contains(PASSPHRASE) && !e.detail.contains(PASSWORD));
        assert!(!e.detail.contains(SECRET_VAR));
    }
    let d = |e: &opsbox_store::AuditEntry| -> Value { serde_json::from_str(&e.detail).unwrap() };
    assert_eq!(d(without)["includeSecrets"], false);
    assert_eq!(d(with)["includeSecrets"], true);
    // 2 secret service vars + 1 Compose project + 1 backup destination + 1 alert channel.
    assert_eq!(d(with)["secretsExported"], 5);
    f.h.shutdown().await;
}

#[tokio::test]
async fn a_wrong_password_exports_nothing_and_is_audited() {
    let mut f = fixture().await;
    for body in [
        json!({"password": "nope"}),
        json!({"password": "", "passphrase": PASSPHRASE}),
    ] {
        let r = export(&mut f, body).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
        assert!(!r.text().contains("opsbox-config/1"));
    }
    let rows = audit_rows(&f.h);
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|e| e.result == "denied" && !e.detail.contains(PASSPHRASE))
    );
    f.h.shutdown().await;
}

#[tokio::test]
async fn only_an_administrator_with_a_session_may_export() {
    let mut f = fixture().await;
    let mut anon = f.h.browser();
    assert_eq!(
        anon.post("/api/v1/config/export", json!({"password": PASSWORD}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    for role in [Role::Viewer, Role::Deployer] {
        let mut b = f.h.signed_in(&format!("{role:?}-user"), role);
        let r = b
            .post("/api/v1/config/export", json!({"password": PASSWORD}))
            .await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{role:?}");
    }
    // A passphrase that is too short is refused before anything is read.
    let r = export(&mut f, json!({"password": PASSWORD, "passphrase": "short"})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text());
    // Unknown fields are refused too.
    let r = export(
        &mut f,
        json!({"password": PASSWORD, "includeSecrets": true}),
    )
    .await;
    assert!(r.status.is_client_error());
    assert!(
        audit_rows(&f.h).is_empty(),
        "nothing was exported, nothing is logged as exported"
    );
    f.h.shutdown().await;
}

#[tokio::test]
async fn an_empty_passphrase_means_no_secrets() {
    let mut f = fixture().await;
    let doc = export(&mut f, json!({"password": PASSWORD, "passphrase": ""}))
        .await
        .json();
    assert_eq!(doc["secrets"]["included"], false);
    f.h.shutdown().await;
}

// ------------------------------------------------------------- every service field counted

fn service_record_fields() -> Vec<String> {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../opsbox-store/src/deploy.rs"
    ))
    .unwrap();
    let body = src
        .split("pub struct ServiceRecord {")
        .nth(1)
        .and_then(|s| s.split("\n}").next())
        .expect("ServiceRecord is in deploy.rs");
    body.lines()
        .filter_map(|l| l.trim().strip_prefix("pub "))
        .filter_map(|l| l.split(':').next())
        .map(str::to_owned)
        .collect()
}

#[test]
fn every_service_field_is_exported_or_deliberately_excluded() {
    use opsbox_api::export::{SERVICE_EXCLUDED, SERVICE_EXPORTED};
    let fields = service_record_fields();
    assert!(fields.len() > 25, "could not read the struct: {fields:?}");
    for f in &fields {
        let exported = SERVICE_EXPORTED.iter().filter(|(n, _)| n == f).count();
        let excluded = SERVICE_EXCLUDED.iter().filter(|(n, _)| n == f).count();
        assert_eq!(
            exported + excluded,
            1,
            "`{f}` must be exported or excluded, once"
        );
    }
    for (n, _) in SERVICE_EXPORTED.iter().chain(SERVICE_EXCLUDED) {
        assert!(
            fields.contains(&(*n).to_owned()),
            "`{n}` is not a ServiceRecord field"
        );
    }
    for (n, why) in SERVICE_EXCLUDED {
        assert!(!why.is_empty(), "`{n}` needs a reason");
    }
}

#[tokio::test]
async fn the_exported_fields_really_are_in_the_document() {
    use opsbox_api::export::SERVICE_EXPORTED;
    let mut f = fixture().await;
    let doc = export(&mut f, json!({"password": PASSWORD})).await.json();
    let web = doc["services"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "web")
        .unwrap();
    for (field, path) in SERVICE_EXPORTED {
        let node = path
            .split('.')
            .try_fold(web, |n, k| n.get(k))
            .unwrap_or_else(|| panic!("`{field}` should be at `{path}`"));
        // The fixture sets every optional field, so each one carries a real value.
        if !matches!(*field, "compose_stack_id") {
            assert!(!node.is_null(), "`{field}` at `{path}` is empty");
        }
    }
    // The excluded ones are listed in the file itself.
    let listed: Vec<&str> = doc["notExported"]["service"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["field"].as_str().unwrap())
        .collect();
    assert!(listed.contains(&"active_container") && listed.contains(&"project_id"));
    f.h.shutdown().await;
}

// ---------------------------------------------------------------------------- unit checks

#[test]
fn sealing_round_trips_and_every_seal_is_different() {
    let kdf = Kdf {
        memory_kib: 64,
        iterations: 1,
        parallelism: 1,
    };
    let a = seal_secrets("a passphrase", br#"{"k":"v"}"#, kdf).unwrap();
    let b = seal_secrets("a passphrase", br#"{"k":"v"}"#, kdf).unwrap();
    assert_ne!(a["ciphertext"], b["ciphertext"]);
    assert_ne!(a["encryption"]["salt"], b["encryption"]["salt"]);
    let doc = |s: &Value| {
        let mut d = json!({"secrets": {"included": true}});
        d["secrets"]["encryption"] = s["encryption"].clone();
        d["secrets"]["ciphertext"] = s["ciphertext"].clone();
        d
    };
    assert_eq!(
        decrypt_secrets(&doc(&a), "a passphrase").unwrap(),
        json!({"k": "v"})
    );
    assert_eq!(
        decrypt_secrets(&doc(&a), "another"),
        Err(DecryptError::WrongPassphrase)
    );
}

#[test]
fn file_names_are_safe() {
    use opsbox_api::export::{filename, filename_host};
    assert_eq!(filename_host("Web-01.Example.COM"), "web-01.example.com");
    assert_eq!(filename_host("../../etc/passwd"), "etc-passwd");
    assert_eq!(filename_host("  "), "opsbox");
    assert_eq!(filename_host("a\"b\r\nc"), "a-b--c");
    let t = time::OffsetDateTime::from_unix_timestamp(1_795_000_000).unwrap();
    assert_eq!(filename("box", t), "opsbox-config-box-2026-11-18.json");
}
