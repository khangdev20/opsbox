//! `POST /api/v1/config/export`: the whole configuration as one versioned JSON document
//! (`"format": "opsbox-config/1"`, schema in `docs/schemas/config-export-v1.json`), for a
//! person who wants a copy or a new server (the import is a later prompt).
//!
//! Safe by default: no secret is in the file. Everything that would have been one is listed
//! under `secrets.needsReentry`. If the caller types a passphrase, the secrets that opsbox
//! can read back (secret environment variables, backup destination credentials, alert
//! channel settings) go into one sealed block: Argon2id turns the passphrase into a key,
//! XChaCha20-Poly1305 seals the block. The passphrase is only ever held for the length of
//! the request; it is not stored, logged or audited.
//!
//! Admin only, an interactive session only (not an API token), and the account password is
//! asked for again. It is a `POST` so neither the password nor the passphrase ends up in a
//! URL or an access log.

use axum::extract::{Extension, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD as B64;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{KeyInit, XChaCha20Poly1305, XNonce};
use opsbox_proto::{ErrorCode, Secret};
use opsbox_store::{
    ActorKind, AuditResult, BackupDestinationRecord, ComposeStackRecord, ServiceKind,
    ServiceRecord, ServiceVarRecord,
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::json::ApiJson;
use crate::session::{Admin, Client, deny_token};
use crate::system::patient;
use crate::{AppState, HttpError, RequestId};

/// The document's `format` value.
pub const FORMAT: &str = "opsbox-config/1";
const AUDIT_ACTION: &str = "config.export";
/// Authenticated with the sealed block, so it cannot be pasted into another format version.
const AAD: &[u8] = b"opsbox-config/1:secrets";
const MIN_PASSPHRASE_CHARS: usize = 8;

// ---------------------------------------------------------------------------- encryption

/// Argon2id cost recorded in the file, so a later version can change the defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kdf {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for Kdf {
    fn default() -> Self {
        Self {
            memory_kib: 32 * 1024,
            iterations: 3,
            parallelism: 1,
        }
    }
}

/// Upper bounds accepted when reading a file, so a crafted file cannot ask for gigabytes.
const KDF_MAX_MEMORY_KIB: u32 = 1024 * 1024;
const KDF_MAX_ITERATIONS: u32 = 16;
const KDF_MAX_PARALLELISM: u32 = 16;

/// Why a sealed block could not be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecryptError {
    /// The file has no sealed block (it was exported without secrets).
    NotEncrypted,
    /// The block is not in the shape this version writes.
    Malformed,
    /// Wrong passphrase, or the file was changed after export.
    WrongPassphrase,
}

impl DecryptError {
    pub fn message(self) -> &'static str {
        match self {
            Self::NotEncrypted => "This file has no secrets in it.",
            Self::Malformed => "The secrets block in this file is damaged or not from opsbox.",
            Self::WrongPassphrase => {
                "That passphrase does not open this file, or the file was changed."
            }
        }
    }
}

fn derive_key(passphrase: &str, salt: &[u8], kdf: Kdf) -> Option<[u8; 32]> {
    let params =
        argon2::Params::new(kdf.memory_kib, kdf.iterations, kdf.parallelism, Some(32)).ok()?;
    let mut key = [0u8; 32];
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .ok()?;
    Some(key)
}

/// Seal `plaintext` under `passphrase`. Returns the `secrets` block fields
/// (`encryption`, `ciphertext`).
pub fn seal_secrets(passphrase: &str, plaintext: &[u8], kdf: Kdf) -> Result<Value, HttpError> {
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 24];
    getrandom::fill(&mut salt).map_err(|_| HttpError::internal())?;
    getrandom::fill(&mut nonce).map_err(|_| HttpError::internal())?;
    let key = derive_key(passphrase, &salt, kdf).ok_or_else(HttpError::internal)?;
    let ct = XChaCha20Poly1305::new(&key.into())
        .encrypt(
            &XNonce::from(nonce),
            Payload {
                msg: plaintext,
                aad: AAD,
            },
        )
        .map_err(|_| HttpError::internal())?;
    Ok(json!({
        "encryption": {
            "cipher": "xchacha20poly1305",
            "kdf": "argon2id",
            "memoryKib": kdf.memory_kib,
            "iterations": kdf.iterations,
            "parallelism": kdf.parallelism,
            "salt": B64.encode(salt),
            "nonce": B64.encode(nonce),
        },
        "ciphertext": B64.encode(ct),
    }))
}

/// Open the secrets of an exported document. The helper the import (and the tests) use: a
/// wrong passphrase, a changed file and a damaged file all fail with an error, never a panic.
pub fn decrypt_secrets(doc: &Value, passphrase: &str) -> Result<Value, DecryptError> {
    let secrets = doc.get("secrets").ok_or(DecryptError::Malformed)?;
    if secrets.get("included") != Some(&Value::Bool(true)) {
        return Err(DecryptError::NotEncrypted);
    }
    let enc = secrets.get("encryption").ok_or(DecryptError::Malformed)?;
    let text = |k: &str| enc.get(k).and_then(Value::as_str);
    let number = |k: &str| {
        enc.get(k)
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
    };
    if text("cipher") != Some("xchacha20poly1305") || text("kdf") != Some("argon2id") {
        return Err(DecryptError::Malformed);
    }
    let kdf = Kdf {
        memory_kib: number("memoryKib").ok_or(DecryptError::Malformed)?,
        iterations: number("iterations").ok_or(DecryptError::Malformed)?,
        parallelism: number("parallelism").ok_or(DecryptError::Malformed)?,
    };
    if kdf.memory_kib > KDF_MAX_MEMORY_KIB
        || kdf.iterations > KDF_MAX_ITERATIONS
        || kdf.parallelism > KDF_MAX_PARALLELISM
    {
        return Err(DecryptError::Malformed);
    }
    let decode = |s: Option<&str>| {
        s.and_then(|s| B64.decode(s).ok())
            .ok_or(DecryptError::Malformed)
    };
    let salt = decode(text("salt"))?;
    let nonce: [u8; 24] = decode(text("nonce"))?
        .try_into()
        .map_err(|_| DecryptError::Malformed)?;
    let ct = decode(secrets.get("ciphertext").and_then(Value::as_str))?;
    let key = derive_key(passphrase, &salt, kdf).ok_or(DecryptError::Malformed)?;
    let plain = XChaCha20Poly1305::new(&key.into())
        .decrypt(&XNonce::from(nonce), Payload { msg: &ct, aad: AAD })
        .map_err(|_| DecryptError::WrongPassphrase)?;
    serde_json::from_slice(&plain).map_err(|_| DecryptError::Malformed)
}

// ---------------------------------------------------------------------- field accounting

/// Every `ServiceRecord` field and where it goes in the document. A test compares this with
/// the struct, so a new column has to be exported or excluded on purpose.
pub const SERVICE_EXPORTED: &[(&str, &str)] = &[
    ("id", "id"),
    ("name", "name"),
    ("kind", "kind"),
    ("owner_id", "owner"),
    ("source_kind", "source.kind"),
    ("repo", "source.repo"),
    ("branch", "source.branch"),
    ("image_ref", "source.imageRef"),
    ("deploy_key_id", "source.deployKeyId"),
    ("build_method", "build.method"),
    ("dockerfile_path", "build.dockerfilePath"),
    ("start_cmd", "build.startCmd"),
    ("port", "port"),
    ("health_path", "healthPath"),
    ("domain", "domain"),
    ("auto_deploy", "autoDeploy"),
    ("memory_limit_mb", "limits.memoryMb"),
    ("cpu_limit_millis", "limits.cpuMillis"),
    ("graph_x", "graph.x"),
    ("graph_y", "graph.y"),
    ("note", "note"),
    ("compose_stack_id", "composeStack"),
    ("template_id", "templateId"),
    ("internal_only", "exposure.internalOnly"),
    ("public_port", "exposure.publicPort"),
];

/// Fields left out on purpose, and why (also written into the file as `notExported`).
pub const SERVICE_EXCLUDED: &[(&str, &str)] = &[
    (
        "project_id",
        "Everything lives in the default project; the new server makes its own.",
    ),
    (
        "env_id",
        "Everything lives in the production environment; the new server makes its own.",
    ),
    ("state", "Running state; a new server starts from scratch."),
    ("active_deploy_id", "Deploy history is not configuration."),
    (
        "active_container",
        "The running container belongs to this server.",
    ),
    (
        "active_host_port",
        "The host port is chosen again on deploy.",
    ),
    (
        "credentials_rotated_at",
        "A timestamp of this server's history.",
    ),
    ("created_at", "A timestamp of this server's history."),
    ("updated_at", "A timestamp of this server's history."),
];

/// What is deliberately not in the file at all.
const NOT_EXPORTED: &[(&str, &str)] = &[
    (
        "data",
        "Database contents, volumes and uploaded files. Use backups for data.",
    ),
    (
        "deploys",
        "Deploy history, build logs and the audit log stay on this server.",
    ),
    (
        "databases.size",
        "Sizes and installed extensions are read from the running server and not stored.",
    ),
    (
        "users.passwordHash",
        "Password hashes and two-factor secrets never leave the server.",
    ),
    (
        "sessions",
        "Sign-in sessions, API tokens and pending invitations.",
    ),
    (
        "github",
        "The GitHub App connection and its keys; connect GitHub again on the new server.",
    ),
    (
        "scheduledJobs",
        "Scheduled commands run as root on this server and may contain credentials.",
    ),
    (
        "certificates",
        "TLS certificates and account keys are issued again for the new server.",
    ),
];

fn service_json(svc: &ServiceRecord, owner: Option<&str>, stack: Option<&str>) -> Value {
    // No `..`: adding a column to `ServiceRecord` breaks this line until it is placed in
    // `SERVICE_EXPORTED` or `SERVICE_EXCLUDED`.
    let ServiceRecord {
        id,
        project_id: _,
        env_id: _,
        name,
        kind,
        owner_id: _,
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
        auto_deploy,
        memory_limit_mb,
        cpu_limit_millis,
        state: _,
        active_deploy_id: _,
        active_container: _,
        active_host_port: _,
        graph_x,
        graph_y,
        note,
        compose_stack_id: _,
        template_id,
        internal_only,
        public_port,
        credentials_rotated_at: _,
        created_at: _,
        updated_at: _,
    } = svc;
    json!({
        "id": id,
        "name": name,
        "kind": kind.as_str(),
        "owner": owner,
        "source": {
            "kind": source_kind.as_str(),
            "repo": repo,
            "branch": branch,
            "imageRef": image_ref,
            "deployKeyId": deploy_key_id,
        },
        "build": {
            "method": build_method,
            "dockerfilePath": dockerfile_path,
            "startCmd": start_cmd,
        },
        "port": port,
        "healthPath": health_path,
        "domain": domain,
        "autoDeploy": auto_deploy,
        "limits": { "memoryMb": memory_limit_mb, "cpuMillis": cpu_limit_millis },
        "graph": { "x": graph_x, "y": graph_y },
        "note": note,
        "composeStack": stack,
        "templateId": template_id,
        "exposure": { "internalOnly": internal_only, "publicPort": public_port },
    })
}

// ------------------------------------------------------------------------------- building

/// Secrets gathered while building the document. They only leave this struct sealed.
#[derive(Default)]
struct Collected {
    service_vars: Vec<Value>,
    backup_destinations: Vec<Value>,
    alert_channels: Vec<Value>,
    needs_reentry: Vec<Value>,
    unavailable: Vec<Value>,
}

fn store_err(_: opsbox_store::StoreError) -> HttpError {
    HttpError::internal()
}

fn timestamp(t: OffsetDateTime) -> String {
    t.format(&Rfc3339).unwrap_or_default()
}

/// A host name that is safe in a file name.
pub fn filename_host(host: &str) -> String {
    let cleaned: String = host
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(['-', '.']).to_owned();
    let cleaned: String = cleaned.chars().take(63).collect();
    if cleaned.is_empty() {
        "opsbox".to_owned()
    } else {
        cleaned
    }
}

/// `opsbox-config-<host>-<YYYY-MM-DD>.json`
pub fn filename(host: &str, now: OffsetDateTime) -> String {
    format!(
        "opsbox-config-{}-{:04}-{:02}-{:02}.json",
        filename_host(host),
        now.year(),
        u8::from(now.month()),
        now.day()
    )
}

fn setting_str(state: &AppState, key: &str) -> Option<String> {
    state
        .store
        .get_setting(key)
        .ok()
        .flatten()
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|s| !s.is_empty())
}

async fn open(
    state: &AppState,
    rid: &str,
    actor: &str,
    context: &str,
    sealed: &str,
) -> Option<Secret> {
    state
        .priv_client
        .secret_open(rid, Some(actor), context, sealed)
        .await
        .ok()
}

#[allow(clippy::too_many_lines)]
async fn build(
    state: &AppState,
    rid: &str,
    who: &opsbox_auth::Principal,
    host: &str,
    now: OffsetDateTime,
    keep_secrets: bool,
) -> Result<(Value, Collected), HttpError> {
    let actor = who.user.name.as_str();
    let mut col = Collected::default();
    let store = &state.store;

    let users = store.list_users().map_err(store_err)?;
    let owner_of = |id: &Option<String>| {
        id.as_deref()
            .and_then(|id| users.iter().find(|u| u.id == id))
            .map(|u| u.name.clone())
    };
    let stacks = store.list_all_compose_stacks().map_err(store_err)?;
    let stack_name = |id: &Option<String>| {
        id.as_deref()
            .and_then(|id| stacks.iter().find(|s| s.id == id))
            .map(|s| s.name.clone())
    };

    // ---- services (apps, databases, tools, compose members) with their variables, domains
    let all = store.list_all_services().map_err(store_err)?;
    let mut services = Vec::new();
    let mut databases = Vec::new();
    let mut templates = Vec::new();
    for svc in &all {
        let mut j = service_json(
            svc,
            owner_of(&svc.owner_id).as_deref(),
            stack_name(&svc.compose_stack_id).as_deref(),
        );

        let mut vars = Vec::new();
        for v in store.list_service_vars(&svc.id).map_err(store_err)? {
            vars.push(var_json(state, rid, actor, svc, &v, keep_secrets, &mut col).await);
        }
        j["vars"] = Value::Array(vars);

        let domains = store.list_service_domains(&svc.id).map_err(store_err)?;
        j["domains"] = Value::Array(
            domains
                .iter()
                .map(|d| {
                    json!({
                        "domain": d.domain,
                        "primary": d.is_primary,
                        "redirectTo": d.redirect_to,
                        "forceHttps": d.force_https,
                        "hsts": d.hsts,
                    })
                })
                .collect(),
        );
        services.push(j);

        if svc.kind == ServiceKind::Db {
            let settings = store.get_database_settings(&svc.id).map_err(store_err)?;
            let tmpl = opsbox_deploy::templates::get_template(
                svc.template_id.as_deref().unwrap_or_default(),
            );
            let mut apps = Vec::new();
            for a in store.list_database_apps(&svc.id).map_err(store_err)? {
                let app = store
                    .get_service(&a.app_service_id)
                    .map_err(store_err)?
                    .map(|s| s.name);
                apps.push(json!({
                    "app": app,
                    "database": a.db_name,
                    "user": a.role_name,
                    "varPrefix": a.var_prefix,
                    "pooled": a.pooled,
                }));
            }
            databases.push(json!({
                "service": svc.name,
                "templateId": svc.template_id,
                "engine": tmpl.map(|t| t.name.clone()),
                "version": svc.branch,
                "dataVolume": settings.data_volume,
                "pooler": { "enabled": settings.pooler_enabled, "mode": settings.pooler_mode },
                "apps": apps,
            }));
        } else if let Some(t) = &svc.template_id {
            templates.push(json!({ "service": svc.name, "templateId": t, "version": svc.branch }));
        }
    }

    // ---- compose projects
    let compose: Vec<Value> = stacks
        .iter()
        .map(|s| compose_json(s, owner_of(&s.owner_id)))
        .collect();

    // ---- backups
    let dests = store.list_backup_destinations().map_err(store_err)?;
    let mut destinations = Vec::new();
    for d in &dests {
        destinations.push(destination_json(state, rid, actor, d, keep_secrets, &mut col).await);
    }
    let name_of = |id: &Option<String>| {
        id.as_deref()
            .and_then(|id| all.iter().find(|s| s.id == id))
            .map(|s| s.name.clone())
    };
    let jobs: Vec<Value> = store
        .list_backup_jobs()
        .map_err(store_err)?
        .iter()
        .map(|j| {
            json!({
                "id": j.id,
                "name": j.name,
                "kind": j.kind.as_str(),
                "what": j.what,
                "service": name_of(&j.service_id),
                "templateId": j.template_id,
                "method": j.method,
                "cron": j.cron,
                "retainDaily": j.retain_daily,
                "retainWeekly": j.retain_weekly,
                "destinationId": j.destination_id,
                "enabled": j.enabled,
            })
        })
        .collect();

    // ---- alerts
    let engine: opsbox_alerts::Engine = store
        .get_setting("alerts.engine.v1")
        .map_err(store_err)?
        .map(serde_json::from_value)
        .transpose()
        .map_err(|_| HttpError::internal())?
        .unwrap_or_default();
    let mut channels = Vec::new();
    for (id, c) in &engine.channels {
        let entry = json!({ "id": id, "verified": c.verified });
        match open(
            state,
            rid,
            actor,
            &format!("alerts:channel:{id}"),
            &c.sealed,
        )
        .await
        {
            Some(cfg) if keep_secrets => col
                .alert_channels
                .push(json!({ "id": id, "config": cfg.expose() })),
            _ => col.needs_reentry.push(json!({
                "kind": "alertChannel",
                "id": id,
                "reason": "Webhook addresses and tokens are secrets.",
            })),
        }
        channels.push(entry);
    }
    let rules: Vec<Value> = engine
        .rules
        .iter()
        .map(|r| json!({ "id": r.id, "name": r.name, "condition": r.cond, "level": r.level, "enabled": r.on, "durationSecs": r.duration }))
        .collect();

    // ---- panel, proxy and updates
    let allow_ips: Vec<Value> = store
        .get_setting("security.allow_ips")
        .ok()
        .flatten()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let network = json!({
        "panelDomain": crate::panel::active_panel_domain(state),
        "panelLocalOnly": state.listen.ip().is_loopback(),
        "allowedIps": allow_ips,
    });
    let updates = json!({
        "channel": setting_str(state, "opsbox.update.channel"),
        "auto": store.get_setting("opsbox.update.auto").ok().flatten(),
        "window": store.get_setting("opsbox.update.window").ok().flatten(),
    });

    // ---- firewall (read by priv; the document still works without it)
    let firewall = match patient(state, 20).firewall_list(rid).await {
        Ok(f) => json!({
            "backend": f.backend,
            "rules": f.rules.iter().map(|r| json!({ "port": r.port, "proto": r.proto, "from": r.src, "note": r.note })).collect::<Vec<_>>(),
        }),
        Err(_) => {
            col.unavailable.push(json!({
                "section": "firewall",
                "reason": "The firewall could not be read on this server.",
            }));
            Value::Null
        }
    };

    // ---- people
    let people: Vec<Value> = users
        .iter()
        .map(|u| json!({ "name": u.name, "role": u.role.as_str(), "twoFactorEnabled": u.totp_enabled }))
        .collect();
    for u in &users {
        col.needs_reentry.push(json!({
            "kind": "user",
            "name": u.name,
            "reason": "Password and two-factor are set again on the new server.",
        }));
    }
    col.needs_reentry.push(json!({
        "kind": "apiTokens",
        "reason": "API tokens are created again.",
    }));
    col.needs_reentry.push(json!({
        "kind": "github",
        "reason": "Connect GitHub again on the new server.",
    }));
    for k in store.list_deploy_keys().map_err(store_err)? {
        col.needs_reentry.push(json!({
            "kind": "deployKey",
            "id": k.id,
            "repo": k.repo,
            "reason": "Private deploy keys are not exported. Create a new key.",
        }));
    }

    let doc = json!({
        "format": FORMAT,
        "exportedAt": timestamp(now),
        "exportedBy": actor,
        "opsboxVersion": state.version,
        "host": host,
        "services": services,
        "templates": templates,
        "composeProjects": compose,
        "databases": databases,
        "backups": {
            "defaultDestinationId": setting_str(state, crate::backups::SETTING_DEFAULT_DEST),
            "destinations": destinations,
            "jobs": jobs,
        },
        "alerts": {
            "rules": rules,
            "quiet": { "from": engine.quiet.from, "to": engine.quiet.to },
            "channels": channels,
        },
        "network": network,
        "updates": updates,
        "firewall": firewall,
        "users": people,
        "notExported": {
            "service": SERVICE_EXCLUDED.iter().map(|(f, why)| json!({ "field": f, "reason": why })).collect::<Vec<_>>(),
            "other": NOT_EXPORTED.iter().map(|(f, why)| json!({ "what": f, "reason": why })).collect::<Vec<_>>(),
        },
    });
    Ok((doc, col))
}

fn compose_json(s: &ComposeStackRecord, owner: Option<String>) -> Value {
    json!({
        "name": s.name,
        "owner": owner,
        "projectName": s.project_name,
        "composeYaml": s.compose_yaml,
        "services": s.service_names,
        "dependsOn": s.depends_edges.iter().map(|(a, b)| json!({ "service": a, "dependsOn": b })).collect::<Vec<_>>(),
        "confirmed": s.confirmed,
    })
}

async fn var_json(
    state: &AppState,
    rid: &str,
    actor: &str,
    svc: &ServiceRecord,
    v: &ServiceVarRecord,
    keep_secrets: bool,
    col: &mut Collected,
) -> Value {
    let mut out = json!({ "key": v.key, "secret": v.is_secret, "ref": v.ref_expr, "value": null });
    // A reference is rebuilt from the linked service; there is no value to carry.
    if v.ref_expr.is_some() {
        return out;
    }
    let ctx = crate::deploy::var_seal_context(&svc.id, &v.key);
    let opened = open(state, rid, actor, &ctx, &v.value_enc).await;
    match (v.is_secret, opened) {
        (false, Some(val)) => out["value"] = json!(val.expose()),
        (true, Some(val)) if keep_secrets => col
            .service_vars
            .push(json!({ "service": svc.name, "key": v.key, "value": val.expose() })),
        _ => col.needs_reentry.push(json!({
            "kind": "serviceVar",
            "service": svc.name,
            "key": v.key,
            "reason": if v.is_secret { "Secret value." } else { "The value could not be read." },
        })),
    }
    out
}

async fn destination_json(
    state: &AppState,
    rid: &str,
    actor: &str,
    d: &BackupDestinationRecord,
    keep_secrets: bool,
    col: &mut Collected,
) -> Value {
    if let Some(sealed) = &d.secrets_enc {
        match open(state, rid, actor, &format!("backup:dest:{}", d.id), sealed).await {
            Some(secret) if keep_secrets => col
                .backup_destinations
                .push(json!({ "id": d.id, "secrets": secret.expose() })),
            _ => col.needs_reentry.push(json!({
                "kind": "backupDestination",
                "id": d.id,
                "name": d.name,
                "reason": "Storage keys and passwords are secrets.",
            })),
        }
    }
    json!({
        "id": d.id,
        "provider": d.provider,
        "name": d.name,
        "config": d.config,
        "hasSecrets": d.secrets_enc.is_some(),
    })
}

/// Put the secrets block into the document: either the list of what must be typed again, or
/// that list plus the sealed block.
fn finish(
    mut doc: Value,
    col: Collected,
    passphrase: Option<&str>,
    kdf: Kdf,
) -> Result<Value, HttpError> {
    let mut secrets = Map::new();
    match passphrase {
        None => {
            secrets.insert("included".into(), Value::Bool(false));
            secrets.insert("needsReentry".into(), Value::Array(col.needs_reentry));
        }
        Some(p) => {
            let payload = serde_json::to_vec(&json!({
                "serviceVars": col.service_vars,
                "backupDestinations": col.backup_destinations,
                "alertChannels": col.alert_channels,
            }))
            .map_err(|_| HttpError::internal())?;
            let sealed = seal_secrets(p, &payload, kdf)?;
            secrets.insert("included".into(), Value::Bool(true));
            if let Value::Object(m) = sealed {
                secrets.extend(m);
            }
            // Only what the sealed block could not carry still has to be typed again.
            let kept: Vec<Value> = col
                .needs_reentry
                .into_iter()
                .filter(|e| {
                    !matches!(
                        e["kind"].as_str(),
                        Some("serviceVar" | "backupDestination" | "alertChannel")
                    ) || e["reason"] == "The value could not be read."
                })
                .collect();
            secrets.insert("needsReentry".into(), Value::Array(kept));
        }
    }
    doc["secrets"] = Value::Object(secrets);
    if !col.unavailable.is_empty() {
        doc["unavailable"] = Value::Array(col.unavailable);
    }
    Ok(doc)
}

// -------------------------------------------------------------------------------- handler

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ExportBody {
    password: String,
    /// Present and not empty: include secrets, sealed with this passphrase.
    #[serde(default)]
    passphrase: Option<String>,
}

/// `POST /api/v1/config/export`
pub(crate) async fn export(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<ExportBody>,
) -> Result<Response, HttpError> {
    deny_token(&who)?;
    let audit = |result: AuditResult, detail: Value| {
        state.auth.audit(
            &who.user.name,
            ActorKind::User,
            Some(&client.ip),
            AUDIT_ACTION,
            None,
            result,
            detail,
        );
    };
    let ok = state
        .auth
        .verify_user_password(&who.user.id, &body.password)
        .await
        .map_err(|_| HttpError::internal())?;
    if !ok {
        audit(AuditResult::Denied, json!({ "reason": "wrong_password" }));
        return Err(HttpError::new(
            ErrorCode::Forbidden,
            "Incorrect account password.",
        ));
    }
    let passphrase = body.passphrase.as_deref().filter(|p| !p.is_empty());
    if let Some(p) = passphrase
        && p.chars().count() < MIN_PASSPHRASE_CHARS
    {
        return Err(crate::system::invalid(
            "passphrase",
            "Use a passphrase of at least 8 characters, or leave it empty to export without secrets.",
        ));
    }

    let now = OffsetDateTime::now_utc();
    let host = state
        .priv_client
        .host_info(&rid, Some(&who.user.name))
        .await
        .map_or_else(|_| "opsbox".to_owned(), |h| h.hostname);

    let built = build(&state, &rid, &who, &host, now, passphrase.is_some()).await;
    let doc = built.and_then(|(doc, col)| {
        let counts = json!({
            "services": doc["services"].as_array().map_or(0, Vec::len),
            "secrets": col.service_vars.len() + col.backup_destinations.len() + col.alert_channels.len(),
        });
        let doc = finish(doc, col, passphrase, Kdf::default())?;
        Ok((doc, counts))
    });
    let (doc, counts) = match doc {
        Ok(v) => v,
        Err(e) => {
            audit(
                AuditResult::Failed,
                json!({ "includeSecrets": passphrase.is_some() }),
            );
            return Err(e);
        }
    };
    let file = filename(&host, now);
    audit(
        AuditResult::Ok,
        json!({
            "file": file,
            "includeSecrets": passphrase.is_some(),
            "encrypted": passphrase.is_some(),
            "services": counts["services"],
            "secretsExported": counts["secrets"],
        }),
    );

    let text = serde_json::to_string_pretty(&doc).map_err(|_| HttpError::internal())?;
    let mut resp = (StatusCode::OK, text).into_response();
    let h = resp.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{file}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    Ok(resp)
}
