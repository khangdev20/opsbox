//! `POST /api/v1/config/import/preview` and `…/apply`: bring an `opsbox-config/1` file onto
//! this server. Preview plans what will be created, what needs input, and what is skipped.
//! Apply runs as a background task. Existing objects are never overwritten.
//!
//! Interactive admin only (no tokens); the account password is asked again. When the file
//! has sealed secrets, a passphrase opens them; a wrong passphrase fails cleanly with no
//! partial write. Services missing secrets are created but not deployed.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, State};
use axum::http::StatusCode;
use opsbox_proto::{BackupTargetKind, ErrorCode, NewRule, Secret};
use opsbox_store::{
    ActorKind, AuditResult, NewBackupJob, NewService, NewServiceDomain, ServiceKind, SourceKind,
    UpsertComposeStack,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::export::{decrypt_secrets, DecryptError, FORMAT};
use crate::json::ApiJson;
use crate::session::{Admin, Client, deny_token};
use crate::system::patient;
use crate::tasks::Outcome;
use crate::{AppState, HttpError, RequestId};

const AUDIT_PREVIEW: &str = "config.import.preview";
const AUDIT_APPLY: &str = "config.import";
const TASK_KIND: &str = "config.import";

// --------------------------------------------------------------------------------- body

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ImportBody {
    password: String,
    #[serde(default)]
    passphrase: Option<String>,
    /// The parsed `opsbox-config/1` document.
    document: Value,
}

// --------------------------------------------------------------------------------- plan

#[derive(Debug, Default)]
struct Plan {
    will_create: Map<String, Value>,
    needs_input: Vec<Value>,
    skipped: Vec<Value>,
    /// Opened secrets payload, when the file had them and the passphrase worked.
    opened: Option<Value>,
}

fn store_err(_: opsbox_store::StoreError) -> HttpError {
    HttpError::internal()
}

fn invalid(field: &'static str, msg: impl Into<String>) -> HttpError {
    HttpError(
        opsbox_proto::ApiError::new(ErrorCode::Validation, msg.into()).with_field(field),
        None,
    )
}

/// Refuse anything that is not `opsbox-config/1` with the required top-level shape.
pub fn validate_document(doc: &Value) -> Result<(), HttpError> {
    let Some(obj) = doc.as_object() else {
        return Err(invalid("document", "The file must be a JSON object."));
    };
    match obj.get("format").and_then(Value::as_str) {
        Some(FORMAT) => {}
        Some(other) => {
            return Err(invalid(
                "format",
                format!("This file uses format `{other}`, which this opsbox cannot import. Expected `{FORMAT}`."),
            ));
        }
        None => {
            return Err(invalid(
                "format",
                format!("This file has no format. Expected `{FORMAT}`."),
            ));
        }
    }
    for key in [
        "services",
        "templates",
        "composeProjects",
        "databases",
        "backups",
        "alerts",
        "network",
        "updates",
        "firewall",
        "users",
        "notExported",
        "secrets",
    ] {
        if !obj.contains_key(key) {
            return Err(invalid(
                "document",
                format!("This file is missing `{key}` and is not a valid {FORMAT} export."),
            ));
        }
    }
    if !obj.get("services").map(Value::is_array).unwrap_or(false) {
        return Err(invalid("services", "services must be an array."));
    }
    if !obj.get("secrets").map(Value::is_object).unwrap_or(false) {
        return Err(invalid("secrets", "secrets must be an object."));
    }
    Ok(())
}

fn push_create(plan: &mut Plan, section: &str, name: impl Into<String>) {
    let arr = plan
        .will_create
        .entry(section.to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Some(a) = arr.as_array_mut() {
        a.push(Value::String(name.into()));
    }
}

fn push_skip(plan: &mut Plan, kind: &str, name: &str, reason: &str) {
    plan.skipped.push(json!({
        "kind": kind,
        "name": name,
        "reason": reason,
    }));
}

fn push_need(plan: &mut Plan, entry: Value) {
    plan.needs_input.push(entry);
}

fn text<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn bool_val(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}

/// Build the import plan against the current store. Never mutates the store.
fn build_plan(
    state: &AppState,
    doc: &Value,
    passphrase: Option<&str>,
) -> Result<Plan, HttpError> {
    validate_document(doc)?;
    let mut plan = Plan::default();

    let secrets = doc.get("secrets").cloned().unwrap_or(json!({}));
    let included = secrets.get("included") == Some(&Value::Bool(true));
    if included {
        let Some(p) = passphrase.filter(|s| !s.is_empty()) else {
            return Err(invalid(
                "passphrase",
                "This file has sealed secrets. Enter the passphrase used when it was exported.",
            ));
        };
        match decrypt_secrets(doc, p) {
            Ok(opened) => plan.opened = Some(opened),
            Err(DecryptError::WrongPassphrase) => {
                return Err(HttpError::new(
                    ErrorCode::Forbidden,
                    DecryptError::WrongPassphrase.message(),
                ));
            }
            Err(e) => {
                return Err(invalid("document", e.message()));
            }
        }
    } else if passphrase.is_some_and(|p| !p.is_empty()) {
        // Passphrase given but file has no sealed block: ignore it, still list needsReentry.
    }

    // Seed needs_input from the file's own list, then add DNS / GitHub / missing secrets.
    if let Some(arr) = secrets.get("needsReentry").and_then(Value::as_array) {
        for e in arr {
            push_need(&mut plan, e.clone());
        }
    }

    let opened = plan.opened.clone().unwrap_or(json!({}));
    let opened_vars: HashSet<(String, String)> = opened
        .get("serviceVars")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            Some((
                text(v, "service")?.to_owned(),
                text(v, "key")?.to_owned(),
            ))
        })
        .collect();
    let opened_dests: HashSet<String> = opened
        .get("backupDestinations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| text(v, "id").map(str::to_owned))
        .collect();
    let opened_channels: HashSet<String> = opened
        .get("alertChannels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| text(v, "id").map(str::to_owned))
        .collect();

    let store = &state.store;
    let existing_services: HashSet<String> = store
        .list_all_services()
        .map_err(store_err)?
        .into_iter()
        .map(|s| s.name)
        .collect();
    let existing_stacks: HashSet<String> = store
        .list_all_compose_stacks()
        .map_err(store_err)?
        .into_iter()
        .map(|s| s.name)
        .collect();
    let existing_dests: HashSet<String> = store
        .list_backup_destinations()
        .map_err(store_err)?
        .into_iter()
        .map(|d| d.id)
        .collect();
    let existing_dest_names: HashSet<String> = store
        .list_backup_destinations()
        .map_err(store_err)?
        .into_iter()
        .map(|d| d.name)
        .collect();
    let engine: opsbox_alerts::Engine = store
        .get_setting("alerts.engine.v1")
        .map_err(store_err)?
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| HttpError::internal())?
        .unwrap_or_default();
    let existing_rule_ids: HashSet<String> = engine.rules.iter().map(|r| r.id.clone()).collect();
    let existing_channel_ids: HashSet<String> = engine.channels.keys().cloned().collect();

    // ---- compose projects
    for c in doc
        .get("composeProjects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = text(c, "name").filter(|s| !s.is_empty()) else {
            continue;
        };
        if existing_stacks.contains(name) {
            push_skip(
                &mut plan,
                "composeProject",
                name,
                "A compose project with this name already exists.",
            );
        } else {
            push_create(&mut plan, "composeProjects", name);
        }
    }

    // ---- services
    let mut github_needed = false;
    for svc in doc
        .get("services")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = text(svc, "name").filter(|s| !s.is_empty()) else {
            continue;
        };
        if existing_services.contains(name) {
            push_skip(
                &mut plan,
                "service",
                name,
                "A service with this name already exists.",
            );
            continue;
        }
        push_create(&mut plan, "services", name);
        if text(svc, "kind") == Some("db") {
            push_create(&mut plan, "databases", name);
        }
        if text(svc, "templateId").is_some_and(|t| !t.is_empty()) {
            push_create(&mut plan, "templates", name);
        }

        let source_kind = svc
            .pointer("/source/kind")
            .and_then(Value::as_str)
            .unwrap_or("");
        if source_kind == "github" {
            github_needed = true;
        }
        if source_kind == "deploy_key"
            || svc
                .pointer("/source/deployKeyId")
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        {
            push_need(
                &mut plan,
                json!({
                    "kind": "deployKey",
                    "service": name,
                    "reason": "Private deploy keys are not imported. Create a new key for this repository.",
                }),
            );
        }

        for domain in svc
            .get("domains")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|d| text(d, "domain"))
            .chain(text(svc, "domain"))
        {
            if domain.is_empty() {
                continue;
            }
            push_need(
                &mut plan,
                json!({
                    "kind": "dns",
                    "domain": domain,
                    "service": name,
                    "reason": "Point this domain's DNS at the new server, then check it in Domains.",
                }),
            );
        }

        for var in svc
            .get("vars")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(key) = text(var, "key") else {
                continue;
            };
            let is_secret = bool_val(var, "secret", false);
            let has_ref = text(var, "ref").is_some_and(|s| !s.is_empty());
            let has_value = var.get("value").is_some_and(|v| !v.is_null());
            if has_ref || has_value {
                continue;
            }
            if is_secret && !opened_vars.contains(&(name.to_owned(), key.to_owned())) {
                // Avoid duplicating entries already listed in needsReentry.
                let already = plan.needs_input.iter().any(|e| {
                    e.get("kind").and_then(Value::as_str) == Some("serviceVar")
                        && text(e, "service") == Some(name)
                        && text(e, "key") == Some(key)
                });
                if !already {
                    push_need(
                        &mut plan,
                        json!({
                            "kind": "serviceVar",
                            "service": name,
                            "key": key,
                            "reason": "Secret value.",
                        }),
                    );
                }
            }
        }
    }
    if github_needed {
        let already = plan
            .needs_input
            .iter()
            .any(|e| e.get("kind").and_then(Value::as_str) == Some("github"));
        if !already {
            push_need(
                &mut plan,
                json!({
                    "kind": "github",
                    "reason": "Connect GitHub again on the new server.",
                }),
            );
        }
    }

    // ---- backup destinations
    for d in doc
        .pointer("/backups/destinations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = text(d, "id") else {
            continue;
        };
        let name = text(d, "name").unwrap_or(id);
        if existing_dests.contains(id) || existing_dest_names.contains(name) {
            push_skip(
                &mut plan,
                "backupDestination",
                name,
                "A backup destination with this id or name already exists.",
            );
            continue;
        }
        push_create(&mut plan, "backupDestinations", name);
        let needs_secret = bool_val(d, "hasSecrets", false) && !opened_dests.contains(id);
        if needs_secret {
            let already = plan.needs_input.iter().any(|e| {
                e.get("kind").and_then(Value::as_str) == Some("backupDestination")
                    && (text(e, "id") == Some(id) || text(e, "name") == Some(name))
            });
            if !already {
                push_need(
                    &mut plan,
                    json!({
                        "kind": "backupDestination",
                        "id": id,
                        "name": name,
                        "reason": "Storage keys and passwords are secrets.",
                    }),
                );
            }
        }
    }

    // ---- backup jobs (by name; skip if a job with the same name exists)
    let existing_job_names: HashSet<String> = store
        .list_backup_jobs()
        .map_err(store_err)?
        .into_iter()
        .map(|j| j.name)
        .collect();
    for j in doc
        .pointer("/backups/jobs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = text(j, "name").filter(|s| !s.is_empty()) else {
            continue;
        };
        if existing_job_names.contains(name) {
            push_skip(
                &mut plan,
                "backupJob",
                name,
                "A backup job with this name already exists.",
            );
        } else {
            push_create(&mut plan, "backupJobs", name);
        }
    }

    // ---- alert rules / channels
    for r in doc
        .pointer("/alerts/rules")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = text(r, "id") else {
            continue;
        };
        let name = text(r, "name").unwrap_or(id);
        if existing_rule_ids.contains(id) {
            push_skip(
                &mut plan,
                "alertRule",
                name,
                "An alert rule with this id already exists.",
            );
        } else {
            push_create(&mut plan, "alertRules", name);
        }
    }
    for c in doc
        .pointer("/alerts/channels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = text(c, "id") else {
            continue;
        };
        if existing_channel_ids.contains(id) {
            push_skip(
                &mut plan,
                "alertChannel",
                id,
                "An alert channel with this id already exists.",
            );
            continue;
        }
        if opened_channels.contains(id) {
            push_create(&mut plan, "alertChannels", id);
        } else {
            let already = plan.needs_input.iter().any(|e| {
                e.get("kind").and_then(Value::as_str) == Some("alertChannel")
                    && text(e, "id") == Some(id)
            });
            if !already {
                push_need(
                    &mut plan,
                    json!({
                        "kind": "alertChannel",
                        "id": id,
                        "reason": "Webhook addresses and tokens are secrets.",
                    }),
                );
            }
        }
    }

    // ---- firewall
    if let Some(rules) = doc.pointer("/firewall/rules").and_then(Value::as_array) {
        for r in rules {
            let port = r.get("port").and_then(Value::as_u64).unwrap_or(0);
            let proto = text(r, "proto").unwrap_or("tcp");
            let from = text(r, "from").unwrap_or("anywhere");
            let label = format!("{port}/{proto} from {from}");
            push_create(&mut plan, "firewallRules", label);
        }
    }

    Ok(plan)
}

fn plan_json(plan: &Plan) -> Value {
    json!({
        "willCreate": plan.will_create,
        "needsInput": plan.needs_input,
        "skipped": plan.skipped,
        "secretsDecrypted": plan.opened.is_some(),
    })
}

async fn verify_password(
    state: &AppState,
    who: &opsbox_auth::Principal,
    client: &Client,
    password: &str,
    action: &str,
) -> Result<(), HttpError> {
    let ok = state
        .auth
        .verify_user_password(&who.user.id, password)
        .await
        .map_err(|_| HttpError::internal())?;
    if !ok {
        state.auth.audit(
            &who.user.actor_label(),
            ActorKind::User,
            Some(&client.ip),
            action,
            None,
            AuditResult::Denied,
            json!({ "reason": "wrong_password" }),
        );
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "Incorrect account password.",
        ));
    }
    Ok(())
}

// -------------------------------------------------------------------------------- preview

/// `POST /api/v1/config/import/preview`
pub(crate) async fn preview(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    ApiJson(body): ApiJson<ImportBody>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    verify_password(&state, &who, &client, &body.password, AUDIT_PREVIEW).await?;
    let passphrase = body.passphrase.as_deref().filter(|p| !p.is_empty());
    let plan = build_plan(&state, &body.document, passphrase)?;
    state.auth.audit(
        &who.user.actor_label(),
        ActorKind::User,
        Some(&client.ip),
        AUDIT_PREVIEW,
        None,
        AuditResult::Ok,
        json!({
            "willCreate": plan.will_create.keys().cloned().collect::<Vec<_>>(),
            "needsInput": plan.needs_input.len(),
            "skipped": plan.skipped.len(),
            "secretsDecrypted": plan.opened.is_some(),
        }),
    );
    Ok(Json(plan_json(&plan)))
}

// ---------------------------------------------------------------------------------- apply

/// `POST /api/v1/config/import/apply`
pub(crate) async fn apply(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<ImportBody>,
) -> Result<(StatusCode, Json<Value>), HttpError> {
    deny_token(&who)?;
    verify_password(&state, &who, &client, &body.password, AUDIT_APPLY).await?;
    let passphrase = body.passphrase.as_deref().filter(|p| !p.is_empty());
    // Fail before starting the task if the file or passphrase is wrong.
    let plan = build_plan(&state, &body.document, passphrase)?;

    let actor = who.user.name.clone();
    let actor_label = who.user.actor_label();
    let ip = client.ip.clone();
    let doc = body.document;
    let opened = plan.opened.clone();
    let state_bg = state.clone();
    let rid_bg = rid.clone();

    let task = state.tasks.start(TASK_KIND, async move {
        let outcome = run_import(&state_bg, &rid_bg, &actor, doc, opened).await;
        match &outcome {
            Ok(o) => {
                state_bg.auth.audit(
                    &actor_label,
                    ActorKind::User,
                    Some(&ip),
                    AUDIT_APPLY,
                    None,
                    AuditResult::Ok,
                    o.result.clone(),
                );
            }
            Err(e) => {
                state_bg.auth.audit(
                    &actor_label,
                    ActorKind::User,
                    Some(&ip),
                    AUDIT_APPLY,
                    None,
                    AuditResult::Failed,
                    json!({ "error": e.0.error.message }),
                );
            }
        }
        outcome
    })?;

    state.auth.audit(
        &who.user.actor_label(),
        ActorKind::User,
        Some(&client.ip),
        AUDIT_APPLY,
        None,
        AuditResult::Ok,
        json!({ "task": task, "started": true }),
    );

    Ok((StatusCode::ACCEPTED, Json(json!({ "task": task }))))
}

#[allow(clippy::too_many_lines)]
async fn run_import(
    state: &AppState,
    rid: &str,
    actor: &str,
    doc: Value,
    opened: Option<Value>,
) -> Result<Outcome, HttpError> {
    let mut log = Vec::new();
    let mut created = Map::new();
    let mut skipped = Vec::new();
    let store = Arc::clone(&state.store);
    let now = crate::deploy::now_secs();
    let (proj, env) = store.ensure_default_project(now).map_err(store_err)?;

    let opened = opened.unwrap_or(json!({}));
    let var_secrets: HashMap<(String, String), String> = opened
        .get("serviceVars")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            Some((
                (
                    text(v, "service")?.to_owned(),
                    text(v, "key")?.to_owned(),
                ),
                text(v, "value")?.to_owned(),
            ))
        })
        .collect();
    let dest_secrets: HashMap<String, String> = opened
        .get("backupDestinations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let id = text(v, "id")?.to_owned();
            let secrets = v.get("secrets")?;
            let s = if secrets.is_string() {
                secrets.as_str()?.to_owned()
            } else {
                serde_json::to_string(secrets).ok()?
            };
            Some((id, s))
        })
        .collect();
    let channel_secrets: HashMap<String, String> = opened
        .get("alertChannels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            let id = text(v, "id")?.to_owned();
            let cfg = v.get("config")?;
            let s = if cfg.is_string() {
                cfg.as_str()?.to_owned()
            } else {
                serde_json::to_string(cfg).ok()?
            };
            Some((id, s))
        })
        .collect();

    // Map export owner names → local user ids (users themselves are not created).
    let users = store.list_users().map_err(store_err)?;
    let owner_id = |name: Option<&str>| -> Option<String> {
        name.filter(|n| !n.is_empty())
            .and_then(|n| users.iter().find(|u| u.name == n).map(|u| u.id.clone()))
    };

    // ---- compose projects
    let mut stack_ids: HashMap<String, String> = HashMap::new();
    for c in doc
        .get("composeProjects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = text(c, "name").filter(|s| !s.is_empty()) else {
            continue;
        };
        if let Some(existing) = store.get_compose_stack(name).map_err(store_err)? {
            skipped.push(json!({ "kind": "composeProject", "name": name }));
            stack_ids.insert(name.to_owned(), existing.id);
            continue;
        }
        let service_names: Vec<String> = c
            .get("services")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        let depends: Vec<(String, String)> = c
            .get("dependsOn")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|e| {
                Some((
                    text(e, "service")?.to_owned(),
                    text(e, "dependsOn")?.to_owned(),
                ))
            })
            .collect();
        let yaml = text(c, "composeYaml").unwrap_or("");
        let project_name = text(c, "projectName").unwrap_or(name);
        let created_stack = store
            .create_compose_stack_if_absent(UpsertComposeStack {
                project_id: &proj.id,
                env_id: &env.id,
                name,
                owner_id: owner_id(text(c, "owner")).as_deref(),
                compose_yaml: yaml,
                project_name,
                service_names: &service_names,
                depends_edges: &depends,
                confirmed: bool_val(c, "confirmed", false),
                now,
            })
            .map_err(store_err)?;
        match created_stack {
            Some(rec) => {
                stack_ids.insert(name.to_owned(), rec.id);
                push_created(&mut created, "composeProjects", name);
                log.push(format!("Created compose project `{name}`."));
                state.tasks.progress(TASK_KIND, log.clone());
            }
            None => {
                skipped.push(json!({ "kind": "composeProject", "name": name }));
                if let Some(existing) = store.get_compose_stack(name).map_err(store_err)? {
                    stack_ids.insert(name.to_owned(), existing.id);
                }
            }
        }
    }

    // ---- services (create rows; vars in a second pass)
    let mut service_ids: HashMap<String, String> = HashMap::new();
    let mut missing_secret_services: HashSet<String> = HashSet::new();
    let services_arr = doc
        .get("services")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let existing_count = store
        .list_services(&proj.id, &env.id)
        .map_err(store_err)?
        .len();

    for (idx, svc) in services_arr.iter().enumerate() {
        let Some(name) = text(svc, "name").filter(|s| !s.is_empty()) else {
            continue;
        };
        if let Some(existing) = store.get_service(name).map_err(store_err)? {
            skipped.push(json!({ "kind": "service", "name": name }));
            service_ids.insert(name.to_owned(), existing.id);
            continue;
        }

        let kind = ServiceKind::parse(text(svc, "kind").unwrap_or("app")).unwrap_or(ServiceKind::App);
        let source_kind = SourceKind::parse(
            svc.pointer("/source/kind")
                .and_then(Value::as_str)
                .unwrap_or("git_url"),
        )
        .unwrap_or(SourceKind::GitUrl);
        let repo = svc
            .pointer("/source/repo")
            .and_then(Value::as_str)
            .unwrap_or("");
        let image_ref = svc
            .pointer("/source/imageRef")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        // Deploy keys are never imported; clear the reference.
        let deploy_key_id: Option<&str> = None;
        let branch = svc
            .pointer("/source/branch")
            .or_else(|| svc.get("branch"))
            .and_then(Value::as_str)
            .unwrap_or(if kind == ServiceKind::Db { "16" } else { "main" });
        let build_method = svc
            .pointer("/build/method")
            .and_then(Value::as_str)
            .unwrap_or(if source_kind == SourceKind::DockerImage {
                "Docker image"
            } else {
                "auto"
            });
        let dockerfile_path = svc
            .pointer("/build/dockerfilePath")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let start_cmd = svc
            .pointer("/build/startCmd")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        let port = svc
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(3000);
        let health_path = text(svc, "healthPath").unwrap_or("/");
        let domain = text(svc, "domain").filter(|s| !s.is_empty());
        let auto_deploy = bool_val(svc, "autoDeploy", false);
        let memory_limit_mb = svc
            .pointer("/limits/memoryMb")
            .and_then(Value::as_i64);
        let cpu_limit_millis = svc
            .pointer("/limits/cpuMillis")
            .and_then(Value::as_i64);
        let graph_x = svc
            .pointer("/graph/x")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| crate::deploy::next_graph_pos(existing_count + idx).0);
        let graph_y = svc
            .pointer("/graph/y")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| crate::deploy::next_graph_pos(existing_count + idx).1);
        let note = text(svc, "note").filter(|s| !s.is_empty());
        let template_id = text(svc, "templateId").filter(|s| !s.is_empty());
        let compose_name = text(svc, "composeStack").filter(|s| !s.is_empty());
        let compose_stack_id = compose_name.and_then(|n| stack_ids.get(n).map(String::as_str));
        let internal_only = svc
            .pointer("/exposure/internalOnly")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let public_port = svc
            .pointer("/exposure/publicPort")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Any secret var without a decrypted value → do not deploy.
        let needs_secret = svc
            .get("vars")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .any(|v| {
                let key = text(v, "key").unwrap_or("");
                bool_val(v, "secret", false)
                    && text(v, "ref").is_none_or(|s| s.is_empty())
                    && v.get("value").is_none_or(Value::is_null)
                    && !var_secrets.contains_key(&(name.to_owned(), key.to_owned()))
            });
        if needs_secret {
            missing_secret_services.insert(name.to_owned());
        }

        if let Err(e) = state.priv_client.deploy_ensure_dir(rid, actor, name).await {
            log.push(format!("Could not prepare directory for `{name}`: {e}"));
            state.tasks.progress(TASK_KIND, log.clone());
            continue;
        }

        let rec = store
            .create_service_with_exposure(
                NewService {
                    project_id: &proj.id,
                    env_id: &env.id,
                    name,
                    kind,
                    owner_id: owner_id(text(svc, "owner")).as_deref(),
                    source_kind,
                    repo,
                    branch,
                    build_method,
                    dockerfile_path,
                    start_cmd,
                    port,
                    health_path,
                    domain,
                    image_ref,
                    deploy_key_id,
                    auto_deploy: auto_deploy && !needs_secret,
                    memory_limit_mb,
                    cpu_limit_millis,
                    graph_x,
                    graph_y,
                    note,
                    now,
                },
                compose_stack_id,
                template_id,
                internal_only,
                public_port,
            )
            .map_err(store_err)?;
        service_ids.insert(name.to_owned(), rec.id.clone());
        push_created(&mut created, "services", name);
        if kind == ServiceKind::Db {
            push_created(&mut created, "databases", name);
        }
        if template_id.is_some() {
            push_created(&mut created, "templates", name);
        }
        log.push(if needs_secret {
            format!("Created service `{name}` (needs secret; not deployed).")
        } else {
            format!("Created service `{name}` (not deployed; review then deploy).")
        });
        state.tasks.progress(TASK_KIND, log.clone());

        // Extra domains (primary domain is on the service row).
        let mut domains: Vec<NewServiceDomain<'_>> = Vec::new();
        let domain_owned: Vec<(String, Option<String>, bool, bool, bool)> = svc
            .get("domains")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|d| {
                let dom = text(d, "domain")?.to_owned();
                if domain == Some(dom.as_str()) {
                    return None;
                }
                Some((
                    dom,
                    text(d, "redirectTo").map(str::to_owned),
                    bool_val(d, "primary", false),
                    bool_val(d, "forceHttps", true),
                    bool_val(d, "hsts", false),
                ))
            })
            .collect();
        for (dom, redir, primary, force_https, hsts) in &domain_owned {
            if store.get_service_domain(dom).map_err(store_err)?.is_some() {
                skipped.push(json!({ "kind": "domain", "name": dom }));
                continue;
            }
            domains.push(NewServiceDomain {
                domain: dom,
                is_primary: *primary,
                redirect_to: redir.as_deref(),
                force_https: *force_https,
                hsts: *hsts,
            });
        }
        if !domains.is_empty() {
            let _ = store.add_service_domains(&rec.id, &domains, now);
        }
    }

    // ---- variables (after all services exist, so refs resolve)
    for svc in &services_arr {
        let Some(name) = text(svc, "name") else {
            continue;
        };
        let Some(sid) = service_ids.get(name).cloned() else {
            continue;
        };
        let Some(rec) = store.get_service(&sid).map_err(store_err)? else {
            continue;
        };
        let mut to_store: Vec<(String, String, bool)> = Vec::new();
        for var in svc
            .get("vars")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(key) = text(var, "key") else {
                continue;
            };
            let is_secret = bool_val(var, "secret", false);
            if let Some(r) = text(var, "ref").filter(|s| !s.is_empty()) {
                to_store.push((key.to_owned(), r.to_owned(), is_secret));
                continue;
            }
            if let Some(v) = var.get("value").and_then(Value::as_str) {
                to_store.push((key.to_owned(), v.to_owned(), is_secret));
                continue;
            }
            if let Some(v) = var_secrets.get(&(name.to_owned(), key.to_owned())) {
                to_store.push((key.to_owned(), v.clone(), true));
            }
            // Missing secret: leave unset; listed under needsInput in the preview.
        }
        if !to_store.is_empty() {
            if let Err(e) =
                crate::deploy::seal_and_store_vars(state, rid, actor, &rec, &to_store, now).await
            {
                log.push(format!("Variables for `{name}`: {}", e.0.error.message));
            }
        }
    }

    // ---- database settings and app links
    for db in doc
        .get("databases")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(svc_name) = text(db, "service") else {
            continue;
        };
        let Some(sid) = service_ids.get(svc_name).cloned() else {
            continue;
        };
        if let Some(vol) = text(db, "dataVolume").filter(|s| !s.is_empty()) {
            let _ = store.set_database_data_volume(&sid, Some(vol), now);
        }
        let pooler_on = db
            .pointer("/pooler/enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let pooler_mode = db
            .pointer("/pooler/mode")
            .and_then(Value::as_str)
            .unwrap_or("transaction");
        let _ = store.set_database_pooler(&sid, pooler_on, None, pooler_mode, now);
        for app in db
            .get("apps")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(app_name) = text(app, "app") else {
                continue;
            };
            let Some(app_id) = service_ids.get(app_name).cloned() else {
                continue;
            };
            let db_name = text(app, "database").unwrap_or(app_name);
            let role = text(app, "user").unwrap_or(app_name);
            let prefix = text(app, "varPrefix").unwrap_or("");
            let pooled = bool_val(app, "pooled", false);
            let _ = store.upsert_database_app(&sid, &app_id, db_name, role, prefix, pooled, now);
        }
    }

    // ---- backup destinations
    let mut dest_id_map: HashMap<String, String> = HashMap::new();
    for d in store.list_backup_destinations().map_err(store_err)? {
        dest_id_map.insert(d.id.clone(), d.id.clone());
    }
    for d in doc
        .pointer("/backups/destinations")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = text(d, "id") else {
            continue;
        };
        let name = text(d, "name").unwrap_or(id);
        if store.get_backup_destination(id).map_err(store_err)?.is_some()
            || store
                .list_backup_destinations()
                .map_err(store_err)?
                .iter()
                .any(|x| x.name == name)
        {
            skipped.push(json!({ "kind": "backupDestination", "name": name }));
            dest_id_map.insert(id.to_owned(), id.to_owned());
            continue;
        }
        let provider = text(d, "provider").unwrap_or("s3");
        let config = d.get("config").cloned().unwrap_or(json!({}));
        let sealed = if let Some(plain) = dest_secrets.get(id) {
            match state
                .priv_client
                .secret_seal(rid, Some(actor), &format!("backup:dest:{id}"), Secret::new(plain.clone()))
                .await
            {
                Ok(s) => Some(s),
                Err(_) => None,
            }
        } else {
            None
        };
        match store.create_backup_destination(id, provider, name, &config, sealed.as_deref(), None, now)
        {
            Ok(_) => {
                dest_id_map.insert(id.to_owned(), id.to_owned());
                push_created(&mut created, "backupDestinations", name);
                log.push(format!("Created backup destination `{name}`."));
                state.tasks.progress(TASK_KIND, log.clone());
            }
            Err(_) => {
                skipped.push(json!({ "kind": "backupDestination", "name": name }));
            }
        }
    }

    // ---- backup jobs
    let existing_job_names: HashSet<String> = store
        .list_backup_jobs()
        .map_err(store_err)?
        .into_iter()
        .map(|j| j.name)
        .collect();
    for j in doc
        .pointer("/backups/jobs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(name) = text(j, "name").filter(|s| !s.is_empty()) else {
            continue;
        };
        if existing_job_names.contains(name) {
            skipped.push(json!({ "kind": "backupJob", "name": name }));
            continue;
        }
        let kind = BackupTargetKind::parse(text(j, "kind").unwrap_or("database"))
            .unwrap_or(BackupTargetKind::Database);
        let what = j.get("what").cloned().unwrap_or(json!({}));
        let service_id = text(j, "service")
            .and_then(|n| service_ids.get(n).cloned());
        let template_id = text(j, "templateId").filter(|s| !s.is_empty());
        let method = text(j, "method").unwrap_or("dump");
        let cron = text(j, "cron").unwrap_or("0 3 * * *");
        let retain_daily = j.get("retainDaily").and_then(Value::as_i64).unwrap_or(7);
        let retain_weekly = j.get("retainWeekly").and_then(Value::as_i64).unwrap_or(4);
        let dest_export = text(j, "destinationId").unwrap_or("local");
        let destination_id = dest_id_map
            .get(dest_export)
            .map(String::as_str)
            .unwrap_or(dest_export);
        let enabled = bool_val(j, "enabled", true);
        match store.create_backup_job(NewBackupJob {
            name,
            kind,
            what: &what,
            service_id: service_id.as_deref(),
            template_id,
            method,
            cron,
            retain_daily,
            retain_weekly,
            destination_id,
            enabled,
            next_run_at: None,
            now,
        }) {
            Ok(_) => {
                push_created(&mut created, "backupJobs", name);
                log.push(format!("Created backup job `{name}`."));
                state.tasks.progress(TASK_KIND, log.clone());
            }
            Err(_) => skipped.push(json!({ "kind": "backupJob", "name": name })),
        }
    }

    // ---- alerts: merge rules and channels; never overwrite
    {
        let mut engine: opsbox_alerts::Engine = store
            .get_setting("alerts.engine.v1")
            .map_err(store_err)?
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| HttpError::internal())?
            .unwrap_or_default();
        let mut dirty = false;
        for r in doc
            .pointer("/alerts/rules")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(id) = text(r, "id") else {
                continue;
            };
            if engine.rules.iter().any(|x| x.id == id) {
                skipped.push(json!({
                    "kind": "alertRule",
                    "name": text(r, "name").unwrap_or(id)
                }));
                continue;
            }
            let name = text(r, "name").unwrap_or(id).to_owned();
            engine.rules.push(opsbox_alerts::Rule {
                id: id.to_owned(),
                name: name.clone(),
                cond: text(r, "condition").unwrap_or("").to_owned(),
                level: text(r, "level").unwrap_or("warning").to_owned(),
                on: bool_val(r, "enabled", true),
                duration: r.get("durationSecs").and_then(Value::as_i64).unwrap_or(0),
            });
            push_created(&mut created, "alertRules", name);
            dirty = true;
        }
        for (id, plain) in &channel_secrets {
            if engine.channels.contains_key(id) {
                skipped.push(json!({ "kind": "alertChannel", "name": id }));
                continue;
            }
            let sealed = match state
                .priv_client
                .secret_seal(
                    rid,
                    Some(actor),
                    &format!("alerts:channel:{id}"),
                    Secret::new(plain.clone()),
                )
                .await
            {
                Ok(s) => s,
                Err(_) => continue,
            };
            let verified = doc
                .pointer("/alerts/channels")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|c| text(c, "id") == Some(id.as_str()))
                .map(|c| bool_val(c, "verified", false))
                .unwrap_or(false);
            engine.channels.insert(
                id.clone(),
                opsbox_alerts::Channel {
                    id: id.clone(),
                    sealed,
                    verified,
                    error: None,
                },
            );
            push_created(&mut created, "alertChannels", id.clone());
            dirty = true;
        }
        if dirty {
            store
                .set_setting("alerts.engine.v1", &serde_json::to_value(&engine).unwrap())
                .map_err(store_err)?;
            log.push("Updated alert rules and channels.".into());
            state.tasks.progress(TASK_KIND, log.clone());
        }
    }

    // ---- firewall (best effort; never fail the whole import)
    if let Some(rules) = doc.pointer("/firewall/rules").and_then(Value::as_array) {
        let existing = patient(state, 20)
            .firewall_list(rid)
            .await
            .ok();
        for r in rules {
            let port = match r
                .get("port")
                .and_then(Value::as_u64)
                .and_then(|p| u16::try_from(p).ok())
            {
                Some(p) => p,
                None => continue,
            };
            let proto = text(r, "proto").unwrap_or("tcp").to_owned();
            let from = text(r, "from").unwrap_or("anywhere").to_owned();
            let note = text(r, "note").unwrap_or("").to_owned();
            let label = format!("{port}/{proto} from {from}");
            let already = existing.as_ref().is_some_and(|f| {
                f.rules.iter().any(|er| {
                    er.port == port
                        && er.proto.eq_ignore_ascii_case(&proto)
                        && er.src == from
                })
            });
            if already {
                skipped.push(json!({ "kind": "firewallRule", "name": label }));
                continue;
            }
            match patient(state, 90)
                .firewall_add(
                    rid,
                    actor,
                    NewRule {
                        port,
                        proto,
                        src: from,
                        note,
                    },
                    None,
                    true,
                )
                .await
            {
                Ok(_) => {
                    push_created(&mut created, "firewallRules", label.clone());
                    log.push(format!("Added firewall rule {label}."));
                    state.tasks.progress(TASK_KIND, log.clone());
                }
                Err(e) => {
                    log.push(format!("Firewall rule {label} skipped: {e}"));
                    state.tasks.progress(TASK_KIND, log.clone());
                }
            }
        }
    }

    let needs_secret: Vec<String> = missing_secret_services.into_iter().collect();
    if !needs_secret.is_empty() {
        log.push(format!(
            "Services that still need secrets (not deployed): {}.",
            needs_secret.join(", ")
        ));
    }
    log.push("Import finished. Existing objects were left unchanged.".into());

    Ok(Outcome {
        log,
        result: json!({
            "created": created,
            "skipped": skipped,
            "needsSecret": needs_secret,
        }),
    })
}

fn push_created(created: &mut Map<String, Value>, section: &str, name: impl Into<String>) {
    let arr = created
        .entry(section.to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    if let Some(a) = arr.as_array_mut() {
        a.push(Value::String(name.into()));
    }
}
