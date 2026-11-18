//! Integration tests for Prompt 10: multi-domain routing per service, DNS checks,
//! ACME HTTP-01 certificates, primary selection, www redirects, Force HTTPS, HSTS,
//! RBAC/token scoping, and renewal-failure attention items.

mod common;

use std::collections::HashMap;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::os::unix::fs::MetadataExt;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::http::{Method, Request, StatusCode};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::Response;
use hyper::header::{CONTENT_TYPE, LOCATION};
use hyper_util::rt::TokioIo;
use instant_acme::{BodyWrapper, BytesResponse, HttpClient};
use opsbox_api::{AppState, SecureCookies, router};
use opsbox_auth::AuthConfig;
use opsbox_auth::password::HashParams;
use opsbox_priv::{PrivClient, PrivConfig, PrivServer};
use opsbox_proto::Role;
use opsbox_proxy::{AcmeDirectory, ProxyClient, ProxyConfig, ProxyServer};
use opsbox_store::{NewService, ServiceKind, SourceKind, TokenScope};
use rcgen14::{
    BasicConstraints, CertificateParams, CertificateSigningRequestParams, DistinguishedName, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::PemObject;
use serde_json::{Value, json};
use time::{Duration as TimeDuration, OffsetDateTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{RwLock, oneshot};
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::crypto::ring;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};
use tower::ServiceExt;

struct MockAcmeState {
    ca_cert_pem: String,
    ca_issuer: Issuer<'static, KeyPair>,
    proxy_http_addr: RwLock<Option<SocketAddr>>,
    validity_days: Mutex<i64>,
    orders: Mutex<HashMap<String, MockOrder>>,
    nonces: Mutex<u64>,
    fail_challenges: Mutex<bool>,
}

#[derive(Clone)]
struct MockOrder {
    domain: String,
    token: String,
    ready: bool,
    cert_pem: Option<String>,
}

impl MockAcmeState {
    fn new(validity_days: i64) -> Arc<Self> {
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        let mut dn = DistinguishedName::new();
        dn.push(rcgen14::DnType::CommonName, "Opsbox Test App ACME CA");
        ca_params.distinguished_name = dn;
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca_key = KeyPair::generate().unwrap();
        let ca_cert = ca_params.self_signed(&ca_key).unwrap();
        let ca_cert_pem = ca_cert.pem();
        let ca_issuer = Issuer::new(ca_params, ca_key);
        Arc::new(Self {
            ca_cert_pem,
            ca_issuer,
            proxy_http_addr: RwLock::new(None),
            validity_days: Mutex::new(validity_days),
            orders: Mutex::new(HashMap::new()),
            nonces: Mutex::new(0),
            fail_challenges: Mutex::new(false),
        })
    }

    fn next_nonce(&self) -> String {
        let mut g = self.nonces.lock().unwrap();
        *g += 1;
        format!("n-{}", *g)
    }
}

struct MockAcmeClient {
    state: Arc<MockAcmeState>,
}

impl HttpClient for MockAcmeClient {
    fn request(
        &self,
        req: Request<BodyWrapper<Bytes>>,
    ) -> Pin<Box<dyn Future<Output = Result<BytesResponse, instant_acme::Error>> + Send>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            let uri = req.uri().to_string();
            let raw_body = req
                .into_body()
                .collect()
                .await
                .unwrap_or_else(|never| match never {})
                .to_bytes();

            let jws_payload: Option<Value> = if raw_body.is_empty() {
                None
            } else {
                let jws: Value = serde_json::from_slice(&raw_body)?;
                let b64 = jws["payload"].as_str().unwrap_or("");
                if b64.is_empty() {
                    None
                } else {
                    let decoded = B64
                        .decode(b64)
                        .map_err(|e| instant_acme::Error::Other(Box::new(e)))?;
                    Some(serde_json::from_slice(&decoded)?)
                }
            };

            let nonce = state.next_nonce();
            let make_json = |status: StatusCode, val: Value, loc: Option<&str>| {
                let mut builder = Response::builder()
                    .status(status)
                    .header("replay-nonce", &nonce)
                    .header(CONTENT_TYPE, "application/json");
                if let Some(l) = loc {
                    builder = builder.header(LOCATION, l);
                }
                let body = Full::new(Bytes::from(val.to_string()));
                Ok(BytesResponse::from(builder.body(body).unwrap()))
            };

            if uri == "https://acme.test/dir" {
                return make_json(
                    StatusCode::OK,
                    json!({
                        "newNonce": "https://acme.test/new-nonce",
                        "newAccount": "https://acme.test/new-acct",
                        "newOrder": "https://acme.test/new-order"
                    }),
                    None,
                );
            }
            if uri == "https://acme.test/new-nonce" {
                let resp = Response::builder()
                    .status(StatusCode::OK)
                    .header("replay-nonce", &nonce)
                    .body(Full::new(Bytes::new()))
                    .unwrap();
                return Ok(BytesResponse::from(resp));
            }
            if uri == "https://acme.test/new-acct" {
                return make_json(
                    StatusCode::CREATED,
                    json!({"status": "valid"}),
                    Some("https://acme.test/acct/1"),
                );
            }
            if uri == "https://acme.test/new-order" {
                let payload = jws_payload.unwrap();
                let domain = payload["identifiers"][0]["value"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let oid = format!("ord-{}", state.orders.lock().unwrap().len() + 1);
                let token = format!("tok-{oid}");
                state.orders.lock().unwrap().insert(
                    oid.clone(),
                    MockOrder {
                        domain: domain.clone(),
                        token,
                        ready: false,
                        cert_pem: None,
                    },
                );
                return make_json(
                    StatusCode::CREATED,
                    json!({
                        "status": "pending",
                        "identifiers": [{"type": "dns", "value": domain}],
                        "authorizations": [format!("https://acme.test/authz/{oid}")],
                        "finalize": format!("https://acme.test/finalize/{oid}")
                    }),
                    Some(&format!("https://acme.test/order/{oid}")),
                );
            }
            if let Some(oid) = uri.strip_prefix("https://acme.test/authz/") {
                let ord = state.orders.lock().unwrap().get(oid).unwrap().clone();
                let status = if ord.ready { "valid" } else { "pending" };
                return make_json(
                    StatusCode::OK,
                    json!({
                        "identifier": {"type": "dns", "value": ord.domain},
                        "status": status,
                        "challenges": [{
                            "type": "http-01",
                            "url": format!("https://acme.test/chal/{oid}"),
                            "status": status,
                            "token": ord.token
                        }]
                    }),
                    None,
                );
            }
            if let Some(oid) = uri.strip_prefix("https://acme.test/chal/") {
                if *state.fail_challenges.lock().unwrap() {
                    return make_json(
                        StatusCode::BAD_REQUEST,
                        json!({
                            "type": "urn:ietf:params:acme:error:connection",
                            "detail": "simulated HTTP-01 timeout"
                        }),
                        None,
                    );
                }
                let (domain, token) = {
                    let g = state.orders.lock().unwrap();
                    let o = g.get(oid).unwrap();
                    (o.domain.clone(), o.token.clone())
                };
                let http_addr = state.proxy_http_addr.read().await.unwrap();
                let mut stream = TcpStream::connect(http_addr).await.unwrap();
                let req_str = format!(
                    "GET /.well-known/acme-challenge/{token} HTTP/1.1\r\nHost: {domain}\r\nConnection: close\r\n\r\n"
                );
                stream.write_all(req_str.as_bytes()).await.unwrap();
                let mut resp_bytes = Vec::new();
                stream.read_to_end(&mut resp_bytes).await.unwrap();
                let resp_text = String::from_utf8_lossy(&resp_bytes);
                assert!(resp_text.starts_with("HTTP/1.1 200 OK"), "{resp_text}");
                state.orders.lock().unwrap().get_mut(oid).unwrap().ready = true;
                return make_json(
                    StatusCode::OK,
                    json!({
                        "type": "http-01",
                        "url": format!("https://acme.test/chal/{oid}"),
                        "status": "valid",
                        "token": token
                    }),
                    None,
                );
            }
            if let Some(oid) = uri.strip_prefix("https://acme.test/order/") {
                let ord = state.orders.lock().unwrap().get(oid).unwrap().clone();
                let status = if ord.cert_pem.is_some() {
                    "valid"
                } else if ord.ready {
                    "ready"
                } else {
                    "pending"
                };
                return make_json(
                    StatusCode::OK,
                    json!({
                        "status": status,
                        "identifiers": [{"type": "dns", "value": ord.domain}],
                        "authorizations": [format!("https://acme.test/authz/{oid}")],
                        "finalize": format!("https://acme.test/finalize/{oid}"),
                        "certificate": ord.cert_pem.as_ref().map(|_| format!("https://acme.test/cert/{oid}"))
                    }),
                    None,
                );
            }
            if let Some(oid) = uri.strip_prefix("https://acme.test/finalize/") {
                let payload = jws_payload.unwrap();
                let csr_b64 = payload["csr"].as_str().unwrap();
                let csr_der = B64.decode(csr_b64).unwrap();
                let mut csr_params =
                    CertificateSigningRequestParams::from_der(&csr_der.into()).unwrap();
                let days = *state.validity_days.lock().unwrap();
                let now = OffsetDateTime::now_utc();
                csr_params.params.not_before = now - TimeDuration::hours(1);
                csr_params.params.not_after = now + TimeDuration::days(days);
                let leaf = csr_params.signed_by(&state.ca_issuer).unwrap();
                let chain_pem = format!("{}{}", leaf.pem(), state.ca_cert_pem);
                let mut g = state.orders.lock().unwrap();
                let ord = g.get_mut(oid).unwrap();
                ord.cert_pem = Some(chain_pem);
                return make_json(
                    StatusCode::OK,
                    json!({
                        "status": "valid",
                        "identifiers": [{"type": "dns", "value": ord.domain}],
                        "authorizations": [format!("https://acme.test/authz/{oid}")],
                        "finalize": format!("https://acme.test/finalize/{oid}"),
                        "certificate": format!("https://acme.test/cert/{oid}")
                    }),
                    None,
                );
            }
            if let Some(oid) = uri.strip_prefix("https://acme.test/cert/") {
                let pem = state
                    .orders
                    .lock()
                    .unwrap()
                    .get(oid)
                    .unwrap()
                    .cert_pem
                    .clone()
                    .unwrap();
                let resp = Response::builder()
                    .status(StatusCode::OK)
                    .header("replay-nonce", &nonce)
                    .header(CONTENT_TYPE, "application/pem-certificate-chain")
                    .body(Full::new(Bytes::from(pem)))
                    .unwrap();
                return Ok(BytesResponse::from(resp));
            }
            panic!("unexpected ACME URI: {uri}");
        })
    }
}

async fn spawn_dynamic_dns(records: Arc<Mutex<HashMap<String, Ipv4Addr>>>) -> SocketAddr {
    let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = sock.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        loop {
            let Ok((len, peer)) = sock.recv_from(&mut buf).await else {
                break;
            };
            if len < 12 {
                continue;
            }
            let mut pos = 12;
            let mut labels = Vec::new();
            while pos < len && buf[pos] != 0 {
                let l = buf[pos] as usize;
                if pos + 1 + l > len {
                    break;
                }
                labels
                    .push(String::from_utf8_lossy(&buf[pos + 1..pos + 1 + l]).to_ascii_lowercase());
                pos += 1 + l;
            }
            if pos + 5 > len {
                continue;
            }
            let qname = labels.join(".");
            let qend = pos + 5;
            let qtype = u16::from_be_bytes([buf[pos + 1], buf[pos + 2]]);
            let answer_ip = records.lock().unwrap().get(&qname).copied();

            let mut resp = Vec::new();
            resp.extend_from_slice(&buf[0..2]);
            let rcode = if answer_ip.is_none() && qtype == 1 {
                3u8
            } else {
                0u8
            };
            resp.push(0x81);
            resp.push(0x80 | rcode);
            resp.extend_from_slice(&[0, 1]);
            if qtype == 1 && answer_ip.is_some() {
                resp.extend_from_slice(&[0, 1]);
            } else {
                resp.extend_from_slice(&[0, 0]);
            }
            resp.extend_from_slice(&[0, 0, 0, 0]);
            resp.extend_from_slice(&buf[12..qend]);
            if qtype == 1
                && let Some(ip) = answer_ip
            {
                resp.extend_from_slice(&[0xc0, 0x0c]);
                resp.extend_from_slice(&1u16.to_be_bytes());
                resp.extend_from_slice(&1u16.to_be_bytes());
                resp.extend_from_slice(&60u32.to_be_bytes());
                resp.extend_from_slice(&4u16.to_be_bytes());
                resp.extend_from_slice(&ip.octets());
            }
            let _ = sock.send_to(&resp, peer).await;
        }
    });
    addr
}

async fn send_cookie(
    app: &axum::Router,
    cookie_header: &str,
    csrf: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("host", common::HOST)
        .header("origin", common::ORIGIN)
        .header("cookie", cookie_header)
        .header("x-opsbox-csrf", csrf);
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            axum::body::Body::from(serde_json::to_vec(&v).unwrap())
        }
        None => axum::body::Body::empty(),
    };
    let resp = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val)
}

async fn send_bearer(
    app: &axum::Router,
    token: &str,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header("host", common::HOST)
        .header("authorization", format!("Bearer {token}"));
    let body = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            axum::body::Body::from(serde_json::to_vec(&v).unwrap())
        }
        None => axum::body::Body::empty(),
    };
    let resp = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val)
}

async fn wait_task(app: &axum::Router, cookie: &str, csrf: &str, task_id: &str) -> Value {
    for _ in 0..100 {
        let (st, val) = send_cookie(
            app,
            cookie,
            csrf,
            Method::GET,
            &format!("/api/v1/tasks/{task_id}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{val}");
        if val["status"] != "running" {
            return val;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("task {task_id} timed out");
}

#[tokio::test]
async fn domains_rbac_validation_and_collisions() {
    let h = common::Harness::new(true);
    let mut anon = h.browser();
    assert_eq!(
        anon.get("/api/v1/services/web/domains").await.status,
        StatusCode::UNAUTHORIZED
    );

    let admin = h
        .store
        .create_user("boss", Role::Admin, "unused", 0)
        .unwrap();
    let d1 = h
        .store
        .create_user("d1", Role::Deployer, "unused", 0)
        .unwrap();
    let d2 = h
        .store
        .create_user("d2", Role::Deployer, "unused", 0)
        .unwrap();
    let viewer = h
        .store
        .create_user("v1", Role::Viewer, "unused", 0)
        .unwrap();

    let (proj, env) = h.store.ensure_default_project(10).unwrap();
    let _web = h
        .store
        .create_service(NewService {
            project_id: &proj.id,
            env_id: &env.id,
            name: "web",
            kind: ServiceKind::App,
            owner_id: Some(&d1.id),
            source_kind: SourceKind::DockerImage,
            repo: "nginx:1.27-alpine",
            branch: "main",
            build_method: "Docker image",
            dockerfile_path: None,
            start_cmd: None,
            port: 3000,
            health_path: "/",
            domain: Some("web.example.com"),
            image_ref: Some("nginx:1.27-alpine"),
            deploy_key_id: None,
            auto_deploy: false,
            memory_limit_mb: None,
            cpu_limit_millis: None,
            graph_x: 0,
            graph_y: 0,
            note: None,
            now: 10,
        })
        .unwrap();
    let _worker = h
        .store
        .create_service(NewService {
            project_id: &proj.id,
            env_id: &env.id,
            name: "worker",
            kind: ServiceKind::App,
            owner_id: Some(&d1.id),
            source_kind: SourceKind::DockerImage,
            repo: "busybox",
            branch: "main",
            build_method: "Docker image",
            dockerfile_path: None,
            start_cmd: None,
            port: 0,
            health_path: "tcp",
            domain: None,
            image_ref: Some("busybox"),
            deploy_key_id: None,
            auto_deploy: false,
            memory_limit_mb: None,
            cpu_limit_millis: None,
            graph_x: 0,
            graph_y: 0,
            note: None,
            now: 10,
        })
        .unwrap();
    let _api = h
        .store
        .create_service(NewService {
            project_id: &proj.id,
            env_id: &env.id,
            name: "api",
            kind: ServiceKind::App,
            owner_id: Some(&d2.id),
            source_kind: SourceKind::DockerImage,
            repo: "nginx:1.27-alpine",
            branch: "main",
            build_method: "Docker image",
            dockerfile_path: None,
            start_cmd: None,
            port: 4000,
            health_path: "/",
            domain: None,
            image_ref: Some("nginx:1.27-alpine"),
            deploy_key_id: None,
            auto_deploy: false,
            memory_limit_mb: None,
            cpu_limit_millis: None,
            graph_x: 0,
            graph_y: 0,
            note: None,
            now: 10,
        })
        .unwrap();

    h.store
        .set_setting(
            opsbox_api::panel::KEY_PANEL_DOMAIN,
            &json!("panel.example.com"),
        )
        .unwrap();

    let mut sess = HashMap::new();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    for (label, u) in [
        ("admin", &admin),
        ("d1", &d1),
        ("d2", &d2),
        ("viewer", &viewer),
    ] {
        let tok = format!("sess-{label}-123456");
        h.store
            .create_session(
                &u.id,
                &opsbox_auth::tokens::hash_token(&tok),
                opsbox_store::SessionKind::Web,
                None,
                None,
                now,
            )
            .unwrap();
        let csrf = opsbox_auth::tokens::csrf_token(&tok);
        sess.insert(label, (format!("opsbox_session={tok}"), csrf));
    }

    let (_, read_tok) = h
        .state
        .auth
        .create_token(&admin, "read-tok", TokenScope::Read, None)
        .unwrap();
    let (_, web_tok) = h
        .state
        .auth
        .create_token(
            &admin,
            "web-tok",
            TokenScope::DeployService("web".into()),
            None,
        )
        .unwrap();

    // Viewer can read, cannot mutate.
    let (vc, vc_csrf) = &sess["viewer"];
    assert_eq!(
        send_cookie(
            &h.app,
            vc,
            vc_csrf,
            Method::GET,
            "/api/v1/services/web/domains",
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    for (m, p, b) in [
        (
            Method::POST,
            "/api/v1/services/web/domains",
            Some(json!({"domain": "x.example.com"})),
        ),
        (Method::POST, "/api/v1/domains/web.example.com/check", None),
        (
            Method::POST,
            "/api/v1/domains/web.example.com/certificate",
            None,
        ),
        (
            Method::PATCH,
            "/api/v1/domains/web.example.com",
            Some(json!({"hsts": false})),
        ),
        (
            Method::POST,
            "/api/v1/domains/web.example.com/primary",
            None,
        ),
        (Method::DELETE, "/api/v1/domains/web.example.com", None),
    ] {
        assert_eq!(
            send_cookie(&h.app, vc, vc_csrf, m, p, b).await.0,
            StatusCode::FORBIDDEN
        );
    }

    // Deployer d2 cannot read or mutate d1's service `web`.
    let (d2c, d2_csrf) = &sess["d2"];
    assert_eq!(
        send_cookie(
            &h.app,
            d2c,
            d2_csrf,
            Method::GET,
            "/api/v1/services/web/domains",
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send_cookie(
            &h.app,
            d2c,
            d2_csrf,
            Method::POST,
            "/api/v1/services/web/domains",
            Some(json!({"domain": "hijack.example.com"})),
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );

    // `read` token cannot read domains; `deploy:web` token can read `web` only and cannot mutate.
    assert_eq!(
        send_bearer(
            &h.app,
            &read_tok,
            Method::GET,
            "/api/v1/services/web/domains",
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        send_bearer(
            &h.app,
            &web_tok,
            Method::GET,
            "/api/v1/services/web/domains",
            None
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send_bearer(
            &h.app,
            &web_tok,
            Method::GET,
            "/api/v1/services/api/domains",
            None
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    for (m, p, b) in [
        (
            Method::POST,
            "/api/v1/services/web/domains",
            Some(json!({"domain": "t.example.com"})),
        ),
        (Method::POST, "/api/v1/domains/web.example.com/check", None),
        (Method::DELETE, "/api/v1/domains/web.example.com", None),
    ] {
        assert_eq!(
            send_bearer(&h.app, &web_tok, m, p, b).await.0,
            StatusCode::FORBIDDEN
        );
    }

    // Worker (port 0), invalid domains, panel collision, and cross-service collision.
    let (d1c, d1_csrf) = &sess["d1"];
    assert_eq!(
        send_cookie(
            &h.app,
            d1c,
            d1_csrf,
            Method::POST,
            "/api/v1/services/worker/domains",
            Some(json!({"domain": "worker.example.com"})),
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    for bad in [
        "localhost",
        "127.0.0.1",
        "*.example.com",
        "http://a.com",
        "a.com/p",
    ] {
        assert_eq!(
            send_cookie(
                &h.app,
                d1c,
                d1_csrf,
                Method::POST,
                "/api/v1/services/web/domains",
                Some(json!({"domain": bad})),
            )
            .await
            .0,
            StatusCode::BAD_REQUEST,
            "{bad}"
        );
    }
    assert_eq!(
        send_cookie(
            &h.app,
            d1c,
            d1_csrf,
            Method::POST,
            "/api/v1/services/web/domains",
            Some(json!({"domain": "panel.example.com"})),
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        send_cookie(
            &h.app,
            d2c,
            d2_csrf,
            Method::POST,
            "/api/v1/services/api/domains",
            Some(json!({"domain": "web.example.com"})),
        )
        .await
        .0,
        StatusCode::CONFLICT
    );

    // Panel domain check refuses domain already owned by `web`.
    let (ac, a_csrf) = &sess["admin"];
    assert_eq!(
        send_cookie(
            &h.app,
            ac,
            a_csrf,
            Method::POST,
            "/api/v1/panel/dns-check",
            Some(json!({"domain": "web.example.com"})),
        )
        .await
        .0,
        StatusCode::CONFLICT
    );

    h.shutdown().await;
}

#[tokio::test]
async fn app_domains_dns_acme_www_redirect_force_https_hsts_primary_and_renewal_attention() {
    let _ = ring::default_provider().install_default();
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("probe");
    std::fs::write(&probe, b"").unwrap();
    let uid = std::fs::metadata(&probe).unwrap().uid();

    let priv_sock = dir.path().join("priv.sock");
    let mut priv_cfg = PrivConfig::new(&priv_sock, vec![uid]);
    priv_cfg.capacity_profile = Some(common::capacity_profile());
    priv_cfg.capacity_image_size_mb = Some(64);
    priv_cfg.master_key_path = dir.path().join("master.key");
    let priv_srv = PrivServer::bind(priv_cfg).unwrap();
    let (priv_stop, priv_rx) = oneshot::channel::<()>();
    let priv_task = tokio::spawn(priv_srv.run(async {
        let _ = priv_rx.await;
    }));

    // Upstream HTTP app server.
    let up_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let up_port = up_listener.local_addr().unwrap().port();
    let up_task = tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = up_listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 512];
                let _ = s.read(&mut buf).await;
                let _ = s
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\napp-v1",
                    )
                    .await;
            });
        }
    });

    let server_ip = Ipv4Addr::new(203, 0, 113, 77);
    let dns_records = Arc::new(Mutex::new(HashMap::<String, Ipv4Addr>::new()));
    let dns_addr = spawn_dynamic_dns(dns_records.clone()).await;

    let acme_state = MockAcmeState::new(15); // 15 days => within 30-day renewal window
    let acme_factory_state = Arc::clone(&acme_state);
    let proxy_sock = dir.path().join("proxy.sock");
    let mut proxy_cfg = ProxyConfig::new(&priv_sock);
    proxy_cfg.listen_http = "127.0.0.1:0".parse().unwrap();
    proxy_cfg.listen_https = "127.0.0.1:0".parse().unwrap();
    proxy_cfg.socket_path = proxy_sock.clone();
    proxy_cfg.state_dir = dir.path().join("proxy-state");
    proxy_cfg.acme_directory = AcmeDirectory::Custom("https://acme.test/dir".into());
    proxy_cfg.custom_http = Some(Arc::new(move || {
        Box::new(MockAcmeClient {
            state: Arc::clone(&acme_factory_state),
        })
    }));
    let proxy_srv = ProxyServer::bind(proxy_cfg).await.unwrap();
    let proxy_http_addr = proxy_srv.http_addr();
    let proxy_https_addr = proxy_srv.https_addr();
    let acme_mgr = proxy_srv.acme();
    *acme_state.proxy_http_addr.write().await = Some(proxy_http_addr);
    let (proxy_stop, proxy_rx) = oneshot::channel::<()>();
    let proxy_task = tokio::spawn(async move {
        proxy_srv
            .run(async {
                let _ = proxy_rx.await;
            })
            .await
            .unwrap();
    });

    let store = Arc::new(opsbox_store::Store::open_in_memory().unwrap());
    let admin = store
        .create_user("alice", Role::Admin, "unused", 0)
        .unwrap();
    store.set_pending_totp(&admin.id, "unused", 0).unwrap();
    store.enable_totp(&admin.id, 0, 0).unwrap();
    let d1 = store
        .create_user("d1", Role::Deployer, "unused", 0)
        .unwrap();
    let d2 = store
        .create_user("d2", Role::Deployer, "unused", 0)
        .unwrap();

    let state = AppState::new(
        store.clone(),
        PrivClient::new(&priv_sock).with_timeout(Duration::from_secs(5)),
        "0.1.0-test",
        SecureCookies::Auto,
        AuthConfig {
            hash_params: HashParams {
                memory_kib: 8,
                iterations: 1,
                parallelism: 1,
            },
            ..Default::default()
        },
    )
    .with_proxy(
        ProxyClient::new(&proxy_sock).with_timeout(Duration::from_secs(5)),
        None,
        vec![IpAddr::V4(server_ip)],
    )
    .with_dns_nameservers(vec![dns_addr]);

    let app = router(state.clone());

    let (proj, env) = store.ensure_default_project(10).unwrap();
    let svc = store
        .create_service(NewService {
            project_id: &proj.id,
            env_id: &env.id,
            name: "web",
            kind: ServiceKind::App,
            owner_id: Some(&d1.id),
            source_kind: SourceKind::DockerImage,
            repo: "nginx:1.27-alpine",
            branch: "main",
            build_method: "Docker image",
            dockerfile_path: None,
            start_cmd: None,
            port: 3000,
            health_path: "/",
            domain: None,
            image_ref: Some("nginx:1.27-alpine"),
            deploy_key_id: None,
            auto_deploy: false,
            memory_limit_mb: None,
            cpu_limit_millis: None,
            graph_x: 0,
            graph_y: 0,
            note: None,
            now: 10,
        })
        .unwrap();
    let dep = store
        .create_deploy(
            &svc.id,
            None,
            "init",
            opsbox_store::DeployTrigger::Manual,
            "d1",
            11,
        )
        .unwrap();
    store
        .complete_deploy_switch(
            &svc.id,
            &dep.id,
            "nginx:1.27-alpine",
            "cid-web",
            up_port,
            None,
            12,
        )
        .unwrap();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let tok_a = "sess-alice-domains";
    let tok_d1 = "sess-d1-domains";
    let tok_d2 = "sess-d2-domains";
    for (u, t) in [(&admin, tok_a), (&d1, tok_d1), (&d2, tok_d2)] {
        store
            .create_session(
                &u.id,
                &opsbox_auth::tokens::hash_token(t),
                opsbox_store::SessionKind::Web,
                None,
                None,
                now,
            )
            .unwrap();
    }
    let ca = format!("opsbox_session={tok_a}");
    let ca_csrf = opsbox_auth::tokens::csrf_token(tok_a);
    let cd1 = format!("opsbox_session={tok_d1}");
    let cd1_csrf = opsbox_auth::tokens::csrf_token(tok_d1);
    let cd2 = format!("opsbox_session={tok_d2}");
    let cd2_csrf = opsbox_auth::tokens::csrf_token(tok_d2);

    // 1. Internal URL reference is returned by GET /services/web/domains.
    let (st0, list0) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::GET,
        "/api/v1/services/web/domains",
        None,
    )
    .await;
    assert_eq!(st0, StatusCode::OK);
    assert_eq!(list0["internalHost"], "web");
    assert_eq!(list0["internalUrl"], "http://web:3000");
    assert_eq!(list0["network"], "opsbox-default");

    // 2. Add `app.example.com`, verify pending DNS, then point DNS and verify ok.
    let (st_add1, add1) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::POST,
        "/api/v1/services/web/domains",
        Some(json!({ "name": "app.example.com", "www": "none", "force": true })),
    )
    .await;
    assert_eq!(st_add1, StatusCode::CREATED, "{add1}");
    assert_eq!(add1["name"], "app.example.com");
    assert_eq!(add1["primary"], true);

    let (st_chk1, chk1) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::POST,
        "/api/v1/domains/app.example.com/check",
        None,
    )
    .await;
    assert_eq!(st_chk1, StatusCode::OK);
    assert_eq!(chk1["dns"], "pending");
    assert_eq!(chk1["pointsHere"], false);

    // Pre-cert HTTP works directly over :80.
    let mut http_s = TcpStream::connect(proxy_http_addr).await.unwrap();
    http_s
        .write_all(b"GET / HTTP/1.1\r\nHost: app.example.com\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut http_buf = Vec::new();
    http_s.read_to_end(&mut http_buf).await.unwrap();
    let http_txt = String::from_utf8_lossy(&http_buf);
    assert!(
        http_txt.starts_with("HTTP/1.1 200 OK") && http_txt.ends_with("app-v1"),
        "{http_txt}"
    );

    // Point DNS and re-check -> ok!
    dns_records
        .lock()
        .unwrap()
        .insert("app.example.com".into(), server_ip);
    let (st_chk2, chk2) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::POST,
        "/api/v1/domains/app.example.com/check",
        None,
    )
    .await;
    assert_eq!(st_chk2, StatusCode::OK);
    assert_eq!(chk2["dns"], "ok");
    assert_eq!(chk2["pointsHere"], true);

    // 3. Issue cert for app.example.com -> HTTP 308 -> HTTPS + HSTS!
    let (st_cert1, cert1_start) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::POST,
        "/api/v1/domains/app.example.com/certificate",
        None,
    )
    .await;
    assert_eq!(st_cert1, StatusCode::ACCEPTED, "{cert1_start}");
    let task1 = wait_task(&app, &cd1, &cd1_csrf, cert1_start["task"].as_str().unwrap()).await;
    assert_eq!(task1["status"], "ok", "{task1}");

    let mut roots = RootCertStore::empty();
    for der in CertificateDer::pem_slice_iter(acme_state.ca_cert_pem.as_bytes()) {
        roots.add(der.unwrap()).unwrap();
    }
    let tls_cfg = Arc::new(
        ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let connector = TlsConnector::from(tls_cfg);

    let tls_tcp = TcpStream::connect(proxy_https_addr).await.unwrap();
    let tls_conn = connector
        .connect(ServerName::try_from("app.example.com").unwrap(), tls_tcp)
        .await
        .unwrap();
    let (mut s_app, c_app) = hyper::client::conn::http1::handshake(TokioIo::new(tls_conn))
        .await
        .unwrap();
    tokio::spawn(c_app);
    let r_app = s_app
        .send_request(
            Request::get("/")
                .header("host", "app.example.com")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r_app.status(), StatusCode::OK);
    assert_eq!(
        r_app
            .headers()
            .get("strict-transport-security")
            .unwrap()
            .to_str()
            .unwrap(),
        "max-age=31536000"
    );

    // 4. Add `www.app.example.com` with redirect to `app.example.com`, DNS check, cert, verify 308!
    let (st_add_www, add_www) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::POST,
        "/api/v1/services/web/domains",
        Some(json!({ "name": "www.app.example.com", "www": "www_to_bare", "force": true })),
    )
    .await;
    assert_eq!(st_add_www, StatusCode::CREATED, "{add_www}");
    assert_eq!(add_www["redirect"], "app.example.com");

    dns_records
        .lock()
        .unwrap()
        .insert("www.app.example.com".into(), server_ip);
    let (_, _) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::POST,
        "/api/v1/domains/www.app.example.com/check",
        None,
    )
    .await;
    let (_, cert_www_start) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::POST,
        "/api/v1/domains/www.app.example.com/certificate",
        None,
    )
    .await;
    let task_www = wait_task(
        &app,
        &cd1,
        &cd1_csrf,
        cert_www_start["task"].as_str().unwrap(),
    )
    .await;
    assert_eq!(task_www["status"], "ok", "{task_www}");

    let tls_www = connector
        .connect(
            ServerName::try_from("www.app.example.com").unwrap(),
            TcpStream::connect(proxy_https_addr).await.unwrap(),
        )
        .await
        .unwrap();
    let (mut s_www, c_www) = hyper::client::conn::http1::handshake(TokioIo::new(tls_www))
        .await
        .unwrap();
    tokio::spawn(c_www);
    let r_www = s_www
        .send_request(
            Request::get("/cart?x=1")
                .header("host", "www.app.example.com")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r_www.status(), StatusCode::PERMANENT_REDIRECT);
    assert_eq!(
        r_www.headers().get("location").unwrap().to_str().unwrap(),
        "https://app.example.com/cart?x=1"
    );

    // 5. Add second serving domain `shop.example.com`, issue cert, make primary, toggle forceHttps/hsts off!
    let (st_add2, _) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::POST,
        "/api/v1/services/web/domains",
        Some(json!({ "name": "shop.example.com", "www": "none", "force": true })),
    )
    .await;
    assert_eq!(st_add2, StatusCode::CREATED);
    dns_records
        .lock()
        .unwrap()
        .insert("shop.example.com".into(), server_ip);
    let (_, _) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::POST,
        "/api/v1/domains/shop.example.com/check",
        None,
    )
    .await;
    let (_, cert2_start) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::POST,
        "/api/v1/domains/shop.example.com/certificate",
        None,
    )
    .await;
    assert_eq!(
        wait_task(&app, &cd1, &cd1_csrf, cert2_start["task"].as_str().unwrap()).await["status"],
        "ok"
    );

    // Cannot make redirecting domain primary; CAN make shop.example.com primary.
    assert_eq!(
        send_cookie(
            &app,
            &cd1,
            &cd1_csrf,
            Method::POST,
            "/api/v1/domains/www.app.example.com/primary",
            None,
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let (st_prim, prim) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::POST,
        "/api/v1/domains/shop.example.com/primary",
        None,
    )
    .await;
    assert_eq!(st_prim, StatusCode::OK, "{prim}");
    assert_eq!(prim["primary"], true);

    let (_, svc_view) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::GET,
        "/api/v1/services/web",
        None,
    )
    .await;
    assert_eq!(svc_view["domain"], "shop.example.com");
    assert_eq!(svc_view["domainScheme"], "https");

    // Disable Force HTTPS and HSTS on shop.example.com.
    let (st_patch, patched) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::PATCH,
        "/api/v1/domains/shop.example.com",
        Some(json!({ "forceHttps": false, "hsts": false })),
    )
    .await;
    assert_eq!(st_patch, StatusCode::OK, "{patched}");
    assert_eq!(patched["forceHttps"], false);
    assert_eq!(patched["hsts"], false);

    // HTTP to shop.example.com now serves 200 directly (not 308), and HTTPS omits HSTS.
    let mut http_shop = TcpStream::connect(proxy_http_addr).await.unwrap();
    http_shop
        .write_all(b"GET / HTTP/1.1\r\nHost: shop.example.com\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut shop_buf = Vec::new();
    http_shop.read_to_end(&mut shop_buf).await.unwrap();
    let shop_txt = String::from_utf8_lossy(&shop_buf);
    assert!(shop_txt.starts_with("HTTP/1.1 200 OK"), "{shop_txt}");

    let tls_shop = connector
        .connect(
            ServerName::try_from("shop.example.com").unwrap(),
            TcpStream::connect(proxy_https_addr).await.unwrap(),
        )
        .await
        .unwrap();
    let (mut s_shop, c_shop) = hyper::client::conn::http1::handshake(TokioIo::new(tls_shop))
        .await
        .unwrap();
    tokio::spawn(c_shop);
    let r_shop = s_shop
        .send_request(
            Request::get("/")
                .header("host", "shop.example.com")
                .body(Full::new(Bytes::new()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r_shop.status(), StatusCode::OK);
    assert!(r_shop.headers().get("strict-transport-security").is_none());

    // 6. Renewal failure raises Attention item for admin & owner deployer d1, not d2.
    *acme_state.fail_challenges.lock().unwrap() = true;
    acme_mgr.renew_due_certs().await;

    let (_, attn_a) =
        send_cookie(&app, &ca, &ca_csrf, Method::GET, "/api/v1/attention", None).await;
    assert!(
        attn_a["items"].as_array().unwrap().iter().any(|i| {
            i["kind"] == "tls"
                && i["id"] == "cert:shop.example.com"
                && (i["link"][1] == "#/apps?s=web&tab=domains"
                    || i["link"][1] == "#/alerts?id=tls%3Ashop.example.com")
                && (i["text"].as_str().unwrap_or("").contains("renewal failed")
                    || i["text"].as_str().unwrap_or("").contains("could not renew"))
        }),
        "{attn_a}"
    );
    let (_, attn_d1) = send_cookie(
        &app,
        &cd1,
        &cd1_csrf,
        Method::GET,
        "/api/v1/attention",
        None,
    )
    .await;
    assert!(
        attn_d1["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["kind"] == "tls"),
        "{attn_d1}"
    );
    let (_, attn_d2) = send_cookie(
        &app,
        &cd2,
        &cd2_csrf,
        Method::GET,
        "/api/v1/attention",
        None,
    )
    .await;
    assert!(
        !attn_d2["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["kind"] == "tls"),
        "{attn_d2}"
    );

    // 7. Cannot delete app.example.com while www.app.example.com redirects to it; removing www first succeeds.
    assert_eq!(
        send_cookie(
            &app,
            &cd1,
            &cd1_csrf,
            Method::DELETE,
            "/api/v1/domains/app.example.com",
            None,
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        send_cookie(
            &app,
            &cd1,
            &cd1_csrf,
            Method::DELETE,
            "/api/v1/domains/www.app.example.com",
            None,
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        send_cookie(
            &app,
            &cd1,
            &cd1_csrf,
            Method::DELETE,
            "/api/v1/domains/app.example.com",
            None,
        )
        .await
        .0,
        StatusCode::OK
    );

    // Removing remaining primary domain leaves only internal host, and clears attention item.
    assert_eq!(
        send_cookie(
            &app,
            &cd1,
            &cd1_csrf,
            Method::DELETE,
            "/api/v1/domains/shop.example.com",
            None,
        )
        .await
        .0,
        StatusCode::OK
    );
    let (_, attn_clear) =
        send_cookie(&app, &ca, &ca_csrf, Method::GET, "/api/v1/attention", None).await;
    assert!(
        !attn_clear["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["kind"] == "tls"),
        "{attn_clear}"
    );

    // Proxy returns 404 for removed domains.
    let mut http_gone = TcpStream::connect(proxy_http_addr).await.unwrap();
    http_gone
        .write_all(b"GET / HTTP/1.1\r\nHost: shop.example.com\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut gone_buf = Vec::new();
    http_gone.read_to_end(&mut gone_buf).await.unwrap();
    assert!(
        String::from_utf8_lossy(&gone_buf).starts_with("HTTP/1.1 404"),
        "{}",
        String::from_utf8_lossy(&gone_buf)
    );

    up_task.abort();
    let _ = proxy_stop.send(());
    proxy_task.await.unwrap();
    let _ = priv_stop.send(());
    priv_task.await.unwrap();
}
