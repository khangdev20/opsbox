//! Database management for PostgreSQL services (docs/BACKEND_SPEC.md 7.6, Databases part B):
//! the Database page, one login per connected app, extensions, PgBouncer, major version
//! upgrades with a snapshot first, and connection strings behind a fresh two-factor code.
//!
//! `serve` only validates, keeps metadata in SQLite and relays sealed values; every SQL
//! statement and container action runs in `opsbox-priv` (`PrivOp::Database*`).

use axum::Json;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use opsbox_deploy::refs::{VarRef, canonical_ref_expr};
use opsbox_deploy::templates::{get_template, is_postgres_template, template_volume_name};
use opsbox_proto::{
    DatabaseAction, DatabaseInspectRequest, DatabaseManageRequest, DatabasePoolerRequest,
    DatabaseUpgradeRequest, DbTarget, ErrorCode, Secret, allowed_extension, app_connect_pairs,
    app_pg_name, app_var_prefix, pg_major, valid_db_identifier, valid_pg_name,
};
use opsbox_store::{DatabaseAppRecord, ServiceKind, ServiceRecord};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::deploy::{
    audit_user, forbidden, invalid, now_secs, service_env_specs, service_readable,
    service_writable, var_seal_context,
};
use crate::json::ApiJson;
use crate::session::{Client, Deployer, deny_token};
use crate::tasks::Outcome;
use crate::{AppState, HttpError, RequestId};

pub(crate) const POOLER_PORT: u16 = 6432;

fn pooler_host(db: &ServiceRecord) -> String {
    format!("{}-pooler", db.name)
}

fn store_err<E>(_: E) -> HttpError {
    HttpError::internal()
}

/// Load a PostgreSQL database service the caller may read or change.
fn load_db(
    state: &AppState,
    who: &opsbox_auth::Principal,
    id: &str,
    write: bool,
) -> Result<ServiceRecord, HttpError> {
    let svc = state
        .store
        .get_service(id.trim())
        .map_err(store_err)?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "Database not found."))?;
    let allowed = if write {
        service_writable(who, &svc)
    } else {
        service_readable(who, &svc)
    };
    if !allowed {
        return Err(forbidden("You do not have access to this database."));
    }
    if svc.kind != ServiceKind::Db || !svc.template_id.as_deref().is_some_and(is_postgres_template)
    {
        return Err(invalid(
            "database",
            "Management is available for PostgreSQL databases.",
        ));
    }
    Ok(svc)
}

fn running_target(state: &AppState, svc: &ServiceRecord) -> Result<DbTarget, HttpError> {
    let container = svc.active_container.clone().ok_or_else(|| {
        HttpError::new(
            ErrorCode::Conflict,
            "Start this database first. It is not running.",
        )
    })?;
    Ok(DbTarget {
        service_id: svc.id.clone(),
        service_name: svc.name.clone(),
        template_id: svc.template_id.clone().unwrap_or_default(),
        container_id: container,
        vars: service_env_specs(state, &svc.id),
    })
}

fn project_for(
    state: &AppState,
    db: &ServiceRecord,
) -> Result<opsbox_store::ProjectRecord, HttpError> {
    state
        .store
        .resolve_project(&db.project_id, now_secs())
        .map_err(store_err)?
        .map(|(p, _)| p)
        .ok_or_else(HttpError::internal)
}

fn guard_idle(state: &AppState, svc: &ServiceRecord) -> Result<(), HttpError> {
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
            "Wait for the running deploy or change on this database to finish.",
        ));
    }
    Ok(())
}

/// Store sealed variables returned by `priv` on the database service.
fn store_specs(
    state: &AppState,
    db: &ServiceRecord,
    specs: &[opsbox_proto::DeployEnvSpec],
    now: i64,
) -> Result<Vec<String>, HttpError> {
    let mut keys = Vec::with_capacity(specs.len());
    for spec in specs {
        state
            .store
            .upsert_service_var(&db.id, &spec.key, &spec.value_enc, spec.is_secret, now)
            .map_err(store_err)?;
        keys.push(spec.key.clone());
    }
    Ok(keys)
}

fn app_names(app: &ServiceRecord) -> Result<(String, String), HttpError> {
    let name = app_pg_name(&app.name);
    if !valid_pg_name(&name) {
        return Err(invalid(
            "app",
            "This app's name is too long to get its own database login (59 characters at most).",
        ));
    }
    Ok((name, app_var_prefix(&app.name)))
}

/// Create (once) the database and least-privilege login for `app` on `db`.
pub(crate) async fn ensure_app_login(
    state: &AppState,
    rid: &str,
    actor: &str,
    db: &ServiceRecord,
    app: &ServiceRecord,
) -> Result<DatabaseAppRecord, HttpError> {
    if let Some(existing) = state
        .store
        .get_database_app(&db.id, &app.id)
        .map_err(store_err)?
    {
        return Ok(existing);
    }
    let (pg_name, prefix) = app_names(app)?;
    let target = running_target(state, db)?;
    let done = state
        .priv_client
        .database_manage(
            rid,
            actor,
            DatabaseManageRequest {
                target,
                action: DatabaseAction::ProvisionApp {
                    app_service: app.name.clone(),
                    db_name: pg_name.clone(),
                    role: pg_name.clone(),
                    var_prefix: prefix.clone(),
                    pooler_host: pooler_host(db),
                    pooler_port: POOLER_PORT,
                },
            },
        )
        .await?;
    let now = now_secs();
    store_specs(state, db, &done.vars, now)?;
    state
        .store
        .upsert_database_app(&db.id, &app.id, &pg_name, &pg_name, &prefix, false, now)
        .map_err(store_err)?;
    state
        .store
        .get_database_app(&db.id, &app.id)
        .map_err(store_err)?
        .ok_or_else(HttpError::internal)
}

/// `(variable the app sees, key on the database service)` for an app login.
pub(crate) fn app_link_pairs(rec: &DatabaseAppRecord, pooled: bool) -> Vec<(String, String)> {
    app_connect_pairs(pooled)
        .into_iter()
        .map(|(env, suffix)| (env.to_owned(), format!("{}{suffix}", rec.var_prefix)))
        .collect()
}

/// Point an app's variables at its login on `db` (direct or through the pooler).
async fn link_app_vars(
    state: &AppState,
    rid: &str,
    actor: &str,
    db: &ServiceRecord,
    app: &ServiceRecord,
    pairs: &[(String, String)],
) -> Result<Vec<Value>, HttpError> {
    let db_vars = state.store.list_service_vars(&db.id).map_err(store_err)?;
    let now = now_secs();
    let mut linked = Vec::new();
    for (env_key, src_key) in pairs {
        let Some(src) = db_vars.iter().find(|v| &v.key == src_key) else {
            continue;
        };
        let ref_expr = canonical_ref_expr(&VarRef {
            service: db.name.clone(),
            key: src_key.clone(),
        });
        let sealed = state
            .priv_client
            .secret_seal(
                rid,
                Some(actor),
                &var_seal_context(&app.id, env_key),
                Secret::new(ref_expr.clone()),
            )
            .await?;
        state
            .store
            .upsert_service_var_with_refs(
                &app.id,
                env_key,
                &sealed,
                src.is_secret,
                Some(&ref_expr),
                &[(db.id.clone(), src_key.clone())],
                now,
            )
            .map_err(store_err)?;
        linked.push(json!([env_key, ref_expr, src.is_secret, true]));
    }
    Ok(linked)
}

fn affected_json(
    who: &opsbox_auth::Principal,
    affected: Vec<opsbox_store::ReferencingServiceInfo>,
) -> Vec<Value> {
    affected
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
        .collect()
}

// -------------------------------------------------------------------------------- overview

#[derive(Deserialize)]
pub(crate) struct OverviewQuery {
    database: Option<String>,
}

fn live_json(l: &opsbox_proto::DatabaseInspectResult) -> Value {
    json!({
        "serverVersion": l.server_version,
        "database": l.database,
        "databases": l.databases.iter().map(|d| json!({"name": d.name, "owner": d.owner, "sizeBytes": d.size_bytes})).collect::<Vec<_>>(),
        "users": l.users.iter().map(|u| json!({
            "name": u.name, "canLogin": u.can_login, "superuser": u.superuser,
            "connections": u.connections, "databases": u.databases,
        })).collect::<Vec<_>>(),
        "grants": l.grants.iter().filter(|g| g.connect).map(|g| json!({
            "role": g.role, "database": g.database, "connect": g.connect, "create": g.create, "temp": g.temp,
        })).collect::<Vec<_>>(),
        "extensions": l.extensions.iter().map(|e| json!({
            "name": e.name, "installed": e.installed, "installedVersion": e.installed_version,
            "defaultVersion": e.default_version, "comment": e.comment,
        })).collect::<Vec<_>>(),
        "tables": l.tables.iter().map(|t| json!({
            "schema": t.schema, "table": t.table, "totalBytes": t.total_bytes, "rowsEstimate": t.rows_estimate,
        })).collect::<Vec<_>>(),
        "connections": { "current": l.connections_current, "max": l.connections_max },
        "statStatements": { "loaded": l.stat_statements_loaded, "enabled": l.stat_statements_enabled },
        "slowQueries": l.slow_queries.iter().map(|q| json!({
            "query": q.query, "calls": q.calls, "meanMs": q.mean_ms, "totalMs": q.total_ms, "rows": q.rows,
        })).collect::<Vec<_>>(),
    })
}

/// `GET /api/v1/databases/{id}`: databases, users, grants, extensions, table sizes,
/// connection count and slow queries, plus apps, pooler and upgrade options.
pub(crate) async fn overview(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    Query(q): Query<OverviewQuery>,
) -> Result<Json<Value>, HttpError> {
    let svc = load_db(&state, &who, &id, false)?;
    let tmpl_id = svc.template_id.clone().unwrap_or_default();
    let tmpl = get_template(&tmpl_id);
    let settings = state
        .store
        .get_database_settings(&svc.id)
        .map_err(store_err)?;
    let apps = state.store.list_database_apps(&svc.id).map_err(store_err)?;

    let mut live = Value::Null;
    let mut live_error = Value::Null;
    if let Some(database) = q.database.as_deref()
        && !valid_db_identifier(database)
    {
        return Err(invalid("database", "Invalid database name."));
    }
    match running_target(&state, &svc) {
        Ok(target) => {
            match state
                .priv_client
                .database_inspect(
                    &rid,
                    &who.user.name,
                    DatabaseInspectRequest {
                        target,
                        database: q.database.clone(),
                    },
                )
                .await
            {
                Ok(l) => live = live_json(&l),
                Err(e) => live_error = json!(HttpError::from(e).0.error.message),
            }
        }
        Err(e) => live_error = json!(e.0.error.message),
    }

    let current_major = pg_major(&svc.branch).unwrap_or(0);
    let mut upgrades: Vec<Value> = Vec::new();
    if let Some(t) = tmpl {
        let mut seen = std::collections::HashSet::new();
        for v in &t.versions {
            if let Some(m) = pg_major(v)
                && m > current_major
                && seen.insert(m)
            {
                upgrades.push(json!({ "version": v, "major": m }));
            }
        }
    }

    let mut app_rows = Vec::new();
    for a in &apps {
        let app_name = state
            .store
            .get_service(&a.app_service_id)
            .map_err(store_err)?
            .map(|s| s.name)
            .unwrap_or_default();
        let conns = live
            .get("users")
            .and_then(Value::as_array)
            .and_then(|u| u.iter().find(|u| u["name"] == a.role_name.as_str()))
            .and_then(|u| u["connections"].as_u64());
        app_rows.push(json!({
            "app": app_name,
            "database": a.db_name,
            "user": a.role_name,
            "pooled": a.pooled,
            "connections": conns,
        }));
    }

    Ok(Json(json!({
        "id": svc.name,
        "name": svc.name,
        "templateId": tmpl_id,
        "flavor": tmpl.map(|t| t.name.clone()),
        "version": svc.branch,
        "state": svc.state.as_str(),
        "running": svc.active_container.is_some(),
        "live": live,
        "liveError": live_error,
        "apps": app_rows,
        "pooler": {
            "enabled": settings.pooler_enabled,
            "mode": settings.pooler_mode,
            "host": pooler_host(&svc),
            "port": POOLER_PORT,
        },
        "upgrades": upgrades,
        "dataVolume": settings
            .data_volume
            .clone()
            .unwrap_or_else(|| template_volume_name(&svc.name, "data")),
        "credentialsRotatedAt": svc.credentials_rotated_at.map(|t| t.saturating_mul(1000)),
    })))
}

// ------------------------------------------------------------------------------ app logins

/// `POST /api/v1/databases/{id}/apps/{app}/rotate`: new password for one app's login.
pub(crate) async fn rotate_app(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path((id, app)): Path<(String, String)>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let db = load_db(&state, &who, &id, true)?;
    let app_svc = state
        .store
        .get_service(app.trim())
        .map_err(store_err)?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "App not found."))?;
    let rec = state
        .store
        .get_database_app(&db.id, &app_svc.id)
        .map_err(store_err)?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "That app has no login here."))?;
    guard_idle(&state, &db)?;
    let target = running_target(&state, &db)?;
    let done = state
        .priv_client
        .database_manage(
            &rid,
            &who.user.name,
            DatabaseManageRequest {
                target,
                action: DatabaseAction::RotateApp {
                    app_service: app_svc.name.clone(),
                    db_name: rec.db_name.clone(),
                    role: rec.role_name.clone(),
                    var_prefix: rec.var_prefix.clone(),
                    pooler_host: pooler_host(&db),
                    pooler_port: POOLER_PORT,
                },
            },
        )
        .await?;
    let now = now_secs();
    let keys = store_specs(&state, &db, &done.vars, now)?;
    let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();
    let affected = state
        .store
        .mark_credentials_rotated(&db.id, &key_refs, now)
        .map_err(store_err)?;
    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "database.app.rotate",
        Some(&db.name),
        json!({ "app": app_svc.name, "user": rec.role_name }),
    );
    Ok(Json(json!({
        "ok": true,
        "app": app_svc.name,
        "rotatedAt": now.saturating_mul(1000),
        "affectedApps": affected_json(&who, affected),
    })))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RemoveAppBody {
    #[serde(default)]
    drop_database: bool,
    /// Must equal the database name when `drop_database` is set.
    confirm: Option<String>,
}

/// `POST /api/v1/databases/{id}/apps/{app}/remove`: remove an app's login, optionally with
/// its database (typed confirmation).
pub(crate) async fn remove_app(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path((id, app)): Path<(String, String)>,
    ApiJson(body): ApiJson<RemoveAppBody>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let db = load_db(&state, &who, &id, true)?;
    let app_svc = state
        .store
        .get_service(app.trim())
        .map_err(store_err)?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "App not found."))?;
    let rec = state
        .store
        .get_database_app(&db.id, &app_svc.id)
        .map_err(store_err)?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "That app has no login here."))?;
    if body.drop_database && body.confirm.as_deref() != Some(rec.db_name.as_str()) {
        return Err(HttpError::new(
            ErrorCode::ConfirmationRequired,
            format!(
                "This permanently deletes the database {} and everything in it. Type its name to confirm.",
                rec.db_name
            ),
        ));
    }
    if !state
        .store
        .list_service_var_refs(&app_svc.id)
        .map_err(store_err)?
        .iter()
        .all(|r| r.target_service_id != db.id)
    {
        return Err(HttpError::new(
            ErrorCode::Conflict,
            format!(
                "{} still uses this login. Disconnect it from the app's Variables first.",
                app_svc.name
            ),
        ));
    }
    guard_idle(&state, &db)?;
    let target = running_target(&state, &db)?;
    let done = state
        .priv_client
        .database_manage(
            &rid,
            &who.user.name,
            DatabaseManageRequest {
                target,
                action: DatabaseAction::DropApp {
                    role: rec.role_name.clone(),
                    db_name: rec.db_name.clone(),
                    drop_database: body.drop_database,
                },
            },
        )
        .await?;
    state
        .store
        .remove_database_app(&db.id, &app_svc.id)
        .map_err(store_err)?;
    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "database.app.remove",
        Some(&db.name),
        json!({ "app": app_svc.name, "user": rec.role_name, "droppedDatabase": body.drop_database }),
    );
    Ok(Json(json!({ "ok": true, "detail": done.detail })))
}

#[derive(Deserialize)]
pub(crate) struct PooledBody {
    pooled: bool,
}

/// `PUT /api/v1/databases/{id}/apps/{app}/pooled`: point an app at the pooler or back at the
/// database directly.
pub(crate) async fn set_app_pooled(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path((id, app)): Path<(String, String)>,
    ApiJson(body): ApiJson<PooledBody>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let db = load_db(&state, &who, &id, true)?;
    let app_svc = state
        .store
        .get_service(app.trim())
        .map_err(store_err)?
        .filter(|a| a.project_id == db.project_id && a.env_id == db.env_id)
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "App not found."))?;
    if !service_writable(&who, &app_svc) {
        return Err(forbidden("You cannot change this app."));
    }
    let rec = state
        .store
        .get_database_app(&db.id, &app_svc.id)
        .map_err(store_err)?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "That app has no login here."))?;
    let settings = state
        .store
        .get_database_settings(&db.id)
        .map_err(store_err)?;
    if body.pooled && !settings.pooler_enabled {
        return Err(HttpError::new(
            ErrorCode::Conflict,
            "Turn on the connection pooler first.",
        ));
    }
    let linked_now = state
        .store
        .list_service_var_refs(&app_svc.id)
        .map_err(store_err)?
        .iter()
        .any(|r| r.target_service_id == db.id);
    state
        .store
        .set_database_app_pooled(&db.id, &app_svc.id, body.pooled, now_secs())
        .map_err(store_err)?;
    let mut vars = Vec::new();
    if linked_now {
        vars = link_app_vars(
            &state,
            &rid,
            &who.user.name,
            &db,
            &app_svc,
            &app_link_pairs(&rec, body.pooled),
        )
        .await?;
    }
    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "database.app.pooled",
        Some(&db.name),
        json!({ "app": app_svc.name, "pooled": body.pooled }),
    );
    Ok(Json(json!({
        "ok": true,
        "pooled": body.pooled,
        "vars": vars,
        "needsRedeploy": linked_now
            && (app_svc.active_deploy_id.is_some() || app_svc.active_container.is_some()),
    })))
}

// ---------------------------------------------------------------------------- extensions

#[derive(Deserialize)]
pub(crate) struct ExtensionBody {
    enabled: bool,
    database: Option<String>,
}

/// `PUT /api/v1/databases/{id}/extensions/{name}`: enable or disable an allowed extension.
pub(crate) async fn set_extension(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path((id, name)): Path<(String, String)>,
    ApiJson(body): ApiJson<ExtensionBody>,
) -> Result<Json<Value>, HttpError> {
    deny_token(&who)?;
    let db = load_db(&state, &who, &id, true)?;
    let name = name.trim().to_owned();
    if !allowed_extension(&name) {
        return Err(invalid(
            "extension",
            "That extension cannot be changed here.",
        ));
    }
    let target = running_target(&state, &db)?;
    let database = match body
        .database
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
    {
        Some(d) if valid_db_identifier(d) => d.to_owned(),
        Some(_) => return Err(invalid("database", "Invalid database name.")),
        None => main_database_name(&state, &db, &rid, &who.user.name).await?,
    };
    let done = state
        .priv_client
        .database_manage(
            &rid,
            &who.user.name,
            DatabaseManageRequest {
                target,
                action: DatabaseAction::SetExtension {
                    database: database.clone(),
                    extension: name.clone(),
                    enable: body.enabled,
                },
            },
        )
        .await?;
    audit_user(
        &state,
        &who.user.name,
        who.via_token,
        Some(&client.ip),
        "database.extension",
        Some(&db.name),
        json!({ "extension": name, "database": database, "enabled": body.enabled }),
    );
    Ok(Json(json!({ "ok": true, "detail": done.detail })))
}

/// Name of the main database (`POSTGRES_DB`, not secret).
async fn main_database_name(
    state: &AppState,
    db: &ServiceRecord,
    rid: &str,
    actor: &str,
) -> Result<String, HttpError> {
    let vars = state.store.list_service_vars(&db.id).map_err(store_err)?;
    if let Some(v) = vars.iter().find(|v| v.key == "POSTGRES_DB") {
        let ctx = var_seal_context(&db.id, &v.key);
        if let Ok(s) = state
            .priv_client
            .secret_open(rid, Some(actor), &ctx, &v.value_enc)
            .await
        {
            let name = s.expose().to_owned();
            if valid_db_identifier(&name) {
                return Ok(name);
            }
        }
    }
    Ok("app".to_owned())
}

// ------------------------------------------------------------------------------- pooler

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PoolerBody {
    enabled: bool,
    mode: Option<String>,
}

/// `PUT /api/v1/databases/{id}/pooler`: start or remove PgBouncer in front of the database.
/// Returns a task id.
pub(crate) async fn set_pooler(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<PoolerBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    deny_token(&who)?;
    let db = load_db(&state, &who, &id, true)?;
    let mode = body.mode.as_deref().unwrap_or("transaction").to_owned();
    if !matches!(mode.as_str(), "transaction" | "session") {
        return Err(invalid("mode", "Pool mode must be transaction or session."));
    }
    let project = project_for(&state, &db)?;
    let target = running_target(&state, &db)?;
    let settings = state
        .store
        .get_database_settings(&db.id)
        .map_err(store_err)?;
    let guard = state
        .deploy_hub
        .try_acquire(&db.id, "pooler")
        .map_err(|_| {
            HttpError::new(
                ErrorCode::Conflict,
                "Another change is running on this database.",
            )
        })?;

    let s = state.clone();
    let actor = who.user.name.clone();
    let ip = client.ip.clone();
    let via_token = who.via_token;
    let who2 = who.clone();
    let task = state
        .tasks
        .start(&format!("database.pooler:{}", db.id), async move {
            let _guard = guard;
            let deploy_id = format!("pool{}", ulid::Ulid::generate().to_string().to_lowercase());
            let done = s
                .priv_client
                .database_pooler(
                    &rid,
                    &actor,
                    DatabasePoolerRequest {
                        target,
                        project: project.name.clone(),
                        enable: body.enabled,
                        pool_mode: mode.clone(),
                        max_client_conn: 200,
                        default_pool_size: 20,
                        existing_container: settings.pooler_container.clone(),
                        deploy_id,
                    },
                )
                .await?;
            let now = now_secs();
            store_specs(&s, &db, &done.vars, now)?;
            if !body.enabled {
                // Apps go back to talking to the database directly.
                for rec in s.store.list_database_apps(&db.id).map_err(store_err)? {
                    if rec.pooled {
                        s.store
                            .set_database_app_pooled(&db.id, &rec.app_service_id, false, now)
                            .map_err(store_err)?;
                        if let Some(app) = s
                            .store
                            .get_service(&rec.app_service_id)
                            .map_err(store_err)?
                        {
                            link_app_vars(
                                &s,
                                &rid,
                                &actor,
                                &db,
                                &app,
                                &app_link_pairs(&rec, false),
                            )
                            .await?;
                        }
                    }
                }
            }
            s.store
                .set_database_pooler(
                    &db.id,
                    body.enabled,
                    done.container_id.as_deref(),
                    &mode,
                    now,
                )
                .map_err(store_err)?;
            s.metrics.notify_docker();
            audit_user(
                &s,
                &who2.user.name,
                via_token,
                Some(&ip),
                if body.enabled {
                    "database.pooler.enable"
                } else {
                    "database.pooler.disable"
                },
                Some(&db.name),
                json!({ "mode": mode }),
            );
            Ok(Outcome {
                log: vec![if body.enabled {
                    "Connection pooler is running.".to_owned()
                } else {
                    "Connection pooler removed.".to_owned()
                }],
                result: json!({ "enabled": body.enabled }),
            })
        })?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({ "task": task })),
    ))
}

// ------------------------------------------------------------------------------- upgrade

#[derive(Deserialize)]
pub(crate) struct UpgradeBody {
    version: String,
    #[serde(default)]
    confirm: bool,
}

/// `POST /api/v1/databases/{id}/upgrade`: major version upgrade. A backup is taken first (in
/// `priv`); nothing changes if it fails. Returns a task id.
pub(crate) async fn upgrade(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<UpgradeBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), HttpError> {
    deny_token(&who)?;
    let db = load_db(&state, &who, &id, true)?;
    let tmpl = get_template(db.template_id.as_deref().unwrap_or_default())
        .ok_or_else(HttpError::internal)?;
    let to_version = body.version.trim().to_owned();
    let (Some(from), Some(to)) = (pg_major(&db.branch), pg_major(&to_version)) else {
        return Err(invalid("version", "Unknown PostgreSQL version."));
    };
    if !tmpl.versions.iter().any(|v| v == &to_version) || to <= from {
        return Err(invalid(
            "version",
            "Choose a newer major version from the list.",
        ));
    }
    if !body.confirm {
        return Err(HttpError::new(
            ErrorCode::ConfirmationRequired,
            format!(
                "{} will be stopped for a few minutes while a safety backup is taken and the data is moved to PostgreSQL {to}. Apps reconnect on their own afterwards. Your current data volume is kept until you delete it.",
                db.name
            ),
        ));
    }
    let project = project_for(&state, &db)?;
    let target = running_target(&state, &db)?;
    let now = now_secs();

    // The snapshot goes through the normal backup job so it appears in Backups and honours
    // the chosen destination. Create the job if this database never had one.
    let job = match state
        .store
        .get_backup_job_for_service(&db.id)
        .map_err(store_err)?
    {
        Some(j) => j,
        None => {
            state
                .store
                .upsert_backup_job(
                    &db.id,
                    db.template_id.as_deref(),
                    &tmpl.backup.method,
                    &tmpl.backup.cron,
                    false,
                    now,
                )
                .map_err(store_err)?;
            let created = state
                .store
                .get_backup_job_for_service(&db.id)
                .map_err(store_err)?
                .ok_or_else(HttpError::internal)?;
            let dest = crate::backups::resolve_default_destination_id(&state.store);
            if dest != opsbox_store::LOCAL_DESTINATION_ID {
                let _ = state
                    .store
                    .set_backup_job_destination(&created.id, &dest, now);
            }
            state
                .store
                .get_backup_job_for_service(&db.id)
                .map_err(store_err)?
                .ok_or_else(HttpError::internal)?
        }
    };
    let snapshot = crate::backups::build_run_spec(&state.store, &job, false, false)?;

    let settings = state
        .store
        .get_database_settings(&db.id)
        .map_err(store_err)?;
    let old_volume = settings
        .data_volume
        .clone()
        .unwrap_or_else(|| template_volume_name(&db.name, "data"));
    let new_volume = format!("opsbox-{}-data-v{to}", db.name);
    let guard = state
        .deploy_hub
        .try_acquire(&db.id, "upgrade")
        .map_err(|_| {
            HttpError::new(
                ErrorCode::Conflict,
                "Another change is running on this database.",
            )
        })?;

    let s = state.clone();
    let actor = who.user.name.clone();
    let ip = client.ip.clone();
    let via_token = who.via_token;
    let image_ref = format!("{}:{to_version}", tmpl.image);
    let task = state.tasks.start(&format!("database.upgrade:{}", db.id), async move {
        let _guard = guard;
        let deploy_id = format!("upg{}", ulid::Ulid::generate().to_string().to_lowercase());
        let res = s
            .priv_client
            .database_upgrade(
                &rid,
                &actor,
                DatabaseUpgradeRequest {
                    target,
                    project: project.name.clone(),
                    deploy_id,
                    from_version: db.branch.clone(),
                    to_version: to_version.clone(),
                    image_ref: image_ref.clone(),
                    internal_only: db.internal_only,
                    public_port: db.public_port,
                    confirm_public_port: db.public_port,
                    memory_limit_mb: db.memory_limit_mb,
                    cpu_limit_millis: db.cpu_limit_millis,
                    old_volume: old_volume.clone(),
                    new_volume: new_volume.clone(),
                    snapshot,
                },
            )
            .await;
        match res {
            Ok(done) => {
                s.store
                    .complete_database_upgrade(
                        &db.id,
                        &to_version,
                        &image_ref,
                        &done.container_id,
                        done.host_port,
                        &done.new_volume,
                        now_secs(),
                    )
                    .map_err(store_err)?;
                crate::backups::sync_all_snapshots(
                    &s.store,
                    &s.priv_client,
                    &rid,
                    Some(&actor),
                    Some(&job.destination_id),
                    None,
                )
                .await;
                let _ = crate::panel::sync_proxy_routes(&s).await;
                s.metrics.notify_docker();
                audit_user(
                    &s,
                    &actor,
                    via_token,
                    Some(&ip),
                    "database.upgrade",
                    Some(&db.name),
                    json!({ "from": db.branch, "to": to_version, "snapshot": done.snapshot_id, "oldVolume": done.old_volume }),
                );
                Ok(Outcome {
                    log: done.log.clone(),
                    result: json!({
                        "version": to_version,
                        "oldVolume": done.old_volume,
                        "databases": done.databases,
                    }),
                })
            }
            Err(e) => {
                let err: HttpError = e.into();
                audit_user(
                    &s,
                    &actor,
                    via_token,
                    Some(&ip),
                    "database.upgrade.failed",
                    Some(&db.name),
                    json!({ "to": to_version, "error": err.0.error.message }),
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

// ------------------------------------------------------------------- connection string

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RevealBody {
    /// Current two-factor code.
    code: String,
    /// App whose login to show; omit for the main administrator login.
    app: Option<String>,
    #[serde(default)]
    pooled: bool,
}

/// `POST /api/v1/databases/{id}/connection-string`: show a connection string after the
/// user re-enters a fresh two-factor code. Every attempt is audited; the response is never
/// cached.
pub(crate) async fn reveal_connection_string(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    Path(id): Path<String>,
    ApiJson(body): ApiJson<RevealBody>,
) -> Result<(HeaderMap, Json<Value>), HttpError> {
    if who.via_token {
        return Err(forbidden(
            "Connection strings can only be shown in the panel, not with an API token.",
        ));
    }
    let db = load_db(&state, &who, &id, true)?;
    let accepted = state
        .auth
        .verify_user_totp(&rid, &who.user.id, &body.code, &client.ip)
        .await?;
    if !accepted {
        audit_user_denied(&state, &who, &client.ip, &db.name);
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "That code was not accepted. Codes work once, so wait for the next one and try again.",
        ));
    }

    let (key, who_label) = match body.app.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        Some(app) => {
            let app_svc = state
                .store
                .get_service(app)
                .map_err(store_err)?
                .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "App not found."))?;
            let rec = state
                .store
                .get_database_app(&db.id, &app_svc.id)
                .map_err(store_err)?
                .ok_or_else(|| {
                    HttpError::new(ErrorCode::NotFound, "That app has no login here.")
                })?;
            let suffix = if body.pooled {
                "POOLED_DATABASE_URL"
            } else {
                "DATABASE_URL"
            };
            (format!("{}{suffix}", rec.var_prefix), app_svc.name)
        }
        None => ("DATABASE_URL".to_owned(), "administrator".to_owned()),
    };
    let var = state
        .store
        .list_service_vars(&db.id)
        .map_err(store_err)?
        .into_iter()
        .find(|v| v.key == key)
        .ok_or_else(|| {
            HttpError::new(
                ErrorCode::NotFound,
                "No connection string is stored for this login.",
            )
        })?;
    let secret = state
        .priv_client
        .secret_open(
            &rid,
            Some(&who.user.name),
            &var_seal_context(&db.id, &key),
            &var.value_enc,
        )
        .await?;
    audit_user(
        &state,
        &who.user.name,
        false,
        Some(&client.ip),
        "database.connection_string.reveal",
        Some(&db.name),
        json!({ "login": who_label, "pooled": body.pooled }),
    );
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok((
        headers,
        Json(json!({
            "connectionString": secret.expose(),
            "login": who_label,
            "note": "This name only works from other containers in the same project. Your database is not reachable from the internet.",
        })),
    ))
}

fn audit_user_denied(state: &AppState, who: &opsbox_auth::Principal, ip: &str, db: &str) {
    use opsbox_store::{ActorKind, AuditResult, NewAudit};
    let _ = state.store.append_audit(NewAudit {
        actor: who.user.name.clone(),
        actor_kind: ActorKind::User,
        ip: Some(ip.to_owned()),
        action: "database.connection_string.denied".to_owned(),
        target: Some(db.to_owned()),
        result: AuditResult::Denied,
        detail: json!({ "reason": "bad_code" }),
    });
}
