//! Admission and dry-run endpoints use the same pure policy. No host IO in serve.
use crate::{
    AppState, HttpError, RequestId,
    session::{Deployer, Viewer},
};
use axum::{
    Json,
    extract::{Extension, FromRequestParts, Query, Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use opsbox_capacity::{Feature, Fit, Request as CapacityRequest, Snapshot, requirements};
use opsbox_proto::{ErrorCode, PrivOp, Role};
use serde_json::{Value, json};

pub(crate) struct Pending {
    reservation: opsbox_capacity::Reservation,
    armed: bool,
}

pub(crate) async fn snapshot(state: &AppState, rid: &str) -> Result<Snapshot, HttpError> {
    let mut s = state.priv_client.capacity(rid).await?;
    // Queued/building releases have already committed RAM, even before Docker starts.
    for svc in state
        .store
        .list_all_services()
        .map_err(|_| HttpError::internal())?
    {
        if svc.state == opsbox_store::ServiceState::Building {
            let memory = svc
                .memory_limit_mb
                .unwrap_or(requirements::DEFAULT_MB as i64)
                .max(64) as u64;
            s.reservations.push(opsbox_capacity::Reservation {
                name: format!("{} (deploy)", svc.name),
                memory_mb: if svc.kind == opsbox_store::ServiceKind::App
                    && svc.source_kind != opsbox_store::SourceKind::DockerImage
                {
                    memory.max(1536)
                } else {
                    memory
                },
                cpu_millis: svc.cpu_limit_millis.unwrap_or(0).max(0) as u64,
                database: svc.kind == opsbox_store::ServiceKind::Db,
            });
        }
    }
    let mut pending = state.capacity_pending.lock().unwrap();
    pending.retain(|kind, p| !p.armed || state.tasks.is_running(kind));
    for p in pending.values() {
        if !s.reservations.iter().any(|r| r.name == p.reservation.name) {
            s.reservations.push(p.reservation.clone());
        }
    }
    Ok(s)
}
pub(crate) fn conflict(s: &Snapshot, r: &CapacityRequest, fit: &Fit) -> HttpError {
    let mut err = HttpError::new(ErrorCode::Conflict, fit.reasons().join(" "));
    err.0.error.capacity = Some(
        json!({"asked":r,"free":s.budget(r.replacing.as_deref()),"reserves":s.budget(None),"reservations":s.reservations,"fit":fit}),
    );
    err
}
pub(crate) async fn get(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let s = snapshot(&state, &rid).await?;
    let features:Vec<_>=requirements::FEATURES.iter().map(|(feature,min,rec,disk)|json!({"feature":feature,"minimum_mb":min,"recommended_mb":rec,"disk_mb":disk,"fit":s.admit(&CapacityRequest {feature:*feature,..Default::default()})})).collect();
    Ok(Json(
        json!({"profile":s.profile,"reserves":s.budget(None),"reservations":s.reservations,"allocatable":s.budget(None),"pressure":s.budget(None).pressure,"features":features,"swap_offer_mb":s.swap_offer_mb()}),
    ))
}
pub(crate) async fn check(
    State(state): State<AppState>,
    Viewer(_): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
    Query(r): Query<CapacityRequest>,
) -> Result<Json<Value>, HttpError> {
    let s = snapshot(&state, &rid).await?;
    Ok(Json(
        json!({"request":r,"fit":s.admit(&r),"free":s.budget(r.replacing.as_deref())}),
    ))
}
#[derive(serde::Deserialize)]
pub(crate) struct SwapBody {
    size_mb: u64,
}
pub(crate) async fn swap(
    State(state): State<AppState>,
    crate::session::Admin(who): crate::session::Admin,
    Extension(RequestId(rid)): Extension<RequestId>,
    crate::json::ApiJson(body): crate::json::ApiJson<SwapBody>,
) -> Result<Json<Value>, HttpError> {
    crate::session::deny_token(&who)?;
    state
        .priv_client
        .call(
            &rid,
            Some(&who.user.name),
            PrivOp::CapacityAddSwap {
                size_mb: body.size_mb,
            },
        )
        .await?;
    crate::deploy::audit_user(
        &state,
        &who.user.name,
        false,
        None,
        "capacity.swap",
        None,
        json!({"size_mb":body.size_mb}),
    );
    Ok(Json(json!({"ok":true})))
}
fn numeric(body: &Value, camel: &str, snake: &str) -> Option<u64> {
    body.get(camel)
        .or_else(|| body.get(snake))
        .and_then(Value::as_u64)
}
fn template_request(
    t: &opsbox_deploy::templates::TemplateSpec,
    memory: Option<u64>,
) -> CapacityRequest {
    CapacityRequest {
        database: t.kind == opsbox_deploy::templates::TemplateKind::Db,
        feature: if opsbox_deploy::templates::is_postgres_template(&t.id) {
            Feature::Database
        } else {
            Feature::Template
        },
        memory_mb: memory.unwrap_or(t.min_memory_mb),
        min_memory_mb: t.min_memory_mb,
        recommended_memory_mb: t.recommended_memory_mb,
        ..Default::default()
    }
}
/// Route middleware holds an admission lock through the handler's persistent enqueue.
/// It covers every HTTP workload mutation before spawning a background task.
pub(crate) async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    match guard_inner(state, req, next).await {
        Ok(r) => r,
        Err(e) => e.into_response(),
    }
}
async fn guard_inner(state: AppState, req: Request, next: Next) -> Result<Response, HttpError> {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let p: Vec<_> = path.trim_start_matches("/api/v1/").split('/').collect();
    let interested = matches!(
        (method.as_str(), p.as_slice()),
        ("POST", ["services"])
            | ("PATCH", ["services", _])
            | ("POST", ["services", _, "deploys"])
            | ("POST", ["services", _, "deploys", _, "rollback"])
            | ("POST", ["templates", _, "instances"])
            | ("POST", ["compose"])
            | ("PUT", ["databases", _, "pooler"])
            | ("PUT", ["databases", _, "extensions", _])
            | ("POST", ["databases", _, "upgrade"])
            | ("POST", ["docker", "install"])
            | ("POST", ["docker", "containers", _, "start" | "restart"])
            | ("POST", ["backups", "run"])
            | ("POST", ["backups", "jobs", _, "run"])
            | ("POST", ["backups", "snapshots", _, "restore"])
    );
    if !interested {
        return Ok(next.run(req).await);
    }
    let (mut parts, body) = req.into_parts();
    let Deployer(who) = Deployer::from_request_parts(&mut parts, &state).await?;
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    let bytes = axum::body::to_bytes(body, 1024 * 1024)
        .await
        .map_err(|_| HttpError::new(ErrorCode::TooLarge, "Request is too large."))?;
    let mut body: Value = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes)
            .map_err(|_| HttpError::new(ErrorCode::Validation, "Request must be JSON."))?
    };
    let overriding = body
        .get("override")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if overriding && (who.via_token || who.user.role != Role::Admin) {
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "Only a signed-in Owner can override capacity.",
        ));
    }
    if overriding
        && body
            .get("capacityConfirmation")
            .or_else(|| body.get("capacity_confirmation"))
            .and_then(Value::as_str)
            != Some(requirements::CONFIRMATION)
    {
        return Err(HttpError::new(
            ErrorCode::ConfirmationRequired,
            format!("Type: {}", requirements::CONFIRMATION),
        ));
    }
    let _guard = state.capacity_lock.lock().await;
    let rid = parts
        .extensions
        .get::<RequestId>()
        .map(|r| r.0.as_str())
        .unwrap_or("capacity");
    let mut requests = Vec::new();
    let mut images = Vec::new();
    let mut pending = None;
    let mut target = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("workload")
        .to_owned();
    let memory = numeric(&body, "memoryLimitMb", "memory_limit_mb");
    match p.as_slice() {
        ["services"] => {
            if let Some(image) = body
                .get("image")
                .or_else(|| body.get("image_ref"))
                .and_then(Value::as_str)
            {
                images.push(image.to_owned());
            }
            requests.push(CapacityRequest {
                memory_mb: memory.unwrap_or(requirements::DEFAULT_MB),
                cpu_millis: numeric(&body, "cpuLimitMillis", "cpu_limit_millis").unwrap_or(0),
                ..Default::default()
            });
            body.as_object_mut().map(|b| b.remove("memory_limit_mb"));
            body["memoryLimitMb"] = json!(memory.unwrap_or(requirements::DEFAULT_MB));
            if body
                .get("sourceKind")
                .or_else(|| body.get("source_kind"))
                .and_then(Value::as_str)
                != Some("docker_image")
                && body.get("image").is_none()
            {
                requests.push(CapacityRequest {
                    feature: Feature::Build,
                    ..Default::default()
                });
            }
        }
        ["templates", id, "instances"] => {
            if let Some(t) = opsbox_deploy::templates::get_template(id) {
                images.push(format!(
                    "{}:{}",
                    t.image,
                    body.get("version")
                        .and_then(Value::as_str)
                        .unwrap_or(&t.versions[0])
                ));
                let r = template_request(t, memory);
                body.as_object_mut().map(|b| b.remove("memory_limit_mb"));
                body["memoryLimitMb"] = json!(r.memory_mb);
                requests.push(r);
            }
        }
        ["services", id, ..] | ["databases", id, ..] => {
            if let Some(svc) = state
                .store
                .get_service(id)
                .map_err(|_| HttpError::internal())?
            {
                // Preserve scope/ownership refusals before revealing host numbers.
                if !(if p.get(2) == Some(&"deploys") {
                    crate::deploy::service_deployable(&who, &svc)
                } else {
                    crate::deploy::service_writable(&who, &svc)
                }) {
                    return Err(HttpError::new(
                        ErrorCode::Forbidden,
                        "You cannot modify this service.",
                    ));
                }
                target = svc.name.clone();
                let t = svc
                    .template_id
                    .as_deref()
                    .and_then(opsbox_deploy::templates::get_template);
                let mut r = t.map_or_else(
                    || CapacityRequest {
                        memory_mb: memory.unwrap_or(
                            svc.memory_limit_mb
                                .unwrap_or(requirements::DEFAULT_MB as i64)
                                .max(64) as u64,
                        ),
                        ..Default::default()
                    },
                    |t| {
                        template_request(
                            t,
                            memory.or(svc.memory_limit_mb.map(|m| m.max(64) as u64)),
                        )
                    },
                );
                if p.get(2) == Some(&"pooler") {
                    if body.get("enabled") == Some(&json!(true)) {
                        r = CapacityRequest {
                            feature: Feature::Database,
                            memory_mb: 128,
                            min_memory_mb: 128,
                            ..Default::default()
                        };
                        r.feature = Feature::Template;
                        r.database = true;
                        images.push("edoburu/pgbouncer:v1.26.1-p0".to_owned());
                        pending = Some((
                            format!("database.pooler:{}", svc.id),
                            opsbox_capacity::Reservation {
                                name: format!("{}-pooler", svc.name),
                                memory_mb: requirements::POOLER_MB,
                                cpu_millis: 0,
                                database: true,
                            },
                        ));
                        requests.push(r);
                    }
                } else if p.get(2) == Some(&"extensions") {
                    if body.get("enabled") == Some(&json!(true))
                        && matches!(
                            p.get(3),
                            Some(&"vector" | &"postgis" | &"pg_stat_statements")
                        )
                        && state
                            .store
                            .get_setting(&format!("capacity.extension.{}.{}", svc.id, p[3]))
                            .map_err(|_| HttpError::internal())?
                            .is_none()
                    {
                        requests.push(CapacityRequest {
                            feature: Feature::Database,
                            memory_mb: r.memory_mb.saturating_add(requirements::EXTENSION_MB),
                            replacing: Some(svc.name.clone()),
                            ..r
                        });
                    }
                } else if method == axum::http::Method::PATCH {
                    if memory.is_some() {
                        r.replacing = Some(svc.name.clone());
                        requests.push(r);
                    }
                } else if let Some(stack_id) = &svc.compose_stack_id {
                    if let Some(stack) = state
                        .store
                        .get_compose_stack(stack_id)
                        .map_err(|_| HttpError::internal())?
                    {
                        let doc = opsbox_deploy::compose::parse_compose_yaml(&stack.compose_yaml)
                            .map_err(|e| {
                            HttpError::new(ErrorCode::Validation, e.to_string())
                        })?;
                        let (memory_mb, cpu_millis, build) =
                            opsbox_deploy::compose::capacity_requirements(&doc).map_err(|e| {
                                HttpError::new(ErrorCode::Validation, e.to_string())
                            })?;
                        requests.push(CapacityRequest {
                            memory_mb,
                            cpu_millis,
                            ..Default::default()
                        });
                        if build {
                            requests.push(CapacityRequest {
                                feature: Feature::Build,
                                ..Default::default()
                            });
                        }
                    }
                } else {
                    if let Some(image) = &svc.image_ref {
                        images.push(image.clone());
                    }
                    requests.push(r);
                    if svc.kind == opsbox_store::ServiceKind::App
                        && svc.source_kind != opsbox_store::SourceKind::DockerImage
                    {
                        requests.push(CapacityRequest {
                            feature: Feature::Build,
                            ..Default::default()
                        });
                    }
                }
            }
        }
        ["compose"] => {
            if let Some(yaml) = body.get("yaml").and_then(Value::as_str) {
                let doc = opsbox_deploy::compose::parse_compose_yaml(yaml)
                    .map_err(|e| HttpError::new(ErrorCode::Validation, e.to_string()))?;
                let (mb, cpu, build) = opsbox_deploy::compose::capacity_requirements(&doc)
                    .map_err(|e| HttpError::new(ErrorCode::Validation, e.to_string()))?;
                requests.push(CapacityRequest {
                    memory_mb: mb,
                    cpu_millis: cpu,
                    ..Default::default()
                });
                if build {
                    requests.push(CapacityRequest {
                        feature: Feature::Build,
                        ..Default::default()
                    });
                }
            }
        }
        ["docker", "containers", id, "start" | "restart"] => {
            let inspected = state
                .priv_client
                .call(
                    rid,
                    Some(&who.user.name),
                    PrivOp::DockerInspect {
                        id: (*id).to_owned(),
                    },
                )
                .await?;
            let opsbox_proto::PrivReply::DockerInspect(i) = inspected else {
                return Err(HttpError::internal());
            };
            target = i.service.clone().unwrap_or(i.name.clone());
            requests.push(CapacityRequest {
                memory_mb: i.memory_limit_mb.unwrap_or(requirements::DEFAULT_MB),
                replacing: i.running.then(|| target.clone()),
                ..Default::default()
            });
        }
        ["backups", "snapshots", id, "restore"] => {
            requests.push(CapacityRequest {
                feature: Feature::Backup,
                ..Default::default()
            });
            if body.get("mode").and_then(Value::as_str).unwrap_or("copy") == "copy"
                && state
                    .store
                    .get_backup_snapshot_by_any_id(id)
                    .map_err(|_| HttpError::internal())?
                    .is_some_and(|s| s.kind == opsbox_proto::BackupTargetKind::Database)
            {
                requests.push(CapacityRequest {
                    feature: Feature::Database,
                    memory_mb: 256,
                    ..Default::default()
                });
            }
        }
        ["backups", ..] => requests.push(CapacityRequest {
            feature: Feature::Backup,
            ..Default::default()
        }),
        ["docker", "install"] => requests.push(CapacityRequest {
            feature: Feature::Docker,
            ..Default::default()
        }),
        _ => {}
    }
    if requests.is_empty() {
        return Ok(next
            .run(Request::from_parts(
                parts,
                axum::body::Body::from(
                    serde_json::to_vec(&body).map_err(|_| HttpError::internal())?,
                ),
            ))
            .await);
    }
    for image in images {
        let reply = state
            .priv_client
            .call(
                rid,
                Some(&who.user.name),
                PrivOp::CapacityImageSize { image },
            )
            .await?;
        let opsbox_proto::PrivReply::CapacityImageSize { disk_mb } = reply else {
            return Err(HttpError::internal());
        };
        requests.push(CapacityRequest {
            feature: Feature::ImagePull,
            disk_mb,
            ..Default::default()
        });
    }
    let s = snapshot(&state, rid).await?;
    let mut blocked = None;
    for r in &requests {
        let fit = s.admit(r);
        if fit.blocked() {
            if !overriding
                || (!s.profile.cgroup_v2
                    && !matches!(
                        r.feature,
                        Feature::Docker | Feature::Install | Feature::Backup | Feature::ImagePull
                    ))
            {
                return Err(conflict(&s, r, &fit));
            }
            blocked = Some((r.clone(), fit));
        }
    }
    if let Some((kind, reservation)) = &pending {
        state.capacity_pending.lock().unwrap().insert(
            kind.clone(),
            Pending {
                reservation: reservation.clone(),
                armed: false,
            },
        );
    }
    let approval_id = rid.to_owned();
    state
        .capacity_approvals
        .lock()
        .unwrap()
        .insert(approval_id.clone(), overriding);
    let _approval = Approval {
        state: state.clone(),
        rid: approval_id,
        pending_kind: pending.as_ref().map(|(kind, _)| kind.clone()),
    };
    let resp = next
        .run(Request::from_parts(
            parts,
            axum::body::Body::from(serde_json::to_vec(&body).map_err(|_| HttpError::internal())?),
        ))
        .await;
    if let Some((kind, _)) = pending {
        let mut items = state.capacity_pending.lock().unwrap();
        if resp.status().is_success() {
            if let Some(p) = items.get_mut(&kind) {
                p.armed = true;
            }
        } else {
            items.remove(&kind);
        }
    }
    if resp.status().is_success()
        && let Some((r, fit)) = blocked
    {
        state
            .store
            .set_setting(
                &format!("capacity.override.{target}"),
                &json!({"request":r,"at":crate::deploy::now_secs()}),
            )
            .map_err(|_| HttpError::internal())?;
        crate::deploy::audit_user(
            &state,
            &who.user.name,
            false,
            None,
            "capacity.override",
            Some(&target),
            json!({"request":r,"fit":fit}),
        );
    }
    Ok(resp)
}

struct Approval {
    state: AppState,
    rid: String,
    pending_kind: Option<String>,
}
impl Drop for Approval {
    fn drop(&mut self) {
        if let Some(kind) = &self.pending_kind {
            let mut pending = self.state.capacity_pending.lock().unwrap();
            if pending.get(kind).is_some_and(|p| !p.armed) {
                pending.remove(kind);
            }
        }
        self.state
            .capacity_approvals
            .lock()
            .unwrap()
            .remove(&self.rid);
    }
}

pub(crate) async fn admit_service(
    state: &AppState,
    rid: &str,
    svc: &opsbox_store::ServiceRecord,
    own_pending: bool,
) -> Result<(), HttpError> {
    let mut s = snapshot(state, rid).await?;
    if own_pending {
        s.reservations
            .retain(|r| r.name != format!("{} (deploy)", svc.name));
    }
    let t = svc
        .template_id
        .as_deref()
        .and_then(opsbox_deploy::templates::get_template);
    let r = t.map_or_else(
        || CapacityRequest {
            memory_mb: svc
                .memory_limit_mb
                .unwrap_or(requirements::DEFAULT_MB as i64)
                .max(64) as u64,
            ..Default::default()
        },
        |t| template_request(t, svc.memory_limit_mb.map(|m| m.max(64) as u64)),
    );
    let fit = s.admit(&r);
    if fit.blocked() {
        return Err(conflict(&s, &r, &fit));
    }
    if svc.kind == opsbox_store::ServiceKind::App
        && svc.source_kind != opsbox_store::SourceKind::DockerImage
    {
        let r = CapacityRequest {
            feature: Feature::Build,
            ..Default::default()
        };
        let fit = s.admit(&r);
        if fit.blocked() {
            return Err(conflict(&s, &r, &fit));
        }
    }
    Ok(())
}

pub(crate) async fn wait_backup<F, Fut>(
    scheduled: bool,
    mut sample: F,
) -> Result<Option<Fit>, HttpError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Snapshot, HttpError>>,
{
    let delayed = tokio::time::Instant::now();
    loop {
        let s = sample().await?;
        let request = CapacityRequest {
            feature: Feature::Backup,
            ..Default::default()
        };
        let fit = s.admit(&request);
        if !fit.blocked() {
            return Ok(None);
        }
        if !scheduled {
            return Err(conflict(&s, &request, &fit));
        }
        if delayed.elapsed().as_secs() >= requirements::BACKUP_DELAY_SECONDS {
            return Ok(Some(fit));
        }
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn profile(blocked: bool) -> Snapshot {
        Snapshot {
            profile: opsbox_capacity::Profile {
                memory_mb: 4096,
                available_mb: if blocked { 100 } else { 4096 },
                cpu_millis: 1000,
                cgroup_v2: true,
                ..Default::default()
            },
            reservations: vec![],
        }
    }
    #[tokio::test(start_paused = true)]
    async fn scheduled_backup_retries_then_runs_when_capacity_recovers() {
        let mut calls = 0;
        let began = tokio::time::Instant::now();
        let result = wait_backup(true, || {
            calls += 1;
            std::future::ready(Ok(profile(calls < 3)))
        })
        .await
        .unwrap();
        assert_eq!(result, None);
        assert_eq!(calls, 3);
        assert_eq!(began.elapsed().as_secs(), 60);
    }
    #[tokio::test(start_paused = true)]
    async fn scheduled_backup_falls_back_after_exactly_thirty_minutes() {
        let mut calls = 0;
        let began = tokio::time::Instant::now();
        assert!(
            wait_backup(true, || {
                calls += 1;
                std::future::ready(Ok(profile(true)))
            })
            .await
            .unwrap()
            .is_some()
        );
        assert_eq!(began.elapsed().as_secs(), 1800);
        assert_eq!(calls, 61);
        assert!(
            wait_backup(false, || std::future::ready(Ok(profile(true))))
                .await
                .is_err()
        );
    }
}
