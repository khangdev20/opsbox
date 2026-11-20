//! Elevated Compose must be refused before contacting the helper or creating rows.
mod common;

use axum::http::StatusCode;
use common::Harness;
use opsbox_proto::Role;
use serde_json::json;

#[tokio::test]
async fn deployer_cannot_confirm_elevated_compose() {
    let h = Harness::new(false);
    let mut dep = h.signed_in("dep", Role::Deployer);
    for setting in [
        "privileged: true",
        "volumes: ['/:/host']",
        "volumes: ['/var/run/docker.sock:/var/run/docker.sock']",
        "volumes: [{type: bind, source: /etc, target: /host}]",
        "cap_add: [SYS_ADMIN]",
        "pid: host",
        "ipc: host",
        "devices: ['/dev/sda:/dev/sda']",
        "security_opt: ['apparmor:unconfined']",
        "userns_mode: host",
        "sysctls: {kernel.shmmax: 1000000}",
        "volumes_from: [other]",
        "network_mode: host",
    ] {
        for confirm in [false, true] {
            let yaml = format!("services:\n  web:\n    image: nginx:alpine\n    {setting}\n");
            let r = dep
                .post(
                    "/api/v1/compose",
                    json!({
                        "stack": "risky", "yaml": yaml, "confirm": confirm,
                    }),
                )
                .await;
            assert_eq!(
                r.status,
                StatusCode::FORBIDDEN,
                "{setting}, confirm={confirm}: {}",
                r.text()
            );
            assert!(r.text().contains("needs an administrator"), "{}", r.text());
            assert!(h.store.get_compose_stack("risky").unwrap().is_none());
        }
    }
}

#[tokio::test]
async fn deployer_cannot_submit_uninspectable_compose() {
    let h = Harness::new(false);
    let mut dep = h.signed_in("dep", Role::Deployer);
    let r = dep
        .post(
            "/api/v1/compose",
            json!({
                "stack": "bad", "yaml": "services: [", "confirm": true,
            }),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
    assert!(r.text().contains("needs an administrator"));
}

#[tokio::test]
async fn deployer_cannot_redeploy_previously_confirmed_elevated_stack() {
    use opsbox_store::{NewService, ServiceKind, SourceKind, UpsertComposeStack};
    let h = Harness::new(false);
    let mut dep = h.signed_in("dep", Role::Deployer);
    let owner = h.store.user_by_name("dep").unwrap().unwrap();
    let (proj, env) = h.store.ensure_default_project(1).unwrap();
    let stack = h
        .store
        .upsert_compose_stack(UpsertComposeStack {
            project_id: &proj.id,
            env_id: &env.id,
            name: "legacy",
            owner_id: Some(&owner.id),
            compose_yaml: "services:\n  web:\n    image: nginx:alpine\n    privileged: true\n",
            project_name: "opsbox-legacy",
            service_names: &["web".into()],
            depends_edges: &[],
            confirmed: true,
            now: 1,
        })
        .unwrap();
    let svc = h
        .store
        .create_service_with_meta(
            NewService {
                project_id: &proj.id,
                env_id: &env.id,
                name: "legacy-web",
                kind: ServiceKind::Compose,
                owner_id: Some(&owner.id),
                source_kind: SourceKind::DockerImage,
                repo: "",
                branch: "web",
                build_method: "Compose",
                dockerfile_path: None,
                start_cmd: None,
                port: 80,
                health_path: "tcp",
                domain: None,
                image_ref: Some("nginx:alpine"),
                deploy_key_id: None,
                auto_deploy: false,
                memory_limit_mb: Some(256),
                cpu_limit_millis: None,
                graph_x: 0,
                graph_y: 0,
                note: None,
                now: 1,
            },
            Some(&stack.id),
            None,
            true,
        )
        .unwrap();
    let r = dep
        .post(&format!("/api/v1/services/{}/deploys", svc.id), json!({}))
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
    assert!(r.text().contains("needs an administrator"));
}

#[tokio::test]
async fn full_token_cannot_redeploy_previously_confirmed_elevated_stack() {
    use opsbox_store::{NewService, ServiceKind, SourceKind, UpsertComposeStack};
    let h = Harness::new(false);
    let mut dep = h.signed_in("dep", Role::Admin);
    let secret = dep
        .post(
            "/api/v1/tokens",
            json!({"name":"compose-probe","scope":"full"}),
        )
        .await
        .json()["secret"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut script = h.browser();
    let owner = h.store.user_by_name("dep").unwrap().unwrap();
    let (proj, env) = h.store.ensure_default_project(1).unwrap();
    let stack = h
        .store
        .upsert_compose_stack(UpsertComposeStack {
            project_id: &proj.id,
            env_id: &env.id,
            name: "legacy",
            owner_id: Some(&owner.id),
            compose_yaml: "services:\n  web:\n    image: nginx:alpine\n    privileged: true\n",
            project_name: "opsbox-legacy",
            service_names: &["web".into()],
            depends_edges: &[],
            confirmed: true,
            now: 1,
        })
        .unwrap();
    let svc = h
        .store
        .create_service_with_meta(
            NewService {
                project_id: &proj.id,
                env_id: &env.id,
                name: "legacy-web",
                kind: ServiceKind::Compose,
                owner_id: Some(&owner.id),
                source_kind: SourceKind::DockerImage,
                repo: "",
                branch: "web",
                build_method: "Compose",
                dockerfile_path: None,
                start_cmd: None,
                port: 80,
                health_path: "tcp",
                domain: None,
                image_ref: Some("nginx:alpine"),
                deploy_key_id: None,
                auto_deploy: false,
                memory_limit_mb: Some(256),
                cpu_limit_millis: None,
                graph_x: 0,
                graph_y: 0,
                note: None,
                now: 1,
            },
            Some(&stack.id),
            None,
            true,
        )
        .unwrap();
    let r = script
        .send(
            axum::http::Method::POST,
            &format!("/api/v1/services/{}/deploys", svc.id),
            Some(json!({})),
            common::Opts {
                csrf: false,
                origin: None,
                extra: vec![("authorization".into(), format!("Bearer {secret}"))],
                ..common::Opts::default()
            },
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text());
    assert!(r.text().contains("needs an administrator"));
}
