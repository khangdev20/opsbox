//! GitHub App manifest flow, repo picker, webhook HMAC verification, panel-proxy
//! reachability, public repo URL inspection, and SSH deploy keys.

mod common;

use std::collections::HashMap;
use std::process::Command;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, patch, post};
use common::{Harness, Opts};
use hmac::{Hmac, KeyInit, Mac};
use http_body_util::BodyExt;
use opsbox_deploy::GitHubConfig;
use opsbox_proto::Role;
use opsbox_proxy::proxy::{ProxyBody, RouteTable, handle_https};
use opsbox_proxy::{Route, UpstreamTls};
use serde_json::{Value, json};
use sha2::Sha256;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

async fn send(b: &mut common::Browser, m: Method, p: &str, body: Option<Value>) -> common::Reply {
    b.send(m, p, body, Opts::default()).await
}

fn generate_test_rsa_pkcs1() -> String {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.pem");
    let out = Command::new("openssl")
        .args(["genrsa", "-out"])
        .arg(&path)
        .arg("2048")
        .output()
        .expect("openssl binary available in test environment");
    assert!(
        out.status.success(),
        "openssl genrsa failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::read_to_string(&path).unwrap()
}

fn sign_webhook(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let digest = mac.finalize().into_bytes();
    let mut out = String::from("sha256=");
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

#[derive(Clone, Default)]
struct MockGitHubState {
    pem: String,
    webhook_secret: String,
    installed: Arc<Mutex<bool>>,
    synced_webhook: Arc<Mutex<Option<(String, String)>>>,
    saw_jwt: Arc<Mutex<Vec<String>>>,
}

async fn mock_gh_server(state: MockGitHubState) -> (String, oneshot::Sender<()>) {
    async fn convert(
        State(st): State<MockGitHubState>,
        Path(code): Path<String>,
    ) -> impl IntoResponse {
        if code != "valid-code-123" {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(json!({"message": "Not Found"})),
            );
        }
        (
            StatusCode::CREATED,
            axum::Json(json!({
                "id": 98765,
                "slug": "opsbox-test-srv",
                "name": "opsbox test-srv",
                "owner": { "login": "acme-co", "type": "Organization" },
                "html_url": "https://github.com/organizations/acme-co/settings/apps/opsbox-test-srv",
                "pem": st.pem,
                "webhook_secret": st.webhook_secret,
            })),
        )
    }

    async fn installations(
        State(st): State<MockGitHubState>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        if let Some(auth) = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            && let Some(jwt) = auth.strip_prefix("Bearer ")
        {
            st.saw_jwt.lock().unwrap().push(jwt.to_owned());
        } else {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(json!({"message": "Bad credentials"})),
            );
        }
        if *st.installed.lock().unwrap() {
            (
                StatusCode::OK,
                axum::Json(json!([{
                    "id": 555111,
                    "account": { "login": "acme-co", "type": "Organization" },
                    "html_url": "https://github.com/organizations/acme-co/settings/installations/555111",
                    "repository_selection": "selected",
                }])),
            )
        } else {
            (StatusCode::OK, axum::Json(json!([])))
        }
    }

    async fn access_token(
        State(st): State<MockGitHubState>,
        Path(id): Path<i64>,
        headers: HeaderMap,
    ) -> impl IntoResponse {
        if id != 555111 || !headers.contains_key(header::AUTHORIZATION) {
            return (
                StatusCode::NOT_FOUND,
                axum::Json(json!({"message": "Not Found"})),
            );
        }
        let _ = st;
        (
            StatusCode::CREATED,
            axum::Json(json!({ "token": "ghs_mock_installation_token" })),
        )
    }

    async fn inst_repos(headers: HeaderMap) -> impl IntoResponse {
        let auth = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if auth != "Bearer ghs_mock_installation_token" {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(json!({"message": "Unauthorized"})),
            );
        }
        (
            StatusCode::OK,
            axum::Json(json!({
                "total_count": 4,
                "repositories": [
                    {
                        "full_name": "acme-co/shop",
                        "name": "shop",
                        "owner": { "login": "acme-co", "type": "Organization" },
                        "private": true,
                        "language": "TypeScript",
                        "default_branch": "main",
                        "html_url": "https://github.com/acme-co/shop",
                        "pushed_at": "2026-11-17T20:00:00Z"
                    },
                    {
                        "full_name": "acme-co/shop-api",
                        "name": "shop-api",
                        "owner": { "login": "acme-co", "type": "Organization" },
                        "private": true,
                        "language": "TypeScript",
                        "default_branch": "main",
                        "html_url": "https://github.com/acme-co/shop-api",
                        "pushed_at": "2026-11-17T19:00:00Z"
                    },
                    {
                        "full_name": "acme-co/landing",
                        "name": "landing",
                        "owner": { "login": "acme-co", "type": "Organization" },
                        "private": false,
                        "language": "Astro",
                        "default_branch": "main",
                        "html_url": "https://github.com/acme-co/landing",
                        "pushed_at": "2026-11-16T10:00:00Z"
                    },
                    {
                        "full_name": "acme-co/docs",
                        "name": "docs",
                        "owner": { "login": "acme-co", "type": "Organization" },
                        "private": true,
                        "language": "Markdown",
                        "default_branch": "main",
                        "html_url": "https://github.com/acme-co/docs",
                        "pushed_at": "2026-11-10T10:00:00Z"
                    }
                ]
            })),
        )
    }

    async fn public_repo(Path((owner, repo)): Path<(String, String)>) -> impl IntoResponse {
        match (owner.as_str(), repo.as_str()) {
            ("public-org", "rust-app") => (
                StatusCode::OK,
                axum::Json(json!({
                    "full_name": "public-org/rust-app",
                    "name": "rust-app",
                    "owner": { "login": "public-org", "type": "Organization" },
                    "private": false,
                    "language": "Rust",
                    "default_branch": "trunk",
                    "html_url": "https://github.com/public-org/rust-app",
                    "pushed_at": "2026-11-17T12:00:00Z"
                })),
            ),
            ("acme-co", "secret-repo") => (
                StatusCode::OK,
                axum::Json(json!({
                    "full_name": "acme-co/secret-repo",
                    "name": "secret-repo",
                    "owner": { "login": "acme-co", "type": "Organization" },
                    "private": true,
                    "language": "Go",
                    "default_branch": "main",
                    "html_url": "https://github.com/acme-co/secret-repo",
                    "pushed_at": "2026-11-17T12:00:00Z"
                })),
            ),
            _ => (
                StatusCode::NOT_FOUND,
                axum::Json(json!({"message": "Not Found"})),
            ),
        }
    }

    async fn contents(Path((owner, repo)): Path<(String, String)>) -> impl IntoResponse {
        let entries: Value = match (owner.as_str(), repo.as_str()) {
            ("acme-co", "shop") => json!([{"name": "Dockerfile"}, {"name": "package.json"}]),
            ("acme-co", "shop-api") => json!([{"name": "package.json"}, {"name": "src"}]),
            ("acme-co", "landing") => {
                json!([{"name": "package.json"}, {"name": "astro.config.mjs"}])
            }
            ("acme-co", "docs") => json!([{"name": "README.md"}]),
            ("public-org", "rust-app") => json!([{"name": "Cargo.toml"}, {"name": "src"}]),
            _ => {
                return (
                    StatusCode::NOT_FOUND,
                    axum::Json(json!({"message": "Not Found"})),
                );
            }
        };
        (StatusCode::OK, axum::Json(entries))
    }

    async fn package_json(Path((owner, repo)): Path<(String, String)>) -> impl IntoResponse {
        match (owner.as_str(), repo.as_str()) {
            ("acme-co", "shop-api") => (
                StatusCode::OK,
                r#"{"scripts":{"build":"tsc","start":"node dist/server.js"},"engines":{"node":">=22"},"dependencies":{"express":"^4.19.0"}}"#,
            ),
            ("acme-co", "landing") => (
                StatusCode::OK,
                r#"{"scripts":{"build":"astro build"},"dependencies":{"astro":"^4.0.0"}}"#,
            ),
            _ => (StatusCode::NOT_FOUND, "Not Found"),
        }
    }

    async fn hook_config(
        State(st): State<MockGitHubState>,
        axum::Json(body): axum::Json<Value>,
    ) -> impl IntoResponse {
        let url = body["url"].as_str().unwrap_or("").to_owned();
        let secret = body["secret"].as_str().unwrap_or("").to_owned();
        *st.synced_webhook.lock().unwrap() = Some((url, secret));
        (StatusCode::OK, axum::Json(json!({ "url": body["url"] })))
    }

    let app = Router::new()
        .route("/app-manifests/{code}/conversions", post(convert))
        .route("/app/installations", get(installations))
        .route("/app/installations/{id}/access_tokens", post(access_token))
        .route("/app/hook/config", patch(hook_config))
        .route("/installation/repositories", get(inst_repos))
        .route("/repos/{owner}/{repo}", get(public_repo))
        .route("/repos/{owner}/{repo}/contents/", get(contents))
        .route(
            "/repos/{owner}/{repo}/contents/package.json",
            get(package_json),
        )
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });
    (format!("http://{addr}"), tx)
}

#[tokio::test]
async fn github_roles_manifest_repos_webhooks_and_fallbacks_end_to_end() {
    let pem = generate_test_rsa_pkcs1();
    let wh_secret = "whsec_super_secret_test_key_12345".to_owned();
    let gh_state = MockGitHubState {
        pem: pem.clone(),
        webhook_secret: wh_secret.clone(),
        installed: Arc::new(Mutex::new(false)),
        synced_webhook: Arc::new(Mutex::new(None)),
        saw_jwt: Arc::new(Mutex::new(Vec::new())),
    };
    let (api_base, stop_gh) = mock_gh_server(gh_state.clone()).await;

    let h = Harness::with_github(
        true,
        GitHubConfig {
            api_base: api_base.clone(),
            web_base: "https://github.example.test".into(),
        },
    );

    // 1. Authorization & token scope matrix.
    let mut anon = h.browser();
    assert_eq!(
        anon.get("/api/v1/github").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        anon.get("/api/v1/github/repos").await.status,
        StatusCode::UNAUTHORIZED
    );

    let mut viewer = h.signed_in("view", Role::Viewer);
    assert_eq!(
        viewer.get("/api/v1/github").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        viewer.get("/api/v1/github/repos").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(
            &mut viewer,
            Method::POST,
            "/api/v1/github/deploy-keys",
            Some(json!({"repo": "acme/app"})),
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );

    let mut deployer = h.signed_in("dep", Role::Deployer);
    assert_eq!(deployer.get("/api/v1/github").await.status, StatusCode::OK);
    assert_eq!(
        deployer.get("/api/v1/github/repos").await.status,
        StatusCode::OK
    );
    assert_eq!(
        send(
            &mut deployer,
            Method::POST,
            "/api/v1/github/manifest",
            Some(json!({})),
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send(&mut deployer, Method::DELETE, "/api/v1/github", None)
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    // A `deploy` API token can read /github and /github/repos, never mutate.
    let mut admin = h.signed_in("boss", Role::Admin);
    let tok_resp = send(
        &mut admin,
        Method::POST,
        "/api/v1/tokens",
        Some(json!({ "name": "ci-read", "scope": "deploy" })),
    )
    .await;
    assert_eq!(tok_resp.status, StatusCode::OK, "{}", tok_resp.text());
    let deploy_token = tok_resp.json()["secret"].as_str().unwrap().to_owned();
    let req_read = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/github/repos")
        .header(header::AUTHORIZATION, format!("Bearer {deploy_token}"))
        .body(Body::empty())
        .unwrap();
    let resp_read = tower::ServiceExt::oneshot(h.app.clone(), req_read)
        .await
        .unwrap();
    assert_eq!(resp_read.status(), StatusCode::OK);

    let req_mut = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/github/deploy-keys")
        .header(header::AUTHORIZATION, format!("Bearer {deploy_token}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"repo":"acme/app"}"#))
        .unwrap();
    let resp_mut = tower::ServiceExt::oneshot(h.app.clone(), req_mut)
        .await
        .unwrap();
    assert_eq!(resp_mut.status(), StatusCode::FORBIDDEN);

    // 2. Callback redirect endpoints handle valid and invalid inputs safely without session cookies.
    let cb = anon
        .get("/api/v1/github/callback?code=valid-code-123&state=state_abc")
        .await;
    assert_eq!(cb.status, StatusCode::OK);
    assert!(
        cb.text()
            .contains("/#/settings?github_code=valid-code-123&amp;github_state=state_abc"),
        "{}",
        cb.text()
    );

    let bad_cb = anon
        .get("/api/v1/github/callback?code=%3Cscript%3E&state=ok")
        .await;
    assert_eq!(bad_cb.status, StatusCode::BAD_REQUEST);

    let setup_cb = anon
        .get("/api/v1/github/setup?installation_id=555111&setup_action=install")
        .await;
    assert_eq!(setup_cb.status, StatusCode::OK);
    assert!(
        setup_cb
            .text()
            .contains("/#/settings?github_installation_id=555111&amp;setup_action=install"),
        "{}",
        setup_cb.text()
    );

    // 3. Manifest flow without panel domain omits hook_attributes and warns panelDomainRequired.
    let m1 = send(
        &mut admin,
        Method::POST,
        "/api/v1/github/manifest",
        Some(json!({ "org": "acme-co" })),
    )
    .await;
    assert_eq!(m1.status, StatusCode::OK, "{}", m1.text());
    assert_eq!(m1.json()["panelDomainRequired"], true);
    assert_eq!(m1.json()["webhookConfigured"], false);
    let state_tok = m1.json()["state"].as_str().unwrap().to_owned();
    assert!(
        m1.json()["actionUrl"].as_str().unwrap().starts_with(
            "https://github.example.test/organizations/acme-co/settings/apps/new?state="
        )
    );
    let manifest_v: Value = serde_json::from_str(m1.json()["manifest"].as_str().unwrap()).unwrap();
    assert!(manifest_v.get("hook_attributes").is_none());

    // Set panel domain and start another manifest -> includes webhook URL.
    h.store
        .set_setting("opsbox.panel.domain", &json!("panel.example.com"))
        .unwrap();
    let m2 = send(
        &mut admin,
        Method::POST,
        "/api/v1/github/manifest",
        Some(json!({ "org": "acme-co" })),
    )
    .await;
    assert_eq!(m2.status, StatusCode::OK);
    assert_eq!(m2.json()["panelDomainRequired"], false);
    assert_eq!(m2.json()["webhookConfigured"], true);
    let state_tok_2 = m2.json()["state"].as_str().unwrap().to_owned();
    let manifest_v2: Value = serde_json::from_str(m2.json()["manifest"].as_str().unwrap()).unwrap();
    assert_eq!(
        manifest_v2["hook_attributes"]["url"],
        "https://panel.example.com/api/v1/github/webhook"
    );

    // Wrong state is rejected.
    let bad_complete = send(
        &mut admin,
        Method::POST,
        "/api/v1/github/complete",
        Some(json!({ "code": "valid-code-123", "state": "wrong-state" })),
    )
    .await;
    assert_eq!(bad_complete.status, StatusCode::BAD_REQUEST);

    // Valid completion seals RSA private key and webhook secret via `priv`.
    let complete = send(
        &mut admin,
        Method::POST,
        "/api/v1/github/complete",
        Some(json!({ "code": "valid-code-123", "state": state_tok_2 })),
    )
    .await;
    assert_eq!(complete.status, StatusCode::OK, "{}", complete.text());
    assert_eq!(complete.json()["connected"], true);
    assert_eq!(complete.json()["installed"], false);
    assert_eq!(complete.json()["appSlug"], "opsbox-test-srv");
    assert!(!complete.text().contains(&pem));
    assert!(!complete.text().contains(&wh_secret));

    // Single-use state cannot be replayed.
    let replay = send(
        &mut admin,
        Method::POST,
        "/api/v1/github/complete",
        Some(json!({ "code": "valid-code-123", "state": state_tok_2 })),
    )
    .await;
    assert_eq!(replay.status, StatusCode::BAD_REQUEST);

    // Verify stored SQLite row holds sealed `enc:v1:...` ciphertexts, never plaintext!
    let db_app = h.store.get_github_app().unwrap().unwrap();
    assert!(db_app.private_key_enc.starts_with("enc:v1:"));
    assert!(db_app.webhook_secret_enc.starts_with("enc:v1:"));
    assert!(!db_app.private_key_enc.contains("RSA PRIVATE KEY"));
    assert!(!db_app.webhook_secret_enc.contains(&wh_secret));
    let _ = state_tok;

    // 4. Mark app installed on GitHub, sync installation, and fetch repos + detected build methods!
    *gh_state.installed.lock().unwrap() = true;
    let inst = send(
        &mut admin,
        Method::POST,
        "/api/v1/github/installation",
        Some(json!({ "installationId": 555111 })),
    )
    .await;
    assert_eq!(inst.status, StatusCode::OK, "{}", inst.text());
    assert_eq!(inst.json()["installed"], true);
    assert_eq!(inst.json()["account"], "acme-co");
    assert_eq!(
        inst.json()["manageUrl"],
        "https://github.com/organizations/acme-co/settings/installations/555111"
    );
    assert!(!gh_state.saw_jwt.lock().unwrap().is_empty());

    let repos = deployer.get("/api/v1/github/repos").await;
    assert_eq!(repos.status, StatusCode::OK, "{}", repos.text());
    let list = repos.json()["repos"].as_array().unwrap().clone();
    assert_eq!(list.len(), 4);
    let by_name: HashMap<String, String> = list
        .iter()
        .map(|r| {
            (
                r["name"].as_str().unwrap().to_owned(),
                r["build"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(by_name["acme-co/shop"], "Dockerfile");
    assert_eq!(by_name["acme-co/shop-api"], "Nixpacks · Node 22");
    assert_eq!(by_name["acme-co/landing"], "static site");
    assert_eq!(by_name["acme-co/docs"], "nothing to build");

    let filtered = deployer.get("/api/v1/github/repos?q=shop-api").await;
    assert_eq!(filtered.status, StatusCode::OK);
    assert_eq!(filtered.json()["repos"].as_array().unwrap().len(), 1);

    // Webhook sync pushes URL & unsealed secret to GitHub.
    let wh_sync = send(
        &mut admin,
        Method::POST,
        "/api/v1/github/webhook/sync",
        None,
    )
    .await;
    assert_eq!(wh_sync.status, StatusCode::OK, "{}", wh_sync.text());
    assert_eq!(
        *gh_state.synced_webhook.lock().unwrap(),
        Some((
            "https://panel.example.com/api/v1/github/webhook".into(),
            wh_secret.clone()
        ))
    );

    // 5. Webhook verification (unsigned/tampered rejected; ping & push persisted).
    let push_body = br#"{"ref":"refs/heads/main","after":"a3f9c21abcdef","deleted":false,"repository":{"full_name":"acme-co/shop"},"pusher":{"name":"alice"},"head_commit":{"message":"Ship feature"}}"#;
    let bad_sig_req = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/github/webhook")
        .header("x-github-event", "push")
        .header(
            "x-hub-signature-256",
            "sha256=0000000000000000000000000000000000000000000000000000000000000000",
        )
        .body(Body::from(&push_body[..]))
        .unwrap();
    let bad_sig_resp = tower::ServiceExt::oneshot(h.app.clone(), bad_sig_req)
        .await
        .unwrap();
    assert_eq!(bad_sig_resp.status(), StatusCode::UNAUTHORIZED);

    let sig = sign_webhook(&wh_secret, push_body);
    let ok_req = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/github/webhook")
        .header("x-github-event", "push")
        .header("x-github-delivery", "deliv-direct-1")
        .header("x-hub-signature-256", &sig)
        .body(Body::from(&push_body[..]))
        .unwrap();
    let ok_resp = tower::ServiceExt::oneshot(h.app.clone(), ok_req)
        .await
        .unwrap();
    assert_eq!(ok_resp.status(), StatusCode::OK);
    let deliveries = h.store.list_github_webhook_deliveries(10).unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].event, "push");
    assert_eq!(deliveries[0].repo.as_deref(), Some("acme-co/shop"));

    // 6. Fallbacks: public repo URL & SSH deploy key.
    let pub_ok = send(
        &mut deployer,
        Method::POST,
        "/api/v1/github/public-repo",
        Some(json!({ "url": "https://github.com/public-org/rust-app.git" })),
    )
    .await;
    assert_eq!(pub_ok.status, StatusCode::OK, "{}", pub_ok.text());
    assert_eq!(pub_ok.json()["build"], "Nixpacks · Rust");
    assert_eq!(pub_ok.json()["defaultBranch"], "trunk");

    let pub_priv = send(
        &mut deployer,
        Method::POST,
        "/api/v1/github/public-repo",
        Some(json!({ "url": "https://github.com/acme-co/secret-repo" })),
    )
    .await;
    assert_eq!(pub_priv.status, StatusCode::BAD_REQUEST);

    let pub_ssrf = send(
        &mut deployer,
        Method::POST,
        "/api/v1/github/public-repo",
        Some(json!({ "url": "https://127.0.0.1/acme/shop" })),
    )
    .await;
    assert_eq!(pub_ssrf.status, StatusCode::BAD_REQUEST);

    // Deploy key creation seals private key via `priv`, returns public key only.
    let dk_bad = send(
        &mut deployer,
        Method::POST,
        "/api/v1/github/deploy-keys",
        Some(json!({ "repo": "-oProxyCommand=id" })),
    )
    .await;
    assert_eq!(dk_bad.status, StatusCode::BAD_REQUEST);

    let dk = send(
        &mut deployer,
        Method::POST,
        "/api/v1/github/deploy-keys",
        Some(json!({ "repo": "acme-co/secret-repo" })),
    )
    .await;
    assert_eq!(dk.status, StatusCode::OK, "{}", dk.text());
    assert_eq!(dk.json()["repo"], "acme-co/secret-repo");
    assert_eq!(
        dk.json()["sshUrl"],
        "git@github.com:acme-co/secret-repo.git"
    );
    assert!(
        dk.json()["publicKey"]
            .as_str()
            .unwrap()
            .starts_with("ssh-ed25519 ")
    );
    assert!(!dk.text().contains("OPENSSH PRIVATE KEY"));

    let dk_id = dk.json()["id"].as_str().unwrap().to_owned();
    let stored_dk = h.store.get_deploy_key(&dk_id).unwrap().unwrap();
    assert!(stored_dk.private_key_enc.starts_with("enc:v1:"));
    assert!(!stored_dk.private_key_enc.contains("OPENSSH PRIVATE KEY"));

    let dk_list = deployer.get("/api/v1/github/deploy-keys").await;
    assert_eq!(dk_list.status, StatusCode::OK);
    assert_eq!(dk_list.json().as_array().unwrap().len(), 1);

    // Settings returns real github & deployKeys fields.
    let settings = admin.get("/api/v1/settings").await;
    assert_eq!(settings.status, StatusCode::OK);
    assert_eq!(settings.json()["github"]["connected"], true);
    assert_eq!(settings.json()["github"]["installed"], true);
    assert_eq!(settings.json()["deployKeys"].as_array().unwrap().len(), 1);

    // Delete deploy key and disconnect GitHub.
    let dk_del = send(
        &mut deployer,
        Method::DELETE,
        &format!("/api/v1/github/deploy-keys/{dk_id}"),
        None,
    )
    .await;
    assert_eq!(dk_del.status, StatusCode::OK);
    assert!(h.store.list_deploy_keys().unwrap().is_empty());

    let disc = send(&mut admin, Method::DELETE, "/api/v1/github", None).await;
    assert_eq!(disc.status, StatusCode::OK);
    assert!(h.store.get_github_app().unwrap().is_none());

    let repos_after = deployer.get("/api/v1/github/repos").await;
    assert_eq!(repos_after.status, StatusCode::OK);
    assert_eq!(repos_after.json()["connected"], false);
    assert!(repos_after.json()["repos"].as_array().unwrap().is_empty());

    let _ = stop_gh.send(());
    h.shutdown().await;
}

#[tokio::test]
async fn webhook_is_reachable_through_panel_proxy_even_with_allow_list_active() {
    let pem = generate_test_rsa_pkcs1();
    let wh_secret = "whsec_proxy_test_secret_67890".to_owned();
    let gh_state = MockGitHubState {
        pem,
        webhook_secret: wh_secret.clone(),
        installed: Arc::new(Mutex::new(true)),
        synced_webhook: Arc::new(Mutex::new(None)),
        saw_jwt: Arc::new(Mutex::new(Vec::new())),
    };
    let (api_base, stop_gh) = mock_gh_server(gh_state).await;

    let h = Harness::with_github(
        true,
        GitHubConfig {
            api_base,
            web_base: "https://github.example.test".into(),
        },
    );
    h.store
        .set_setting("opsbox.panel.domain", &json!("panel.example.com"))
        .unwrap();

    let mut admin = h.signed_in("boss", Role::Admin);
    let m = send(
        &mut admin,
        Method::POST,
        "/api/v1/github/manifest",
        Some(json!({})),
    )
    .await;
    let state_tok = m.json()["state"].as_str().unwrap().to_owned();
    let c = send(
        &mut admin,
        Method::POST,
        "/api/v1/github/complete",
        Some(json!({ "code": "valid-code-123", "state": state_tok })),
    )
    .await;
    assert_eq!(c.status, StatusCode::OK);

    // Restrict panel access to an office CIDR only.
    let allow_set = send(
        &mut admin,
        Method::PUT,
        "/api/v1/security/allow-ips",
        Some(json!({ "value": "10.99.0.0/16", "confirm": true })),
    )
    .await;
    assert_eq!(allow_set.status, StatusCode::OK);

    let api_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_addr = api_listener.local_addr().unwrap();
    // The proxy proves itself to `serve` with a shared secret; only then are the client
    // address and scheme it forwards believed.
    const PROXY_SECRET: &str = "test-proxy-secret-test-proxy-secret";
    let app_for_srv = opsbox_api::router(h.state.clone().with_proxy_secret(PROXY_SECRET));
    let (stop_api_tx, stop_api_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        let svc = app_for_srv.into_make_service_with_connect_info::<std::net::SocketAddr>();
        let _ = axum::serve(api_listener, svc)
            .with_graceful_shutdown(async {
                let _ = stop_api_rx.await;
            })
            .await;
    });

    let routes = RouteTable::default();
    routes.set(vec![Route {
        host: "panel.example.com".into(),
        upstream: api_addr.to_string(),
        upstream_tls: UpstreamTls::Plain,
        redirect_www: false,
        redirect_to: None,
        redirect_https: true,
        hsts: true,
        max_body_bytes: 1024 * 1024,
        is_panel: true,
        forward_secret: Some(opsbox_proto::Secret::new(PROXY_SECRET)),
    }]);

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();
    let routes_clone = routes.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = proxy_listener.accept().await {
            // Simulate an external GitHub webhook sender IP outside the 10.99.0.0/16 allow-list.
            let external_peer = std::net::SocketAddr::from(([140, 82, 115, 10], 44321));
            let rc = routes_clone.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |req| {
                    handle_https(req, external_peer, rc.clone(), 1024 * 1024)
                });
                let _ = hyper_util::server::conn::auto::Builder::new(
                    hyper_util::rt::TokioExecutor::new(),
                )
                .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
                .await;
            });
        }
    });

    // External IP hitting /api/v1/settings through the proxy is blocked by the allow-list.
    {
        let s = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(s))
                .await
                .unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let req = Request::builder()
            .method(Method::GET)
            .uri("/api/v1/settings")
            .header(header::HOST, "panel.example.com")
            .body(
                http_body_util::Empty::<Bytes>::new()
                    .map_err(|n| match n {})
                    .boxed(),
            )
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // External GitHub IP hitting /api/v1/github/webhook through the proxy succeeds with valid HMAC!
    let push_body = br#"{"ref":"refs/heads/main","after":"deadbeef1234","deleted":false,"repository":{"full_name":"acme-co/shop"},"pusher":{"name":"alice"}}"#;
    let sig = sign_webhook(&wh_secret, push_body);
    {
        let s = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
        let (mut sender, conn) =
            hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(s))
                .await
                .unwrap();
        tokio::spawn(async move {
            let _ = conn.await;
        });
        let full_body: ProxyBody = http_body_util::Full::new(Bytes::from_static(push_body))
            .map_err(|never| match never {})
            .boxed();
        let req = Request::builder()
            .method(Method::POST)
            .uri("/api/v1/github/webhook")
            .header(header::HOST, "panel.example.com")
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-github-event", "push")
            .header("x-github-delivery", "deliv-proxy-ext")
            .header("x-hub-signature-256", &sig)
            .body(full_body)
            .unwrap();
        let resp = sender.send_request(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["event"], "push");
        assert_eq!(v["repo"], "acme-co/shop");
        assert_eq!(v["branch"], "main");
    }

    let _ = stop_api_tx.send(());
    let _ = stop_gh.send(());
    h.shutdown().await;
}
