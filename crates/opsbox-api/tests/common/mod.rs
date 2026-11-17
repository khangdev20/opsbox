//! Shared test harness: a real router, a real privileged helper on a temp socket,
//! and a tiny "browser" that keeps cookies.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode, header};
use http_body_util::BodyExt;
use opsbox_api::{AppState, FilesSettings, SecureCookies, router};
use opsbox_auth::AuthConfig;
use opsbox_auth::password::HashParams;
use opsbox_priv::{PrivClient, PrivConfig, PrivServer};
use opsbox_proto::Role;
use opsbox_store::{SessionKind, Store};
use serde_json::Value;
use tokio::sync::oneshot;
use tower::ServiceExt;

pub const HOST: &str = "127.0.0.1:8443";
pub const ORIGIN: &str = "http://127.0.0.1:8443";
pub const PASSWORD: &str = "correct horse battery staple";

pub struct Harness {
    pub app: Router,
    pub state: AppState,
    pub store: Arc<Store>,
    /// A private scratch area: `apps/` (the deployers' jail), `xfer/` and `trash/` live here.
    pub root: std::path::PathBuf,
    _dir: tempfile::TempDir,
    stop: Option<oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl Harness {
    /// `with_priv = false` leaves nothing listening on the socket path.
    pub fn new(with_priv: bool) -> Self {
        Self::with_upload_limit(with_priv, 2 << 20)
    }

    /// A fresh harness with no admin yet, for testing first-run setup gating.
    pub fn new_unclaimed(with_priv: bool) -> Self {
        Self::build(with_priv, 2 << 20, false, None)
    }

    pub fn with_upload_limit(with_priv: bool, max_upload: u64) -> Self {
        Self::build(with_priv, max_upload, true, None)
    }

    pub fn with_github(with_priv: bool, github: opsbox_deploy::GitHubConfig) -> Self {
        Self::build(with_priv, 2 << 20, true, Some(github))
    }

    fn build(
        with_priv: bool,
        max_upload: u64,
        seed_admin: bool,
        github: Option<opsbox_deploy::GitHubConfig>,
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("apps")).unwrap();
        let socket = dir.path().join("priv.sock");
        let probe = dir.path().join("probe");
        std::fs::write(&probe, b"").unwrap();
        let uid = std::fs::metadata(&probe).unwrap().uid();

        let (mut stop, mut task) = (None, None);
        if with_priv {
            let mut cfg = PrivConfig::new(&socket, vec![uid]);
            cfg.master_key_path = dir.path().join("master.key");
            cfg.xfer_dir = root.join("xfer");
            cfg.trash_dir = root.join("trash");
            cfg.apps_dir = root.join("apps");
            let server = PrivServer::bind(cfg).unwrap();
            let (tx, rx) = oneshot::channel::<()>();
            task = Some(tokio::spawn(server.run(async {
                let _ = rx.await;
            })));
            stop = Some(tx);
        }
        let store = Arc::new(Store::open_in_memory().unwrap());
        if seed_admin {
            let u = store
                .create_user("__seed_admin", Role::Admin, "unused-hash", 0)
                .unwrap();
            store.set_pending_totp(&u.id, "unused", 0).unwrap();
            store.enable_totp(&u.id, 0, 0).unwrap();
        }
        let mut state = AppState::new(
            store.clone(),
            PrivClient::new(&socket).with_timeout(Duration::from_secs(5)),
            "9.9.9-test",
            SecureCookies::Auto,
            AuthConfig {
                hash_params: HashParams {
                    memory_kib: 8,
                    iterations: 1,
                    parallelism: 1,
                },
                ..AuthConfig::default()
            },
        )
        .with_files(FilesSettings {
            apps_dir: root.join("apps"),
            xfer_dir: root.join("xfer"),
            max_upload,
        });
        if let Some(gh) = github {
            state = state.with_github(gh);
        }
        Self {
            app: router(state.clone()),
            state,
            store,
            root,
            _dir: dir,
            stop,
            task,
        }
    }

    pub async fn shutdown(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
            self.task.take().unwrap().await.unwrap();
        }
    }

    /// A browser with no cookies yet.
    pub fn browser(&self) -> Browser {
        Browser {
            app: self.app.clone(),
            jar: BTreeMap::new(),
            set_cookies: Vec::new(),
        }
    }

    /// A browser that is already signed in as a new user of `role`, bypassing the
    /// password and TOTP steps (those have their own tests).
    pub fn signed_in(&self, name: &str, role: Role) -> Browser {
        let user = self
            .store
            .create_user(name, role, "unused-hash", 0)
            .unwrap();
        let token = opsbox_auth::tokens::new_token().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        self.store
            .create_session(
                &user.id,
                &opsbox_auth::tokens::hash_token(&token),
                SessionKind::Web,
                None,
                None,
                now,
            )
            .unwrap();
        let mut b = self.browser();
        b.jar.insert("opsbox_session".into(), token.clone());
        b.jar.insert(
            "opsbox_csrf".into(),
            opsbox_auth::tokens::csrf_token(&token),
        );
        b
    }
}

pub struct Browser {
    pub app: Router,
    pub jar: BTreeMap<String, String>,
    /// Every Set-Cookie header seen, raw, for attribute checks.
    pub set_cookies: Vec<String>,
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: axum::http::HeaderMap,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {:?}", String::from_utf8_lossy(&self.body)))
    }
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
    pub fn header(&self, name: &str) -> &str {
        self.headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }
    pub fn error_code(&self) -> String {
        self.json()["error"]["code"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    }
}

/// What to send besides the method, path and body.
#[derive(Clone)]
pub struct Opts {
    pub csrf: bool,
    pub origin: Option<String>,
    pub host: String,
    pub extra: Vec<(String, String)>,
}

impl Default for Opts {
    /// What the page's own JavaScript sends: CSRF header, same-origin Origin.
    fn default() -> Self {
        Self {
            csrf: true,
            origin: Some(ORIGIN.into()),
            host: HOST.into(),
            extra: Vec::new(),
        }
    }
}

impl Opts {
    pub fn bare() -> Self {
        Self {
            csrf: false,
            origin: None,
            ..Self::default()
        }
    }
}

impl Browser {
    pub async fn send(
        &mut self,
        method: Method,
        path: &str,
        body: Option<Value>,
        opts: Opts,
    ) -> Reply {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, &opts.host);
        if !self.jar.is_empty() {
            let cookie = self
                .jar
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            req = req.header(header::COOKIE, cookie);
        }
        if let (true, Some(c)) = (opts.csrf, self.jar.get("opsbox_csrf")) {
            req = req.header("x-opsbox-csrf", c);
        }
        if let Some(o) = &opts.origin {
            req = req.header(header::ORIGIN, o);
        }
        for (k, v) in &opts.extra {
            req = req.header(k, v);
        }
        let req = match body {
            Some(v) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(v.to_string()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let resp: Response<Body> = self.app.clone().oneshot(req).await.unwrap();
        let (parts, body) = resp.into_parts();
        for c in parts.headers.get_all(header::SET_COOKIE) {
            let raw = c.to_str().unwrap().to_owned();
            let (kv, _) = raw.split_once(';').unwrap_or((&raw, ""));
            let (k, v) = kv.split_once('=').unwrap();
            if raw.contains("Max-Age=0") {
                self.jar.remove(k);
            } else {
                self.jar.insert(k.to_owned(), v.to_owned());
            }
            self.set_cookies.push(raw);
        }
        Reply {
            status: parts.status,
            headers: parts.headers,
            body: body.collect().await.unwrap().to_bytes().to_vec(),
        }
    }

    /// Send raw bytes (an upload).
    pub async fn send_bytes(
        &mut self,
        method: Method,
        path: &str,
        bytes: Vec<u8>,
        opts: Opts,
    ) -> Reply {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, &opts.host)
            .header(header::CONTENT_TYPE, "application/octet-stream");
        if !self.jar.is_empty() {
            req = req.header(
                header::COOKIE,
                self.jar
                    .iter()
                    .map(|(k, v)| format!("{k}={v}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            );
        }
        if let (true, Some(c)) = (opts.csrf, self.jar.get("opsbox_csrf")) {
            req = req.header("x-opsbox-csrf", c);
        }
        if let Some(o) = &opts.origin {
            req = req.header(header::ORIGIN, o);
        }
        let resp: Response<Body> = self
            .app
            .clone()
            .oneshot(req.body(Body::from(bytes)).unwrap())
            .await
            .unwrap();
        let (parts, body) = resp.into_parts();
        Reply {
            status: parts.status,
            headers: parts.headers,
            body: body.collect().await.unwrap().to_bytes().to_vec(),
        }
    }

    pub async fn get(&mut self, path: &str) -> Reply {
        self.send(Method::GET, path, None, Opts::bare()).await
    }

    pub async fn post(&mut self, path: &str, body: Value) -> Reply {
        self.send(Method::POST, path, Some(body), Opts::default())
            .await
    }
}
