//! Service creation, deploy pipeline orchestration (BACKEND_SPEC 7.5 steps 3-10),
//! build log streaming, environment variables, rollback, and webhook auto-deploys.
//!
//! `serve` never accesses `/var/run/docker.sock`, runs `git`/`docker`, or modifies
//! `/var/lib/opsbox/apps` directly: every system/Docker/filesystem step executes in
//! `opsbox-priv` over the typed `PrivOp` protocol.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use opsbox_auth::Principal;
use opsbox_deploy::refs::{VarRef, canonical_ref_expr, extract_var_refs, parse_pure_var_ref};
use opsbox_deploy::templates::{get_template, is_postgres_template};
use opsbox_deploy::{looks_secret_var_key, normalize_repo_source, validate_public_repo_url};
use opsbox_proto::{
    DeployBuildFrame, DeployBuildRequest, DeployEnvSpec, DeployGitSource, DeployPullRequest,
    DeployRefSource, DeployStartRequest, ErrorCode, Role, Secret, TemplateStartRequest,
    valid_commit_sha, valid_docker_image_ref, valid_domain_name, valid_env_key, valid_git_branch,
    valid_health_path, valid_rel_repo_path, valid_service_name,
};
use opsbox_store::{
    ActorKind, AuditResult, DeployRecord, DeployStatus, DeployTrigger, NewAudit, NewService,
    ServiceKind, ServiceRecord, SourceKind, UpdateService,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tracing::warn;

use crate::json::ApiJson;
use crate::session::{Client, Deployer, Viewer, deny_token, origin_matches_host};
use crate::{AppState, HttpError, RequestId};

const KEEP_RETAINED_IMAGES: usize = 5;
const MAX_DEPLOY_LOG_HISTORY: usize = 2000;
const DEPLOY_GRAPH_STEP_X: i64 = 290;
const DEPLOY_GRAPH_COLS: usize = 3;
const DEPLOY_GRAPH_STEP_Y: i64 = 140;

#[derive(Debug, Clone)]
pub struct DeploySettings {
    pub drain_secs: u64,
    pub build_timeout_secs: u64,
    pub health_retries: u8,
    pub allow_test_git: bool,
}

impl Default for DeploySettings {
    fn default() -> Self {
        Self {
            drain_secs: 10,
            build_timeout_secs: 900,
            health_retries: 12,
            allow_test_git: false,
        }
    }
}

#[derive(Clone, Default)]
pub struct DeployHub {
    pub(crate) inner: Arc<DeployHubInner>,
}

#[derive(Default)]
pub(crate) struct DeployHubInner {
    pub(crate) active_service_tasks: Mutex<HashMap<String, (String, u64)>>,
    pub(crate) next_nonce: AtomicU64,
    pub(crate) streams: Mutex<HashMap<String, Arc<DeployStream>>>,
}

pub(crate) struct DeployStream {
    pub(crate) sender: broadcast::Sender<Value>,
    pub(crate) history: Mutex<Vec<Value>>,
    pub(crate) finished: Mutex<bool>,
}

pub(crate) struct ActiveDeployGuard {
    pub(crate) hub: DeployHub,
    pub(crate) service_id: String,
    pub(crate) nonce: u64,
}

impl Drop for ActiveDeployGuard {
    fn drop(&mut self) {
        let mut active = self.hub.inner.active_service_tasks.lock().unwrap();
        if active
            .get(&self.service_id)
            .is_some_and(|(_, n)| *n == self.nonce)
        {
            active.remove(&self.service_id);
        }
    }
}

impl DeployHub {
    pub(crate) fn try_acquire(
        &self,
        service_id: &str,
        task_id: &str,
    ) -> Result<ActiveDeployGuard, String> {
        let mut active = self.inner.active_service_tasks.lock().unwrap();
        if let Some((running, _)) = active.get(service_id) {
            return Err(running.clone());
        }
        let nonce = self.inner.next_nonce.fetch_add(1, Ordering::Relaxed) + 1;
        active.insert(service_id.to_owned(), (task_id.to_owned(), nonce));
        Ok(ActiveDeployGuard {
            hub: self.clone(),
            service_id: service_id.to_owned(),
            nonce,
        })
    }

    pub(crate) fn set_task_id(&self, service_id: &str, nonce: u64, real_task_id: String) {
        let mut active = self.inner.active_service_tasks.lock().unwrap();
        if let Some((tid, cur_nonce)) = active.get_mut(service_id)
            && *cur_nonce == nonce
        {
            *tid = real_task_id;
        }
    }

    pub(crate) fn register_stream(&self, deploy_id: &str) -> Arc<DeployStream> {
        let mut streams = self.inner.streams.lock().unwrap();
        // Keep active streams and recent finished ones bounded so memory stays tiny.
        if streams.len() > 64 {
            let done_keys: Vec<String> = streams
                .iter()
                .filter(|(_, s)| *s.finished.lock().unwrap())
                .map(|(k, _)| k.clone())
                .collect();
            for k in done_keys {
                streams.remove(&k);
            }
        }
        let (sender, _) = broadcast::channel(512);
        let stream = Arc::new(DeployStream {
            sender,
            history: Mutex::new(Vec::new()),
            finished: Mutex::new(false),
        });
        streams.insert(deploy_id.to_owned(), Arc::clone(&stream));
        stream
    }

    fn subscribe(&self, deploy_id: &str) -> Option<(Vec<Value>, bool, broadcast::Receiver<Value>)> {
        let streams = self.inner.streams.lock().unwrap();
        let stream = streams.get(deploy_id)?;
        let history = stream.history.lock().unwrap().clone();
        let finished = *stream.finished.lock().unwrap();
        let rx = stream.sender.subscribe();
        Some((history, finished, rx))
    }

    pub(crate) fn emit(
        &self,
        store: &opsbox_store::Store,
        stream: &DeployStream,
        deploy_id: &str,
        line: &str,
    ) {
        let ts = now_secs();
        let _ = store.append_deploy_log(deploy_id, line, ts);
        let msg = json!({ "type": "log", "ts": ts, "line": line });
        {
            let mut h = stream.history.lock().unwrap();
            if h.len() < MAX_DEPLOY_LOG_HISTORY {
                h.push(msg.clone());
            }
        }
        let _ = stream.sender.send(msg);
    }

    pub(crate) fn finish(&self, stream: &DeployStream, status: &str, error: Option<&str>) {
        let msg = json!({ "type": "done", "status": status, "error": error });
        *stream.finished.lock().unwrap() = true;
        {
            let mut h = stream.history.lock().unwrap();
            if h.len() < MAX_DEPLOY_LOG_HISTORY {
                h.push(msg.clone());
            }
        }
        let _ = stream.sender.send(msg);
    }
}

pub(crate) fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

pub(crate) fn next_graph_pos(idx: usize) -> (i64, i64) {
    let col = i64::try_from(idx % DEPLOY_GRAPH_COLS).unwrap_or(0);
    let row = i64::try_from(idx / DEPLOY_GRAPH_COLS).unwrap_or(0);
    (col * DEPLOY_GRAPH_STEP_X, row * DEPLOY_GRAPH_STEP_Y + 20)
}

pub(crate) fn resolve_owner_id(
    state: &AppState,
    who: &Principal,
    requested_owner: Option<&str>,
) -> Result<Option<String>, HttpError> {
    if who.user.role == Role::Deployer {
        return Ok(Some(who.user.id.clone()));
    }
    let Some(owner_ref) = requested_owner.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let u = state
        .store
        .user_by_name(owner_ref)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| invalid("owner", "That user account does not exist."))?;
    Ok(Some(u.id))
}

fn has_ref_path(state: &AppState, start_id: &str, goal_id: &str) -> Result<bool, HttpError> {
    let mut visited = HashSet::new();
    let mut stack = vec![start_id.to_owned()];
    while let Some(cur) = stack.pop() {
        if cur == goal_id {
            return Ok(true);
        }
        if !visited.insert(cur.clone()) {
            continue;
        }
        let refs = state
            .store
            .list_service_var_refs(&cur)
            .map_err(|_| HttpError::internal())?;
        for r in refs {
            stack.push(r.target_service_id);
        }
    }
    Ok(false)
}

pub(crate) type ParsedVarRefs = (Option<String>, Vec<(String, String)>);

#[allow(clippy::too_many_arguments)]
pub(crate) fn parse_and_validate_var_refs(
    state: &AppState,
    project_id: &str,
    env_id: &str,
    self_service_name: Option<&str>,
    self_service_id: Option<&str>,
    field: &'static str,
    value: &str,
) -> Result<ParsedVarRefs, HttpError> {
    let refs = extract_var_refs(value).map_err(|e| invalid(field, e.to_string()))?;
    if refs.is_empty() {
        return Ok((None, Vec::new()));
    }

    let project_services = state
        .store
        .list_services(project_id, env_id)
        .map_err(|_| HttpError::internal())?;
    let by_name: HashMap<&str, &ServiceRecord> = project_services
        .iter()
        .map(|s| (s.name.as_str(), s))
        .collect();

    let mut targets: Vec<(String, String)> = Vec::new();
    let mut seen = HashSet::new();
    let mut vars_cache: HashMap<String, Vec<opsbox_store::ServiceVarRecord>> = HashMap::new();

    for r in refs {
        if self_service_name.is_some_and(|n| n == r.service) {
            return Err(invalid(
                field,
                "A service variable cannot reference the same service.",
            ));
        }
        let target = by_name.get(r.service.as_str()).copied().ok_or_else(|| {
            invalid(
                field,
                format!(
                    "Referenced service '{}' was not found in this project.",
                    r.service
                ),
            )
        })?;
        if let Some(svc_id) = self_service_id
            && has_ref_path(state, &target.id, svc_id)?
        {
            return Err(invalid(
                field,
                format!(
                    "Circular variable reference detected involving '{}'.",
                    target.name
                ),
            ));
        }
        let t_vars = match vars_cache.get(&target.id) {
            Some(v) => v,
            None => {
                let loaded = state
                    .store
                    .list_service_vars(&target.id)
                    .map_err(|_| HttpError::internal())?;
                vars_cache.insert(target.id.clone(), loaded);
                vars_cache.get(&target.id).expect("inserted")
            }
        };
        if !t_vars.iter().any(|v| v.key == r.key) {
            return Err(invalid(
                field,
                format!(
                    "Variable '{}' was not found on service '{}'.",
                    r.key, target.name
                ),
            ));
        }
        let pair = (target.id.clone(), r.key);
        if seen.insert(pair.clone()) {
            targets.push(pair);
        }
    }

    let ref_expr = parse_pure_var_ref(value).map(|r| canonical_ref_expr(&r));
    Ok((ref_expr, targets))
}

pub(crate) fn validate_vars_refs_before_create(
    state: &AppState,
    project_id: &str,
    env_id: &str,
    service_name: &str,
    vars: &[(String, String, bool)],
) -> Result<(), HttpError> {
    for (_, val, _) in vars {
        parse_and_validate_var_refs(
            state,
            project_id,
            env_id,
            Some(service_name),
            None,
            "vars",
            val,
        )?;
    }
    Ok(())
}

pub(crate) async fn seal_and_store_vars(
    state: &AppState,
    rid: &str,
    actor: &str,
    svc: &ServiceRecord,
    vars: &[(String, String, bool)],
    now: i64,
) -> Result<(), HttpError> {
    for (k, val, is_secret) in vars {
        let (ref_expr, targets) = parse_and_validate_var_refs(
            state,
            &svc.project_id,
            &svc.env_id,
            Some(&svc.name),
            Some(&svc.id),
            "vars",
            val,
        )?;
        let ctx = var_seal_context(&svc.id, k);
        let sealed = state
            .priv_client
            .secret_seal(rid, Some(actor), &ctx, Secret::new(val.clone()))
            .await?;
        state
            .store
            .upsert_service_var_with_refs(
                &svc.id,
                k,
                &sealed,
                *is_secret,
                ref_expr.as_deref(),
                &targets,
                now,
            )
            .map_err(|_| HttpError::internal())?;
    }
    Ok(())
}

pub(crate) fn invalid(field: &'static str, msg: impl Into<String>) -> HttpError {
    HttpError(
        opsbox_proto::ApiError::new(ErrorCode::Validation, msg.into()).with_field(field),
        None,
    )
}

pub(crate) fn forbidden(msg: &str) -> HttpError {
    HttpError::new(ErrorCode::Forbidden, msg)
}

pub(crate) fn var_seal_context(service_id: &str, key: &str) -> String {
    format!("deploy:var:{service_id}:{key}")
}

pub(crate) fn service_readable(who: &Principal, svc: &ServiceRecord) -> bool {
    if !who.allows_service(&svc.name) {
        return false;
    }
    if who.user.role == Role::Deployer && !who.via_token {
        return svc.owner_id.as_deref() == Some(who.user.id.as_str());
    }
    true
}

pub(crate) fn service_writable(who: &Principal, svc: &ServiceRecord) -> bool {
    if !who.allows_service(&svc.name) {
        return false;
    }
    if who.user.role == Role::Admin {
        return true;
    }
    if who.user.role == Role::Deployer {
        if who.via_token {
            return false;
        }
        return svc.owner_id.as_deref() == Some(who.user.id.as_str());
    }
    false
}

pub(crate) fn service_deployable(who: &Principal, svc: &ServiceRecord) -> bool {
    if !who.allows_service(&svc.name) {
        return false;
    }
    if who.user.role == Role::Admin {
        return true;
    }
    if who.user.role == Role::Deployer {
        if who.via_token {
            return true;
        }
        return svc.owner_id.as_deref() == Some(who.user.id.as_str());
    }
    false
}

pub(crate) fn audit_user(
    state: &AppState,
    actor: &str,
    via_token: bool,
    ip: Option<&str>,
    action: &str,
    target: Option<&str>,
    detail: Value,
) {
    let _ = state.store.append_audit(NewAudit {
        actor: actor.to_owned(),
        actor_kind: if via_token {
            ActorKind::Token
        } else {
            ActorKind::User
        },
        ip: ip.map(str::to_owned),
        action: action.to_owned(),
        target: target.map(str::to_owned),
        result: AuditResult::Ok,
        detail,
    });
}

pub(crate) async fn fetch_live_cert_domains(state: &AppState) -> HashSet<String> {
    let mut set = HashSet::new();
    let Some(ref proxy) = state.proxy else {
        return set;
    };
    if let Ok(st) = proxy
        .clone()
        .with_timeout(Duration::from_secs(1))
        .status("svc-certs")
        .await
    {
        for c in st.certs {
            if matches!(
                c.status,
                opsbox_proxy::CertState::Valid | opsbox_proxy::CertState::Expiring
            ) {
                set.insert(c.domain.to_ascii_lowercase());
            }
        }
    }
    set
}

pub(crate) async fn resolve_service_view(
    state: &AppState,
    rid: &str,
    who: &Principal,
    svc: &ServiceRecord,
    live_certs: &HashSet<String>,
) -> Result<Value, HttpError> {
    let override_key = format!("capacity.override.{}", svc.name);
    let mut over_capacity = state
        .store
        .get_setting(&override_key)
        .ok()
        .flatten()
        .is_some();
    if over_capacity
        && let Ok(s) = crate::capacity::snapshot(state, rid).await
        && !s.over_capacity()
    {
        let _ = state.store.delete_setting(&override_key);
        over_capacity = false;
    }
    let store = state.store.clone();
    let svc_id = svc.id.clone();
    let stack_id = svc.compose_stack_id.clone();
    let (vars, deploys, stack_name, backup, referencing) = tokio::task::spawn_blocking(move || {
        let stack_name = stack_id
            .as_deref()
            .and_then(|sid| store.get_compose_stack(sid).ok().flatten())
            .map(|st| st.name);
        let backup = store.get_backup_job_for_service(&svc_id).ok().flatten();
        let referencing = store.list_referencing_services(&svc_id).unwrap_or_default();
        (
            store.list_service_vars(&svc_id).unwrap_or_default(),
            store.list_deploys(&svc_id, 30).unwrap_or_default(),
            stack_name,
            backup,
            referencing,
        )
    })
    .await
    .map_err(|_| HttpError::internal())?;

    let mut rendered_vars = Vec::with_capacity(vars.len());
    for v in vars {
        // Per-app logins and the pooler login are managed on the Database page.
        if v.key.starts_with(opsbox_proto::INTERNAL_VAR_PREFIX) {
            continue;
        }
        if let Some(ref expr) = v.ref_expr {
            // Never open or resolve reference vars in `serve` — show reference syntax only.
            rendered_vars.push(json!([v.key, expr, v.is_secret, true]));
            continue;
        }
        let display_val = if v.is_secret {
            "••••••••".to_owned()
        } else {
            let ctx = var_seal_context(&svc.id, &v.key);
            match state
                .priv_client
                .secret_open(rid, Some(&who.user.name), &ctx, &v.value_enc)
                .await
            {
                Ok(sec) => sec.expose().to_owned(),
                Err(_) => "••••••••".to_owned(),
            }
        };
        rendered_vars.push(json!([v.key, display_val, v.is_secret]));
    }

    let connected_apps: Vec<Value> = referencing
        .into_iter()
        .filter(|r| service_readable(who, &r.service))
        .map(|r| {
            json!({
                "id": r.service.name,
                "name": r.service.name,
                "kind": r.service.kind.as_str(),
                "state": r.service.state.as_str(),
                "vars": r.var_keys,
                "needsRedeploy": r.needs_redeploy,
            })
        })
        .collect();

    let rendered_deploys: Vec<Value> = deploys
        .into_iter()
        .map(|d| {
            let secs = d
                .finished_at
                .map(|f| (f - d.started_at).max(0))
                .unwrap_or(0);
            json!({
                "id": d.id,
                "commit": d.commit_sha,
                "msg": d.message,
                "trigger": d.trigger_kind.as_str(),
                "status": d.status.as_str(),
                "error": d.error_message,
                "at": d.started_at.saturating_mul(1000),
                "secs": secs,
            })
        })
        .collect();

    let domain_scheme = svc
        .domain
        .as_deref()
        .filter(|d| live_certs.contains(&d.to_ascii_lowercase()))
        .map_or("http", |_| "https");

    let backup_json = backup.map(|b| {
        json!({
            "method": b.method,
            "cron": b.cron,
            "enabled": b.enabled,
            "retainDaily": b.retain_daily,
            "retainWeekly": b.retain_weekly,
        })
    });

    Ok(json!({
        "id": svc.name,
        "uid": svc.id,
        "name": svc.name,
        "kind": svc.kind.as_str(),
        "state": svc.state.as_str(),
        "sourceKind": svc.source_kind.as_str(),
        "repo": svc.repo,
        "branch": svc.branch,
        "build": svc.build_method,
        "dockerfilePath": svc.dockerfile_path,
        "startCmd": svc.start_cmd,
        "port": svc.port,
        "hostPort": svc.active_host_port,
        "healthPath": svc.health_path,
        "domain": svc.domain,
        "domainScheme": domain_scheme,
        "imageRef": svc.image_ref,
        "deployKeyId": svc.deploy_key_id,
        "autoDeploy": svc.auto_deploy,
        "memoryLimitMb": svc.memory_limit_mb,
        "overCapacity": over_capacity,
        "cpuLimitMillis": svc.cpu_limit_millis,
        "ownerId": svc.owner_id,
        "x": svc.graph_x,
        "y": svc.graph_y,
        "note": svc.note,
        "templateId": svc.template_id,
        "composeStack": stack_name,
        "internalOnly": svc.internal_only,
        "publicPort": svc.public_port,
        "networkMode": if svc.internal_only { "internal" } else if svc.public_port { "public" } else { "loopback" },
        "credentialsRotatedAt": svc.credentials_rotated_at.map(|ts| ts.saturating_mul(1000)),
        "connectedApps": connected_apps,
        "backup": backup_json,
        "vars": rendered_vars,
        "deploys": rendered_deploys,
    }))
}

/// `GET /api/v1/projects/{id}`
pub(crate) async fn get_project(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    let store = state.store.clone();
    let id_q = id.clone();
    let now = now_secs();
    let Some((proj, env)) = tokio::task::spawn_blocking(move || store.resolve_project(&id_q, now))
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())?
    else {
        return Err(HttpError::new(ErrorCode::NotFound, "Project not found."));
    };

    let store = state.store.clone();
    let pid = proj.id.clone();
    let eid = env.id.clone();
    let (services, compose_stacks, var_ref_edges) = tokio::task::spawn_blocking(move || {
        Ok::<_, opsbox_store::StoreError>((
            store.list_services(&pid, &eid)?,
            store.list_compose_stacks(&pid, &eid)?,
            store.list_project_var_ref_edges(&pid, &eid)?,
        ))
    })
    .await
    .map_err(|_| HttpError::internal())?
    .map_err(|_| HttpError::internal())?;

    let live_certs = fetch_live_cert_domains(&state).await;
    let mut out_services = Vec::new();
    let mut visible_names: HashSet<String> = HashSet::new();
    let mut compose_node_map: HashMap<(String, String), String> = HashMap::new();
    for svc in services {
        if service_readable(&who, &svc) {
            visible_names.insert(svc.name.clone());
            if let Some(ref stack_id) = svc.compose_stack_id {
                compose_node_map.insert((stack_id.clone(), svc.branch.clone()), svc.name.clone());
            }
            out_services.push(resolve_service_view(&state, &rid, &who, &svc, &live_certs).await?);
        }
    }

    let mut edges = Vec::new();
    let mut seen_edges: HashSet<(String, String)> = HashSet::new();
    for stack in compose_stacks {
        for (from_raw, to_raw) in stack.depends_edges {
            if let (Some(from_id), Some(to_id)) = (
                compose_node_map.get(&(stack.id.clone(), from_raw)),
                compose_node_map.get(&(stack.id.clone(), to_raw)),
            ) && visible_names.contains(from_id)
                && visible_names.contains(to_id)
                && seen_edges.insert((from_id.clone(), to_id.clone()))
            {
                edges.push(json!([from_id, to_id]));
            }
        }
    }
    for (from_id, to_id) in var_ref_edges {
        if visible_names.contains(&from_id)
            && visible_names.contains(&to_id)
            && seen_edges.insert((from_id.clone(), to_id.clone()))
        {
            edges.push(json!([from_id, to_id]));
        }
    }

    Ok(Json(json!({
        "id": proj.id,
        "name": proj.name,
        "env": env.name,
        "services": out_services,
        "edges": edges,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateServiceBody {
    project: Option<String>,
    name: String,
    #[serde(alias = "source_kind")]
    source_kind: Option<String>,
    repo: Option<String>,
    #[serde(alias = "image_ref", alias = "imageRef")]
    image: Option<String>,
    branch: Option<String>,
    #[serde(alias = "buildMethod", alias = "build_method")]
    build: Option<String>,
    #[serde(alias = "dockerfile_path")]
    dockerfile_path: Option<String>,
    #[serde(alias = "start_cmd")]
    start_cmd: Option<String>,
    port: Option<Value>,
    #[serde(alias = "health_path")]
    health_path: Option<String>,
    domain: Option<String>,
    #[serde(alias = "deploy_key_id")]
    deploy_key_id: Option<String>,
    #[serde(alias = "auto_deploy")]
    auto_deploy: Option<bool>,
    #[serde(alias = "memory_limit_mb")]
    memory_limit_mb: Option<i64>,
    #[serde(alias = "cpu_limit_millis")]
    cpu_limit_millis: Option<i64>,
    owner: Option<String>,
    vars: Option<Value>,
    #[serde(alias = "deploy_now")]
    deploy_now: Option<bool>,
}

fn parse_port(val: Option<&Value>, default_port: u16) -> Result<u16, HttpError> {
    let Some(v) = val else {
        return Ok(default_port);
    };
    let n = match v {
        Value::Number(num) => num.as_u64().and_then(|u| u16::try_from(u).ok()),
        Value::String(s) if s.trim().is_empty() => return Ok(default_port),
        Value::String(s) => s.trim().parse::<u16>().ok(),
        _ => None,
    };
    n.ok_or_else(|| invalid("port", "Port must be between 0 and 65535."))
}

pub(crate) fn parse_vars_input(
    raw: Option<&Value>,
) -> Result<Vec<(String, String, bool)>, HttpError> {
    let Some(v) = raw else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    match v {
        Value::Array(items) => {
            for item in items {
                if let Some(pair) = item.as_array()
                    && pair.len() >= 2
                {
                    let k = pair[0].as_str().unwrap_or("").trim().to_owned();
                    let val = pair[1].as_str().unwrap_or("").to_owned();
                    let sec = pair
                        .get(2)
                        .and_then(Value::as_bool)
                        .unwrap_or_else(|| looks_secret_var_key(&k));
                    if !valid_env_key(&k) {
                        return Err(invalid(
                            "vars",
                            format!(
                                "Invalid variable name '{k}'. Use letters, digits, and underscores."
                            ),
                        ));
                    }
                    out.push((k, val, sec));
                } else if let Some(obj) = item.as_object() {
                    let k = obj
                        .get("key")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .trim()
                        .to_owned();
                    let val = obj
                        .get("value")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    let sec = obj
                        .get("isSecret")
                        .and_then(Value::as_bool)
                        .unwrap_or_else(|| looks_secret_var_key(&k));
                    if !valid_env_key(&k) {
                        return Err(invalid(
                            "vars",
                            format!(
                                "Invalid variable name '{k}'. Use letters, digits, and underscores."
                            ),
                        ));
                    }
                    out.push((k, val, sec));
                } else {
                    return Err(invalid("vars", "Invalid environment variable entry."));
                }
            }
        }
        Value::Object(map) => {
            for (k, val_v) in map {
                let k = k.trim().to_owned();
                let val = val_v.as_str().unwrap_or("").to_owned();
                if !valid_env_key(&k) {
                    return Err(invalid(
                        "vars",
                        format!(
                            "Invalid variable name '{k}'. Use letters, digits, and underscores."
                        ),
                    ));
                }
                let sec = looks_secret_var_key(&k);
                out.push((k, val, sec));
            }
        }
        Value::Null => {}
        _ => return Err(invalid("vars", "Variables must be a list or object.")),
    }
    Ok(out)
}

fn validate_git_source_inputs(
    state: &AppState,
    source_kind: SourceKind,
    repo: &str,
    deploy_key_id: Option<&str>,
) -> Result<(String, Option<String>), HttpError> {
    match source_kind {
        SourceKind::DockerImage => {
            if !valid_docker_image_ref(repo) {
                return Err(invalid("image", "Enter a valid Docker image reference."));
            }
            Ok((repo.to_owned(), Some(repo.to_owned())))
        }
        SourceKind::Github => {
            let trimmed = repo.trim();
            if state.deploy.allow_test_git && trimmed.starts_with("file://") {
                return Ok((trimmed.to_owned(), None));
            }
            let norm =
                normalize_repo_source(trimmed).map_err(|e| invalid("repo", e.to_string()))?;
            Ok((norm.repo, None))
        }
        SourceKind::DeployKey => {
            let dk_id = deploy_key_id
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| invalid("deployKeyId", "Choose an SSH deploy key."))?;
            let dk = state
                .store
                .get_deploy_key(dk_id)
                .map_err(|_| HttpError::internal())?
                .ok_or_else(|| invalid("deployKeyId", "That deploy key was not found."))?;
            let target_repo = if repo.trim().is_empty() {
                dk.ssh_url
            } else {
                repo.trim().to_owned()
            };
            Ok((target_repo, None))
        }
        SourceKind::GitUrl => {
            let trimmed = repo.trim();
            if state.deploy.allow_test_git && trimmed.starts_with("file://") {
                return Ok((trimmed.to_owned(), None));
            }
            validate_public_repo_url(trimmed).map_err(|e| invalid("repo", e.to_string()))?;
            Ok((trimmed.to_owned(), None))
        }
    }
}

/// `POST /api/v1/services`: create a service, ensure its app directory, seal initial vars, and
/// trigger an immediate first deploy (unless `deployNow = false`).
pub(crate) async fn create_service(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<CreateServiceBody>,
) -> Result<Response, HttpError> {
    if who.via_token && who.user.role != Role::Admin {
        return Err(forbidden("Deploy tokens cannot create new services."));
    }

    let name = body.name.trim().to_ascii_lowercase();
    if !valid_service_name(&name) {
        return Err(invalid(
            "name",
            "Service name must be 1 to 63 lowercase letters, digits, or dashes.",
        ));
    }

    let image_opt = body
        .image
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let dk_opt = body
        .deploy_key_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let repo_raw = body.repo.as_deref().map(str::trim).unwrap_or("");

    let inferred_kind = match body.source_kind.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => SourceKind::parse(s).ok_or_else(|| {
            invalid(
                "sourceKind",
                "Source kind must be github, deploy_key, git_url, or docker_image.",
            )
        })?,
        _ if image_opt.is_some() => SourceKind::DockerImage,
        _ if dk_opt.is_some() => SourceKind::DeployKey,
        _ if repo_raw.contains("://") || repo_raw.starts_with("git@") => SourceKind::GitUrl,
        _ => SourceKind::Github,
    };

    let raw_source = match inferred_kind {
        SourceKind::DockerImage => image_opt.as_deref().unwrap_or(repo_raw),
        _ => repo_raw,
    };
    let (repo_stored, image_stored) =
        validate_git_source_inputs(&state, inferred_kind, raw_source, dk_opt.as_deref())?;

    let branch = body
        .branch
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("main");
    if inferred_kind != SourceKind::DockerImage && !valid_git_branch(branch) {
        return Err(invalid("branch", "That is not a valid git branch name."));
    }

    let build_method = body
        .build
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(if inferred_kind == SourceKind::DockerImage {
            "Docker image"
        } else {
            "auto"
        });
    if build_method.len() > 64 {
        return Err(invalid("build", "Build method name is too long."));
    }

    let dockerfile_path = body
        .dockerfile_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(p) = dockerfile_path
        && !valid_rel_repo_path(p)
    {
        return Err(invalid(
            "dockerfilePath",
            "Dockerfile path must stay inside the repository.",
        ));
    }

    let port = parse_port(body.port.as_ref(), 3000)?;
    let health_path = body
        .health_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("/");
    if !valid_health_path(health_path) {
        return Err(invalid(
            "healthPath",
            "Health check path must start with '/' (or be 'tcp').",
        ));
    }

    let domain = body
        .domain
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase);
    if let Some(ref dom) = domain
        && !valid_domain_name(dom)
    {
        return Err(invalid(
            "domain",
            "Enter a valid domain name (example.com).",
        ));
    }

    if let Some(mb) = body.memory_limit_mb
        && !(64..=1_048_576).contains(&mb)
    {
        return Err(invalid(
            "memoryLimitMb",
            "Memory limit must be at least 64 MB.",
        ));
    }
    if let Some(cpu) = body.cpu_limit_millis
        && !(100..=128_000).contains(&cpu)
    {
        return Err(invalid(
            "cpuLimitMillis",
            "CPU limit must be at least 100m.",
        ));
    }

    let initial_vars = parse_vars_input(body.vars.as_ref())?;
    let owner_id = resolve_owner_id(&state, &who, body.owner.as_deref())?;

    let project_key = body
        .project
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default")
        .to_owned();
    let now = now_secs();
    let (proj, env) = state
        .store
        .resolve_project(&project_key, now)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Project not found."))?;

    validate_vars_refs_before_create(&state, &proj.id, &env.id, &name, &initial_vars)?;

    if state
        .store
        .get_service(&name)
        .map_err(|_| HttpError::internal())?
        .is_some()
    {
        return Err(invalid(
            "name",
            format!("A service named '{name}' already exists."),
        ));
    }

    let existing = state
        .store
        .list_services(&proj.id, &env.id)
        .map_err(|_| HttpError::internal())?;
    if let Some(ref dom) = domain {
        if crate::panel::active_panel_domain(&state).is_some_and(|p| p.eq_ignore_ascii_case(dom)) {
            return Err(invalid(
                "domain",
                format!("Domain '{dom}' is already used as the panel domain."),
            ));
        }
        if state
            .store
            .get_service_domain(dom)
            .map_err(|_| HttpError::internal())?
            .is_some()
        {
            return Err(invalid(
                "domain",
                format!("Domain '{dom}' is already attached to another service."),
            ));
        }
    }

    let (graph_x, graph_y) = next_graph_pos(existing.len());

    // Ensure /var/lib/opsbox/apps/<svc> exists via opsbox-priv before inserting DB row.
    state
        .priv_client
        .deploy_ensure_dir(&rid, &who.user.name, &name)
        .await?;

    let svc = state
        .store
        .create_service(NewService {
            project_id: &proj.id,
            env_id: &env.id,
            name: &name,
            kind: ServiceKind::App,
            owner_id: owner_id.as_deref(),
            source_kind: inferred_kind,
            repo: &repo_stored,
            branch,
            build_method,
            dockerfile_path,
            start_cmd: body.start_cmd.as_deref(),
            port,
            health_path,
            domain: domain.as_deref(),
            image_ref: image_stored.as_deref(),
            deploy_key_id: dk_opt.as_deref(),
            auto_deploy: body.auto_deploy.unwrap_or(true),
            memory_limit_mb: body.memory_limit_mb,
            cpu_limit_millis: body.cpu_limit_millis,
            graph_x,
            graph_y,
            note: None,
            now,
        })
        .map_err(|_| HttpError::internal())?;

    seal_and_store_vars(&state, &rid, &who.user.name, &svc, &initial_vars, now).await?;

    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "service.create",
        Some(&svc.name),
        json!({ "sourceKind": svc.source_kind.as_str(), "repo": svc.repo }),
    );

    let (deploy_id, task_id) = if body.deploy_now.unwrap_or(true) {
        let (dep, tid) = enqueue_deploy(
            &state,
            &rid,
            &svc,
            &proj.name,
            DeployTrigger::Manual,
            &who.user.name,
            who.via_token,
            Some(&client.ip),
            None,
            Some("Initial deploy"),
        )
        .await?;
        (Some(dep.id), Some(tid))
    } else {
        (None, None)
    };

    let live_certs = fetch_live_cert_domains(&state).await;
    let view = resolve_service_view(&state, &rid, &who, &svc, &live_certs).await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": svc.name,
            "service": view,
            "deployId": deploy_id,
            "task": task_id,
        })),
    )
        .into_response())
}

/// `GET /api/v1/services/{id}`
pub(crate) async fn get_service(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    let svc = state
        .store
        .get_service(&id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_readable(&who, &svc) {
        return Err(forbidden("You do not have access to this service."));
    }
    let live_certs = fetch_live_cert_domains(&state).await;
    Ok(Json(
        resolve_service_view(&state, &rid, &who, &svc, &live_certs).await?,
    ))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateServiceBody {
    branch: Option<String>,
    #[serde(alias = "buildMethod", alias = "build_method")]
    build: Option<String>,
    #[serde(alias = "dockerfile_path")]
    dockerfile_path: Option<String>,
    port: Option<Value>,
    #[serde(alias = "health_path")]
    health_path: Option<String>,
    domain: Option<String>,
    #[serde(alias = "auto_deploy")]
    auto_deploy: Option<bool>,
    #[serde(alias = "memory_limit_mb")]
    memory_limit_mb: Option<i64>,
    #[serde(alias = "cpu_limit_millis")]
    cpu_limit_millis: Option<i64>,
    owner: Option<String>,
}

/// `PATCH /api/v1/services/{id}`
pub(crate) async fn update_service(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpdateServiceBody>,
) -> Result<Json<Value>, HttpError> {
    let svc = state
        .store
        .get_service(&id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_writable(&who, &svc) {
        return Err(forbidden("You cannot modify this service."));
    }

    if let Some(ref b) = body.branch
        && !valid_git_branch(b.trim())
    {
        return Err(invalid("branch", "Invalid git branch name."));
    }
    let port = if body.port.is_some() {
        Some(parse_port(body.port.as_ref(), svc.port)?)
    } else {
        None
    };
    if let Some(ref hp) = body.health_path
        && !valid_health_path(hp.trim())
    {
        return Err(invalid(
            "healthPath",
            "Health check path must start with '/' (or be 'tcp').",
        ));
    }
    let old_domains: Vec<String> = if body.domain.is_some() {
        state
            .store
            .list_service_domains(&svc.id)
            .map_err(|_| HttpError::internal())?
            .into_iter()
            .map(|d| d.domain)
            .collect()
    } else {
        Vec::new()
    };
    let domain_upd = match body.domain {
        Some(ref d) if d.trim().is_empty() => Some(None),
        Some(ref d) => {
            if svc.kind == ServiceKind::Db || svc.internal_only {
                return Err(invalid(
                    "domain",
                    "Internal or database services cannot have public domains.",
                ));
            }
            let clean = d.trim().to_ascii_lowercase();
            if !valid_domain_name(&clean) {
                return Err(invalid("domain", "Enter a valid domain name."));
            }
            if crate::panel::active_panel_domain(&state)
                .is_some_and(|p| p.eq_ignore_ascii_case(&clean))
            {
                return Err(invalid(
                    "domain",
                    format!("Domain '{clean}' is already used as the panel domain."),
                ));
            }
            if let Some(other) = state
                .store
                .get_service_domain(&clean)
                .map_err(|_| HttpError::internal())?
                && other.service_id != svc.id
            {
                return Err(invalid(
                    "domain",
                    format!("Domain '{clean}' is already attached to another service."),
                ));
            }
            Some(Some(clean))
        }
        None => None,
    };

    let owner_upd: Option<Option<String>> = if let Some(ref owner_raw) = body.owner {
        if who.user.role != Role::Admin {
            return Err(forbidden(
                "Only administrators can reassign service ownership.",
            ));
        }
        let clean = owner_raw.trim();
        if clean.is_empty() {
            Some(None)
        } else {
            let u = state
                .store
                .user_by_name(clean)
                .map_err(|_| HttpError::internal())?
                .ok_or_else(|| invalid("owner", "That user account does not exist."))?;
            Some(Some(u.id))
        }
    } else {
        None
    };

    if let Some(mb) = body.memory_limit_mb {
        if !(64..=1_048_576).contains(&mb) {
            return Err(invalid(
                "memoryLimitMb",
                "Memory limit must be between 64 MB and 1 TB.",
            ));
        }
        if let Some(container) = &svc.active_container {
            state
                .priv_client
                .call(
                    &rid,
                    Some(&who.user.name),
                    opsbox_proto::PrivOp::WorkloadMemory {
                        container: container.clone(),
                        service: svc.name.clone(),
                        memory_mb: mb as u64,
                    },
                )
                .await?;
        }
    }
    let updated = state
        .store
        .update_service(
            &svc.id,
            UpdateService {
                branch: body.branch.as_deref().map(str::trim),
                build_method: body.build.as_deref().map(str::trim),
                dockerfile_path: body
                    .dockerfile_path
                    .as_deref()
                    .map(|s| (!s.trim().is_empty()).then(|| s.trim())),
                port,
                health_path: body.health_path.as_deref().map(str::trim),
                domain: domain_upd.as_ref().map(|o| o.as_deref()),
                auto_deploy: body.auto_deploy,
                memory_limit_mb: body.memory_limit_mb.map(Some),
                cpu_limit_millis: body.cpu_limit_millis.map(Some),
                owner_id: owner_upd.as_ref().map(|o| o.as_deref()),
                now: now_secs(),
                ..Default::default()
            },
        )
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;

    if body.domain.is_some() {
        if let Some(ref proxy) = state.proxy {
            let new_domains: HashSet<String> = state
                .store
                .list_service_domains(&svc.id)
                .unwrap_or_default()
                .into_iter()
                .map(|d| d.domain.to_ascii_lowercase())
                .collect();
            for old in old_domains {
                if !new_domains.contains(&old.to_ascii_lowercase()) {
                    let _ = proxy.remove_cert(&rid, &old).await;
                }
            }
        }
        let _ = crate::panel::sync_proxy_routes(&state).await;
    }

    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "service.update",
        Some(&updated.name),
        json!({}),
    );

    let live_certs = fetch_live_cert_domains(&state).await;
    Ok(Json(
        resolve_service_view(&state, &rid, &who, &updated, &live_certs).await?,
    ))
}

/// `DELETE /api/v1/services/{id}`
pub(crate) async fn delete_service(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let svc = state
        .store
        .get_service(&id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_writable(&who, &svc) {
        return Err(forbidden("You cannot delete this service."));
    }
    if state
        .deploy_hub
        .inner
        .active_service_tasks
        .lock()
        .unwrap()
        .contains_key(&svc.id)
    {
        return Err(HttpError::new(
            ErrorCode::Conflict,
            "Wait for the running deploy to finish before deleting this service.",
        ));
    }

    let deploys = state
        .store
        .list_deploys(&svc.id, 100)
        .map_err(|_| HttpError::internal())?;
    let mut cids: Vec<String> = deploys
        .iter()
        .filter_map(|d| d.container_id.clone())
        .collect();
    if let Some(ref c) = svc.active_container {
        cids.push(c.clone());
    }
    let mut seen_cids = HashSet::new();
    cids.retain(|c| seen_cids.insert(c.clone()));

    let mut imgs: Vec<String> = deploys.iter().filter_map(|d| d.image_ref.clone()).collect();
    let mut seen_imgs = HashSet::new();
    imgs.retain(|i| seen_imgs.insert(i.clone()));

    let svc_domains = state
        .store
        .list_service_domains(&svc.id)
        .map_err(|_| HttpError::internal())?;

    if svc.kind == ServiceKind::Compose
        && let Some(ref stack_id) = svc.compose_stack_id
        && let Ok(Some(stack)) = state.store.get_compose_stack(stack_id)
    {
        let siblings = state
            .store
            .list_services_by_compose_stack(stack_id)
            .map_err(|_| HttpError::internal())?;
        if siblings.len() <= 1 {
            state
                .priv_client
                .compose_down(&rid, &who.user.name, &stack.name, true)
                .await?;
            state
                .store
                .delete_compose_stack(&stack.id)
                .map_err(|_| HttpError::internal())?;
        } else {
            state
                .priv_client
                .compose_remove_service(&rid, &who.user.name, &stack.name, &svc.branch)
                .await?;
            state
                .store
                .delete_service(&svc.id)
                .map_err(|_| HttpError::internal())?;
        }
    } else {
        state
            .priv_client
            .deploy_delete_service(&rid, &who.user.name, &svc.name, cids, imgs)
            .await?;

        state
            .store
            .delete_service(&svc.id)
            .map_err(|_| HttpError::internal())?;
    }
    if let Some(ref proxy) = state.proxy {
        for d in svc_domains {
            let _ = proxy.remove_cert(&rid, &d.domain).await;
        }
    }
    let _ = crate::panel::sync_proxy_routes(&state).await;
    state.metrics.notify_docker();

    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "service.delete",
        Some(&svc.name),
        json!({}),
    );

    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RedeployBody {
    commit: Option<String>,
    message: Option<String>,
}

/// `POST /api/v1/services/{id}/deploys`
pub(crate) async fn redeploy(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    body: Option<ApiJson<RedeployBody>>,
) -> Result<Response, HttpError> {
    let svc = state
        .store
        .get_service(&id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_deployable(&who, &svc) {
        return Err(forbidden(
            "You do not have permission to deploy this service.",
        ));
    }

    let opts = body.map(|b| b.0).unwrap_or_default();
    if let Some(ref sha) = opts.commit
        && !valid_commit_sha(sha.trim())
    {
        return Err(invalid("commit", "Invalid commit SHA."));
    }

    let trigger = if who.via_token {
        DeployTrigger::Token
    } else {
        DeployTrigger::Manual
    };
    let proj_name = state
        .store
        .resolve_project(&svc.project_id, now_secs())
        .ok()
        .flatten()
        .map_or_else(|| "default".to_owned(), |(p, _)| p.name);

    let (dep, task_id) = enqueue_deploy(
        &state,
        &rid,
        &svc,
        &proj_name,
        trigger,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        opts.commit.as_deref().map(str::trim),
        opts.message.as_deref().map(str::trim),
    )
    .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "id": dep.id, "task": task_id })),
    )
        .into_response())
}

/// `POST /api/v1/services/{id}/deploys/{deployId}/rollback`
pub(crate) async fn rollback(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path((id, target_deploy_id)): Path<(String, String)>,
) -> Result<Response, HttpError> {
    let svc = state
        .store
        .get_service(&id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_deployable(&who, &svc) {
        return Err(forbidden(
            "You do not have permission to roll back this service.",
        ));
    }

    let target = state
        .store
        .get_deploy(&target_deploy_id)
        .map_err(|_| HttpError::internal())?
        .filter(|d| d.service_id == svc.id)
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Deploy not found."))?;

    if target.status == DeployStatus::Building {
        return Err(invalid(
            "deployId",
            "Cannot roll back to a deploy that is still building.",
        ));
    }
    let Some(target_image) = target.image_ref.clone() else {
        return Err(invalid(
            "deployId",
            "That deploy has no retained Docker image to roll back to.",
        ));
    };

    if !state
        .priv_client
        .deploy_image_exists(&rid, &target_image)
        .await?
    {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "The image for that deploy is no longer available locally to roll back to.",
        ));
    }

    let proj_name = state
        .store
        .resolve_project(&svc.project_id, now_secs())
        .ok()
        .flatten()
        .map_or_else(|| "default".to_owned(), |(p, _)| p.name);

    let (dep, task_id) = enqueue_rollback(
        &state,
        &rid,
        &svc,
        &proj_name,
        &target,
        &target_image,
        &who.user.name,
        who.via_token,
        &client.ip,
    )
    .await?;

    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "ok": true, "id": dep.id, "task": task_id })),
    )
        .into_response())
}

#[derive(Deserialize)]
pub(crate) struct VarPutBody {
    value: String,
    #[serde(alias = "isSecret", default)]
    is_secret: Option<bool>,
}

/// `PUT /api/v1/services/{id}/vars/{key}`
pub(crate) async fn put_var(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path((id, key)): Path<(String, String)>,
    ApiJson(body): ApiJson<VarPutBody>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let key = key.trim().to_owned();
    if !valid_env_key(&key) {
        return Err(invalid(
            "key",
            "Variable name must start with a letter or '_' and contain only letters, digits, and '_'.",
        ));
    }
    let svc = state
        .store
        .get_service(&id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_writable(&who, &svc) {
        return Err(forbidden("You cannot modify variables for this service."));
    }

    let (ref_expr, targets) = parse_and_validate_var_refs(
        &state,
        &svc.project_id,
        &svc.env_id,
        Some(&svc.name),
        Some(&svc.id),
        "value",
        &body.value,
    )?;

    let is_secret = body.is_secret.unwrap_or_else(|| looks_secret_var_key(&key));
    let ctx = var_seal_context(&svc.id, &key);
    let sealed = state
        .priv_client
        .secret_seal(
            &rid,
            Some(&who.user.name),
            &ctx,
            Secret::new(body.value.clone()),
        )
        .await?;
    state
        .store
        .upsert_service_var_with_refs(
            &svc.id,
            &key,
            &sealed,
            is_secret,
            ref_expr.as_deref(),
            &targets,
            now_secs(),
        )
        .map_err(|_| HttpError::internal())?;

    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "service.var.set",
        Some(&svc.name),
        json!({ "key": key, "secret": is_secret, "isRef": ref_expr.is_some() }),
    );

    if let Some(ref expr) = ref_expr {
        return Ok(Json(json!({
            "ok": true,
            "key": key,
            "value": expr,
            "isSecret": is_secret,
            "isRef": true,
        })));
    }

    Ok(Json(json!({
        "ok": true,
        "key": key,
        "value": if is_secret { "••••••••" } else { &body.value },
        "isSecret": is_secret,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectDatabaseBody {
    #[serde(alias = "database_id", alias = "db", alias = "target")]
    database: String,
    #[serde(alias = "source_key", alias = "targetVar", alias = "target_var")]
    source_key: Option<String>,
    #[serde(alias = "env_var", alias = "envVar", alias = "key", alias = "asKey")]
    as_key: Option<String>,
    #[serde(default, alias = "include_all")]
    include_all: bool,
    /// PostgreSQL only: give the app its own database and login (default on).
    #[serde(default, alias = "per_app_user", alias = "ownLogin")]
    per_app_user: Option<bool>,
    /// Connect through the connection pooler (needs the pooler turned on).
    #[serde(default)]
    pooled: bool,
}

/// `POST /api/v1/services/{id}/databases`: insert `${{ db.KEY }}` reference variable(s)
/// linking an app/tool service to a database in the same project/environment.
pub(crate) async fn connect_database(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<ConnectDatabaseBody>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let svc = state
        .store
        .get_service(&id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_writable(&who, &svc) {
        return Err(forbidden("You cannot modify variables for this service."));
    }
    if svc.kind == ServiceKind::Db {
        return Err(invalid(
            "database",
            "Connect databases from an application or tool service.",
        ));
    }

    let db_query = body.database.trim();
    if db_query.is_empty() {
        return Err(invalid("database", "Select a database to connect."));
    }
    let db_svc = state
        .store
        .get_service(db_query)
        .map_err(|_| HttpError::internal())?
        .filter(|d| d.project_id == svc.project_id && d.env_id == svc.env_id)
        .ok_or_else(|| {
            HttpError::new(ErrorCode::NotFound, "Database not found in this project.")
        })?;
    if !service_readable(&who, &db_svc) {
        return Err(forbidden("You do not have access to this database."));
    }
    if db_svc.id == svc.id {
        return Err(invalid("database", "A service cannot connect to itself."));
    }
    if db_svc.kind != ServiceKind::Db {
        return Err(invalid("database", "Selected service is not a database."));
    }
    if has_ref_path(&state, &db_svc.id, &svc.id)? {
        return Err(invalid(
            "database",
            "Circular variable reference detected between services.",
        ));
    }

    // PostgreSQL: one database and one least-privilege login per app, unless the caller asks
    // for the shared administrator login or names a specific variable.
    let own_login = db_svc
        .template_id
        .as_deref()
        .is_some_and(is_postgres_template)
        && body.per_app_user.unwrap_or(true)
        && body.source_key.is_none()
        && body.as_key.is_none();
    let mut own_pairs: Option<Vec<(String, String)>> = None;
    if own_login {
        if body.pooled
            && !state
                .store
                .get_database_settings(&db_svc.id)
                .map_err(|_| HttpError::internal())?
                .pooler_enabled
        {
            return Err(HttpError::new(
                ErrorCode::Conflict,
                "Turn on the connection pooler first.",
            ));
        }
        let rec =
            crate::databases::ensure_app_login(&state, &rid, &who.user.name, &db_svc, &svc).await?;
        state
            .store
            .set_database_app_pooled(&db_svc.id, &svc.id, body.pooled, now_secs())
            .map_err(|_| HttpError::internal())?;
        own_pairs = Some(crate::databases::app_link_pairs(&rec, body.pooled));
    }

    let db_vars = state
        .store
        .list_service_vars(&db_svc.id)
        .map_err(|_| HttpError::internal())?;
    let var_map: HashMap<&str, &opsbox_store::ServiceVarRecord> =
        db_vars.iter().map(|v| (v.key.as_str(), v)).collect();

    let tmpl = db_svc.template_id.as_deref().and_then(get_template);
    let primary_default = tmpl
        .and_then(|t| t.connection.primary_var.as_deref())
        .filter(|k| var_map.contains_key(k))
        .or_else(|| {
            ["DATABASE_URL", "MYSQL_URL", "REDIS_URL", "MONGODB_URI"]
                .into_iter()
                .find(|k| var_map.contains_key(k))
        })
        .or_else(|| db_vars.first().map(|v| v.key.as_str()))
        .ok_or_else(|| {
            invalid(
                "database",
                "Database has no connection variables to reference.",
            )
        })?;

    let pairs: Vec<(String, String)> = if let Some(p) = own_pairs {
        p
    } else if body.source_key.is_some() || body.as_key.is_some() || !body.include_all {
        let src_key = body
            .source_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(primary_default)
            .to_owned();
        if !valid_env_key(&src_key) || !var_map.contains_key(src_key.as_str()) {
            return Err(invalid(
                "sourceKey",
                format!(
                    "Variable '{}' does not exist on database '{}'.",
                    src_key, db_svc.name
                ),
            ));
        }
        let as_key = body
            .as_key
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&src_key)
            .to_owned();
        if !valid_env_key(&as_key) {
            return Err(invalid(
                "asKey",
                "Variable name must start with a letter or '_' and contain only letters, digits, and '_'.",
            ));
        }
        vec![(as_key, src_key)]
    } else {
        let mut out = Vec::new();
        let mut seen_keys = HashSet::new();
        if let Some(t) = tmpl {
            for c in &t.connection.vars {
                if var_map.contains_key(c.key.as_str()) && seen_keys.insert(c.key.clone()) {
                    out.push((c.key.clone(), c.key.clone()));
                }
            }
        }
        if out.is_empty() {
            out.push((primary_default.to_owned(), primary_default.to_owned()));
        }
        out
    };

    let now = now_secs();
    let mut linked_vars = Vec::with_capacity(pairs.len());
    for (env_key, src_key) in pairs {
        let ref_expr = canonical_ref_expr(&VarRef {
            service: db_svc.name.clone(),
            key: src_key.clone(),
        });
        let is_secret = var_map
            .get(src_key.as_str())
            .map(|v| v.is_secret)
            .unwrap_or_else(|| looks_secret_var_key(&env_key));
        let ctx = var_seal_context(&svc.id, &env_key);
        let sealed = state
            .priv_client
            .secret_seal(
                &rid,
                Some(&who.user.name),
                &ctx,
                Secret::new(ref_expr.clone()),
            )
            .await?;
        state
            .store
            .upsert_service_var_with_refs(
                &svc.id,
                &env_key,
                &sealed,
                is_secret,
                Some(&ref_expr),
                &[(db_svc.id.clone(), src_key)],
                now,
            )
            .map_err(|_| HttpError::internal())?;
        linked_vars.push(json!([env_key, ref_expr, is_secret, true]));
    }

    let needs_redeploy = svc.active_deploy_id.is_some() || svc.active_container.is_some();
    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "service.database.connect",
        Some(&svc.name),
        json!({ "database": db_svc.name }),
    );

    Ok(Json(json!({
        "ok": true,
        "database": db_svc.name,
        "vars": linked_vars,
        "needsRedeploy": needs_redeploy,
    })))
}

/// `DELETE /api/v1/services/{id}/databases/{db}`: remove reference variables linking
/// an app/tool service to the named database.
pub(crate) async fn disconnect_database(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Path((id, db)): Path<(String, String)>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let svc = state
        .store
        .get_service(&id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_writable(&who, &svc) {
        return Err(forbidden("You cannot modify variables for this service."));
    }
    let db_svc = state
        .store
        .get_service(db.trim())
        .map_err(|_| HttpError::internal())?
        .filter(|d| d.project_id == svc.project_id && d.env_id == svc.env_id)
        .ok_or_else(|| {
            HttpError::new(ErrorCode::NotFound, "Database not found in this project.")
        })?;

    let removed_keys = state
        .store
        .delete_service_vars_referencing(&svc.id, &db_svc.id)
        .map_err(|_| HttpError::internal())?;
    if removed_keys.is_empty() {
        return Err(HttpError::new(
            ErrorCode::NotFound,
            "Service is not connected to this database.",
        ));
    }

    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "service.database.disconnect",
        Some(&svc.name),
        json!({ "database": db_svc.name, "removedKeys": removed_keys }),
    );
    Ok(Json(json!({
        "ok": true,
        "database": db_svc.name,
        "removedKeys": removed_keys,
    })))
}

/// `DELETE /api/v1/services/{id}/vars/{key}`
pub(crate) async fn delete_var(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Path((id, key)): Path<(String, String)>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let svc = state
        .store
        .get_service(&id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_writable(&who, &svc) {
        return Err(forbidden("You cannot modify variables for this service."));
    }
    let removed = state
        .store
        .delete_service_var(&svc.id, key.trim())
        .map_err(|_| HttpError::internal())?;
    if !removed {
        return Err(HttpError::new(ErrorCode::NotFound, "Variable not found."));
    }
    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "service.var.delete",
        Some(&svc.name),
        json!({ "key": key.trim() }),
    );
    Ok(Json(json!({ "ok": true })))
}

/// `GET /api/v1/deploys/{id}/log`
pub(crate) async fn deploy_log(
    State(state): State<AppState>,
    Viewer(who): Viewer,
    Path(deploy_id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    let dep = state
        .store
        .get_deploy(&deploy_id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Deploy not found."))?;
    let svc = state
        .store
        .get_service(&dep.service_id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_readable(&who, &svc) {
        return Err(forbidden("You do not have access to this deploy log."));
    }

    let rows = state
        .store
        .list_deploy_logs(&dep.id, MAX_DEPLOY_LOG_HISTORY)
        .map_err(|_| HttpError::internal())?;

    Ok(Json(json!({
        "id": dep.id,
        "service": svc.name,
        "status": dep.status.as_str(),
        "commit": dep.commit_sha,
        "message": dep.message,
        "error": dep.error_message,
        "lines": rows.into_iter().map(|r| json!({ "ts": r.ts, "line": r.line })).collect::<Vec<_>>(),
    })))
}

/// `WS /api/v1/stream/deploys/{id}`: stream buffered + live deploy build log lines until finished.
pub(crate) async fn stream_deploy_log(
    Viewer(who): Viewer,
    client: Client,
    State(state): State<AppState>,
    Path(deploy_id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<Response, HttpError> {
    if !who.via_token && !origin_matches_host(&client.headers) {
        return Err(forbidden(
            "This request came from another site and was refused.",
        ));
    }
    let dep = state
        .store
        .get_deploy(&deploy_id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Deploy not found."))?;
    let svc = state
        .store
        .get_service(&dep.service_id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_readable(&who, &svc) {
        return Err(forbidden("You do not have access to this deploy log."));
    }

    let slot = crate::session::stream_slot(&state)?;
    Ok(
        crate::session::bounded_socket(ws).on_upgrade(move |socket| async move {
            let _slot = slot;
            tokio::select! {
                () = crate::session::stream_ended(state.clone(), who, client.ip) => {},
                () = pump_deploy_ws(socket, state, dep.id) => {},
            }
        }),
    )
}

async fn pump_deploy_ws(socket: WebSocket, state: AppState, deploy_id: String) {
    let (mut tx, mut rx) = socket.split();

    let live_sub = state.deploy_hub.subscribe(&deploy_id);
    match live_sub {
        Some((history, finished, mut bcast)) => {
            for item in history {
                if tx.send(Message::text(item.to_string())).await.is_err() {
                    return;
                }
            }
            if finished {
                let _ = tx.send(Message::Close(None)).await;
                return;
            }
            loop {
                tokio::select! {
                    msg = bcast.recv() => match msg {
                        Ok(val) => {
                            let is_done = val.get("type").and_then(Value::as_str) == Some("done");
                            if tx.send(Message::text(val.to_string())).await.is_err() {
                                return;
                            }
                            if is_done {
                                let _ = tx.send(Message::Close(None)).await;
                                return;
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            let _ = tx.send(Message::text(json!({ "type": "resync" }).to_string())).await;
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            let _ = tx.send(Message::Close(None)).await;
                            return;
                        }
                    },
                    client_msg = rx.next() => {
                        match client_msg {
                            None | Some(Ok(Message::Close(_)) | Err(_)) => return,
                            Some(Ok(_)) => {}
                        }
                    }
                }
            }
        }
        None => {
            let rows = state
                .store
                .list_deploy_logs(&deploy_id, MAX_DEPLOY_LOG_HISTORY)
                .unwrap_or_default();
            for row in rows {
                let msg = json!({ "type": "log", "ts": row.ts, "line": row.line }).to_string();
                if tx.send(Message::text(msg)).await.is_err() {
                    return;
                }
            }
            if let Ok(Some(dep)) = state.store.get_deploy(&deploy_id) {
                let done = json!({
                    "type": "done",
                    "status": dep.status.as_str(),
                    "error": dep.error_message,
                })
                .to_string();
                let _ = tx.send(Message::text(done)).await;
            }
            let _ = tx
                .send(Message::Close(Some(CloseFrame {
                    code: 1000,
                    reason: "done".into(),
                })))
                .await;
        }
    }
}

/// Trigger auto-deploy for all services matching `(repo_full_name, branch)` on webhook `push`.
pub(crate) async fn trigger_webhook_deploys(
    state: &AppState,
    rid: &str,
    repo_full_name: &str,
    branch: &str,
    commit_sha: &str,
    commit_message: Option<&str>,
    pusher: Option<&str>,
) -> (usize, Vec<Value>) {
    let branch_owned = branch.to_owned();
    let store = state.store.clone();
    let candidates =
        tokio::task::spawn_blocking(move || store.list_auto_deploy_services(&branch_owned))
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();

    let mut queued = 0usize;
    let mut started = Vec::new();
    let actor = pusher.unwrap_or("github-webhook");

    for svc in candidates {
        let matches_repo = match svc.source_kind {
            SourceKind::Github => svc.repo.eq_ignore_ascii_case(repo_full_name),
            SourceKind::DeployKey | SourceKind::GitUrl => normalize_repo_source(&svc.repo)
                .is_ok_and(|n| n.repo.eq_ignore_ascii_case(repo_full_name)),
            SourceKind::DockerImage => false,
        };
        if !matches_repo {
            continue;
        }
        let proj_name = state
            .store
            .resolve_project(&svc.project_id, now_secs())
            .ok()
            .flatten()
            .map_or_else(|| "default".to_owned(), |(p, _)| p.name);

        if let Ok((dep, tid)) = enqueue_deploy(
            state,
            rid,
            &svc,
            &proj_name,
            DeployTrigger::Webhook,
            actor,
            false,
            None,
            Some(commit_sha),
            commit_message,
        )
        .await
        {
            queued += 1;
            started.push(json!({
                "service": svc.name,
                "deployId": dep.id,
                "task": tid,
            }));
        }
    }

    (queued, started)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn enqueue_deploy(
    state: &AppState,
    rid: &str,
    svc: &ServiceRecord,
    project_name: &str,
    trigger: DeployTrigger,
    actor: &str,
    via_token: bool,
    ip: Option<&str>,
    commit_sha: Option<&str>,
    message: Option<&str>,
) -> Result<(DeployRecord, String), HttpError> {
    let approval = state.capacity_approvals.lock().unwrap().get(rid).copied();
    let _capacity_lock = if approval.is_none() {
        Some(state.capacity_lock.lock().await)
    } else {
        None
    };
    if approval.is_none() {
        crate::capacity::admit_service(state, rid, svc, false).await?;
    }
    let capacity_override = approval == Some(true);
    let default_msg = match trigger {
        DeployTrigger::Manual => "Manual deploy".to_owned(),
        DeployTrigger::Webhook => "GitHub push".to_owned(),
        DeployTrigger::Token => "API token deploy".to_owned(),
        DeployTrigger::Rollback => "Rollback".to_owned(),
    };
    let msg = message
        .filter(|s| !s.is_empty())
        .unwrap_or(&default_msg)
        .chars()
        .take(200)
        .collect::<String>();

    let dep = state
        .store
        .create_deploy(&svc.id, commit_sha, &msg, trigger, actor, now_secs())
        .map_err(|_| HttpError::internal())?;
    let deploy_id = dep.id.clone();

    let state_bg = state.clone();
    let svc_bg = svc.clone();
    let proj_bg = project_name.to_owned();
    let rid_bg = rid.to_owned();
    let actor_bg = actor.to_owned();
    let ip_bg = ip.map(str::to_owned);
    let commit_bg = commit_sha.map(str::to_owned);

    let tid_slot = Arc::new(Mutex::new(None::<String>));
    let guard_attempt = Arc::clone(&tid_slot);
    // Reserve task ID synchronously before starting background task.
    let task_kind = format!("deploy:{}", svc.name);
    let preview_task_id = ulid::Ulid::generate().to_string();
    let active_guard = match state.deploy_hub.try_acquire(&svc.id, &preview_task_id) {
        Ok(g) => g,
        Err(running_task) => {
            let _ = state.store.fail_deploy(
                &svc.id,
                &dep.id,
                "Another deploy is already running for this service.",
                now_secs(),
            );
            return Err(HttpError::new(
                ErrorCode::Conflict,
                format!(
                    "A deploy is already running for {} (task {running_task}). Wait for it to finish.",
                    svc.name
                ),
            ));
        }
    };
    *guard_attempt.lock().unwrap() = Some(preview_task_id.clone());

    let guard_nonce = active_guard.nonce;
    let stream = state.deploy_hub.register_stream(&deploy_id);
    let task_id = state.tasks.start(&task_kind, async move {
        let guard = active_guard;
        let res = run_deploy_pipeline(
            state_bg,
            rid_bg,
            svc_bg,
            proj_bg,
            deploy_id,
            commit_bg,
            actor_bg,
            via_token,
            ip_bg,
            stream,
            capacity_override,
        )
        .await;
        drop(guard);
        res
    })?;

    state
        .deploy_hub
        .set_task_id(&svc.id, guard_nonce, task_id.clone());

    audit_user(
        state,
        actor,
        via_token,
        ip,
        "deploy.start",
        Some(&svc.name),
        json!({ "deployId": dep.id, "trigger": trigger.as_str() }),
    );

    Ok((dep, task_id))
}

#[allow(clippy::too_many_arguments)]
async fn enqueue_rollback(
    state: &AppState,
    rid: &str,
    svc: &ServiceRecord,
    project_name: &str,
    target_deploy: &DeployRecord,
    target_image: &str,
    actor: &str,
    via_token: bool,
    ip: &str,
) -> Result<(DeployRecord, String), HttpError> {
    let short = target_deploy
        .commit_sha
        .as_deref()
        .map(|s| &s[..s.len().min(7)])
        .unwrap_or(&target_deploy.id[..target_deploy.id.len().min(7)]);
    let msg = format!("Roll back to {short}");

    let dep = state
        .store
        .create_deploy(
            &svc.id,
            target_deploy.commit_sha.as_deref(),
            &msg,
            DeployTrigger::Rollback,
            actor,
            now_secs(),
        )
        .map_err(|_| HttpError::internal())?;
    let _ = state
        .store
        .update_deploy_metadata(&dep.id, None, None, Some(target_image), None);

    let preview_task_id = ulid::Ulid::generate().to_string();
    let active_guard = match state.deploy_hub.try_acquire(&svc.id, &preview_task_id) {
        Ok(g) => g,
        Err(running_task) => {
            let _ = state.store.fail_deploy(
                &svc.id,
                &dep.id,
                "Another deploy is already running for this service.",
                now_secs(),
            );
            return Err(HttpError::new(
                ErrorCode::Conflict,
                format!(
                    "A deploy is already running for {} (task {running_task}). Wait for it to finish.",
                    svc.name
                ),
            ));
        }
    };

    let state_bg = state.clone();
    let svc_bg = svc.clone();
    let proj_bg = project_name.to_owned();
    let rid_bg = rid.to_owned();
    let deploy_id = dep.id.clone();
    let img_bg = target_image.to_owned();
    let actor_bg = actor.to_owned();
    let ip_bg = ip.to_owned();
    let stream = state.deploy_hub.register_stream(&deploy_id);
    let task_kind = format!("rollback:{}", svc.name);

    let guard_nonce = active_guard.nonce;
    let task_id = state.tasks.start(&task_kind, async move {
        let guard = active_guard;
        let res = run_release_switch_flow(
            &state_bg,
            &rid_bg,
            &svc_bg,
            &proj_bg,
            &deploy_id,
            &img_bg,
            None,
            &actor_bg,
            via_token,
            Some(&ip_bg),
            &stream,
            true,
        )
        .await;
        drop(guard);
        res
    })?;

    state
        .deploy_hub
        .set_task_id(&svc.id, guard_nonce, task_id.clone());

    audit_user(
        state,
        actor,
        via_token,
        Some(ip),
        "deploy.rollback",
        Some(&svc.name),
        json!({ "deployId": dep.id, "targetDeployId": target_deploy.id }),
    );

    Ok((dep, task_id))
}

#[allow(clippy::too_many_arguments)]
async fn run_deploy_pipeline(
    state: AppState,
    rid: String,
    svc: ServiceRecord,
    project_name: String,
    deploy_id: String,
    commit_sha: Option<String>,
    actor: String,
    via_token: bool,
    ip: Option<String>,
    stream: Arc<DeployStream>,
    capacity_override: bool,
) -> Result<crate::tasks::Outcome, HttpError> {
    let short_id: String = deploy_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(12)
        .collect::<String>()
        .to_ascii_lowercase();
    let image_tag = format!("opsbox/{}:{short_id}", svc.name);

    if !capacity_override
        && let Err(e) = crate::capacity::admit_service(&state, &rid, &svc, true).await
    {
        return fail_and_report(
            &state,
            &svc,
            &deploy_id,
            &actor,
            via_token,
            ip.as_deref(),
            &stream,
            e.0.error.message,
        );
    }
    let vars_specs = service_env_specs(&state, &svc.id);
    let ref_sources = service_ref_sources(&state, &svc.id);
    let mut detected_method: Option<String> = None;

    if let Some(ref stack_id) = svc.compose_stack_id {
        return crate::templates::run_compose_service_redeploy(
            &state,
            &rid,
            &svc,
            stack_id,
            &project_name,
            &deploy_id,
            &actor,
            via_token,
            ip.as_deref(),
            &stream,
        )
        .await;
    }

    if let Some(ref template_id) = svc.template_id {
        let tmpl_image = svc.image_ref.clone().unwrap_or_else(|| svc.repo.clone());
        let tmpl_version = svc.branch.clone();
        let ref_seq = state.store.capture_service_ref_seq(&svc.id).ok();
        state.deploy_hub.emit(
            &state.store,
            &stream,
            &deploy_id,
            &format!("Starting {template_id} ({tmpl_image})..."),
        );
        let started = match state
            .priv_client
            .template_start(
                &rid,
                &actor,
                TemplateStartRequest {
                    project: project_name.clone(),
                    service: svc.name.clone(),
                    deploy_id: deploy_id.clone(),
                    template_id: template_id.clone(),
                    version: tmpl_version,
                    image_ref: tmpl_image.clone(),
                    internal_only: svc.internal_only,
                    public_port: svc.public_port,
                    confirm_public_port: svc.public_port,
                    vars: vars_specs,
                    ref_sources,
                    memory_limit_mb: svc.memory_limit_mb,
                    cpu_limit_millis: svc.cpu_limit_millis,
                    data_volume: state
                        .store
                        .get_database_settings(&svc.id)
                        .ok()
                        .and_then(|d| d.data_volume),
                },
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                return fail_and_report(
                    &state,
                    &svc,
                    &deploy_id,
                    &actor,
                    via_token,
                    ip.as_deref(),
                    &stream,
                    e.to_string(),
                );
            }
        };

        return finalize_release_switch(
            &state,
            &rid,
            &svc,
            &deploy_id,
            &tmpl_image,
            started,
            None,
            ref_seq,
            &actor,
            via_token,
            ip.as_deref(),
            &stream,
            false,
        )
        .await;
    }

    if svc.source_kind == SourceKind::DockerImage {
        let src_image = svc.image_ref.clone().unwrap_or_else(|| svc.repo.clone());
        state.deploy_hub.emit(
            &state.store,
            &stream,
            &deploy_id,
            &format!("Pulling image {src_image}..."),
        );
        if let Err(e) = state
            .priv_client
            .deploy_pull_image(
                &rid,
                &actor,
                DeployPullRequest {
                    source_image: src_image,
                    target_tag: image_tag.clone(),
                },
            )
            .await
        {
            return fail_and_report(
                &state,
                &svc,
                &deploy_id,
                &actor,
                via_token,
                ip.as_deref(),
                &stream,
                e.to_string(),
            );
        }
        let _ = state
            .store
            .update_deploy_metadata(&deploy_id, None, None, Some(&image_tag), None);
    } else {
        let cache_from = state.store.list_deploys(&svc.id, 10).ok().and_then(|deps| {
            deps.into_iter().find_map(|d| {
                matches!(d.status, DeployStatus::Live | DeployStatus::Replaced)
                    .then_some(d.image_ref)
                    .flatten()
                    .filter(|img| img.starts_with("opsbox/"))
            })
        });

        let (git_source, repo_url, github_token, deploy_key_enc, deploy_key_ctx) =
            match svc.source_kind {
                SourceKind::Github => {
                    if state.deploy.allow_test_git && svc.repo.starts_with("file://") {
                        (DeployGitSource::Github, svc.repo.clone(), None, None, None)
                    } else {
                        match crate::github::mint_installation_token_for_deploy(
                            &state,
                            &rid,
                            Some(&actor),
                        )
                        .await
                        {
                            Ok(tok) => (
                                DeployGitSource::Github,
                                format!("https://github.com/{}.git", svc.repo),
                                Some(tok),
                                None,
                                None,
                            ),
                            Err(e) => {
                                return fail_and_report(
                                    &state,
                                    &svc,
                                    &deploy_id,
                                    &actor,
                                    via_token,
                                    ip.as_deref(),
                                    &stream,
                                    e.0.error.message,
                                );
                            }
                        }
                    }
                }
                SourceKind::DeployKey => {
                    let dk_id = svc.deploy_key_id.as_deref().unwrap_or("");
                    match state.store.get_deploy_key(dk_id) {
                        Ok(Some(dk)) => (
                            DeployGitSource::DeployKey,
                            svc.repo.clone(),
                            None,
                            Some(dk.private_key_enc),
                            Some(format!("deploy:deploy_key:{dk_id}")),
                        ),
                        _ => {
                            return fail_and_report(
                                &state,
                                &svc,
                                &deploy_id,
                                &actor,
                                via_token,
                                ip.as_deref(),
                                &stream,
                                "SSH deploy key for this service was deleted or not found.".into(),
                            );
                        }
                    }
                }
                SourceKind::GitUrl => (DeployGitSource::GitUrl, svc.repo.clone(), None, None, None),
                SourceKind::DockerImage => unreachable!(),
            };

        let mut build_session = match state
            .priv_client
            .deploy_build(
                &rid,
                &actor,
                DeployBuildRequest {
                    service: svc.name.clone(),
                    deploy_id: deploy_id.clone(),
                    source_kind: git_source,
                    repo_url,
                    branch: svc.branch.clone(),
                    commit_sha,
                    build_method: svc.build_method.clone(),
                    dockerfile_path: svc.dockerfile_path.clone(),
                    port: svc.port,
                    github_token,
                    deploy_key_enc,
                    deploy_key_context: deploy_key_ctx,
                    image_tag: image_tag.clone(),
                    cache_from,
                    vars: vars_specs.clone(),
                    ref_sources,
                    timeout_secs: state.deploy.build_timeout_secs,
                    memory_limit_mb: svc.memory_limit_mb,
                },
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                return fail_and_report(
                    &state,
                    &svc,
                    &deploy_id,
                    &actor,
                    via_token,
                    ip.as_deref(),
                    &stream,
                    e.to_string(),
                );
            }
        };

        let mut build_result = None;
        loop {
            match build_session.recv().await {
                Ok(Some(DeployBuildFrame::Log { line })) => {
                    state
                        .deploy_hub
                        .emit(&state.store, &stream, &deploy_id, &line);
                }
                Ok(Some(DeployBuildFrame::Done(res))) => {
                    build_result = Some(res);
                    break;
                }
                Ok(Some(DeployBuildFrame::Error(e))) => {
                    return fail_and_report(
                        &state,
                        &svc,
                        &deploy_id,
                        &actor,
                        via_token,
                        ip.as_deref(),
                        &stream,
                        e.message,
                    );
                }
                Ok(None) => {
                    if build_result.is_none() {
                        return fail_and_report(
                            &state,
                            &svc,
                            &deploy_id,
                            &actor,
                            via_token,
                            ip.as_deref(),
                            &stream,
                            "Build helper ended unexpectedly before completion.".into(),
                        );
                    }
                    break;
                }
                Err(e) => {
                    return fail_and_report(
                        &state,
                        &svc,
                        &deploy_id,
                        &actor,
                        via_token,
                        ip.as_deref(),
                        &stream,
                        e.to_string(),
                    );
                }
            }
        }

        if let Some(res) = build_result {
            detected_method = Some(res.detected_build_method.clone());
            let _ = state.store.update_deploy_metadata(
                &deploy_id,
                Some(&res.commit_sha),
                Some(&res.commit_message),
                Some(&image_tag),
                None,
            );
        }
    }

    run_release_switch_flow(
        &state,
        &rid,
        &svc,
        &project_name,
        &deploy_id,
        &image_tag,
        detected_method.as_deref(),
        &actor,
        via_token,
        ip.as_deref(),
        &stream,
        false,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_release_switch_flow(
    state: &AppState,
    rid: &str,
    svc: &ServiceRecord,
    project_name: &str,
    deploy_id: &str,
    image_ref: &str,
    detected_method: Option<&str>,
    actor: &str,
    via_token: bool,
    ip: Option<&str>,
    stream: &Arc<DeployStream>,
    is_rollback: bool,
) -> Result<crate::tasks::Outcome, HttpError> {
    let verb = if is_rollback {
        "Rolling back"
    } else {
        "Starting container"
    };
    state.deploy_hub.emit(
        &state.store,
        stream,
        deploy_id,
        &format!("{verb} using image {image_ref}..."),
    );

    let vars_specs = service_env_specs(state, &svc.id);
    let ref_sources = service_ref_sources(state, &svc.id);
    let ref_seq = state.store.capture_service_ref_seq(&svc.id).ok();
    let started = match state
        .priv_client
        .deploy_start_and_check(
            rid,
            actor,
            DeployStartRequest {
                project: project_name.to_owned(),
                service: svc.name.clone(),
                deploy_id: deploy_id.to_owned(),
                image_ref: image_ref.to_owned(),
                port: svc.port,
                health_path: svc.health_path.clone(),
                health_retries: state.deploy.health_retries,
                vars: vars_specs,
                ref_sources,
                memory_limit_mb: svc.memory_limit_mb,
                cpu_limit_millis: svc.cpu_limit_millis,
            },
        )
        .await
    {
        Ok(s) => s,
        Err(e) => {
            return fail_and_report(
                state,
                svc,
                deploy_id,
                actor,
                via_token,
                ip,
                stream,
                e.to_string(),
            );
        }
    };

    finalize_release_switch(
        state,
        rid,
        svc,
        deploy_id,
        image_ref,
        started,
        detected_method,
        ref_seq,
        actor,
        via_token,
        ip,
        stream,
        is_rollback,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn finalize_release_switch(
    state: &AppState,
    rid: &str,
    svc: &ServiceRecord,
    deploy_id: &str,
    image_ref: &str,
    started: opsbox_proto::DeployStartResult,
    detected_method: Option<&str>,
    max_ref_seq: Option<i64>,
    actor: &str,
    via_token: bool,
    ip: Option<&str>,
    stream: &Arc<DeployStream>,
    is_rollback: bool,
) -> Result<crate::tasks::Outcome, HttpError> {
    let endpoint_desc = if started.host_port > 0 {
        format!("127.0.0.1:{}", started.host_port)
    } else {
        "project network".to_owned()
    };
    state.deploy_hub.emit(
        &state.store,
        stream,
        deploy_id,
        &format!("Health check passed on {endpoint_desc}. Switching traffic..."),
    );

    let prev_container = match state.store.complete_deploy_switch_with_refs(
        &svc.id,
        deploy_id,
        image_ref,
        &started.container_id,
        started.host_port,
        detected_method,
        max_ref_seq,
        now_secs(),
    ) {
        Ok(p) => p,
        Err(e) => {
            let _ = state
                .priv_client
                .deploy_stop_container(rid, actor, &started.container_id)
                .await;
            return fail_and_report(
                state,
                svc,
                deploy_id,
                actor,
                via_token,
                ip,
                stream,
                format!("Failed to record deploy switch: {e}"),
            );
        }
    };

    if let Err(e) = crate::panel::sync_proxy_routes(state).await {
        warn!(error = %e, service = %svc.name, "failed to sync proxy routes after deploy");
    }
    state.metrics.notify_docker();

    if let Some(old_cid) = prev_container
        && old_cid != started.container_id
    {
        if state.deploy.drain_secs > 0 {
            state.deploy_hub.emit(
                &state.store,
                stream,
                deploy_id,
                &format!(
                    "Draining previous container for {}s...",
                    state.deploy.drain_secs
                ),
            );
            tokio::time::sleep(Duration::from_secs(state.deploy.drain_secs)).await;
        }
        let _ = state
            .priv_client
            .deploy_stop_container(rid, actor, &old_cid)
            .await;
    }

    if let Ok(prune_refs) = state
        .store
        .prune_old_deploy_image_refs(&svc.id, KEEP_RETAINED_IMAGES)
        && !prune_refs.is_empty()
    {
        let _ = state
            .priv_client
            .deploy_prune_images(rid, actor, &svc.name, prune_refs)
            .await;
    }

    state
        .deploy_hub
        .emit(&state.store, stream, deploy_id, "Deploy is live.");
    state.deploy_hub.finish(stream, "live", None);
    audit_user(
        state,
        actor,
        via_token,
        ip,
        if is_rollback {
            "deploy.rollback.live"
        } else {
            "deploy.live"
        },
        Some(&svc.name),
        json!({ "deployId": deploy_id, "container": started.container_id }),
    );

    Ok(crate::tasks::Outcome {
        log: vec![format!("{} is live.", svc.name)],
        result: json!({
            "service": svc.name,
            "deployId": deploy_id,
            "status": "live",
            "hostPort": started.host_port,
        }),
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn fail_and_report(
    state: &AppState,
    svc: &ServiceRecord,
    deploy_id: &str,
    actor: &str,
    via_token: bool,
    ip: Option<&str>,
    stream: &Arc<DeployStream>,
    reason: String,
) -> Result<crate::tasks::Outcome, HttpError> {
    state
        .deploy_hub
        .emit(&state.store, stream, deploy_id, &format!("ERROR: {reason}"));
    let _ = state
        .store
        .fail_deploy(&svc.id, deploy_id, &reason, now_secs());
    state.deploy_hub.finish(stream, "failed", Some(&reason));
    audit_user(
        state,
        actor,
        via_token,
        ip,
        "deploy.failed",
        Some(&svc.name),
        json!({ "deployId": deploy_id, "error": reason }),
    );
    Err(HttpError::new(ErrorCode::Internal, reason))
}

pub(crate) fn service_env_specs(state: &AppState, service_id: &str) -> Vec<DeployEnvSpec> {
    state
        .store
        .list_service_vars(service_id)
        .unwrap_or_default()
        .into_iter()
        .map(|v| {
            let ctx = var_seal_context(service_id, &v.key);
            DeployEnvSpec {
                key: v.key,
                value_enc: v.value_enc,
                seal_context: ctx,
                is_secret: v.is_secret,
            }
        })
        .collect()
}

pub(crate) fn service_ref_sources(state: &AppState, service_id: &str) -> Vec<DeployRefSource> {
    let mut seen_ids = HashSet::new();
    seen_ids.insert(service_id.to_owned());
    let mut frontier = vec![service_id.to_owned()];
    let mut out = Vec::new();
    for _depth in 0..4 {
        let mut next = Vec::new();
        for cur in frontier {
            let Ok(refs) = state.store.list_service_var_refs(&cur) else {
                continue;
            };
            for r in refs {
                if seen_ids.insert(r.target_service_id.clone())
                    && let Ok(Some(target)) = state.store.get_service(&r.target_service_id)
                {
                    let vars = service_env_specs(state, &target.id);
                    out.push(DeployRefSource {
                        service: target.name,
                        vars,
                    });
                    next.push(target.id);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    out
}
