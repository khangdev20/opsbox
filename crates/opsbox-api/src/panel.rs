//! Panel domain, DNS verification, ACME certificate issuance, and raw `:8443` listener
//! management (docs/BACKEND_SPEC.md 7.7, 7.18).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use axum::Json;
use axum::extract::{Extension, State};
use opsbox_proto::ErrorCode;
use opsbox_proxy::{CertInfo, CertState, ProxyClientError, Route, UpstreamTls};
use opsbox_store::{ActorKind, AuditResult};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::warn;

use crate::json::ApiJson;
use crate::session::{Admin, Client};
use crate::system::audit;
use crate::tasks::Outcome;
use crate::{AppState, HttpError, RequestId};

pub const KEY_PANEL_DOMAIN: &str = "opsbox.panel.domain";
pub const KEY_PANEL_DRAFT: &str = "opsbox.panel.draft";

pub(crate) fn map_proxy_err(e: ProxyClientError) -> HttpError {
    warn!(error = %e, "opsbox-proxy call failed");
    match e {
        ProxyClientError::Connect { .. } | ProxyClientError::Timeout => HttpError::new(
            ErrorCode::Unavailable,
            "The reverse proxy is not running. Start opsbox-proxy and try again.",
        ),
        ProxyClientError::Remote(err) => {
            let code = match err.code {
                opsbox_proxy::ProxyErrorCode::Invalid
                | opsbox_proxy::ProxyErrorCode::BadRequest => ErrorCode::Validation,
                opsbox_proxy::ProxyErrorCode::Forbidden => ErrorCode::Forbidden,
                opsbox_proxy::ProxyErrorCode::NotFound => ErrorCode::NotFound,
                opsbox_proxy::ProxyErrorCode::Unavailable => ErrorCode::Unavailable,
                opsbox_proxy::ProxyErrorCode::Internal => ErrorCode::Internal,
            };
            HttpError::new(code, err.message)
        }
        ProxyClientError::Io(_) | ProxyClientError::Protocol(_) => HttpError::internal(),
    }
}

/// Address `opsbox proxy` connects to in order to reach `serve`.
pub fn upstream_addr(listen: SocketAddr) -> SocketAddr {
    if listen.ip().is_unspecified() {
        let ip = if listen.is_ipv6() {
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        } else {
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        };
        SocketAddr::new(ip, listen.port())
    } else {
        listen
    }
}

pub fn panel_route(state: &AppState, domain: &str) -> Route {
    let upstream = upstream_addr(state.listen).to_string();
    let upstream_tls = if state.tls {
        UpstreamTls::Https {
            cert_sha256: state.tls_cert_sha256.clone(),
        }
    } else {
        UpstreamTls::Plain
    };
    Route {
        host: domain.to_owned(),
        upstream,
        upstream_tls,
        redirect_www: false,
        redirect_to: None,
        redirect_https: true,
        hsts: true,
        max_body_bytes: state.files.max_upload,
        is_panel: true,
    }
}

pub fn active_panel_domain(state: &AppState) -> Option<String> {
    state
        .store
        .get_setting(KEY_PANEL_DOMAIN)
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|s| !s.is_empty())
}

fn draft_panel_domain(state: &AppState) -> Option<String> {
    state
        .store
        .get_setting(KEY_PANEL_DRAFT)
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|s| !s.is_empty())
}

/// Build the complete proxy route table from an optional panel route and all active
/// service domain rows in SQLite (shared by `serve` and `opsbox web domain`).
pub fn build_routes_with_panel(
    store: &opsbox_store::Store,
    panel_route: Option<Route>,
) -> Vec<Route> {
    let mut routes = Vec::new();
    let panel_host = panel_route.as_ref().map(|r| r.host.clone());
    if let Some(r) = panel_route {
        routes.push(r);
    }
    if let Ok(svc_routes) = store.list_active_service_routes() {
        for sr in svc_routes {
            if panel_host
                .as_deref()
                .is_some_and(|p| p.eq_ignore_ascii_case(&sr.domain))
            {
                continue;
            }
            let upstream_port = sr.host_port.unwrap_or(9);
            routes.push(Route {
                host: sr.domain,
                upstream: format!("127.0.0.1:{upstream_port}"),
                upstream_tls: UpstreamTls::Plain,
                redirect_www: false,
                redirect_to: sr.redirect_to,
                redirect_https: sr.force_https,
                hsts: sr.hsts,
                max_body_bytes: 0,
                is_panel: false,
            });
        }
    }
    routes
}

/// Build the complete proxy route table (panel domain plus active service domains).
pub(crate) fn build_proxy_routes(state: &AppState) -> Vec<Route> {
    let panel = active_panel_domain(state).map(|d| panel_route(state, &d));
    build_routes_with_panel(&state.store, panel)
}

/// Push the current routing table (panel domain + active service routes) to `opsbox-proxy`.
pub async fn sync_proxy_routes(state: &AppState) -> Result<(), ProxyClientError> {
    let Some(ref client) = state.proxy else {
        return Ok(());
    };
    let routes = build_proxy_routes(state);
    client
        .clone()
        .with_timeout(Duration::from_secs(5))
        .set_routes("sync-routes", routes)
        .await
}

pub(crate) fn cert_state_str(s: CertState) -> &'static str {
    match s {
        CertState::None => "none",
        CertState::Pending => "pending",
        CertState::Valid => "valid",
        CertState::Expiring => "expiring",
        CertState::Error => "error",
    }
}

/// Current panel domain, certificate, proxy, and raw-port state for Settings and `/panel`.
pub async fn snapshot(state: &AppState) -> Value {
    let active = active_panel_domain(state);
    let draft = draft_panel_domain(state);
    let shown = active.clone().or(draft);

    let mut proxy_reachable = false;
    let mut cert_info: Option<CertInfo> = None;
    let mut acme_staging = false;

    if let Some(ref client) = state.proxy
        && let Ok(st) = client
            .clone()
            .with_timeout(Duration::from_secs(2))
            .status("panel-status")
            .await
    {
        proxy_reachable = true;
        acme_staging = st.acme_staging;
        if let Some(ref dom) = shown {
            cert_info = st.certs.into_iter().find(|c| c.domain == *dom);
        }
    }

    let cert_status = cert_info
        .as_ref()
        .map_or("none", |c| cert_state_str(c.status));

    json!({
        "domain": shown,
        "activeDomain": active,
        "certStatus": cert_status,
        "certExpiresAt": cert_info.as_ref().and_then(|c| c.not_after),
        "certDaysRemaining": cert_info.as_ref().and_then(|c| c.days_remaining),
        "certIssuer": cert_info.as_ref().and_then(|c| c.issuer.as_deref()),
        "certStaging": cert_info.as_ref().map_or(acme_staging, |c| c.staging),
        "certError": cert_info.as_ref().and_then(|c| c.last_error.as_deref()),
        "proxyReachable": proxy_reachable,
        "rawPortOpen": !state.listen.ip().is_loopback(),
        "listen": state.listen.to_string(),
    })
}

/// `GET /api/v1/panel`
pub(crate) async fn status(State(state): State<AppState>, Admin(_): Admin) -> Json<Value> {
    Json(snapshot(&state).await)
}

#[derive(Deserialize)]
pub(crate) struct DomainBody {
    domain: String,
}

const PUBLIC_IP_CACHE_TTL: Duration = Duration::from_secs(300);

pub(crate) async fn detect_server_public_ips(state: &AppState) -> Result<Vec<IpAddr>, HttpError> {
    if !state.public_ips.is_empty() {
        return Ok(state.public_ips.clone());
    }
    {
        let guard = state.public_ip_cache.lock().await;
        if let Some((at, ref ips)) = *guard
            && at.elapsed() < PUBLIC_IP_CACHE_TTL
        {
            return Ok(ips.clone());
        }
    }
    let overrides = state.public_ips.clone();
    let ips = tokio::task::spawn_blocking(move || opsbox_proxy::detect_public_ips(&overrides))
        .await
        .map_err(|_| HttpError::internal())?;
    let mut guard = state.public_ip_cache.lock().await;
    *guard = Some((std::time::Instant::now(), ips.clone()));
    Ok(ips)
}

pub(crate) async fn run_dns_check(
    state: &AppState,
    domain: &str,
) -> Result<opsbox_proxy::DnsCheckResult, HttpError> {
    let server_ips = detect_server_public_ips(state).await?;
    opsbox_proxy::check_dns_with_nameservers(domain, &server_ips, state.dns_nameservers.as_deref())
        .await
        .map_err(|e| HttpError::new(ErrorCode::Unavailable, e))
}

fn ensure_not_service_domain(state: &AppState, domain: &str) -> Result<(), HttpError> {
    if state
        .store
        .get_service_domain(domain)
        .map_err(|_| HttpError::internal())?
        .is_some()
    {
        return Err(HttpError(
            opsbox_proto::ApiError::new(
                ErrorCode::Conflict,
                format!("Domain '{domain}' is already attached to a service. Remove it from the service first."),
            )
            .with_field("domain"),
            None,
        ));
    }
    Ok(())
}

/// `POST /api/v1/panel/dns-check`
pub(crate) async fn check_dns(
    State(state): State<AppState>,
    Admin(_): Admin,
    ApiJson(body): ApiJson<DomainBody>,
) -> Result<Json<Value>, HttpError> {
    let domain = opsbox_proxy::normalize_domain(&body.domain).map_err(|e| {
        HttpError(
            opsbox_proto::ApiError::new(ErrorCode::Validation, e.0).with_field("domain"),
            None,
        )
    })?;
    ensure_not_service_domain(&state, &domain)?;
    let _ = state.store.set_setting(KEY_PANEL_DRAFT, &json!(domain));

    let result = run_dns_check(&state, &domain).await?;
    Ok(Json(
        serde_json::to_value(result).map_err(|_| HttpError::internal())?,
    ))
}

/// `POST /api/v1/panel/certificate` — asynchronous ACME HTTP-01 issuance task.
pub(crate) async fn issue_cert(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<DomainBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    let domain = opsbox_proxy::normalize_domain(&body.domain).map_err(|e| {
        HttpError(
            opsbox_proto::ApiError::new(ErrorCode::Validation, e.0).with_field("domain"),
            None,
        )
    })?;
    ensure_not_service_domain(&state, &domain)?;
    let proxy = state.proxy.clone().ok_or_else(|| {
        HttpError::new(
            ErrorCode::Unavailable,
            "The reverse proxy is not configured on this server.",
        )
    })?;
    proxy
        .clone()
        .with_timeout(Duration::from_secs(3))
        .ping(&rid)
        .await
        .map_err(map_proxy_err)?;

    let _ = state.store.set_setting(KEY_PANEL_DRAFT, &json!(domain));

    let state_clone = state.clone();
    let actor_name = who.user.name.clone();
    let actor_kind = if who.via_token {
        ActorKind::Token
    } else {
        ActorKind::User
    };
    let ip = client.ip.clone();
    let task = state.tasks.start("panel_cert", async move {
        let mut routes = build_proxy_routes(&state_clone);
        if !routes.iter().any(|r| r.host.eq_ignore_ascii_case(&domain)) {
            routes.push(panel_route(&state_clone, &domain));
        }

        if let Err(e) = proxy
            .clone()
            .with_timeout(Duration::from_secs(5))
            .set_routes(&rid, routes)
            .await
        {
            return Err(map_proxy_err(e));
        }

        match proxy
            .clone()
            .with_timeout(Duration::from_secs(120))
            .issue_cert(&rid, &domain)
            .await
        {
            Ok(cert) => {
                if let Err(e) = state_clone
                    .store
                    .set_setting(KEY_PANEL_DOMAIN, &json!(domain))
                {
                    return Err(HttpError::new(
                        ErrorCode::Internal,
                        format!("Certificate issued, but saving panel domain failed: {e}"),
                    ));
                }
                let _ = state_clone.store.delete_setting(KEY_PANEL_DRAFT);
                let _ = sync_proxy_routes(&state_clone).await;
                if let Err(e) = crate::github::sync_webhook_on_domain_change(
                    &state_clone,
                    &rid,
                    &actor_name,
                    &domain,
                )
                .await
                {
                    warn!(error = %e, "could not sync GitHub App webhook after panel certificate issuance");
                }
                state_clone.auth.audit(
                    &actor_name,
                    actor_kind,
                    Some(&ip),
                    "panel.cert.issue",
                    Some(&domain),
                    AuditResult::Ok,
                    json!({ "staging": cert.staging, "notAfter": cert.not_after }),
                );
                Ok(Outcome {
                    log: vec![format!("Certificate issued for {domain}.")],
                    result: serde_json::to_value(cert).unwrap_or(Value::Null),
                })
            }
            Err(e) => {
                let _ = sync_proxy_routes(&state_clone).await;
                let err = map_proxy_err(e);
                state_clone.auth.audit(
                    &actor_name,
                    actor_kind,
                    Some(&ip),
                    "panel.cert.issue",
                    Some(&domain),
                    AuditResult::Failed,
                    json!({ "error": err.0.error.message }),
                );
                Err(err)
            }
        }
    })?;

    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "task": task })),
    ))
}

#[derive(Deserialize, Default)]
pub(crate) struct ConfirmBody {
    #[serde(default)]
    confirm: bool,
}

/// `DELETE /api/v1/panel/domain`
pub(crate) async fn remove_domain(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    body: Option<ApiJson<ConfirmBody>>,
) -> Result<Json<Value>, HttpError> {
    let confirm = body.is_some_and(|ApiJson(b)| b.confirm);
    if state.listen.ip().is_loopback() && active_panel_domain(&state).is_some() && !confirm {
        return Err(HttpError::new(
            ErrorCode::ConfirmationRequired,
            "Port 8443 is currently closed to the internet. Removing the panel domain leaves only local access through an SSH tunnel until you re-open port 8443.",
        ));
    }

    let previous = active_panel_domain(&state);
    state
        .store
        .delete_setting(KEY_PANEL_DOMAIN)
        .map_err(|_| HttpError::internal())?;
    let _ = state.store.delete_setting(KEY_PANEL_DRAFT);

    if let Some(ref proxy) = state.proxy {
        if let Some(ref dom) = previous {
            let _ = proxy.remove_cert(&rid, dom).await;
        }
        let _ = sync_proxy_routes(&state).await;
    }

    audit(
        &state,
        &who,
        &client.ip,
        "panel.domain.remove",
        previous.as_deref(),
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(crate) struct LocalOnlyBody {
    #[serde(rename = "localOnly")]
    local_only: bool,
    #[serde(default)]
    confirm: bool,
}

/// `POST /api/v1/panel/local-only` — close or re-open the raw `:8443` listener via `priv`.
pub(crate) async fn set_local_only(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<LocalOnlyBody>,
) -> Result<Json<Value>, HttpError> {
    if body.local_only && !body.confirm {
        let domain_live = if let Some(dom) = active_panel_domain(&state)
            && let Some(ref proxy) = state.proxy
            && let Ok(st) = proxy
                .clone()
                .with_timeout(Duration::from_secs(3))
                .status(&rid)
                .await
        {
            st.certs.iter().any(|c| {
                c.domain == dom && matches!(c.status, CertState::Valid | CertState::Expiring)
            })
        } else {
            false
        };
        if !domain_live {
            return Err(HttpError::new(
                ErrorCode::ConfirmationRequired,
                "No panel domain certificate is live yet. Closing port 8443 means the panel will only be reachable over an SSH tunnel.",
            ));
        }
    }

    state
        .priv_client
        .web_local_only(&rid, &who.user.name, body.local_only)
        .await?;

    audit(
        &state,
        &who,
        &client.ip,
        "panel.local_only",
        None,
        json!({ "localOnly": body.local_only }),
    );
    Ok(Json(json!({
        "ok": true,
        "localOnly": body.local_only,
        "restarting": true,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_normalizes_unspecified_to_loopback() {
        assert_eq!(
            upstream_addr("0.0.0.0:8443".parse().unwrap()),
            "127.0.0.1:8443".parse().unwrap()
        );
        assert_eq!(
            upstream_addr("[::]:8443".parse().unwrap()),
            "[::1]:8443".parse().unwrap()
        );
        assert_eq!(
            upstream_addr("127.0.0.1:9443".parse().unwrap()),
            "127.0.0.1:9443".parse().unwrap()
        );
    }
}
