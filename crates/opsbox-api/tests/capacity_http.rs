//! Capacity refusals must happen before host mutations or task enqueue.
mod common;
use axum::http::{Method, StatusCode};
use common::{Harness, Opts};
use opsbox_proto::Role;
use serde_json::json;
fn small(ram: u64) -> opsbox_capacity::Profile {
    let mut p = common::capacity_profile();
    p.memory_mb = ram;
    p.available_mb = ram;
    p
}
fn image(name: &str) -> serde_json::Value {
    json!({"name":name,"sourceKind":"docker_image","image":"nginx:1.27-alpine","deployNow":false})
}
#[tokio::test]
async fn dry_run_and_structured_conflict_have_the_same_policy() {
    let h = Harness::with_capacity(small(1024));
    let mut b = h.signed_in("owner", Role::Admin);
    let response = b.get("/api/v1/capacity").await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.json()["allocatable"]["allocatable_mb"], 554);
    let dry = b
        .get("/api/v1/capacity/check?feature=app&memory_mb=600")
        .await;
    assert_eq!(dry.json()["fit"]["status"], "blocked");
    let mut payload = image("too-large");
    payload["memoryLimitMb"] = json!(600);
    let blocked = b.post("/api/v1/services", payload).await;
    assert_eq!(blocked.status, StatusCode::CONFLICT, "{}", blocked.text());
    let c = &blocked.json()["error"]["capacity"];
    assert_eq!(c["asked"]["memory_mb"], 600);
    assert_eq!(c["free"]["allocatable_mb"], 554);
    assert_eq!(c["fit"], dry.json()["fit"]);
    assert_eq!(c["reserves"]["os_mb"], 192);
    assert!(h.store.list_all_services().unwrap().is_empty());
    h.shutdown().await;
}
#[tokio::test]
async fn defaults_are_persisted_for_app_and_database_workloads() {
    let h = Harness::new(true);
    let mut b = h.signed_in("owner", Role::Admin);
    let app = b.post("/api/v1/services", image("default-app")).await;
    assert_eq!(app.status, StatusCode::CREATED, "{}", app.text());
    assert_eq!(
        h.store
            .get_service("default-app")
            .unwrap()
            .unwrap()
            .memory_limit_mb,
        Some(128)
    );
    let db = b
        .post(
            "/api/v1/templates/postgres/instances",
            json!({"name":"default-db","backup":false}),
        )
        .await;
    assert_eq!(db.status, StatusCode::CREATED, "{}", db.text());
    assert_eq!(
        h.store
            .get_service("default-db")
            .unwrap()
            .unwrap()
            .memory_limit_mb,
        Some(256)
    );
    h.shutdown().await;
}
#[tokio::test]
async fn owner_override_requires_exact_confirmation_and_is_audited() {
    let h = Harness::with_capacity(small(512));
    let mut b = h.signed_in("owner", Role::Admin);
    let mut payload = image("forced");
    payload["override"] = json!(true);
    assert_eq!(
        b.post("/api/v1/services", payload.clone()).await.status,
        StatusCode::CONFLICT
    );
    payload["capacityConfirmation"] = json!(opsbox_capacity::requirements::CONFIRMATION);
    let r = b.post("/api/v1/services", payload).await;
    assert_eq!(r.status, StatusCode::CREATED, "{}", r.text());
    assert!(
        h.store
            .recent_audit(20)
            .unwrap()
            .iter()
            .any(|a| a.action == "capacity.override" && a.actor == "owner")
    );
    assert!(
        h.store
            .get_setting("capacity.override.forced")
            .unwrap()
            .is_some()
    );
    h.store.verify_audit().unwrap();
    h.shutdown().await;
}
#[tokio::test]
async fn deployer_and_full_api_token_cannot_override_even_when_workload_fits() {
    let h = Harness::new(true);
    let mut d = h.signed_in("deployer", Role::Deployer);
    let mut payload = image("unauthorized");
    payload["override"] = json!(true);
    payload["capacityConfirmation"] = json!(opsbox_capacity::requirements::CONFIRMATION);
    assert_eq!(
        d.post("/api/v1/services", payload.clone()).await.status,
        StatusCode::FORBIDDEN
    );
    let mut owner = h.signed_in("owner", Role::Admin);
    let token = owner
        .post(
            "/api/v1/tokens",
            json!({"name":"capacity-test","scope":"full"}),
        )
        .await
        .json();
    let secret = token["secret"].as_str().unwrap();
    let mut script = h.browser();
    let r = script
        .send(
            Method::POST,
            "/api/v1/services",
            Some(payload),
            Opts {
                csrf: false,
                origin: None,
                extra: vec![("authorization".into(), format!("Bearer {secret}"))],
                ..Opts::default()
            },
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
    assert!(h.store.list_all_services().unwrap().is_empty());
    h.shutdown().await;
}
#[tokio::test]
async fn compose_database_resize_pooler_and_extensions_are_admitted_before_actions() {
    let h = Harness::with_capacity(small(1024));
    let mut b = h.signed_in("owner", Role::Admin);
    let r=b.post("/api/v1/compose",json!({"stack":"large","yaml":"services:\n  a:\n    image: nginx:alpine\n    mem_limit: 400m\n  b:\n    image: redis:7\n    mem_limit: 400m\n"})).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text());
    assert_eq!(r.json()["error"]["capacity"]["asked"]["memory_mb"], 800);
    let app = b.post("/api/v1/services", image("resize")).await;
    assert_eq!(app.status, StatusCode::CREATED);
    let r = b
        .send(
            Method::PATCH,
            "/api/v1/services/resize",
            Some(json!({"memoryLimitMb":900})),
            Opts::default(),
        )
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(
        h.store
            .get_service("resize")
            .unwrap()
            .unwrap()
            .memory_limit_mb,
        Some(128)
    );
    h.shutdown().await;
}
#[tokio::test]
async fn missing_cgroup_and_memory_pressure_cannot_silently_start_a_workload() {
    for pressure in [false, true] {
        let mut p = small(4096);
        if pressure {
            p.available_mb = 100;
        } else {
            p.cgroup_v2 = false;
        }
        let h = Harness::with_capacity(p);
        let mut b = h.signed_in("owner", Role::Admin);
        let r = b.post("/api/v1/services", image("blocked")).await;
        assert_eq!(r.status, StatusCode::CONFLICT);
        assert!(
            r.json()["error"]["capacity"]["fit"]["reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r.as_str().unwrap().contains(if pressure {
                    "pressure"
                } else {
                    "Cgroup"
                }))
        );
        h.shutdown().await;
    }
}

#[tokio::test]
async fn queued_workloads_block_database_resize_pooler_and_memory_adding_extensions() {
    let h = Harness::with_capacity(small(1024));
    let mut b = h.signed_in("owner", Role::Admin);
    let mut payload = image("pending");
    payload["memoryLimitMb"] = json!(400);
    assert_eq!(
        b.post("/api/v1/services", payload).await.status,
        StatusCode::CREATED
    );
    let db = b
        .post(
            "/api/v1/templates/postgres/instances",
            json!({"name":"pg","backup":false}),
        )
        .await;
    assert_eq!(db.status, StatusCode::CREATED, "{}", db.text());
    let pg_svc = h.store.get_service("pg").unwrap().unwrap();
    h.store
        .create_deploy(
            &pg_svc.id,
            None,
            "queued",
            opsbox_store::DeployTrigger::Manual,
            "owner",
            0,
        )
        .unwrap();
    let svc = h.store.get_service("pending").unwrap().unwrap();
    h.store
        .create_deploy(
            &svc.id,
            None,
            "queued",
            opsbox_store::DeployTrigger::Manual,
            "owner",
            0,
        )
        .unwrap();
    for (method, path, body, expected) in [
        (
            Method::PATCH,
            "/api/v1/services/pg",
            json!({"memoryLimitMb":512}),
            512,
        ),
        (
            Method::PUT,
            "/api/v1/databases/pg/pooler",
            json!({"enabled":true}),
            128,
        ),
        (
            Method::PUT,
            "/api/v1/databases/pg/extensions/pg_stat_statements",
            json!({"enabled":true}),
            320,
        ),
    ] {
        let r = b.send(method, path, Some(body), Opts::default()).await;
        assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text());
        assert_eq!(
            r.json()["error"]["capacity"]["asked"]["memory_mb"],
            expected
        );
    }
    assert_eq!(
        h.store.get_service("pg").unwrap().unwrap().memory_limit_mb,
        Some(256)
    );
    h.shutdown().await;
}
#[tokio::test]
async fn swap_helper_rejects_deployers_and_non_offered_sizes_without_mutating_host() {
    // Keep free disk ≤20 GB so the offer is 1 GB; 2 GB/4 GB must fail closed before host tools.
    let mut profile = small(1024);
    profile.disks[0].free_mb = 18 * 1024;
    let h = Harness::with_capacity(profile);
    let mut d = h.signed_in("deployer", Role::Deployer);
    assert_eq!(
        d.post("/api/v1/capacity/swap", json!({"size_mb":1024}))
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let mut a = h.signed_in("owner", Role::Admin);
    assert_eq!(
        a.post("/api/v1/capacity/swap", json!({"size_mb":2048}))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        a.post("/api/v1/capacity/swap", json!({"size_mb":4096}))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    h.shutdown().await;
}
