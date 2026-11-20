//! Image settings use the real PATCH/GET router without needing Docker or a helper.
mod common;

use axum::http::{Method, StatusCode};
use common::{Harness, Opts};
use opsbox_proto::Role;
use opsbox_store::{NewService, ServiceKind, SourceKind};
use serde_json::json;

fn image_service(h: &Harness) {
    let (project, env) = h.store.ensure_default_project(1).unwrap();
    h.store
        .create_service(NewService {
            project_id: &project.id,
            env_id: &env.id,
            name: "image-app",
            kind: ServiceKind::App,
            owner_id: None,
            source_kind: SourceKind::DockerImage,
            repo: "nginx:1.26",
            branch: "main",
            build_method: "Docker image",
            dockerfile_path: None,
            start_cmd: None,
            port: 80,
            health_path: "/",
            domain: None,
            image_ref: Some("nginx:1.26"),
            deploy_key_id: None,
            auto_deploy: false,
            memory_limit_mb: Some(128),
            cpu_limit_millis: None,
            graph_x: 0,
            graph_y: 0,
            note: None,
            now: 1,
        })
        .unwrap();
}

#[tokio::test]
async fn patch_image_persists_new_reference_and_preserves_it_when_omitted() {
    let h = Harness::new(false);
    image_service(&h);
    let mut admin = h.signed_in("image-admin", Role::Admin);
    for (field, image) in [
        ("image", "nginx:1.27-alpine"),
        ("image_ref", "registry.example.test:5000/team/app:v2"),
        ("imageRef", "ghcr.io/acme/app@sha256:abcdef"),
    ] {
        let reply = admin
            .send(
                Method::PATCH,
                "/api/v1/services/image-app",
                Some(json!({field: format!(" {image} ")})),
                Opts::default(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        assert_eq!(reply.json()["imageRef"], image);
        let stored = h.store.get_service("image-app").unwrap().unwrap();
        assert_eq!(stored.image_ref.as_deref(), Some(image));
        let fetched = admin.get("/api/v1/services/image-app").await;
        assert_eq!(fetched.status, StatusCode::OK, "{}", fetched.text());
        assert_eq!(fetched.json()["imageRef"], image);
        let reply = admin
            .send(
                Method::PATCH,
                "/api/v1/services/image-app",
                Some(json!({"healthPath": "/healthz"})),
                Opts::default(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK, "{}", reply.text());
        assert_eq!(reply.json()["imageRef"], image);
    }
}

#[tokio::test]
async fn patch_invalid_image_returns_field_error_without_changing_settings() {
    let h = Harness::new(false);
    image_service(&h);
    let mut admin = h.signed_in("image-admin", Role::Admin);
    for image in ["", "   ", "-bad", "nginx;id", "nginx tag", "../nginx"] {
        let reply = admin
            .send(
                Method::PATCH,
                "/api/v1/services/image-app",
                Some(json!({"image": image, "healthPath": "/changed"})),
                Opts::default(),
            )
            .await;
        assert_eq!(reply.status, StatusCode::BAD_REQUEST, "{}", reply.text());
        assert_eq!(reply.json()["error"]["field"], "image", "{}", reply.text());
        let stored = h.store.get_service("image-app").unwrap().unwrap();
        assert_eq!(stored.image_ref.as_deref(), Some("nginx:1.26"));
        assert_eq!(stored.health_path, "/");
    }
}
