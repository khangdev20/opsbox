//! Minimal HTTPS Safe Mode server for `opsbox-watchdog` (spec 7.16).
//!
//! Serves a standalone emergency UI and JSON API on `:8444` with zero shared
//! code with `serve` beyond `opsbox-proto` types and read-only `opsbox-store`
//! lookups. Protected by admin account + TOTP or single-use recovery code.

use std::convert::Infallible;
use std::fs;
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use http_body_util::{BodyExt, Full};
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode, header};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as ConnBuilder;
use nix::unistd::{Group, User, chown};
use opsbox_proto::WatchdogStatus;
use rcgen::generate_simple_self_signed;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls;
use tracing::warn;

use crate::actions;
use crate::auth::{SafeAuth, SafeAuthError, SafeSession, csrf_for, ct_eq};
use crate::cleanup;
use crate::slots::SlotManager;

const SESSION_COOKIE: &str = "opsbox_safemode";
const MAX_BODY_BYTES: usize = 64 * 1024;
const DEFAULT_CSP: &str = "default-src 'none'; script-src 'self'; style-src 'unsafe-inline'; connect-src 'self'; form-action 'self'; frame-ancestors 'none'; base-uri 'self'";

#[derive(Clone)]
pub struct SafeModeState {
    pub auth: Arc<SafeAuth>,
    pub snapshot: Arc<RwLock<WatchdogStatus>>,
    pub slots: SlotManager,
    pub priv_socket: PathBuf,
    pub xfer_dir: PathBuf,
    pub db_path: PathBuf,
    pub panel_port: u16,
    pub wake_loop: Arc<Notify>,
    pub reset_failures: Arc<AtomicBool>,
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

fn detect_outbound_ip() -> Option<IpAddr> {
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0")
        && sock.connect("1.1.1.1:80").is_ok()
        && let Ok(addr) = sock.local_addr()
        && !addr.ip().is_loopback()
    {
        return Some(addr.ip());
    }
    None
}

fn generate_self_signed_pem() -> io::Result<(String, String)> {
    let mut sans = vec![
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
        "::1".to_owned(),
    ];
    if let Ok(host) = fs::read_to_string("/etc/hostname") {
        let h = host.trim();
        if !h.is_empty() && !sans.iter().any(|s| s == h) {
            sans.push(h.to_owned());
        }
    }
    if let Some(ip) = detect_outbound_ip() {
        let s = ip.to_string();
        if !sans.iter().any(|x| x == &s) {
            sans.push(s);
        }
    }
    let certified = generate_simple_self_signed(sans).map_err(io::Error::other)?;
    Ok((certified.cert.pem(), certified.key_pair.serialize_pem()))
}

fn chown_to_opsbox(path: &Path) {
    let Ok(Some(user)) = User::from_name("opsbox") else {
        return;
    };
    let gid = Group::from_name("opsbox")
        .ok()
        .flatten()
        .map_or(user.gid, |g| g.gid);
    let _ = chown(path, Some(user.uid), Some(gid));
}

fn write_cert_pair(
    cert_path: &Path,
    key_path: &Path,
    cert_pem: &str,
    key_pem: &str,
) -> io::Result<()> {
    if let Some(p) = cert_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(p)?;
    }
    if let Some(p) = key_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(p)?;
    }
    fs::write(cert_path, cert_pem)?;
    fs::set_permissions(cert_path, fs::Permissions::from_mode(0o644))?;
    chown_to_opsbox(cert_path);

    fs::write(key_path, key_pem)?;
    fs::set_permissions(key_path, fs::Permissions::from_mode(0o640))?;
    chown_to_opsbox(key_path);
    Ok(())
}

fn load_tls_acceptor_from_files(cert_path: &Path, key_path: &Path) -> io::Result<TlsAcceptor> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(cert_path)
        .map_err(io::Error::other)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io::Error::other)?;
    if certs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no certificates found in PEM file",
        ));
    }
    let key = PrivateKeyDer::from_pem_file(key_path).map_err(io::Error::other)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(io::Error::other)?;
    Ok(TlsAcceptor::from(Arc::new(cfg)))
}

/// Share the panel TLS certificate (`DEFAULT_CERT_PATH` or `<db_dir>/panel-selfsigned.{crt,key}`).
pub fn load_or_create_tls_acceptor(
    cert_path: &Path,
    key_path: &Path,
    fallback_cert_path: &Path,
    fallback_key_path: &Path,
) -> io::Result<(TlsAcceptor, PathBuf)> {
    if cert_path.exists() && key_path.exists() {
        let acc = load_tls_acceptor_from_files(cert_path, key_path)?;
        return Ok((acc, cert_path.to_path_buf()));
    }
    if fallback_cert_path.exists() && fallback_key_path.exists() {
        let acc = load_tls_acceptor_from_files(fallback_cert_path, fallback_key_path)?;
        return Ok((acc, fallback_cert_path.to_path_buf()));
    }
    let (cert_pem, key_pem) = generate_self_signed_pem()?;
    if write_cert_pair(cert_path, key_path, &cert_pem, &key_pem).is_ok() {
        let acc = load_tls_acceptor_from_files(cert_path, key_path)?;
        return Ok((acc, cert_path.to_path_buf()));
    }
    write_cert_pair(fallback_cert_path, fallback_key_path, &cert_pem, &key_pem)?;
    let acc = load_tls_acceptor_from_files(fallback_cert_path, fallback_key_path)?;
    Ok((acc, fallback_cert_path.to_path_buf()))
}

pub async fn serve(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    state: SafeModeState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            () = &mut shutdown => break,
            accepted = listener.accept() => {
                let (stream, peer) = match accepted {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(error = %e, "safe mode accept error");
                        continue;
                    }
                };
                let acceptor = acceptor.clone();
                let st = state.clone();
                tokio::spawn(async move {
                    let Ok(Ok(tls_stream)) = tokio::time::timeout(
                        Duration::from_secs(10),
                        acceptor.accept(stream),
                    )
                    .await else {
                        return;
                    };
                    let io = TokioIo::new(tls_stream);
                    let svc = service_fn(move |req| {
                        let st = st.clone();
                        async move { Ok::<_, Infallible>(handle_request(req, peer, &st).await) }
                    });
                    let _ = ConnBuilder::new(TokioExecutor::new())
                        .serve_connection(io, svc)
                        .await;
                });
            }
        }
    }
    Ok(())
}

type BoxResp = Response<Full<Bytes>>;

#[derive(Serialize)]
struct ErrorEnvelope<'a> {
    error: ErrorBody<'a>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    code: &'a str,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    field: Option<&'a str>,
}

fn with_security_headers(mut resp: BoxResp, is_html: bool) -> BoxResp {
    let h = resp.headers_mut();
    h.insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        header::HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::X_FRAME_OPTIONS,
        header::HeaderValue::from_static("DENY"),
    );
    h.insert(
        header::REFERRER_POLICY,
        header::HeaderValue::from_static("same-origin"),
    );
    if is_html {
        h.insert(
            header::CONTENT_SECURITY_POLICY,
            header::HeaderValue::from_static(DEFAULT_CSP),
        );
    }
    resp
}

fn json_resp<T: Serialize>(status: StatusCode, body: &T) -> BoxResp {
    let bytes = serde_json::to_vec(body).unwrap_or_else(|_| b"{}".to_vec());
    let resp = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Full::new(Bytes::from(bytes)))
        .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())));
    with_security_headers(resp, false)
}

fn error_resp(status: StatusCode, code: &str, message: &str, field: Option<&str>) -> BoxResp {
    json_resp(
        status,
        &ErrorEnvelope {
            error: ErrorBody {
                code,
                message,
                field,
            },
        },
    )
}

fn extract_cookie(req: &Request<Incoming>, name: &str) -> Option<String> {
    for val in req.headers().get_all(header::COOKIE) {
        let Ok(s) = val.to_str() else {
            continue;
        };
        for part in s.split(';') {
            let trimmed = part.trim();
            if let Some((k, v)) = trimmed.split_once('=')
                && k.trim() == name
            {
                let value = v.trim();
                if !value.is_empty() {
                    return Some(value.to_owned());
                }
            }
        }
    }
    None
}

fn normalize_origin_host(raw: &str) -> Option<&str> {
    let rest = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))?;
    Some(rest.split('/').next().unwrap_or(rest))
}

fn check_same_origin(req: &Request<Incoming>) -> Result<(), Box<BoxResp>> {
    let Some(host) = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
    else {
        return Ok(());
    };
    if let Some(origin) = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        && normalize_origin_host(origin) != Some(host)
    {
        return Err(Box::new(error_resp(
            StatusCode::FORBIDDEN,
            "origin_mismatch",
            "Cross-origin requests are not allowed.",
            None,
        )));
    }
    Ok(())
}

fn require_auth(
    req: &Request<Incoming>,
    state: &SafeModeState,
    require_csrf: bool,
) -> Result<(String, SafeSession), Box<BoxResp>> {
    let token = extract_cookie(req, SESSION_COOKIE).ok_or_else(|| {
        Box::new(error_resp(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Sign in to access Safe Mode.",
            None,
        ))
    })?;
    let session = state.auth.authenticate(&token).ok_or_else(|| {
        Box::new(error_resp(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Your Safe Mode session has expired. Sign in again.",
            None,
        ))
    })?;
    if require_csrf {
        let supplied = req
            .headers()
            .get("x-opsbox-csrf")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let expected = csrf_for(&token);
        if supplied.is_empty() || !ct_eq(supplied, &expected) {
            return Err(Box::new(error_resp(
                StatusCode::FORBIDDEN,
                "csrf_invalid",
                "Missing or invalid CSRF token. Reload and try again.",
                None,
            )));
        }
    }
    Ok((token, session))
}

async fn read_json_body<T: for<'de> Deserialize<'de>>(
    req: Request<Incoming>,
) -> Result<T, Box<BoxResp>> {
    let mut body = req.into_body();
    let mut buf = Vec::new();
    while let Some(frame_res) = body.frame().await {
        let frame = frame_res.map_err(|_| {
            Box::new(error_resp(
                StatusCode::BAD_REQUEST,
                "bad_body",
                "Could not read request body.",
                None,
            ))
        })?;
        if let Ok(data) = frame.into_data() {
            if buf.len().saturating_add(data.len()) > MAX_BODY_BYTES {
                return Err(Box::new(error_resp(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "payload_too_large",
                    "Request body exceeds 64 KiB limit.",
                    None,
                )));
            }
            buf.extend_from_slice(&data);
        }
    }
    serde_json::from_slice::<T>(&buf).map_err(|_| {
        Box::new(error_resp(
            StatusCode::BAD_REQUEST,
            "invalid_json",
            "Malformed JSON request body.",
            None,
        ))
    })
}

fn auth_error_resp(err: SafeAuthError) -> BoxResp {
    let msg = err.message();
    match err {
        SafeAuthError::InvalidCredentials => {
            error_resp(StatusCode::UNAUTHORIZED, "invalid_credentials", &msg, None)
        }
        SafeAuthError::AdminOnly => error_resp(StatusCode::FORBIDDEN, "admin_only", &msg, None),
        SafeAuthError::Locked(_) => {
            error_resp(StatusCode::TOO_MANY_REQUESTS, "rate_limited", &msg, None)
        }
        SafeAuthError::Internal(_) => {
            error_resp(StatusCode::INTERNAL_SERVER_ERROR, "internal", &msg, None)
        }
    }
}

fn set_session_cookie(mut resp: BoxResp, token: &str) -> BoxResp {
    let val = format!("{SESSION_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/; Secure");
    if let Ok(hv) = header::HeaderValue::from_str(&val) {
        resp.headers_mut().append(header::SET_COOKIE, hv);
    }
    resp
}

fn clear_session_cookie(mut resp: BoxResp) -> BoxResp {
    let val = format!("{SESSION_COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0; Secure");
    if let Ok(hv) = header::HeaderValue::from_str(&val) {
        resp.headers_mut().append(header::SET_COOKIE, hv);
    }
    resp
}

#[derive(Deserialize)]
struct LoginReq {
    username: String,
    password: String,
    #[serde(default)]
    totp: String,
}

#[derive(Deserialize)]
struct RecoveryCodeReq {
    code: String,
}

#[derive(Deserialize)]
struct ActionReq {
    action: String,
    #[serde(default)]
    ip: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionResp<'a> {
    authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    user: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auth_method: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    csrf: Option<String>,
    panel_port: u16,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryPayload {
    running: bool,
    failed: bool,
    degraded: bool,
    updated_at: i64,
    current_version: String,
    previous_version: Option<String>,
    can_rollback: bool,
    update_message: Option<String>,
    ssh_open_until: Option<i64>,
    panel_port: u16,
    caller_ip: String,
    checks: Vec<opsbox_proto::WatchdogCheck>,
    events: Vec<String>,
    log: Vec<String>,
}

pub async fn handle_request(
    req: Request<Incoming>,
    peer: SocketAddr,
    state: &SafeModeState,
) -> BoxResp {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let peer_ip = peer.ip().to_string();

    if method == Method::POST
        && let Err(e) = check_same_origin(&req)
    {
        return *e;
    }

    match (method, path.as_str()) {
        (Method::GET, "/healthz") => {
            let st = state
                .snapshot
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            json_resp(
                StatusCode::OK,
                &serde_json::json!({
                    "ok": true,
                    "service": "watchdog",
                    "version": opsbox_version::VERSION,
                    "failed": st.failed,
                    "degraded": st.degraded,
                }),
            )
        }
        (Method::GET, "/" | "/index.html") => {
            let resp = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(Full::new(Bytes::from_static(INDEX_HTML.as_bytes())))
                .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())));
            with_security_headers(resp, true)
        }
        (Method::GET, "/safemode.js") => {
            let resp = Response::builder()
                .status(StatusCode::OK)
                .header(
                    header::CONTENT_TYPE,
                    "application/javascript; charset=utf-8",
                )
                .body(Full::new(Bytes::from_static(SAFEMODE_JS.as_bytes())))
                .unwrap_or_else(|_| Response::new(Full::new(Bytes::new())));
            with_security_headers(resp, false)
        }
        (Method::GET, "/api/v1/session") => {
            if let Some(tok) = extract_cookie(&req, SESSION_COOKIE)
                && let Some(sess) = state.auth.authenticate(&tok)
            {
                return json_resp(
                    StatusCode::OK,
                    &SessionResp {
                        authenticated: true,
                        user: Some(&sess.user),
                        auth_method: Some(sess.auth_method),
                        csrf: Some(csrf_for(&tok)),
                        panel_port: state.panel_port,
                    },
                );
            }
            json_resp(
                StatusCode::OK,
                &SessionResp {
                    authenticated: false,
                    user: None,
                    auth_method: None,
                    csrf: None,
                    panel_port: state.panel_port,
                },
            )
        }
        (Method::POST, "/api/v1/auth/login") => {
            let body: LoginReq = match read_json_body(req).await {
                Ok(b) => b,
                Err(r) => return *r,
            };
            match state
                .auth
                .login_account(&body.username, &body.password, &body.totp, &peer_ip)
                .await
            {
                Ok((tok, sess)) => {
                    let resp = json_resp(
                        StatusCode::OK,
                        &SessionResp {
                            authenticated: true,
                            user: Some(&sess.user),
                            auth_method: Some(sess.auth_method),
                            csrf: Some(csrf_for(&tok)),
                            panel_port: state.panel_port,
                        },
                    );
                    set_session_cookie(resp, &tok)
                }
                Err(e) => auth_error_resp(e),
            }
        }
        (Method::POST, "/api/v1/auth/recovery-code") => {
            let body: RecoveryCodeReq = match read_json_body(req).await {
                Ok(b) => b,
                Err(r) => return *r,
            };
            match state.auth.login_recovery_code(&body.code, &peer_ip) {
                Ok((tok, sess)) => {
                    let resp = json_resp(
                        StatusCode::OK,
                        &SessionResp {
                            authenticated: true,
                            user: Some(&sess.user),
                            auth_method: Some(sess.auth_method),
                            csrf: Some(csrf_for(&tok)),
                            panel_port: state.panel_port,
                        },
                    );
                    set_session_cookie(resp, &tok)
                }
                Err(e) => auth_error_resp(e),
            }
        }
        (Method::POST, "/api/v1/auth/logout") => {
            let (tok, _) = match require_auth(&req, state, true) {
                Ok(v) => v,
                Err(r) => return *r,
            };
            state.auth.logout(&tok);
            let resp = json_resp(StatusCode::OK, &serde_json::json!({ "ok": true }));
            clear_session_cookie(resp)
        }
        (Method::GET, "/api/v1/recovery") => {
            if let Err(r) = require_auth(&req, state, false) {
                return *r;
            }
            let st = state
                .snapshot
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            let log = tokio::task::spawn_blocking(|| actions::tail_opsbox_logs(40))
                .await
                .unwrap_or_default();
            let events = st.recent_events;
            let combined_log = if log.is_empty() { events.clone() } else { log };
            let prev = state
                .slots
                .available_previous_slot()
                .or(st.previous_version);
            let can_rollback = st.can_rollback || prev.is_some();
            json_resp(
                StatusCode::OK,
                &RecoveryPayload {
                    running: st.running,
                    failed: st.failed,
                    degraded: st.degraded,
                    updated_at: st.updated_at,
                    current_version: st.current_version,
                    previous_version: prev,
                    can_rollback,
                    update_message: st.update_message,
                    ssh_open_until: st.ssh_open_until,
                    panel_port: state.panel_port,
                    caller_ip: peer_ip,
                    checks: st.checks,
                    events,
                    log: combined_log,
                },
            )
        }
        (Method::POST, "/api/v1/recovery/action") => {
            if let Err(r) = require_auth(&req, state, true) {
                return *r;
            }
            let body: ActionReq = match read_json_body(req).await {
                Ok(b) => b,
                Err(r) => return *r,
            };
            execute_action(state, body, &peer_ip).await
        }
        _ => error_resp(
            StatusCode::NOT_FOUND,
            "not_found",
            "Unknown endpoint.",
            None,
        ),
    }
}

async fn execute_action(state: &SafeModeState, body: ActionReq, peer_ip: &str) -> BoxResp {
    match body.action.as_str() {
        "free" => {
            let slots = state.slots.clone();
            let xfer = state.xfer_dir.clone();
            let db = state.db_path.clone();
            let report = tokio::task::spawn_blocking(move || {
                cleanup::free_opsbox_space(&slots, &xfer, &db, true)
            })
            .await
            .unwrap_or_default();
            state.wake_loop.notify_one();
            json_resp(
                StatusCode::OK,
                &serde_json::json!({ "ok": true, "msg": report.summary() }),
            )
        }
        "restart" => {
            let res = tokio::task::spawn_blocking(actions::restart_all_opsbox).await;
            match res {
                Ok(Ok(())) => {
                    state.reset_failures.store(true, Ordering::Release);
                    state.wake_loop.notify_one();
                    json_resp(
                        StatusCode::OK,
                        &serde_json::json!({
                            "ok": true,
                            "msg": "Restarted opsbox services."
                        }),
                    )
                }
                Ok(Err(e)) => error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "restart_failed",
                    &e,
                    None,
                ),
                Err(_) => error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "restart_failed",
                    "Restart task panicked.",
                    None,
                ),
            }
        }
        "rollback" => {
            let slots = state.slots.clone();
            let db = state.db_path.clone();
            let res = tokio::task::spawn_blocking(move || {
                let target = slots.rollback_to_previous(&db, now_secs())?;
                actions::restart_all_opsbox().map_err(|e| {
                    format!("Rolled back binary to {target}, but restart failed: {e}")
                })?;
                Ok::<String, String>(target)
            })
            .await;
            match res {
                Ok(Ok(ver)) => {
                    state.reset_failures.store(true, Ordering::Release);
                    state.wake_loop.notify_one();
                    json_resp(
                        StatusCode::OK,
                        &serde_json::json!({
                            "ok": true,
                            "msg": format!("Rolled back to {ver} and restarted opsbox.")
                        }),
                    )
                }
                Ok(Err(e)) => {
                    let status = if e.contains("No previous version") {
                        StatusCode::BAD_REQUEST
                    } else {
                        StatusCode::CONFLICT
                    };
                    error_resp(status, "rollback_failed", &e, None)
                }
                Err(_) => error_resp(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "rollback_failed",
                    "Rollback task failed.",
                    None,
                ),
            }
        }
        "ssh" => {
            let target_ip = if body.ip.trim().is_empty() {
                peer_ip.to_owned()
            } else {
                body.ip.trim().to_owned()
            };
            match actions::open_ssh_1h(&state.priv_socket, &target_ip).await {
                Ok(until) => {
                    {
                        let mut guard = state
                            .snapshot
                            .write()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        guard.ssh_open_until = Some(until);
                    }
                    state.wake_loop.notify_one();
                    json_resp(
                        StatusCode::OK,
                        &serde_json::json!({
                            "ok": true,
                            "msg": format!("Emergency SSH opened for {target_ip} for 1 hour."),
                            "sshOpenUntil": until,
                        }),
                    )
                }
                Err(e) => {
                    let status = if e.contains("valid IP") || e.contains("public IP") {
                        StatusCode::BAD_REQUEST
                    } else {
                        StatusCode::CONFLICT
                    };
                    error_resp(status, "ssh_failed", &e, Some("ip"))
                }
            }
        }
        "reboot" => match tokio::task::spawn_blocking(actions::schedule_reboot).await {
            Ok(Ok(msg)) => json_resp(
                StatusCode::OK,
                &serde_json::json!({ "ok": true, "msg": msg }),
            ),
            Ok(Err(e)) => error_resp(StatusCode::INTERNAL_SERVER_ERROR, "reboot_failed", &e, None),
            Err(_) => error_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                "reboot_failed",
                "Reboot task failed.",
                None,
            ),
        },
        _ => error_resp(
            StatusCode::BAD_REQUEST,
            "bad_action",
            "Unknown recovery action.",
            Some("action"),
        ),
    }
}

const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>opsbox Safe Mode</title>
<style>
  :root{--bg:#0d1117;--card:#161b22;--bd:#30363d;--fg:#e6edf3;--muted:#8b949e;--acc:#58a6ff;--ok:#3fb950;--warn:#d29922;--bad:#f85149}
  *{box-sizing:border-box}
  body{margin:0;background:var(--bg);color:var(--fg);font:14px/1.5 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif}
  main{max-width:740px;margin:36px auto;padding:0 18px}
  .tag{display:inline-block;padding:2px 8px;border-radius:4px;background:rgba(248,81,73,.16);color:var(--bad);font:700 11px/1.6 ui-monospace,monospace;letter-spacing:.05em}
  .row{display:flex;flex-wrap:wrap;gap:10px;align-items:center}
  .card{background:var(--card);border:1px solid var(--bd);border-radius:8px;padding:18px;margin-top:16px}
  h1{font-size:22px;margin:12px 0 6px}
  .muted{color:var(--muted)}
  .check{display:flex;justify-content:space-between;gap:12px;padding:8px 0;border-bottom:1px solid var(--bd)}
  .check:last-child{border-bottom:none}
  .dot{display:inline-block;width:8px;height:8px;border-radius:50%;margin-right:8px}
  .ok{background:var(--ok)}.warn{background:var(--warn)}.fail{background:var(--bad)}.off{background:var(--muted)}
  pre{background:#090c10;border:1px solid var(--bd);border-radius:6px;padding:12px;overflow:auto;max-height:260px;font:12px/1.45 ui-monospace,SFMono-Regular,monospace;white-space:pre-wrap;word-break:break-word}
  label{display:block;font-size:12px;color:var(--muted);margin:10px 0 4px}
  input{width:100%;padding:8px 10px;border:1px solid var(--bd);border-radius:6px;background:#090c10;color:var(--fg);font-size:14px}
  button,a.btn{cursor:pointer;display:inline-block;padding:8px 14px;border-radius:6px;border:1px solid var(--bd);background:#21262d;color:var(--fg);font:500 13px/1.2 inherit;text-decoration:none}
  button.primary{background:#1f6feb;border-color:#388bfd;color:#fff}
  button.danger{background:#da3633;border-color:#f85149;color:#fff}
  button:disabled{opacity:.55;cursor:not-allowed}
  .tabs{display:flex;gap:8px;margin-bottom:12px}
  .tab.active{border-color:var(--acc);color:var(--acc)}
  .err{color:var(--bad);min-height:18px;font-size:13px;margin-top:8px}
  .msg{color:var(--ok);min-height:18px;font-size:13px;margin-top:8px}
  .mono{font-family:ui-monospace,SFMono-Regular,monospace;font-size:12px}
</style>
</head>
<body>
<main id="app"><p class="muted">Loading Safe Mode...</p></main>
<script src="/safemode.js"></script>
</body>
</html>
"#;

const SAFEMODE_JS: &str = r#"(() => {
  'use strict';
  const app = document.getElementById('app');
  let session = { authenticated: false, csrf: '', panelPort: 8443 };
  let tab = 'account';
  let statusMsg = '';
  let errorMsg = '';

  const el = (tag, cls, text) => {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text !== undefined) n.textContent = text;
    return n;
  };

  async function req(url, opts = {}) {
    const headers = Object.assign({ 'Content-Type': 'application/json' }, opts.headers || {});
    if (session.csrf && (opts.method || 'GET') !== 'GET') {
      headers['X-Opsbox-CSRF'] = session.csrf;
    }
    const r = await fetch(url, Object.assign({}, opts, { headers }));
    const data = await r.json().catch(() => ({}));
    if (!r.ok) {
      const msg = (data.error && data.error.message) || r.statusText || 'Request failed.';
      const err = new Error(msg);
      err.status = r.status;
      throw err;
    }
    return data;
  }

  function renderLogin() {
    app.replaceChildren();
    const hdr = el('div', 'row');
    hdr.append(el('span', 'tag', 'SAFE MODE'), el('span', 'muted', 'Standalone recovery console on :8444'));
    app.append(hdr, el('h1', '', 'Sign in to Safe Mode'));
    app.append(el('p', 'muted', 'Use an existing administrator account (password + 2FA) or run "opsbox recovery-code" over SSH.'));

    const card = el('div', 'card');
    const tabs = el('div', 'tabs');
    const tAcc = el('button', tab === 'account' ? 'tab active' : 'tab', 'Admin account');
    const tCode = el('button', tab === 'code' ? 'tab active' : 'tab', 'Recovery code');
    tAcc.type = 'button';
    tCode.type = 'button';
    tAcc.onclick = () => { tab = 'account'; errorMsg = ''; renderLogin(); };
    tCode.onclick = () => { tab = 'code'; errorMsg = ''; renderLogin(); };
    tabs.append(tAcc, tCode);
    card.append(tabs);

    const form = el('form');
    const errP = el('p', 'err', errorMsg);

    if (tab === 'account') {
      const u = el('input'); u.autocomplete = 'username'; u.required = true;
      const p = el('input'); p.type = 'password'; p.autocomplete = 'current-password'; p.required = true;
      const totp = el('input'); totp.placeholder = '6-digit code'; totp.inputMode = 'numeric'; totp.required = true;
      const submit = el('button', 'primary', 'Sign in');
      submit.style.marginTop = '14px';
      form.append(
        el('label', '', 'Username'), u,
        el('label', '', 'Password'), p,
        el('label', '', 'Two-factor code'), totp,
        submit, errP,
      );
      form.onsubmit = async (ev) => {
        ev.preventDefault();
        errorMsg = '';
        submit.disabled = true;
        try {
          session = await req('/api/v1/auth/login', {
            method: 'POST',
            body: JSON.stringify({ username: u.value, password: p.value, totp: totp.value }),
          });
          await loadDashboard();
        } catch (e) {
          errorMsg = e.message;
          errP.textContent = errorMsg;
          submit.disabled = false;
        }
      };
    } else {
      const code = el('input'); code.placeholder = 'XXXX-XXXX-XXXX'; code.required = true;
      const submit = el('button', 'primary', 'Sign in with recovery code');
      submit.style.marginTop = '14px';
      form.append(el('label', '', 'One-time recovery code'), code, submit, errP);
      form.onsubmit = async (ev) => {
        ev.preventDefault();
        errorMsg = '';
        submit.disabled = true;
        try {
          session = await req('/api/v1/auth/recovery-code', {
            method: 'POST',
            body: JSON.stringify({ code: code.value }),
          });
          await loadDashboard();
        } catch (e) {
          errorMsg = e.message;
          errP.textContent = errorMsg;
          submit.disabled = false;
        }
      };
    }
    card.append(form);
    app.append(card);
  }

  async function doAction(action, extra = {}, confirmText = '') {
    if (confirmText && !window.confirm(confirmText)) return;
    errorMsg = '';
    statusMsg = 'Running...';
    await loadDashboard();
    try {
      const r = await req('/api/v1/recovery/action', {
        method: 'POST',
        body: JSON.stringify(Object.assign({ action }, extra)),
      });
      statusMsg = r.msg || 'Done.';
    } catch (e) {
      statusMsg = '';
      errorMsg = e.message;
      if (e.status === 401) { session = { authenticated: false }; renderLogin(); return; }
    }
    await loadDashboard();
  }

  function renderDashboard(d) {
    app.replaceChildren();
    const hdr = el('div', 'row');
    hdr.append(
      el('span', 'tag', 'SAFE MODE'),
      el('span', 'muted', 'Signed in as ' + (session.user || 'admin') + ' · :8444'),
    );
    const outBtn = el('button', '', 'Sign out');
    outBtn.style.marginLeft = 'auto';
    outBtn.onclick = async () => {
      await req('/api/v1/auth/logout', { method: 'POST', body: '{}' }).catch(() => {});
      session = { authenticated: false, csrf: '', panelPort: d.panelPort || 8443 };
      statusMsg = ''; errorMsg = '';
      renderLogin();
    };
    hdr.append(outBtn);

    const title = d.failed ? 'opsbox is not running.' : (d.degraded ? 'opsbox is degraded.' : 'opsbox is healthy.');
    app.append(hdr, el('h1', '', title));
    if (d.updateMessage) app.append(el('p', 'muted', d.updateMessage));

    const checksCard = el('div', 'card');
    for (const c of (d.checks || [])) {
      const row = el('div', 'check');
      const left = el('span');
      left.append(el('span', 'dot ' + (c.status || 'off')), document.createTextNode(c.name || ''));
      const right = el('span', 'mono');
      right.append(document.createTextNode((c.state || '') + ' '), el('span', 'muted', '· ' + (c.note || '')));
      row.append(left, right);
      checksCard.append(row);
    }
    app.append(checksCard);

    const actCard = el('div', 'card');
    const btns = el('div', 'row');
    const bFree = el('button', d.failed ? 'primary' : '', 'Free disk space');
    bFree.onclick = () => doAction('free');
    const bRestart = el('button', '', 'Restart opsbox');
    bRestart.onclick = () => doAction('restart');
    btns.append(bFree, bRestart);

    if (d.canRollback && d.previousVersion) {
      const bRoll = el('button', '', 'Roll back to ' + d.previousVersion);
      bRoll.onclick = () => doAction('rollback', {}, 'Roll back opsbox to ' + d.previousVersion + ' and restart services?');
      btns.append(bRoll);
    }

    const sshIp = el('input');
    sshIp.style.width = '180px';
    sshIp.placeholder = 'Your public IP';
    if (d.callerIp && d.callerIp !== '127.0.0.1' && d.callerIp !== '::1') {
      sshIp.value = d.callerIp;
    }
    const bSsh = el('button', '', 'Open SSH for 1 hour');
    bSsh.onclick = () => doAction('ssh', { ip: sshIp.value });

    const bReboot = el('button', 'danger', 'Reboot');
    bReboot.onclick = () => doAction('reboot', {}, 'Schedule a system reboot in 1 minute? Your apps will restart.');

    btns.append(bReboot);
    actCard.append(btns);

    const sshRow = el('div', 'row');
    sshRow.style.marginTop = '12px';
    sshRow.append(sshIp, bSsh);
    if (d.sshOpenUntil) {
      sshRow.append(el('span', 'muted mono', 'SSH open until ' + new Date(d.sshOpenUntil * 1000).toLocaleTimeString()));
    }
    actCard.append(sshRow);
    if (statusMsg) actCard.append(el('p', 'msg', statusMsg));
    if (errorMsg) actCard.append(el('p', 'err', errorMsg));
    app.append(actCard);

    const logCard = el('div', 'card');
    const logHdr = el('div', 'row');
    logHdr.append(el('strong', '', 'Recent logs'));
    const refreshBtn = el('button', '', 'Refresh');
    refreshBtn.style.marginLeft = 'auto';
    refreshBtn.onclick = () => { statusMsg = ''; errorMsg = ''; loadDashboard(); };
    logHdr.append(refreshBtn);
    const pre = el('pre', '', (d.log || []).join('\n') || 'No recent watchdog or opsbox log lines.');
    logCard.append(logHdr, pre);
    app.append(logCard);

    const foot = el('div', 'row');
    foot.style.marginTop = '16px';
    const back = el('a', 'btn', 'Back to panel');
    back.href = 'https://' + window.location.hostname + ':' + (d.panelPort || 8443) + '/#/recovery';
    foot.append(back);
    app.append(foot);
  }

  async function loadDashboard() {
    try {
      const d = await req('/api/v1/recovery');
      renderDashboard(d);
    } catch (e) {
      if (e.status === 401) {
        session = { authenticated: false };
        renderLogin();
      } else {
        errorMsg = e.message;
        renderLogin();
      }
    }
  }

  async function boot() {
    try {
      session = await req('/api/v1/session');
      if (session.authenticated) await loadDashboard();
      else renderLogin();
    } catch (e) {
      errorMsg = e.message;
      renderLogin();
    }
  }

  boot();
})();
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::PasswordHasher;
    use argon2::{Algorithm, Argon2, Params, Version};
    use opsbox_proto::{Role, WatchdogCheck};
    use opsbox_store::Store;
    use serde_json::Value;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    async fn start_test_server() -> (
        SocketAddr,
        PathBuf,
        PathBuf,
        tempfile::TempDir,
        Arc<SafeModeState>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("opsbox.db");
        let mk = dir.path().join("master.key");
        let rt = dir.path().join("run");
        fs::create_dir_all(&rt).unwrap();

        let store = Store::open(&db).unwrap();
        let params = Params::new(1024, 1, 1, None).unwrap();
        let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
        let admin_pw = argon
            .hash_password(b"correct horse battery")
            .unwrap()
            .to_string();
        let viewer_pw = argon.hash_password(b"viewer pw").unwrap().to_string();
        let secret = b"12345678901234567890".to_vec();
        let admin = store
            .create_user("admin", Role::Admin, &admin_pw, 100)
            .unwrap();
        let sealed = crate::auth::seal_for_test(&mk, &secret, &format!("totp:{}", admin.id));
        store.set_pending_totp(&admin.id, &sealed, 100).unwrap();
        store.enable_totp(&admin.id, 0, 100).unwrap();

        store
            .create_user("viewer", Role::Viewer, &viewer_pw, 100)
            .unwrap();
        drop(store);

        let cert = dir.path().join("cert.pem");
        let key = dir.path().join("key.pem");
        let (acceptor, _) = load_or_create_tls_acceptor(
            &cert,
            &key,
            &dir.path().join("fb.crt"),
            &dir.path().join("fb.key"),
        )
        .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let auth = Arc::new(SafeAuth::new(db.clone(), mk, rt.clone()));
        let snapshot = Arc::new(RwLock::new(WatchdogStatus {
            running: true,
            failed: true,
            degraded: true,
            updated_at: 1000,
            current_version: "0.1.5".into(),
            previous_version: None,
            can_rollback: false,
            update_message: None,
            checks: vec![WatchdogCheck {
                name: "serve".into(),
                status: "fail".into(),
                state: "down".into(),
                note: "refused".into(),
            }],
            recent_events: vec!["serve down".into()],
            safe_mode_listen: Some(addr.to_string()),
            ssh_open_until: None,
        }));
        let state = Arc::new(SafeModeState {
            auth,
            snapshot,
            slots: SlotManager {
                slots_dir: dir.path().join("bin"),
                symlink: dir.path().join("opsbox"),
                state_file: dir.path().join("update-state.json"),
            },
            priv_socket: dir.path().join("priv.sock"),
            xfer_dir: dir.path().join("xfer"),
            db_path: db,
            panel_port: 8443,
            wake_loop: Arc::new(Notify::new()),
            reset_failures: Arc::new(AtomicBool::new(false)),
        });

        let st = (*state).clone();
        tokio::spawn(async move {
            let _ = serve(listener, acceptor, st, std::future::pending()).await;
        });

        (addr, cert, rt, dir, state)
    }

    async fn https_raw(
        addr: SocketAddr,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> (u16, String, String) {
        #[derive(Debug)]
        struct AnyCert;
        impl rustls::client::danger::ServerCertVerifier for AnyCert {
            fn verify_server_cert(
                &self,
                _: &CertificateDer<'_>,
                _: &[CertificateDer<'_>],
                _: &rustls_pki_types::ServerName<'_>,
                _: &[u8],
                _: rustls_pki_types::UnixTime,
            ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
                Ok(rustls::client::danger::ServerCertVerified::assertion())
            }
            fn verify_tls12_signature(
                &self,
                _: &[u8],
                _: &CertificateDer<'_>,
                _: &rustls::DigitallySignedStruct,
            ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
            {
                Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
            }
            fn verify_tls13_signature(
                &self,
                _: &[u8],
                _: &CertificateDer<'_>,
                _: &rustls::DigitallySignedStruct,
            ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
            {
                Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
            }
            fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
                rustls::crypto::ring::default_provider()
                    .signature_verification_algorithms
                    .supported_schemes()
            }
        }
        let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCert))
        .with_no_client_auth();
        let conn = tokio_rustls::TlsConnector::from(Arc::new(cfg));
        let tcp = TcpStream::connect(addr).await.unwrap();
        let name = rustls_pki_types::ServerName::try_from("localhost").unwrap();
        let mut tls = conn.connect(name, tcp).await.unwrap();

        let mut raw = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost:{}\r\nConnection: close\r\nContent-Length: {}\r\n",
            addr.port(),
            body.len()
        );
        for (k, v) in headers {
            raw.push_str(&format!("{k}: {v}\r\n"));
        }
        raw.push_str("\r\n");
        raw.push_str(body);
        tls.write_all(raw.as_bytes()).await.unwrap();
        tls.flush().await.unwrap();

        let mut out = Vec::new();
        let _ = tls.read_to_end(&mut out).await;
        let text = String::from_utf8_lossy(&out).into_owned();
        let (head, resp_body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
        let code = head
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        (code, head.to_owned(), resp_body.to_owned())
    }

    fn cookie_from(headers: &str) -> String {
        headers
            .lines()
            .find_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case("set-cookie")
                    .then(|| v.trim().split(';').next().unwrap_or("").to_owned())
            })
            .unwrap()
    }

    #[tokio::test]
    async fn safe_mode_requires_auth_csrf_same_origin_and_recovery_code() {
        let (addr, _, rt, _dir, _st) = start_test_server().await;

        let (code, _, html) = https_raw(addr, "GET", "/", &[], "").await;
        assert_eq!(code, 200);
        assert!(html.contains("/safemode.js"));

        let (code, _, hz) = https_raw(addr, "GET", "/healthz", &[], "").await;
        assert_eq!(code, 200);
        let hz_v: Value = serde_json::from_str(&hz).unwrap();
        assert_eq!(hz_v["service"], "watchdog");

        // Unauthenticated recovery status and action are refused.
        let (code, _, _) = https_raw(addr, "GET", "/api/v1/recovery", &[], "").await;
        assert_eq!(code, 401);
        let (code, _, _) = https_raw(
            addr,
            "POST",
            "/api/v1/recovery/action",
            &[("Content-Type", "application/json")],
            r#"{"action":"free"}"#,
        )
        .await;
        assert_eq!(code, 401);

        // Non-admin viewer cannot log in even with right password.
        let (code, _, _) = https_raw(
            addr,
            "POST",
            "/api/v1/auth/login",
            &[("Content-Type", "application/json")],
            r#"{"username":"viewer","password":"viewer pw","totp":"000000"}"#,
        )
        .await;
        assert_eq!(code, 403);

        // One-time recovery code works once.
        let rec = crate::auth::generate_recovery_code(&rt).unwrap();
        let body = format!(r#"{{"code":"{rec}"}}"#);
        let (code, hdrs, login_json) = https_raw(
            addr,
            "POST",
            "/api/v1/auth/recovery-code",
            &[("Content-Type", "application/json")],
            &body,
        )
        .await;
        assert_eq!(code, 200, "body: {login_json}");
        let lv: Value = serde_json::from_str(&login_json).unwrap();
        let csrf = lv["csrf"].as_str().unwrap().to_owned();
        let cookie = cookie_from(&hdrs);

        // Reuse of same recovery code is refused.
        let (code2, _, _) = https_raw(
            addr,
            "POST",
            "/api/v1/auth/recovery-code",
            &[("Content-Type", "application/json")],
            &body,
        )
        .await;
        assert_eq!(code2, 401);

        // Authenticated GET /api/v1/recovery reflects snapshot & canRollback=false.
        let (code, _, rec_body) =
            https_raw(addr, "GET", "/api/v1/recovery", &[("Cookie", &cookie)], "").await;
        assert_eq!(code, 200);
        let rv: Value = serde_json::from_str(&rec_body).unwrap();
        assert_eq!(rv["failed"], true);
        assert_eq!(rv["canRollback"], false);

        // Action without CSRF token is refused (403).
        let (code, _, _) = https_raw(
            addr,
            "POST",
            "/api/v1/recovery/action",
            &[("Cookie", &cookie), ("Content-Type", "application/json")],
            r#"{"action":"free"}"#,
        )
        .await;
        assert_eq!(code, 403);

        // Cross-origin POST is refused (403).
        let (code, _, _) = https_raw(
            addr,
            "POST",
            "/api/v1/recovery/action",
            &[
                ("Cookie", &cookie),
                ("X-Opsbox-CSRF", &csrf),
                ("Origin", "https://evil.example"),
                ("Content-Type", "application/json"),
            ],
            r#"{"action":"free"}"#,
        )
        .await;
        assert_eq!(code, 403);

        // Unknown action returns 400.
        let (code, _, _) = https_raw(
            addr,
            "POST",
            "/api/v1/recovery/action",
            &[
                ("Cookie", &cookie),
                ("X-Opsbox-CSRF", &csrf),
                ("Content-Type", "application/json"),
            ],
            r#"{"action":"bogus"}"#,
        )
        .await;
        assert_eq!(code, 400);

        // SSH emergency refuses loopback IP cleanly.
        let (code, _, ssh_err) = https_raw(
            addr,
            "POST",
            "/api/v1/recovery/action",
            &[
                ("Cookie", &cookie),
                ("X-Opsbox-CSRF", &csrf),
                ("Content-Type", "application/json"),
            ],
            r#"{"action":"ssh","ip":"127.0.0.1"}"#,
        )
        .await;
        assert_eq!(code, 400, "{ssh_err}");

        // Valid `free` action runs cleanly.
        let (code, _, free_ok) = https_raw(
            addr,
            "POST",
            "/api/v1/recovery/action",
            &[
                ("Cookie", &cookie),
                ("X-Opsbox-CSRF", &csrf),
                ("Content-Type", "application/json"),
            ],
            r#"{"action":"free"}"#,
        )
        .await;
        assert_eq!(code, 200, "{free_ok}");

        // Logout invalidates the session.
        let (code, _, _) = https_raw(
            addr,
            "POST",
            "/api/v1/auth/logout",
            &[
                ("Cookie", &cookie),
                ("X-Opsbox-CSRF", &csrf),
                ("Content-Type", "application/json"),
            ],
            "{}",
        )
        .await;
        assert_eq!(code, 200);
        let (code, _, _) =
            https_raw(addr, "GET", "/api/v1/recovery", &[("Cookie", &cookie)], "").await;
        assert_eq!(code, 401);
    }
}
