//! Per-service domains, DNS checks, ACME certificate tasks, redirects,
//! primary domain selection, Force HTTPS, and HSTS (docs/BACKEND_SPEC.md 7.7, 6.4).

use std::collections::HashMap;
use std::time::Duration;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use opsbox_proto::ErrorCode;
use opsbox_proxy::{CertInfo, CertState};
use opsbox_store::{
    ActorKind, AuditResult, NewServiceDomain, ServiceDomainRecord, ServiceKind, ServiceRecord,
    UpdateServiceDomain,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deploy::{audit_user, now_secs, service_readable, service_writable};
use crate::json::ApiJson;
use crate::panel::{
    active_panel_domain, cert_state_str, detect_server_public_ips, map_proxy_err, run_dns_check,
    sync_proxy_routes,
};
use crate::session::{Client, Deployer, Viewer};
use crate::tasks::Outcome;
use crate::{AppState, HttpError, RequestId};

fn val_err(field: &'static str, msg: impl Into<String>) -> HttpError {
    HttpError(
        opsbox_proto::ApiError::new(ErrorCode::Validation, msg.into()).with_field(field),
        None,
    )
}

fn conflict_err(field: &'static str, msg: impl Into<String>) -> HttpError {
    HttpError(
        opsbox_proto::ApiError::new(ErrorCode::Conflict, msg.into()).with_field(field),
        None,
    )
}

fn normalize_domain_field(field: &'static str, raw: &str) -> Result<String, HttpError> {
    opsbox_proxy::normalize_domain(raw).map_err(|e| val_err(field, e.0))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WwwMode {
    None,
    WwwToBare,
    BareToWww,
    ServeBoth,
}

impl WwwMode {
    fn parse(raw: Option<&str>) -> Result<Self, HttpError> {
        let Some(s) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
            return Ok(Self::None);
        };
        let low = s.to_ascii_lowercase();
        if low == "none" || low == "no www" || low.starts_with("no www ") {
            Ok(Self::None)
        } else if low == "www_to_bare" || low == "redirect www to the bare domain" {
            Ok(Self::WwwToBare)
        } else if low == "bare_to_www" || low == "redirect the bare domain to www" {
            Ok(Self::BareToWww)
        } else if low == "serve_both" || low == "serve both" || low == "serve both bare and www" {
            Ok(Self::ServeBoth)
        } else {
            Err(val_err(
                "www",
                "Choose one of: none, www_to_bare, bare_to_www, or serve_both.",
            ))
        }
    }
}

async fn fetch_cert_map(state: &AppState) -> (bool, HashMap<String, CertInfo>) {
    let mut map = HashMap::new();
    let Some(ref proxy) = state.proxy else {
        return (false, map);
    };
    let Ok(st) = proxy
        .clone()
        .with_timeout(Duration::from_secs(2))
        .status("domains-status")
        .await
    else {
        return (false, map);
    };
    for cert in st.certs {
        map.insert(cert.domain.to_ascii_lowercase(), cert);
    }
    (true, map)
}

fn domain_json(rec: &ServiceDomainRecord, cert: Option<&CertInfo>) -> Value {
    let cert_status = cert.map_or("none", |c| cert_state_str(c.status));
    json!({
        "id": rec.id,
        "name": rec.domain,
        "domain": rec.domain,
        "primary": rec.is_primary,
        "redirect": rec.redirect_to.as_deref().unwrap_or(""),
        "redirectTo": rec.redirect_to,
        "forceHttps": rec.force_https,
        "hsts": rec.hsts,
        "custom": !rec.is_primary,
        "dns": rec.dns_status,
        "dnsCheckedAt": rec.dns_checked_at,
        "cert": cert_status,
        "renews": cert.and_then(|c| c.days_remaining),
        "certExpiresAt": cert.and_then(|c| c.not_after),
        "certIssuer": cert.and_then(|c| c.issuer.as_deref()),
        "certStaging": cert.is_some_and(|c| c.staging),
        "certError": cert.and_then(|c| c.last_error.as_deref()),
        "consecutiveFailures": cert.map_or(0, |c| c.consecutive_failures),
    })
}

fn load_domain_and_service(
    state: &AppState,
    name_raw: &str,
) -> Result<(String, ServiceDomainRecord, ServiceRecord), HttpError> {
    let domain = normalize_domain_field("domain", name_raw)?;
    let rec = state
        .store
        .get_service_domain(&domain)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Domain not found."))?;
    let svc = state
        .store
        .get_service(&rec.service_id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    Ok((domain, rec, svc))
}

fn check_domain_available(
    state: &AppState,
    svc: &ServiceRecord,
    domain: &str,
) -> Result<(), HttpError> {
    if active_panel_domain(state).is_some_and(|p| p.eq_ignore_ascii_case(domain)) {
        return Err(conflict_err(
            "domain",
            format!("Domain '{domain}' is already used as the panel domain."),
        ));
    }
    if let Some(existing) = state
        .store
        .get_service_domain(domain)
        .map_err(|_| HttpError::internal())?
    {
        if existing.service_id == svc.id {
            return Err(conflict_err(
                "domain",
                format!("Domain '{domain}' is already added to this service."),
            ));
        }
        return Err(conflict_err(
            "domain",
            format!("Domain '{domain}' is already attached to another service."),
        ));
    }
    Ok(())
}

/// `GET /api/v1/services/{id}/domains`
pub(crate) async fn get_domains(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    let svc = state
        .store
        .get_service(&id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_readable(&who, &svc) {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "You do not have access to this service.",
        ));
    }

    let store = state.store.clone();
    let svc_id = svc.id.clone();
    let proj_id = svc.project_id.clone();
    let (records, proj_name) = tokio::task::spawn_blocking(move || {
        let recs = store.list_service_domains(&svc_id)?;
        let proj = store
            .resolve_project(&proj_id, now_secs())?
            .map_or_else(|| "default".to_owned(), |(p, _)| p.name);
        Ok::<_, opsbox_store::StoreError>((recs, proj))
    })
    .await
    .map_err(|_| HttpError::internal())?
    .map_err(|_| HttpError::internal())?;

    let proxy_mode = crate::network::proxy_mode(&state).await;
    let (proxy_reachable, cert_map) = if proxy_mode == opsbox_proto::ProxyMode::Builtin {
        fetch_cert_map(&state).await
    } else {
        (false, HashMap::new())
    };
    let proxy_network = if proxy_mode == opsbox_proto::ProxyMode::Builtin {
        if let Some(proxy) = &state.proxy {
            proxy.status("domain-network").await.ok().map(|s| s.network)
        } else {
            None
        }
    } else {
        None
    };
    let has_pending = records.iter().any(|r| r.dns_status == "pending");
    let server_ips = if has_pending {
        detect_server_public_ips(&state)
            .await?
            .into_iter()
            .map(|ip| ip.to_string())
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let rendered: Vec<Value> = records
        .iter()
        .map(|r| domain_json(r, cert_map.get(&r.domain.to_ascii_lowercase())))
        .collect();
    let adopted = if who.user.role == opsbox_proto::Role::Admin {
        if let Some(proxy) = &state.proxy {
            proxy
                .status("adopted-domains")
                .await
                .ok()
                .map(|s| s.adopted.domains)
                .unwrap_or_default()
        } else {
            vec![]
        }
    } else {
        vec![]
    };
    let internal_url = if svc.port > 0 {
        format!("http://{}:{}", svc.name, svc.port)
    } else {
        svc.name.clone()
    };

    Ok(Json(json!({
        "service": svc.name,
        "project": proj_name,
        "network": format!("opsbox-{proj_name}"),
        "port": svc.port,
        "internalHost": svc.name,
        "internalUrl": internal_url,
        "proxyReachable": proxy_reachable,
        "proxyMode": proxy_mode,
        "proxyNetwork": proxy_network,
        "canManageProxy": who.user.role == opsbox_proto::Role::Admin,
        "externalUpstream": svc.active_host_port.filter(|p| *p > 0).map(|p| format!("127.0.0.1:{p}")),
        "serverIps": server_ips,
        "domains": rendered,
        "adopted": adopted,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AddDomainBody {
    #[serde(alias = "name")]
    domain: String,
    #[serde(default)]
    www: Option<String>,
    #[serde(alias = "redirect_to", alias = "redirect")]
    redirect_to: Option<String>,
    #[serde(alias = "force", alias = "force_https")]
    force_https: Option<bool>,
    hsts: Option<bool>,
    primary: Option<bool>,
}

/// `POST /api/v1/services/{id}/domains`
pub(crate) async fn add_domain(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<AddDomainBody>,
) -> Result<Response, HttpError> {
    let svc = state
        .store
        .get_service(&id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_writable(&who, &svc) {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "You cannot manage domains for this service.",
        ));
    }
    if svc.kind == ServiceKind::Db {
        return Err(val_err(
            "service",
            "Database services cannot have public domains.",
        ));
    }
    if svc.internal_only {
        return Err(val_err(
            "service",
            "This service is marked internal-only and has no loopback host port for public domains.",
        ));
    }
    if svc.port == 0 {
        return Err(val_err(
            "service",
            "This service runs on port 0 (background worker). Set a container port before adding a public domain.",
        ));
    }

    let domain = normalize_domain_field("domain", &body.domain)?;
    let www_mode = WwwMode::parse(body.www.as_deref())?;
    let force_https = body.force_https.unwrap_or(true);
    let hsts = body.hsts.unwrap_or(force_https);
    let want_primary = body.primary.unwrap_or(false);

    let existing_for_svc = state
        .store
        .list_service_domains(&svc.id)
        .map_err(|_| HttpError::internal())?;

    let redirect_target = body
        .redirect_to
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    struct Planned {
        domain: String,
        is_primary: bool,
        redirect_to: Option<String>,
    }

    let planned: Vec<Planned> = if let Some(raw_target) = redirect_target {
        let target = normalize_domain_field("redirectTo", raw_target)?;
        if target == domain {
            return Err(val_err("redirectTo", "A domain cannot redirect to itself."));
        }
        let target_rec = existing_for_svc
            .iter()
            .find(|r| r.domain.eq_ignore_ascii_case(&target))
            .ok_or_else(|| {
                val_err(
                    "redirectTo",
                    format!("Redirect target '{target}' must already belong to this service."),
                )
            })?;
        if target_rec.redirect_to.is_some() {
            return Err(val_err(
                "redirectTo",
                format!("Cannot redirect to '{target}' because it already redirects elsewhere."),
            ));
        }
        check_domain_available(&state, &svc, &domain)?;
        vec![Planned {
            domain,
            is_primary: false,
            redirect_to: Some(target),
        }]
    } else {
        let bare = domain.strip_prefix("www.").unwrap_or(&domain).to_owned();
        let www_dom = format!("www.{bare}");
        match www_mode {
            WwwMode::None => {
                check_domain_available(&state, &svc, &domain)?;
                vec![Planned {
                    domain,
                    is_primary: want_primary,
                    redirect_to: None,
                }]
            }
            WwwMode::WwwToBare => {
                if let Some(existing_bare) = existing_for_svc
                    .iter()
                    .find(|r| r.domain.eq_ignore_ascii_case(&bare))
                {
                    if existing_bare.redirect_to.is_some() {
                        return Err(conflict_err(
                            "domain",
                            format!(
                                "'{bare}' already redirects elsewhere on this service. Remove or clear its redirect first."
                            ),
                        ));
                    }
                    check_domain_available(&state, &svc, &www_dom)?;
                    vec![Planned {
                        domain: www_dom,
                        is_primary: false,
                        redirect_to: Some(bare),
                    }]
                } else {
                    check_domain_available(&state, &svc, &bare)?;
                    check_domain_available(&state, &svc, &www_dom)?;
                    vec![
                        Planned {
                            domain: bare.clone(),
                            is_primary: want_primary,
                            redirect_to: None,
                        },
                        Planned {
                            domain: www_dom,
                            is_primary: false,
                            redirect_to: Some(bare),
                        },
                    ]
                }
            }
            WwwMode::BareToWww => {
                if let Some(existing_www) = existing_for_svc
                    .iter()
                    .find(|r| r.domain.eq_ignore_ascii_case(&www_dom))
                {
                    if existing_www.redirect_to.is_some() {
                        return Err(conflict_err(
                            "domain",
                            format!(
                                "'{www_dom}' already redirects elsewhere on this service. Remove or clear its redirect first."
                            ),
                        ));
                    }
                    check_domain_available(&state, &svc, &bare)?;
                    vec![Planned {
                        domain: bare,
                        is_primary: false,
                        redirect_to: Some(www_dom),
                    }]
                } else {
                    check_domain_available(&state, &svc, &www_dom)?;
                    check_domain_available(&state, &svc, &bare)?;
                    vec![
                        Planned {
                            domain: www_dom.clone(),
                            is_primary: want_primary,
                            redirect_to: None,
                        },
                        Planned {
                            domain: bare,
                            is_primary: false,
                            redirect_to: Some(www_dom),
                        },
                    ]
                }
            }
            WwwMode::ServeBoth => {
                let has_bare = existing_for_svc
                    .iter()
                    .any(|r| r.domain.eq_ignore_ascii_case(&bare));
                let has_www = existing_for_svc
                    .iter()
                    .any(|r| r.domain.eq_ignore_ascii_case(&www_dom));
                let mut list = Vec::new();
                if !has_bare {
                    check_domain_available(&state, &svc, &bare)?;
                    list.push(Planned {
                        domain: bare,
                        is_primary: want_primary,
                        redirect_to: None,
                    });
                }
                if !has_www {
                    check_domain_available(&state, &svc, &www_dom)?;
                    list.push(Planned {
                        domain: www_dom,
                        is_primary: false,
                        redirect_to: None,
                    });
                }
                if list.is_empty() {
                    return Err(conflict_err(
                        "domain",
                        "Both the bare and www domains are already added to this service.",
                    ));
                }
                list
            }
        }
    };

    let entries: Vec<NewServiceDomain<'_>> = planned
        .iter()
        .map(|p| NewServiceDomain {
            domain: &p.domain,
            is_primary: p.is_primary,
            redirect_to: p.redirect_to.as_deref(),
            force_https,
            hsts,
        })
        .collect();

    let inserted = state
        .store
        .add_service_domains(&svc.id, &entries, now_secs())
        .map_err(|_| HttpError::internal())?;
    let _ = sync_proxy_routes(&state).await;

    let added_names: Vec<&str> = inserted.iter().map(|r| r.domain.as_str()).collect();
    audit_user(
        &state,
        &who.user.actor_label(),
        who.via_token,
        Some(&client.ip),
        "service.domain.add",
        Some(&svc.name),
        json!({ "domains": added_names }),
    );

    let first = &inserted[0];
    let mut payload = domain_json(first, None);
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(
            "added".into(),
            Value::Array(inserted.iter().map(|r| domain_json(r, None)).collect()),
        );
    }
    Ok((StatusCode::CREATED, Json(payload)).into_response())
}

/// `POST /api/v1/domains/{name}/check`
pub(crate) async fn check_dns(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Path(name): Path<String>,
) -> Result<Json<Value>, HttpError> {
    let (domain, _rec, svc) = load_domain_and_service(&state, &name)?;
    if !service_writable(&who, &svc) {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "You cannot manage domains for this service.",
        ));
    }

    let dns_res = run_dns_check(&state, &domain).await?;
    let next_dns = if dns_res.points_here { "ok" } else { "pending" };
    let now = now_secs();
    let updated = state
        .store
        .update_service_domain(
            &domain,
            UpdateServiceDomain {
                dns_status: Some(next_dns),
                dns_checked_at: Some(Some(now)),
                now,
                ..Default::default()
            },
        )
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Domain not found."))?;

    audit_user(
        &state,
        &who.user.actor_label(),
        who.via_token,
        Some(&client.ip),
        "service.domain.dns_check",
        Some(&domain),
        json!({ "service": svc.name, "pointsHere": dns_res.points_here }),
    );

    let (_, cert_map) = fetch_cert_map(&state).await;
    let cert = cert_map.get(&domain);
    let mut val = serde_json::to_value(&dns_res).map_err(|_| HttpError::internal())?;
    if let Some(obj) = val.as_object_mut() {
        obj.insert("name".into(), json!(updated.domain));
        obj.insert("dns".into(), json!(updated.dns_status));
        obj.insert(
            "cert".into(),
            json!(cert.map_or("none", |c| cert_state_str(c.status))),
        );
        obj.insert("renews".into(), json!(cert.and_then(|c| c.days_remaining)));
        obj.insert(
            "certError".into(),
            json!(cert.and_then(|c| c.last_error.as_deref())),
        );
    }
    Ok(Json(val))
}

/// `POST /api/v1/domains/{name}/certificate` — asynchronous ACME HTTP-01 issuance task.
pub(crate) async fn issue_cert(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(name): Path<String>,
) -> Result<(StatusCode, Json<Value>), HttpError> {
    let (domain, _rec, svc) = load_domain_and_service(&state, &name)?;
    if !service_writable(&who, &svc) {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "You cannot manage domains for this service.",
        ));
    }

    crate::network::require_builtin(&state).await?;
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

    let state_clone = state.clone();
    let actor_name = who.user.actor_label();
    let actor_kind = if who.via_token {
        ActorKind::Token
    } else {
        ActorKind::User
    };
    let ip = client.ip.clone();
    let svc_name = svc.name.clone();
    let task_kind = format!("app_cert:{domain}");

    let task = state.tasks.start(&task_kind, async move {
        let _ = sync_proxy_routes(&state_clone).await;
        match proxy
            .clone()
            .with_timeout(Duration::from_secs(120))
            .issue_cert(&rid, &domain)
            .await
        {
            Ok(cert) => {
                let _ = sync_proxy_routes(&state_clone).await;
                state_clone.auth.audit(
                    &actor_name,
                    actor_kind,
                    Some(&ip),
                    "service.domain.cert.issue",
                    Some(&domain),
                    AuditResult::Ok,
                    json!({ "service": svc_name, "staging": cert.staging, "notAfter": cert.not_after }),
                );
                Ok(Outcome {
                    log: vec![format!("Certificate issued for {domain}.")],
                    result: serde_json::to_value(cert).unwrap_or(Value::Null),
                })
            }
            Err(e) => {
                let err = map_proxy_err(e);
                state_clone.auth.audit(
                    &actor_name,
                    actor_kind,
                    Some(&ip),
                    "service.domain.cert.issue",
                    Some(&domain),
                    AuditResult::Failed,
                    json!({ "service": svc_name, "error": err.0.error.message }),
                );
                Err(err)
            }
        }
    })?;

    Ok((StatusCode::ACCEPTED, Json(json!({ "task": task }))))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateDomainBody {
    #[serde(alias = "force", alias = "force_https")]
    force_https: Option<bool>,
    hsts: Option<bool>,
    #[serde(alias = "redirect_to", alias = "redirect")]
    redirect_to: Option<String>,
    primary: Option<bool>,
}

/// `PATCH /api/v1/domains/{name}`
pub(crate) async fn update_domain(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Path(name): Path<String>,
    ApiJson(body): ApiJson<UpdateDomainBody>,
) -> Result<Json<Value>, HttpError> {
    let (domain, rec, svc) = load_domain_and_service(&state, &name)?;
    if !service_writable(&who, &svc) {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "You cannot manage domains for this service.",
        ));
    }

    let existing_for_svc = state
        .store
        .list_service_domains(&svc.id)
        .map_err(|_| HttpError::internal())?;

    let redirect_upd: Option<Option<String>> = match body.redirect_to {
        None => None,
        Some(ref raw) if raw.trim().is_empty() => Some(None),
        Some(ref raw) => {
            let target = normalize_domain_field("redirectTo", raw)?;
            if target == domain {
                return Err(val_err("redirectTo", "A domain cannot redirect to itself."));
            }
            if rec.is_primary {
                return Err(val_err(
                    "redirectTo",
                    "Make another domain primary before turning this one into a redirect.",
                ));
            }
            if let Some(other) = existing_for_svc
                .iter()
                .find(|r| r.redirect_to.as_deref() == Some(domain.as_str()))
            {
                return Err(conflict_err(
                    "redirectTo",
                    format!(
                        "Cannot redirect '{domain}' while '{}' redirects to it.",
                        other.domain
                    ),
                ));
            }
            let target_rec = existing_for_svc
                .iter()
                .find(|r| r.domain.eq_ignore_ascii_case(&target))
                .ok_or_else(|| {
                    val_err(
                        "redirectTo",
                        format!("Redirect target '{target}' must belong to this service."),
                    )
                })?;
            if target_rec.redirect_to.is_some() {
                return Err(val_err(
                    "redirectTo",
                    format!(
                        "Cannot redirect to '{target}' because it already redirects elsewhere."
                    ),
                ));
            }
            Some(Some(target))
        }
    };

    let now = now_secs();
    let mut updated = state
        .store
        .update_service_domain(
            &domain,
            UpdateServiceDomain {
                redirect_to: redirect_upd.as_ref().map(|o| o.as_deref()),
                force_https: body.force_https,
                hsts: body.hsts,
                now,
                ..Default::default()
            },
        )
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Domain not found."))?;

    if body.primary == Some(true) {
        if updated.redirect_to.is_some() {
            return Err(val_err(
                "primary",
                "A redirecting domain cannot be set as primary. Clear its redirect first.",
            ));
        }
        updated = state
            .store
            .make_primary_service_domain(&svc.id, &domain, now)
            .map_err(|_| HttpError::internal())?
            .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Domain not found."))?;
    }

    let _ = sync_proxy_routes(&state).await;
    audit_user(
        &state,
        &who.user.actor_label(),
        who.via_token,
        Some(&client.ip),
        "service.domain.update",
        Some(&domain),
        json!({
            "service": svc.name,
            "forceHttps": updated.force_https,
            "hsts": updated.hsts,
            "redirectTo": updated.redirect_to,
            "primary": updated.is_primary,
        }),
    );

    let (_, cert_map) = fetch_cert_map(&state).await;
    Ok(Json(domain_json(&updated, cert_map.get(&domain))))
}

/// `POST /api/v1/domains/{name}/primary`
pub(crate) async fn make_primary(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Path(name): Path<String>,
) -> Result<Json<Value>, HttpError> {
    let (domain, rec, svc) = load_domain_and_service(&state, &name)?;
    if !service_writable(&who, &svc) {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "You cannot manage domains for this service.",
        ));
    }
    if rec.redirect_to.is_some() {
        return Err(val_err(
            "domain",
            "A redirecting domain cannot be set as primary. Clear its redirect first.",
        ));
    }

    let updated = state
        .store
        .make_primary_service_domain(&svc.id, &domain, now_secs())
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Domain not found."))?;

    let _ = sync_proxy_routes(&state).await;
    audit_user(
        &state,
        &who.user.actor_label(),
        who.via_token,
        Some(&client.ip),
        "service.domain.primary",
        Some(&domain),
        json!({ "service": svc.name }),
    );

    let (_, cert_map) = fetch_cert_map(&state).await;
    Ok(Json(domain_json(&updated, cert_map.get(&domain))))
}

/// `DELETE /api/v1/domains/{name}`
pub(crate) async fn remove_domain(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(name): Path<String>,
) -> Result<Json<Value>, HttpError> {
    let (domain, _rec, svc) = load_domain_and_service(&state, &name)?;
    if !service_writable(&who, &svc) {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "You cannot manage domains for this service.",
        ));
    }

    let existing = state
        .store
        .list_service_domains(&svc.id)
        .map_err(|_| HttpError::internal())?;
    if let Some(other) = existing
        .iter()
        .find(|r| r.redirect_to.as_deref() == Some(domain.as_str()))
    {
        return Err(conflict_err(
            "domain",
            format!(
                "Cannot remove '{domain}' while '{}' redirects to it. Remove or update '{}' first.",
                other.domain, other.domain
            ),
        ));
    }

    let (_, next_primary) = state
        .store
        .remove_service_domain(&domain, now_secs())
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Domain not found."))?;

    if let Some(ref proxy) = state.proxy {
        let _ = proxy.remove_cert(&rid, &domain).await;
        let _ = sync_proxy_routes(&state).await;
    }

    audit_user(
        &state,
        &who.user.actor_label(),
        who.via_token,
        Some(&client.ip),
        "service.domain.remove",
        Some(&domain),
        json!({ "service": svc.name, "nextPrimary": next_primary }),
    );

    Ok(Json(json!({ "ok": true, "primary": next_primary })))
}

/// Helper used by `metrics::get_attention`: returns true if a persisted certificate has a
/// renewal failure or is in an error/expired state.
pub(crate) fn cert_needs_attention(cert: &CertInfo) -> bool {
    cert.not_after.is_some()
        && (cert.last_error.is_some() || matches!(cert.status, CertState::Error))
}
