//! Built-in service template catalog, one-click template instantiation, and
//! Docker Compose stack validation and deployment (docs/BACKEND_SPEC.md 7.6).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use opsbox_deploy::compose::parse_compose_yaml;
use opsbox_deploy::refs::parse_pure_var_ref;
use opsbox_deploy::templates::{
    TemplateCategory, TemplateKind, TemplateSpec, catalog, generate_instance_env, get_template,
    parse_disk_gb, rotated_secret_key,
};
use opsbox_proto::{
    ComposeDeployRequest, ComposeValidateRequest, DatabaseRotateRequest, DeployEnvSpec,
    DeployRefSource, ErrorCode, Role, valid_service_name,
};
use opsbox_store::{
    DeployRecord, DeployTrigger, NewService, ServiceKind, ServiceRecord, SourceKind, UpdateService,
    UpsertComposeStack,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tracing::warn;

use crate::deploy::{
    DeployStream, audit_user, enqueue_deploy, fail_and_report, fetch_live_cert_domains, forbidden,
    invalid, next_graph_pos, now_secs, parse_vars_input, resolve_owner_id, resolve_service_view,
    seal_and_store_vars, service_env_specs, service_readable, service_ref_sources,
    service_writable, validate_vars_refs_before_create,
};
use crate::json::ApiJson;
use crate::session::{Client, Deployer, Viewer, deny_token};
use crate::{AppState, HttpError, RequestId};

fn serialize_template(t: &TemplateSpec) -> Value {
    json!({
        "id": t.id,
        "name": t.name,
        "category": match t.category {
            TemplateCategory::Databases => "databases",
            TemplateCategory::Tools => "tools",
        },
        "kind": match t.kind {
            TemplateKind::Db => "db",
            TemplateKind::Tool => "tool",
        },
        "description": t.description,
        "desc": t.description,
        "resources": t.resources,
        "res": t.resources,
        "image": t.image,
        "versions": t.versions,
        "port": t.port,
        "extraPorts": t.extra_ports,
        "internalByDefault": t.internal_by_default,
        "requiresDatabase": t.id == "umami",
        "backup": {
            "enabledByDefault": t.backup.enabled_by_default,
            "cron": t.backup.cron,
            "method": t.backup.method,
        },
    })
}

/// `GET /api/v1/templates`: return all embedded database and tool templates.
pub(crate) async fn list_templates(Viewer(_who): Viewer) -> Json<Value> {
    let all = catalog();
    let databases: Vec<Value> = all
        .iter()
        .filter(|t| t.category == TemplateCategory::Databases)
        .map(serialize_template)
        .collect();
    let tools: Vec<Value> = all
        .iter()
        .filter(|t| t.category == TemplateCategory::Tools)
        .map(serialize_template)
        .collect();
    let templates: Vec<Value> = all.iter().map(serialize_template).collect();
    Json(json!({
        "databases": databases,
        "tools": tools,
        "templates": templates,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateTemplateInstanceBody {
    project: Option<String>,
    name: String,
    version: Option<String>,
    disk: Option<String>,
    backup: Option<bool>,
    #[serde(alias = "internal_only")]
    internal_only: Option<bool>,
    #[serde(alias = "public_port", alias = "publishPort", alias = "publish_port")]
    public_port: Option<bool>,
    #[serde(default)]
    confirm: bool,
    #[serde(alias = "database_url")]
    database_url: Option<String>,
    vars: Option<Value>,
    #[serde(alias = "memory_limit_mb")]
    memory_limit_mb: Option<i64>,
    #[serde(alias = "cpu_limit_millis")]
    cpu_limit_millis: Option<i64>,
    owner: Option<String>,
}

/// `POST /api/v1/templates/{id}/instances`: create a template database or tool container
/// with sealed generated credentials, named volume, project network isolation, and default
/// scheduled backup job entry.
pub(crate) async fn create_template_instance(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(template_id): Path<String>,
    ApiJson(body): ApiJson<CreateTemplateInstanceBody>,
) -> Result<Response, HttpError> {
    if who.via_token && who.user.role != Role::Admin {
        return Err(forbidden("Deploy tokens cannot create new services."));
    }

    let tmpl = get_template(template_id.trim())
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Template not found."))?;

    let name = body.name.trim().to_ascii_lowercase();
    if !valid_service_name(&name) {
        return Err(invalid(
            "name",
            "Service name must be 1 to 63 lowercase letters, digits, or dashes.",
        ));
    }

    let version = body
        .version
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&tmpl.versions[0]);
    if !tmpl.versions.iter().any(|v| v == version) {
        return Err(invalid(
            "version",
            format!("Unsupported version '{version}' for {}.", tmpl.name),
        ));
    }

    let disk_gb =
        parse_disk_gb(body.disk.as_deref()).map_err(|e| invalid("disk", e.to_string()))?;
    let backup_enabled = body.backup.unwrap_or(tmpl.backup.enabled_by_default);
    let public_port = body.public_port.unwrap_or(false);
    if body.internal_only == Some(true) && public_port {
        return Err(invalid(
            "publicPort",
            "Cannot enable both internalOnly and publicPort.",
        ));
    }
    let internal_only = if public_port {
        false
    } else {
        body.internal_only.unwrap_or(tmpl.internal_by_default)
    };
    if public_port && tmpl.kind == TemplateKind::Db && !body.confirm {
        return Err(HttpError::new(
            ErrorCode::ConfirmationRequired,
            format!(
                "Publishing {} port {} to the internet (0.0.0.0) exposes the database to external network scanners and brute-force attacks. Keep it on the private project network or bind to 127.0.0.1 instead, or confirm to publish publicly.",
                tmpl.name, tmpl.port
            ),
        ));
    }

    if let Some(mem) = body.memory_limit_mb
        && !(64..=1_048_576).contains(&mem)
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

    let mut overrides = parse_vars_input(body.vars.as_ref())?;
    if let Some(db_url) = body
        .database_url
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        overrides.retain(|(k, _, _)| k != "DATABASE_URL");
        overrides.push(("DATABASE_URL".to_owned(), db_url.to_owned(), true));
    }
    if tmpl.id == "umami" {
        let db_url = overrides
            .iter()
            .find(|(k, _, _)| k == "DATABASE_URL")
            .map(|(_, v, _)| v.trim())
            .unwrap_or("");
        if !(db_url.starts_with("postgres://")
            || db_url.starts_with("postgresql://")
            || parse_pure_var_ref(db_url).is_some())
        {
            return Err(invalid(
                "databaseUrl",
                "Umami needs a PostgreSQL DATABASE_URL connection string or service reference (${{ db.DATABASE_URL }}).",
            ));
        }
    }

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

    validate_vars_refs_before_create(&state, &proj.id, &env.id, &name, &overrides)?;

    let existing = state
        .store
        .list_services(&proj.id, &env.id)
        .map_err(|_| HttpError::internal())?;
    let (graph_x, graph_y) = next_graph_pos(existing.len());

    let generated =
        generate_instance_env(tmpl, &name, &overrides).map_err(|_| HttpError::internal())?;
    let image_ref = format!("{}:{}", tmpl.image, version);
    let kind = match tmpl.kind {
        TemplateKind::Db => ServiceKind::Db,
        TemplateKind::Tool => ServiceKind::Tool,
    };
    let note = format!(
        "{disk_gb} GB · {} · {}",
        if backup_enabled {
            "daily backup"
        } else {
            "no auto backup"
        },
        if internal_only {
            "internal only"
        } else if public_port {
            "public port (0.0.0.0)"
        } else {
            "loopback proxy"
        }
    );

    let svc = state
        .store
        .create_service_with_exposure(
            NewService {
                project_id: &proj.id,
                env_id: &env.id,
                name: &name,
                kind,
                owner_id: owner_id.as_deref(),
                source_kind: SourceKind::DockerImage,
                repo: &image_ref,
                branch: version,
                build_method: &tmpl.name,
                dockerfile_path: None,
                start_cmd: None,
                port: tmpl.port,
                health_path: "tcp",
                domain: None,
                image_ref: Some(&image_ref),
                deploy_key_id: None,
                auto_deploy: false,
                memory_limit_mb: body.memory_limit_mb,
                cpu_limit_millis: body.cpu_limit_millis,
                graph_x,
                graph_y,
                note: Some(&note),
                now,
            },
            None,
            Some(&tmpl.id),
            internal_only,
            public_port,
        )
        .map_err(|_| HttpError::internal())?;

    let seal_pairs: Vec<(String, String, bool)> = generated
        .vars
        .iter()
        .map(|v| (v.key.clone(), v.value.clone(), v.is_secret))
        .collect();
    if let Err(e) = seal_and_store_vars(&state, &rid, &who.user.name, &svc, &seal_pairs, now).await
    {
        let _ = state.store.delete_service(&svc.id);
        return Err(e);
    }

    if let Err(e) = state.store.upsert_backup_job(
        &svc.id,
        Some(&tmpl.id),
        &tmpl.backup.method,
        &tmpl.backup.cron,
        backup_enabled,
        now,
    ) {
        warn!(error = %e, service = %svc.name, "failed to create default backup job");
    }

    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "template.create",
        Some(&svc.name),
        json!({ "template": tmpl.id, "version": version, "internalOnly": internal_only, "publicPort": public_port }),
    );

    let (dep, task_id) = enqueue_deploy(
        &state,
        &rid,
        &svc,
        &proj.name,
        DeployTrigger::Manual,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        None,
        Some(&format!("Create {} ({version})", tmpl.name)),
    )
    .await?;

    let live_certs = fetch_live_cert_domains(&state).await;
    let view = resolve_service_view(&state, &rid, &who, &svc, &live_certs).await?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": svc.name,
            "service": view,
            "deployId": dep.id,
            "task": task_id,
            "connectionVar": generated.primary_var,
            "connectionUrl": generated.primary_url,
        })),
    )
        .into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeployComposeBody {
    project: Option<String>,
    stack: Option<String>,
    yaml: String,
    #[serde(default)]
    confirm: bool,
    vars: Option<Value>,
    owner: Option<String>,
}

fn pick_stack_name(
    requested: Option<&str>,
    yaml: &str,
    existing_check: impl Fn(&str) -> bool,
) -> Result<String, HttpError> {
    if let Some(raw) = requested.map(str::trim).filter(|s| !s.is_empty()) {
        let clean = raw.to_ascii_lowercase();
        if !valid_service_name(&clean) {
            return Err(invalid(
                "stack",
                "Stack name must be 1 to 63 lowercase letters, digits, or dashes.",
            ));
        }
        return Ok(clean);
    }

    let doc = parse_compose_yaml(yaml).map_err(|e| invalid("yaml", e.to_string()))?;
    if let Some(inferred) = doc
        .get("name")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| valid_service_name(s))
    {
        return Ok(inferred);
    }

    if !existing_check("compose") {
        return Ok("compose".to_owned());
    }
    let suffix = ulid::Ulid::generate()
        .to_string()
        .chars()
        .take(6)
        .collect::<String>()
        .to_ascii_lowercase();
    Ok(format!("compose-{suffix}"))
}

fn allocate_compose_service_name(
    stack_name: &str,
    compose_svc: &str,
    taken_names: &mut HashSet<String>,
) -> String {
    let base_clean: String = compose_svc
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = base_clean.trim_matches('-');
    let base = if valid_service_name(trimmed) {
        trimmed.to_owned()
    } else {
        format!("{stack_name}-svc")
    };

    if !taken_names.contains(&base) && valid_service_name(&base) {
        taken_names.insert(base.clone());
        return base;
    }

    let prefixed = format!("{stack_name}-{base}");
    let prefixed_trunc: String = prefixed.chars().take(63).collect();
    let prefixed_clean = prefixed_trunc.trim_end_matches('-').to_owned();
    if valid_service_name(&prefixed_clean) && !taken_names.contains(&prefixed_clean) {
        taken_names.insert(prefixed_clean.clone());
        return prefixed_clean;
    }

    for n in 2..=999 {
        let suffix = format!("-{n}");
        let mut head: String = prefixed_clean
            .chars()
            .take(63usize.saturating_sub(suffix.len()))
            .collect();
        head = head.trim_end_matches('-').to_owned();
        let cand = format!("{head}{suffix}");
        if valid_service_name(&cand) && !taken_names.contains(&cand) {
            taken_names.insert(cand.clone());
            return cand;
        }
    }
    unreachable!("exhausted unique service suffixes")
}

/// `POST /api/v1/compose`: validate a Compose YAML via `docker compose config` inside `priv`,
/// enforce elevated privilege/mount confirmations (`409 confirmation_required`), deploy the
/// stack (`up -d`), connect containers to `opsbox-<project>`, and register graph nodes + edges.
pub(crate) async fn deploy_compose(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<DeployComposeBody>,
) -> Result<Response, HttpError> {
    if who.via_token && who.user.role != Role::Admin {
        return Err(forbidden("Deploy tokens cannot deploy new Compose stacks."));
    }
    if body.yaml.trim().is_empty() {
        return Err(invalid("yaml", "Paste a Docker Compose YAML file."));
    }

    let stack_name = pick_stack_name(body.stack.as_deref(), &body.yaml, |cand| {
        state.store.get_compose_stack(cand).ok().flatten().is_some()
    })?;

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

    let existing_stack = state
        .store
        .get_compose_stack(&stack_name)
        .map_err(|_| HttpError::internal())?;
    if let Some(ref st) = existing_stack {
        if who.user.role != Role::Admin && st.owner_id.as_deref() != Some(who.user.id.as_str()) {
            return Err(forbidden("You cannot modify another user's Compose stack."));
        }
        if st.project_id != proj.id || st.env_id != env.id {
            return Err(HttpError::new(
                ErrorCode::Conflict,
                format!("Compose stack '{stack_name}' belongs to another project."),
            ));
        }
        let siblings = state
            .store
            .list_services_by_compose_stack(&st.id)
            .map_err(|_| HttpError::internal())?;
        let active = state.deploy_hub.inner.active_service_tasks.lock().unwrap();
        for s in &siblings {
            if active.contains_key(&s.id) {
                return Err(HttpError::new(
                    ErrorCode::Conflict,
                    format!("Wait for active deploys on {} to finish.", s.name),
                ));
            }
        }
    }

    let initial_vars = parse_vars_input(body.vars.as_ref())?;

    // Validate YAML + policy in `opsbox-priv` BEFORE creating any DB rows or persisting compose.yml.
    // Unconfirmed elevated flags (`privileged`, `network_mode: host`, external bind mounts)
    // immediately return `409 confirmation_required` here.
    let validation = state
        .priv_client
        .compose_validate(
            &rid,
            &who.user.name,
            ComposeValidateRequest {
                stack: stack_name.clone(),
                yaml: body.yaml.clone(),
                confirm: body.confirm,
            },
        )
        .await?;

    let raw_names: Vec<String> = validation.services.iter().map(|s| s.name.clone()).collect();
    let raw_edges: Vec<(String, String)> = validation
        .services
        .iter()
        .flat_map(|s| {
            s.depends_on
                .iter()
                .map(move |dep| (s.name.clone(), dep.clone()))
        })
        .collect();

    let stack_rec = state
        .store
        .upsert_compose_stack(UpsertComposeStack {
            project_id: &proj.id,
            env_id: &env.id,
            name: &stack_name,
            owner_id: owner_id.as_deref(),
            compose_yaml: &body.yaml,
            project_name: &validation.project_name,
            service_names: &raw_names,
            depends_edges: &raw_edges,
            confirmed: body.confirm,
            now,
        })
        .map_err(|_| HttpError::internal())?;

    let existing_project_services = state
        .store
        .list_services(&proj.id, &env.id)
        .map_err(|_| HttpError::internal())?;
    let prev_stack_services = state
        .store
        .list_services_by_compose_stack(&stack_rec.id)
        .map_err(|_| HttpError::internal())?;

    let mut taken_names: HashSet<String> = existing_project_services
        .iter()
        .map(|s| s.name.clone())
        .collect();
    // Free names currently owned by this stack for clean reuse on redeploy.
    for s in &prev_stack_services {
        taken_names.remove(&s.name);
    }

    let mut existing_idx = existing_project_services.len();
    let mut service_records: Vec<ServiceRecord> = Vec::with_capacity(validation.services.len());
    let mut raw_to_node: HashMap<String, String> = HashMap::new();
    let mut retained_ids: HashSet<String> = HashSet::new();

    for info in &validation.services {
        let repo_label = info
            .image
            .clone()
            .unwrap_or_else(|| format!("{stack_name}/{}", info.name));
        let internal_only = info.port == 0;

        let svc = if let Some(prev) = prev_stack_services.iter().find(|p| p.branch == info.name) {
            taken_names.insert(prev.name.clone());
            state
                .store
                .update_service(
                    &prev.id,
                    UpdateService {
                        branch: Some(&info.name),
                        build_method: Some("Compose"),
                        dockerfile_path: None,
                        start_cmd: None,
                        port: Some(info.port),
                        health_path: None,
                        domain: None,
                        image_ref: Some(info.image.as_deref()),
                        deploy_key_id: None,
                        auto_deploy: None,
                        memory_limit_mb: None,
                        cpu_limit_millis: None,
                        owner_id: None,
                        now,
                    },
                )
                .map_err(|_| HttpError::internal())?
                .unwrap_or_else(|| prev.clone())
        } else {
            let node_name =
                allocate_compose_service_name(&stack_name, &info.name, &mut taken_names);
            let (gx, gy) = next_graph_pos(existing_idx);
            existing_idx += 1;
            state
                .store
                .create_service_with_meta(
                    NewService {
                        project_id: &proj.id,
                        env_id: &env.id,
                        name: &node_name,
                        kind: ServiceKind::Compose,
                        owner_id: owner_id.as_deref(),
                        source_kind: SourceKind::DockerImage,
                        repo: &repo_label,
                        branch: &info.name,
                        build_method: "Compose",
                        dockerfile_path: None,
                        start_cmd: None,
                        port: info.port,
                        health_path: "tcp",
                        domain: None,
                        image_ref: info.image.as_deref(),
                        deploy_key_id: None,
                        auto_deploy: false,
                        memory_limit_mb: None,
                        cpu_limit_millis: None,
                        graph_x: gx,
                        graph_y: gy,
                        note: Some(&format!("Compose stack: {stack_name}")),
                        now,
                    },
                    Some(&stack_rec.id),
                    None,
                    internal_only,
                )
                .map_err(|_| HttpError::internal())?
        };
        retained_ids.insert(svc.id.clone());
        raw_to_node.insert(info.name.clone(), svc.name.clone());
        if !initial_vars.is_empty() {
            seal_and_store_vars(&state, &rid, &who.user.name, &svc, &initial_vars, now).await?;
        }
        service_records.push(svc);
    }

    for stale in &prev_stack_services {
        if !retained_ids.contains(&stale.id) {
            let _ = state.store.delete_service(&stale.id);
        }
    }

    let mut stack_deploys: Vec<(ServiceRecord, DeployRecord, Arc<DeployStream>)> =
        Vec::with_capacity(service_records.len());
    for svc in &service_records {
        let dep = state
            .store
            .create_deploy(
                &svc.id,
                None,
                &format!("Compose deploy ({stack_name})"),
                DeployTrigger::Manual,
                &who.user.name,
                now,
            )
            .map_err(|_| HttpError::internal())?;
        let stream = state.deploy_hub.register_stream(&dep.id);
        stack_deploys.push((svc.clone(), dep, stream));
    }

    let opsbox_names: Vec<String> = service_records.iter().map(|s| s.name.clone()).collect();
    let mapped_edges: Vec<(String, String)> = raw_edges
        .iter()
        .filter_map(|(f, t)| Some((raw_to_node.get(f)?.clone(), raw_to_node.get(t)?.clone())))
        .collect();

    let state_bg = state.clone();
    let rid_bg = rid.clone();
    let proj_bg = proj.name.clone();
    let stack_bg = stack_name.clone();
    let yaml_bg = body.yaml.clone();
    let confirm_bg = body.confirm || stack_rec.confirmed;
    let actor_bg = who.user.name.clone();
    let via_token = who.via_token;
    let ip_bg = client.ip.clone();
    let stack_vars = collect_stack_env_specs(&state, &service_records);

    let task_id = state
        .tasks
        .start(&format!("compose:{stack_name}"), async move {
            execute_compose_deploy(
                state_bg,
                rid_bg,
                proj_bg,
                stack_bg,
                yaml_bg,
                confirm_bg,
                stack_vars,
                stack_deploys,
                actor_bg,
                via_token,
                Some(ip_bg),
            )
            .await
        })?;

    Ok((
        StatusCode::CREATED,
        Json(json!({
            "stack": stack_name,
            "projectName": validation.project_name,
            "names": opsbox_names,
            "warnings": validation.warnings,
            "edges": mapped_edges,
            "task": task_id,
        })),
    )
        .into_response())
}

fn collect_stack_env_specs(state: &AppState, services: &[ServiceRecord]) -> Vec<DeployEnvSpec> {
    let mut seen_keys = HashSet::new();
    let mut out = Vec::new();
    for svc in services {
        for spec in service_env_specs(state, &svc.id) {
            if seen_keys.insert(spec.key.clone()) {
                out.push(spec);
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
async fn execute_compose_deploy(
    state: AppState,
    rid: String,
    project_name: String,
    stack_name: String,
    yaml: String,
    confirm: bool,
    vars: Vec<DeployEnvSpec>,
    targets: Vec<(ServiceRecord, DeployRecord, Arc<DeployStream>)>,
    actor: String,
    via_token: bool,
    ip: Option<String>,
) -> Result<crate::tasks::Outcome, HttpError> {
    let mut ref_sources: Vec<DeployRefSource> = Vec::new();
    let mut seen_sources: HashSet<String> = HashSet::new();
    for (svc, _, _) in &targets {
        for src in service_ref_sources(&state, &svc.id) {
            if seen_sources.insert(src.service.clone()) {
                ref_sources.push(src);
            }
        }
    }
    let ref_seqs: Vec<(String, i64)> = targets
        .iter()
        .filter_map(|(svc, _, _)| {
            state
                .store
                .capture_service_ref_seq(&svc.id)
                .ok()
                .map(|seq| (svc.id.clone(), seq))
        })
        .collect();

    for (svc, dep, stream) in &targets {
        state.deploy_hub.emit(
            &state.store,
            stream,
            &dep.id,
            &format!("Deploying Compose stack {stack_name} ({})...", svc.branch),
        );
    }

    let result = match state
        .priv_client
        .compose_deploy(
            &rid,
            &actor,
            ComposeDeployRequest {
                project: project_name,
                stack: stack_name.clone(),
                yaml,
                confirm,
                vars,
                ref_sources,
            },
        )
        .await
    {
        Ok(res) => res,
        Err(e) => {
            let reason = e.to_string();
            for (svc, dep, stream) in &targets {
                let _ = fail_and_report(
                    &state,
                    svc,
                    &dep.id,
                    &actor,
                    via_token,
                    ip.as_deref(),
                    stream,
                    reason.clone(),
                );
            }
            return Err(HttpError::from(e));
        }
    };

    for (svc, dep, stream) in &targets {
        for line in &result.log_lines {
            state.deploy_hub.emit(&state.store, stream, &dep.id, line);
        }
        let deployed = result.services.iter().find(|r| r.name == svc.branch);
        let cid = deployed
            .and_then(|d| d.container_id.as_deref())
            .unwrap_or("");
        let img = deployed
            .and_then(|d| d.image.as_deref())
            .or(svc.image_ref.as_deref())
            .unwrap_or(&svc.repo);
        let host_port = deployed.and_then(|d| d.host_port).unwrap_or(0);

        let seq_opt = ref_seqs
            .iter()
            .find(|(id, _)| id == &svc.id)
            .map(|(_, s)| *s);
        let _ = state.store.complete_deploy_switch_with_refs(
            &svc.id,
            &dep.id,
            img,
            cid,
            host_port,
            Some("Compose"),
            seq_opt,
            now_secs(),
        );
        state
            .deploy_hub
            .emit(&state.store, stream, &dep.id, "Deploy is live.");
        state.deploy_hub.finish(stream, "live", None);
    }

    if let Err(e) = crate::panel::sync_proxy_routes(&state).await {
        warn!(error = %e, stack = %stack_name, "failed to sync proxy routes after compose deploy");
    }
    state.metrics.notify_docker();
    audit_user(
        &state,
        &actor,
        via_token,
        ip.as_deref(),
        "compose.deploy",
        Some(&stack_name),
        json!({ "services": targets.iter().map(|(s, _, _)| &s.name).collect::<Vec<_>>() }),
    );

    Ok(crate::tasks::Outcome {
        log: result.log_lines,
        result: json!({
            "stack": stack_name,
            "services": targets.iter().map(|(s, _, _)| &s.name).collect::<Vec<_>>(),
        }),
    })
}

/// Redeploy all containers of an existing Compose stack when initiated from one of its service cards.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_compose_service_redeploy(
    state: &AppState,
    rid: &str,
    svc: &ServiceRecord,
    stack_id: &str,
    project_name: &str,
    deploy_id: &str,
    actor: &str,
    via_token: bool,
    ip: Option<&str>,
    stream: &Arc<DeployStream>,
) -> Result<crate::tasks::Outcome, HttpError> {
    let stack = match state.store.get_compose_stack(stack_id) {
        Ok(Some(st)) => st,
        _ => {
            return fail_and_report(
                state,
                svc,
                deploy_id,
                actor,
                via_token,
                ip,
                stream,
                "Compose stack metadata could not be found.".to_owned(),
            );
        }
    };
    let siblings = state
        .store
        .list_services_by_compose_stack(&stack.id)
        .unwrap_or_else(|_| vec![svc.clone()]);
    let vars = collect_stack_env_specs(state, &siblings);
    let Some(dep) = state.store.get_deploy(deploy_id).ok().flatten() else {
        return Err(HttpError::internal());
    };
    let mut targets = vec![(svc.clone(), dep, Arc::clone(stream))];
    // Keep sibling records synchronized with updated container IDs / ports when redeploying stack.
    let now = now_secs();
    for sib in siblings {
        if sib.id != svc.id
            && let Ok(sib_dep) = state.store.create_deploy(
                &sib.id,
                None,
                &format!("Compose stack redeploy ({})", stack.name),
                DeployTrigger::Manual,
                actor,
                now,
            )
        {
            let sib_stream = state.deploy_hub.register_stream(&sib_dep.id);
            targets.push((sib, sib_dep, sib_stream));
        }
    }

    execute_compose_deploy(
        state.clone(),
        rid.to_owned(),
        project_name.to_owned(),
        stack.name,
        stack.compose_yaml,
        stack.confirmed,
        vars,
        targets,
        actor.to_owned(),
        via_token,
        ip.map(str::to_owned),
    )
    .await
}

/// `DELETE /api/v1/compose/{stack}`: tear down an entire Compose stack and delete its service rows.
pub(crate) async fn delete_compose_stack(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(stack_key): Path<String>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let stack = state
        .store
        .get_compose_stack(&stack_key)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Compose stack not found."))?;
    if who.user.role != Role::Admin && stack.owner_id.as_deref() != Some(who.user.id.as_str()) {
        return Err(forbidden("You cannot delete this Compose stack."));
    }

    let siblings = state
        .store
        .list_services_by_compose_stack(&stack.id)
        .map_err(|_| HttpError::internal())?;
    {
        let active = state.deploy_hub.inner.active_service_tasks.lock().unwrap();
        for s in &siblings {
            if active.contains_key(&s.id) {
                return Err(HttpError::new(
                    ErrorCode::Conflict,
                    format!("Wait for active deploys on {} to finish.", s.name),
                ));
            }
        }
    }

    let mut domains_to_clean = Vec::new();
    for s in &siblings {
        if let Ok(doms) = state.store.list_service_domains(&s.id) {
            domains_to_clean.extend(doms);
        }
    }

    state
        .priv_client
        .compose_down(&rid, &who.user.name, &stack.name, true)
        .await?;
    state
        .store
        .delete_compose_stack(&stack.id)
        .map_err(|_| HttpError::internal())?;

    if let Some(ref proxy) = state.proxy {
        for d in domains_to_clean {
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
        "compose.delete",
        Some(&stack.name),
        json!({}),
    );

    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/v1/services/{id}/rotate`: rotate credentials inside the running database
/// container (in `opsbox-priv`), update sealed variables, and mark dependent apps as
/// needing redeploy.
pub(crate) async fn rotate_database(
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
        return Err(forbidden("You cannot rotate credentials for this service."));
    }
    if svc.kind != ServiceKind::Db {
        return Err(invalid(
            "service",
            "Credential rotation is only available for database services.",
        ));
    }
    let tmpl_id = svc.template_id.as_deref().ok_or_else(|| {
        invalid(
            "template",
            "Credential rotation is only supported for managed database templates.",
        )
    })?;
    if rotated_secret_key(tmpl_id).is_none() {
        return Err(invalid(
            "template",
            "Credential rotation is not supported for this database template.",
        ));
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
            "Wait for the running deploy to finish before rotating credentials.",
        ));
    }
    let container_id = svc.active_container.as_deref().ok_or_else(|| {
        HttpError::new(
            ErrorCode::Conflict,
            "Database container must be running to rotate credentials.",
        )
    })?;

    let vars = service_env_specs(&state, &svc.id);
    let rotated = state
        .priv_client
        .database_rotate(
            &rid,
            &who.user.name,
            DatabaseRotateRequest {
                service_id: svc.id.clone(),
                service_name: svc.name.clone(),
                template_id: tmpl_id.to_owned(),
                container_id: container_id.to_owned(),
                vars,
            },
        )
        .await?;

    let now = now_secs();
    let mut rotated_keys = Vec::with_capacity(rotated.vars.len());
    for spec in &rotated.vars {
        state
            .store
            .upsert_service_var(&svc.id, &spec.key, &spec.value_enc, spec.is_secret, now)
            .map_err(|_| HttpError::internal())?;
        rotated_keys.push(spec.key.as_str());
    }

    let affected = state
        .store
        .mark_credentials_rotated(&svc.id, &rotated_keys, now)
        .map_err(|_| HttpError::internal())?;

    let affected_json: Vec<Value> = affected
        .into_iter()
        .filter(|r| service_readable(&who, &r.service))
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

    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "service.database.rotate",
        Some(&svc.name),
        json!({ "rotatedKeys": rotated_keys, "affectedCount": affected_json.len() }),
    );

    Ok(Json(json!({
        "ok": true,
        "service": svc.name,
        "rotatedKeys": rotated_keys,
        "rotatedAt": now.saturating_mul(1000),
        "affectedApps": affected_json,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetExposureBody {
    mode: Option<String>,
    #[serde(alias = "internal_only")]
    internal_only: Option<bool>,
    #[serde(alias = "public_port", alias = "publishPort", alias = "publish_port")]
    public_port: Option<bool>,
    #[serde(default)]
    confirm: bool,
}

/// `PUT /api/v1/services/{id}/exposure`: configure network exposure (`internal`,
/// `loopback`, or `public`) for a database/template service. Publishing a database port
/// publicly requires `confirm: true` (returns `409 confirmation_required` otherwise).
pub(crate) async fn set_service_exposure(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<SetExposureBody>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let svc = state
        .store
        .get_service(&id)
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;
    if !service_writable(&who, &svc) {
        return Err(forbidden(
            "You cannot change network exposure for this service.",
        ));
    }

    let (internal_only, public_port) = if let Some(ref mode_raw) = body.mode {
        match mode_raw.trim().to_ascii_lowercase().as_str() {
            "internal" | "private" => (true, false),
            "loopback" | "local" => (false, false),
            "public" => (false, true),
            _ => {
                return Err(invalid(
                    "mode",
                    "Exposure mode must be 'internal', 'loopback', or 'public'.",
                ));
            }
        }
    } else {
        let pub_flag = body.public_port.unwrap_or(false);
        let int_flag =
            body.internal_only
                .unwrap_or(if pub_flag { false } else { svc.internal_only });
        if int_flag && pub_flag {
            return Err(invalid(
                "publicPort",
                "Cannot enable both internalOnly and publicPort.",
            ));
        }
        (int_flag, pub_flag)
    };

    if public_port && svc.kind == ServiceKind::Db && !body.confirm {
        return Err(HttpError::new(
            ErrorCode::ConfirmationRequired,
            format!(
                "Publishing {} port {} to the internet (0.0.0.0) exposes the database to external network scanners and brute-force attacks. Keep it on the private project network or loopback (127.0.0.1) instead, or confirm to publish publicly.",
                svc.name, svc.port
            ),
        ));
    }

    if internal_only && svc.domain.is_some() {
        return Err(invalid(
            "mode",
            "Remove custom domains before making this service internal-only.",
        ));
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
            "Wait for the running deploy to finish before updating exposure.",
        ));
    }

    let changed = svc.internal_only != internal_only || svc.public_port != public_port;
    let updated = state
        .store
        .set_service_exposure(&svc.id, internal_only, public_port, now_secs())
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Service not found."))?;

    let mode_str = if internal_only {
        "internal"
    } else if public_port {
        "public"
    } else {
        "loopback"
    };
    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "service.exposure.update",
        Some(&updated.name),
        json!({ "mode": mode_str, "internalOnly": internal_only, "publicPort": public_port }),
    );

    let live_certs = fetch_live_cert_domains(&state).await;
    let view = resolve_service_view(&state, &rid, &who, &updated, &live_certs).await?;
    Ok(Json(json!({
        "ok": true,
        "mode": mode_str,
        "redeployRequired": changed && updated.active_container.is_some(),
        "service": view,
    })))
}
