//! HTTP behaviour of the router, driven in-process against a real privileged helper
//! on a temporary Unix socket.

use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use opsbox_api::{AppState, router};
use opsbox_priv::{PrivClient, PrivConfig, PrivServer};
use opsbox_store::Store;
use serde_json::Value;
use tokio::sync::oneshot;
use tower::ServiceExt;

struct Harness {
    app: Router,
    _dir: tempfile::TempDir,
    stop: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Harness {
    /// `with_priv = false` leaves nothing listening on the socket path.
    fn new(with_priv: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("priv.sock");
        let probe = dir.path().join("probe");
        std::fs::write(&probe, b"").unwrap();
        let uid = std::fs::metadata(&probe).unwrap().uid();

        let (mut stop, mut task) = (None, None);
        if with_priv {
            let server = PrivServer::bind(PrivConfig::new(&socket, vec![uid])).unwrap();
            let (tx, rx) = oneshot::channel::<()>();
            task = Some(tokio::spawn(server.run(async {
                let _ = rx.await;
            })));
            stop = Some(tx);
        }
        let state = AppState {
            store: Arc::new(Store::open_in_memory().unwrap()),
            priv_client: PrivClient::new(&socket).with_timeout(Duration::from_secs(5)),
            version: "9.9.9-test",
        };
        Self {
            app: router(state),
            _dir: dir,
            stop,
            task,
        }
    }

    async fn send(&self, req: Request<Body>) -> Response<Body> {
        self.app.clone().oneshot(req).await.unwrap()
    }

    async fn get(&self, path: &str) -> Response<Body> {
        self.send(Request::get(path).body(Body::empty()).unwrap())
            .await
    }

    async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
            self.task.take().unwrap().await.unwrap();
        }
    }
}

async fn json(resp: Response<Body>) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("not JSON ({e}): {bytes:?}"))
}

async fn text(resp: Response<Body>) -> String {
    String::from_utf8(
        resp.into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap()
}

fn header_str<'a>(resp: &'a Response<Body>, name: impl header::AsHeaderName) -> &'a str {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
}

// ---------------------------------------------------------------- health

#[tokio::test]
async fn healthz_is_ok_without_any_dependency() {
    // No helper running: /healthz must still answer, it only says "the process is up".
    let h = Harness::new(false);
    let resp = h.get("/healthz").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(header_str(&resp, "content-type").starts_with("application/json"));
    assert_eq!(
        json(resp).await,
        serde_json::json!({"status": "ok", "version": "9.9.9-test"})
    );
}

#[tokio::test]
async fn readyz_reports_ready_when_database_and_helper_answer() {
    let h = Harness::new(true);
    let resp = h.get("/readyz").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        json(resp).await,
        serde_json::json!({"status": "ready", "checks": {"database": "ok", "privileged_helper": "ok"}})
    );
    h.shutdown().await;
}

#[tokio::test]
async fn readyz_is_503_when_the_helper_is_down() {
    let h = Harness::new(false);
    let resp = h.get("/readyz").await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json(resp).await;
    assert_eq!(body["status"], "not_ready");
    assert_eq!(body["checks"]["database"], "ok");
    assert_eq!(body["checks"]["privileged_helper"], "failing");
}

// ---------------------------------------------------------------- request ids

#[tokio::test]
async fn every_response_carries_a_fresh_request_id_and_inbound_ids_are_ignored() {
    let h = Harness::new(false);
    let a = h.get("/healthz").await;
    let b = h
        .send(
            Request::get("/healthz")
                .header("x-request-id", "attacker-chosen")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    let (ida, idb) = (
        header_str(&a, "x-request-id").to_owned(),
        header_str(&b, "x-request-id").to_owned(),
    );
    assert_eq!(ida.len(), 26, "a ULID: {ida}");
    assert_eq!(idb.len(), 26, "a ULID: {idb}");
    assert_ne!(ida, idb);
}

// ---------------------------------------------------------------- /api/v1/host

#[tokio::test]
async fn host_returns_real_facts_in_the_shape_the_ui_expects() {
    let h = Harness::new(true);
    let resp = h.get("/api/v1/host").await;
    assert_eq!(resp.status(), StatusCode::OK);
    let v = json(resp).await;
    for key in ["name", "os", "kernel"] {
        assert!(v[key].as_str().is_some_and(|s| !s.is_empty()), "{key}: {v}");
    }
    assert!(v["cores"].as_u64().unwrap() >= 1);
    assert!(v["memTotal"].as_f64().unwrap() > 0.0);
    assert!(v["upDays"].is_u64());
    assert!(v["uptimeSecs"].is_u64());
    assert_eq!(v["version"], "9.9.9-test");
    h.shutdown().await;
}

#[tokio::test]
async fn host_error_uses_the_spec_format_when_the_helper_is_down() {
    let h = Harness::new(false);
    let resp = h.get("/api/v1/host").await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let v = json(resp).await;
    assert_eq!(v["error"]["code"], "unavailable");
    let message = v["error"]["message"].as_str().unwrap();
    assert!(
        message.ends_with('.') && message.contains("privileged helper"),
        "{message}"
    );
    // No internals (socket paths, OS error text) leak to the client.
    assert!(
        !message.contains(".sock") && !message.contains("os error"),
        "{message}"
    );
    assert!(v["error"].get("field").is_none());
}

// ---------------------------------------------------------------- errors

#[tokio::test]
async fn unknown_api_paths_are_json_404s_not_the_ui() {
    let h = Harness::new(false);
    for path in ["/api/v1/nope", "/api/v2/host", "/api"] {
        let resp = h.get(path).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(json(resp).await["error"]["code"], "not_found", "{path}");
    }
}

#[tokio::test]
async fn wrong_method_is_a_json_405() {
    let h = Harness::new(false);
    let resp = h
        .send(
            Request::builder()
                .method(Method::POST)
                .uri("/api/v1/host")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(json(resp).await["error"]["code"], "method_not_allowed");

    let resp = h
        .send(
            Request::builder()
                .method(Method::DELETE)
                .uri("/index.html")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(json(resp).await["error"]["code"], "method_not_allowed");
}

// ---------------------------------------------------------------- embedded UI

#[tokio::test]
async fn root_serves_the_ui_and_revalidates() {
    let h = Harness::new(false);
    let resp = h.get("/").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(header_str(&resp, "content-type").starts_with("text/html"));
    assert_eq!(header_str(&resp, "cache-control"), "no-cache");
    assert_eq!(header_str(&resp, "x-content-type-options"), "nosniff");
    let etag = header_str(&resp, "etag").to_owned();
    assert!(etag.starts_with('"') && etag.ends_with('"'));
    let body = text(resp).await;
    assert!(
        body.contains("<div id=\"app\">"),
        "index.html content: {body:.200}"
    );

    // Conditional request: unchanged file answers 304 with no body.
    let resp = h
        .send(
            Request::get("/")
                .header(header::IF_NONE_MATCH, &etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(header_str(&resp, "etag"), etag);
    assert!(text(resp).await.is_empty());
}

#[tokio::test]
async fn cache_headers_follow_the_spec() {
    let h = Harness::new(false);
    let js = h.get("/js/api.js").await;
    assert_eq!(js.status(), StatusCode::OK);
    assert!(
        header_str(&js, "content-type").contains("javascript"),
        "{}",
        header_str(&js, "content-type")
    );
    assert_eq!(header_str(&js, "cache-control"), "no-cache");

    let font = h.get("/fonts/Geist.woff2").await;
    assert_eq!(font.status(), StatusCode::OK);
    assert_eq!(
        header_str(&font, "cache-control"),
        "public, max-age=31536000, immutable"
    );

    let vendor = h.get("/vendor/monaco/LICENSE").await;
    assert_eq!(vendor.status(), StatusCode::OK);
    assert_eq!(
        header_str(&vendor, "cache-control"),
        "public, max-age=31536000, immutable"
    );
}

#[tokio::test]
async fn text_assets_are_gzipped_when_asked_and_fonts_are_not() {
    let h = Harness::new(false);
    let resp = h
        .send(
            Request::get("/js/api.js")
                .header(header::ACCEPT_ENCODING, "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(header_str(&resp, "content-encoding"), "gzip");

    let resp = h
        .send(
            Request::get("/fonts/Geist.woff2")
                .header(header::ACCEPT_ENCODING, "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(header_str(&resp, "content-encoding"), "");
}

#[tokio::test]
async fn head_has_headers_but_no_body() {
    let h = Harness::new(false);
    let resp = h
        .send(
            Request::builder()
                .method(Method::HEAD)
                .uri("/index.html")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        header_str(&resp, "content-length")
            .parse::<usize>()
            .unwrap()
            > 0
    );
    assert!(text(resp).await.is_empty());
}

#[tokio::test]
async fn dev_tooling_and_paths_outside_web_are_not_served() {
    let h = Harness::new(false);
    for path in [
        "/serve.py",
        "/nope.js",
        "/../Cargo.toml",
        "/js/../../Cargo.toml",
        "/vendor/../../Cargo.toml",
    ] {
        let resp = h.get(path).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "{path}");
        assert_eq!(json(resp).await["error"]["code"], "not_found", "{path}");
    }
}
