//! GitHub App manifest flow, repository picker, webhooks, and fallback sources
//! (public URL and per-repo deploy keys) — docs/BACKEND_SPEC.md 7.5 steps 1-2.
//!
//! `serve` never reads `/etc/opsbox/master.key`. GitHub App private keys, webhook
//! secrets, and SSH deploy private keys are sealed/opened exclusively through
//! `opsbox-priv` (`PrivOp::SecretSeal` / `PrivOp::SecretOpen` with `deploy:*` contexts).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Extension, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use futures_util::stream::{self, StreamExt};
use opsbox_deploy::{
    DeployError, GitHubClient, GitHubConfig, GitHubRepoMeta, WebhookEvent, build_manifest,
    format_pushed_ago, generate_ed25519_deploy_key, normalize_repo_source, parse_webhook_event,
    random_hex, sign_app_jwt, validate_org, validate_public_repo_url, verify_github_signature,
};
use opsbox_proto::{ErrorCode, Secret};
use opsbox_store::{
    ActorKind, AuditResult, DeployKeyInfo, GithubAppRecord, NewAudit, NewDeployKey, NewGithubApp,
    NewWebhookDelivery,
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::RwLock;

use crate::json::ApiJson;
use crate::panel::active_panel_domain;
use crate::session::{Admin, Client, Deployer};
use crate::system::audit;
use crate::{AppState, HttpError, RequestId};

const CTX_PRIVATE_KEY: &str = "deploy:github_app:private_key";
const CTX_WEBHOOK_SECRET: &str = "deploy:github_app:webhook_secret";
const MANIFEST_STATE_TTL_SECS: i64 = 15 * 60;
const REPO_CACHE_TTL: Duration = Duration::from_secs(120);
const MAX_DETECT_PER_REFRESH: usize = 40;

fn deploy_key_context(id: &str) -> String {
    format!("deploy:deploy_key:{id}")
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

#[derive(Clone)]
pub struct GithubService {
    config: GitHubConfig,
    cache: Arc<RwLock<RepoCache>>,
}

#[derive(Default)]
struct RepoCache {
    cached_at: Option<Instant>,
    installation_id: Option<i64>,
    repos: Vec<CachedRepo>,
    build_by_push: HashMap<(String, i64), String>,
}

#[derive(Clone)]
struct CachedRepo {
    meta: GitHubRepoMeta,
    build: String,
}

impl Default for GithubService {
    fn default() -> Self {
        Self::new(GitHubConfig::default())
    }
}

impl GithubService {
    pub fn new(config: GitHubConfig) -> Self {
        Self {
            config,
            cache: Arc::new(RwLock::new(RepoCache::default())),
        }
    }

    pub fn client(&self) -> GitHubClient {
        GitHubClient::new(self.config.clone())
    }

    pub async fn clear_cache(&self) {
        let mut guard = self.cache.write().await;
        guard.cached_at = None;
        guard.installation_id = None;
        guard.repos.clear();
    }
}

fn map_deploy_err(e: DeployError) -> HttpError {
    match e {
        DeployError::Validation { field, message } => {
            let mut api = opsbox_proto::ApiError::new(ErrorCode::Validation, message);
            if let Some(f) = field {
                api = api.with_field(f);
            }
            HttpError(api, None)
        }
        DeployError::Unauthorized(msg) => HttpError::new(ErrorCode::Unauthorized, msg),
        DeployError::NotFound(msg) => HttpError::new(ErrorCode::NotFound, msg),
        DeployError::InvalidKey(msg) => HttpError::new(ErrorCode::Internal, msg),
        DeployError::GitHub(msg) => HttpError::new(ErrorCode::Unavailable, msg),
        DeployError::Internal(msg) => {
            tracing::error!(detail = %msg, "internal deploy error");
            HttpError::internal()
        }
    }
}

fn valid_host_header(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.len() > 255
        || trimmed.bytes().any(|b| {
            b.is_ascii_control()
                || b.is_ascii_whitespace()
                || matches!(b, b'/' | b'\\' | b'@' | b'?' | b'#')
        })
    {
        return None;
    }
    Some(trimmed)
}

fn panel_origin_from_request(state: &AppState, headers: &HeaderMap) -> Result<String, HttpError> {
    if let Some(domain) = active_panel_domain(state) {
        return Ok(format!("https://{domain}"));
    }
    if let Some(host) = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .and_then(valid_host_header)
    {
        let forwarded_https = headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("https"));
        let scheme = if state.tls || forwarded_https {
            "https"
        } else {
            "http"
        };
        return Ok(format!("{scheme}://{host}"));
    }
    Err(HttpError::new(
        ErrorCode::Validation,
        "Could not determine the panel address from this request. Set a panel domain or access the panel normally.",
    ))
}

fn install_url(web_base: &str, slug: &str) -> String {
    format!(
        "{}/apps/{slug}/installations/new",
        web_base.trim_end_matches('/')
    )
}

fn valid_token_param(s: &str, max_len: usize) -> bool {
    !s.is_empty()
        && s.len() <= max_len
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn redirect_html(target: &str, title: &str, note: &str) -> Html<String> {
    let esc_target = html_escape(target);
    let esc_title = html_escape(title);
    let esc_note = html_escape(note);
    Html(format!(
        "<!doctype html><meta charset=\"utf-8\"><title>{esc_title}</title>\
         <meta http-equiv=\"refresh\" content=\"0;url={esc_target}\">\
         <p>{esc_note} <a href=\"{esc_target}\">Continue to opsbox</a></p>"
    ))
}

fn deploy_key_json(k: &DeployKeyInfo) -> Value {
    json!({
        "id": k.id,
        "repo": k.repo,
        "sshUrl": k.ssh_url,
        "publicKey": k.public_key,
        "fingerprint": k.fingerprint,
        "createdBy": k.created_by,
        "createdAt": k.created_at,
    })
}

fn status_json(
    state: &AppState,
    app: Option<&GithubAppRecord>,
    deliveries: &[opsbox_store::WebhookDelivery],
) -> Value {
    let panel_domain = active_panel_domain(state);
    let expected_webhook_url = panel_domain
        .as_deref()
        .map(|d| format!("https://{d}/api/v1/github/webhook"));
    let web_base = &state.github.config.web_base;

    let Some(app) = app else {
        return json!({
            "connected": false,
            "installed": false,
            "panelDomain": panel_domain,
            "panelDomainSet": panel_domain.is_some(),
            "expectedWebhookUrl": expected_webhook_url,
        });
    };

    let inst_url = install_url(web_base, &app.slug);
    let manage_url = app
        .installation_html_url
        .clone()
        .unwrap_or_else(|| inst_url.clone());
    let webhook_matches_domain = match (&panel_domain, &app.webhook_url) {
        (Some(dom), Some(url)) => {
            app.webhook_active && url == &format!("https://{dom}/api/v1/github/webhook")
        }
        _ => false,
    };

    json!({
        "connected": true,
        "installed": app.installation_id.is_some(),
        "appId": app.app_id,
        "appSlug": app.slug,
        "appName": app.name,
        "owner": app.owner_login,
        "ownerType": app.owner_type,
        "htmlUrl": app.html_url,
        "installUrl": inst_url,
        "manageUrl": manage_url,
        "account": app.installation_account.clone().unwrap_or_else(|| app.owner_login.clone()),
        "installationId": app.installation_id,
        "repositorySelection": app.repository_selection,
        "webhookUrl": app.webhook_url,
        "webhookActive": app.webhook_active,
        "webhookConfigured": webhook_matches_domain,
        "expectedWebhookUrl": expected_webhook_url,
        "panelDomain": panel_domain,
        "panelDomainSet": panel_domain.is_some(),
        "lastWebhookAt": app.last_webhook_at,
        "lastWebhookEvent": app.last_webhook_event,
        "recentDeliveries": deliveries.iter().map(|d| json!({
            "id": d.id,
            "deliveryId": d.delivery_id,
            "event": d.event,
            "repo": d.repo,
            "ref": d.git_ref,
            "commit": d.commit_sha,
            "pusher": d.pusher,
            "receivedAt": d.received_at,
        })).collect::<Vec<_>>(),
    })
}

async fn open_app_jwt(
    state: &AppState,
    rid: &str,
    actor: Option<&str>,
    app: &GithubAppRecord,
) -> Result<String, HttpError> {
    let pem = state
        .priv_client
        .secret_open(rid, actor, CTX_PRIVATE_KEY, &app.private_key_enc)
        .await?;
    sign_app_jwt(app.app_id, pem.expose(), now_secs()).map_err(map_deploy_err)
}

/// Mint an installation access token for cloning a GitHub App repository during deploy.
pub(crate) async fn mint_installation_token_for_deploy(
    state: &AppState,
    rid: &str,
    actor: Option<&str>,
) -> Result<Secret, HttpError> {
    let store = state.store.clone();
    let app = tokio::task::spawn_blocking(move || store.get_github_app())
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "GitHub is not connected."))?;
    let installation_id = app.installation_id.ok_or_else(|| {
        HttpError::new(
            ErrorCode::Validation,
            "Install the GitHub App on your account before deploying from GitHub.",
        )
    })?;
    let jwt = open_app_jwt(state, rid, actor, &app).await?;
    let gh_client = state.github.client();
    let tok = tokio::task::spawn_blocking(move || {
        gh_client.create_installation_token(&jwt, installation_id)
    })
    .await
    .map_err(|_| HttpError::internal())?
    .map_err(map_deploy_err)?;
    Ok(Secret::new(tok.as_str()))
}

async fn refresh_installation_from_github(
    state: &AppState,
    rid: &str,
    actor: Option<&str>,
    app: &GithubAppRecord,
    preferred_id: Option<i64>,
) -> Result<Option<GithubAppRecord>, HttpError> {
    let jwt = open_app_jwt(state, rid, actor, app).await?;
    let client = state.github.client();
    let installations = tokio::task::spawn_blocking(move || client.list_installations(&jwt))
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(map_deploy_err)?;

    let chosen = preferred_id
        .and_then(|id| installations.iter().find(|i| i.id == id))
        .or_else(|| installations.first());

    let store = state.store.clone();
    let (inst_id, account, html_url, repo_sel) = match chosen {
        Some(i) => (
            Some(i.id),
            Some(i.account_login.clone()),
            i.html_url.clone(),
            i.repository_selection.clone(),
        ),
        None => (None, None, None, None),
    };
    let now = now_secs();
    tokio::task::spawn_blocking(move || {
        store.update_github_installation(
            inst_id,
            account.as_deref(),
            html_url.as_deref(),
            repo_sel.as_deref(),
            now,
        )
    })
    .await
    .map_err(|_| HttpError::internal())?
    .map_err(|_| HttpError::internal())?;

    state.github.clear_cache().await;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.get_github_app())
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())
}

/// Snapshot for `GET /api/v1/settings` (never fails the settings page if GitHub is unreachable).
pub async fn snapshot(state: &AppState) -> Value {
    let store = state.store.clone();
    let (app, deliveries) = tokio::task::spawn_blocking(move || {
        let app = store.get_github_app().ok().flatten();
        let deliveries = store.list_github_webhook_deliveries(5).unwrap_or_default();
        (app, deliveries)
    })
    .await
    .unwrap_or((None, Vec::new()));
    status_json(state, app.as_ref(), &deliveries)
}

/// Snapshot of deploy keys for `GET /api/v1/settings`.
pub async fn deploy_keys_snapshot(state: &AppState) -> Vec<Value> {
    let store = state.store.clone();
    let keys = tokio::task::spawn_blocking(move || store.list_deploy_keys().unwrap_or_default())
        .await
        .unwrap_or_default();
    keys.iter().map(deploy_key_json).collect()
}

/// Best-effort sync of the GitHub App webhook URL when the panel domain certificate goes live.
pub async fn sync_webhook_on_domain_change(
    state: &AppState,
    rid: &str,
    actor: &str,
    domain: &str,
) -> Result<(), String> {
    let store = state.store.clone();
    let Some(app) = tokio::task::spawn_blocking(move || store.get_github_app())
        .await
        .map_err(|_| "store task failed".to_owned())?
        .map_err(|e| e.to_string())?
    else {
        return Ok(());
    };
    let webhook_url = format!("https://{domain}/api/v1/github/webhook");
    let jwt = open_app_jwt(state, rid, Some(actor), &app)
        .await
        .map_err(|e| e.0.error.message)?;
    let secret = state
        .priv_client
        .secret_open(
            rid,
            Some(actor),
            CTX_WEBHOOK_SECRET,
            &app.webhook_secret_enc,
        )
        .await
        .map_err(|e| e.to_string())?;
    let client = state.github.client();
    let url_clone = webhook_url.clone();
    tokio::task::spawn_blocking(move || {
        client.update_webhook_config(&jwt, &url_clone, secret.expose())
    })
    .await
    .map_err(|_| "github task failed".to_owned())?
    .map_err(|e| e.to_string())?;

    let store = state.store.clone();
    let now = now_secs();
    tokio::task::spawn_blocking(move || store.update_github_webhook(Some(&webhook_url), true, now))
        .await
        .map_err(|_| "store task failed".to_owned())?
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[derive(Deserialize, Default)]
pub(crate) struct StatusQuery {
    #[serde(default)]
    refresh: Option<String>,
}

/// `GET /api/v1/github` — GitHub App connection status (for Settings and New Service).
pub(crate) async fn get_status(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    Extension(RequestId(rid)): Extension<RequestId>,
    Query(q): Query<StatusQuery>,
) -> Result<Json<Value>, HttpError> {
    let refresh = matches!(q.refresh.as_deref(), Some("1" | "true"));
    let store = state.store.clone();
    let mut app = tokio::task::spawn_blocking(move || store.get_github_app())
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())?;

    if let Some(ref cur) = app
        && (refresh || cur.installation_id.is_none())
        && let Ok(updated) =
            refresh_installation_from_github(&state, &rid, Some(&who.user.name), cur, None).await
    {
        app = updated;
    }

    let store = state.store.clone();
    let deliveries = tokio::task::spawn_blocking(move || {
        store.list_github_webhook_deliveries(5).unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    Ok(Json(status_json(&state, app.as_ref(), &deliveries)))
}

#[derive(Deserialize, Default)]
pub(crate) struct ManifestBody {
    #[serde(default)]
    org: Option<String>,
}

/// `POST /api/v1/github/manifest` — prepare a user-owned GitHub App manifest form submission.
pub(crate) async fn start_manifest(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    body: Option<ApiJson<ManifestBody>>,
) -> Result<Json<Value>, HttpError> {
    let org =
        validate_org(body.as_ref().and_then(|b| b.0.org.as_deref())).map_err(map_deploy_err)?;
    let panel_origin = panel_origin_from_request(&state, &client.headers)?;
    let panel_domain = active_panel_domain(&state);
    let hostname = state
        .priv_client
        .host_info(&rid, Some(&who.user.name))
        .await
        .map_or_else(|_| "panel".to_owned(), |h| h.hostname);

    let csrf_state = random_hex(32).map_err(map_deploy_err)?;
    let now = now_secs();
    let expires_at = now + MANIFEST_STATE_TTL_SECS;

    let store = state.store.clone();
    let state_clone = csrf_state.clone();
    let org_clone = org.clone();
    let actor = who.user.name.clone();
    tokio::task::spawn_blocking(move || {
        store.save_github_manifest_state(
            &state_clone,
            org_clone.as_deref(),
            &actor,
            expires_at,
            now,
        )
    })
    .await
    .map_err(|_| HttpError::internal())?
    .map_err(|_| HttpError::internal())?;

    let start = build_manifest(
        &state.github.config.web_base,
        &panel_origin,
        panel_domain.as_deref(),
        &hostname,
        org.as_deref(),
        &csrf_state,
    )
    .map_err(map_deploy_err)?;

    Ok(Json(json!({
        "actionUrl": start.action_url,
        "manifest": start.manifest_json,
        "state": start.state,
        "webhookConfigured": start.webhook_configured,
        "webhookUrl": start.webhook_url,
        "panelDomainRequired": start.panel_domain_required,
    })))
}

#[derive(Deserialize)]
pub(crate) struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// `GET /api/v1/github/callback` — browser redirect target after GitHub creates the App.
///
/// Because `opsbox_session` is `SameSite=Strict`, browsers omit it on the initial cross-site
/// navigation from github.com. This endpoint never mutates state and instead performs a
/// same-origin meta-refresh into `/#/settings?...`, where the authenticated UI completes
/// the handshake via `POST /api/v1/github/complete` with CSRF and Origin checks.
pub(crate) async fn manifest_callback(
    Query(q): Query<CallbackQuery>,
) -> Result<Html<String>, HttpError> {
    if let (Some(code), Some(state)) = (q.code.as_deref(), q.state.as_deref()) {
        if !valid_token_param(code, 256) || !valid_token_param(state, 256) {
            return Err(HttpError::new(
                ErrorCode::Validation,
                "Invalid GitHub callback parameters.",
            ));
        }
        let target = format!("/#/settings?github_code={code}&github_state={state}");
        return Ok(redirect_html(
            &target,
            "Connecting GitHub…",
            "Finishing GitHub App setup…",
        ));
    }
    let reason = q
        .error
        .as_deref()
        .filter(|s| valid_token_param(s, 64))
        .unwrap_or("cancelled");
    let target = format!("/#/settings?github_error={reason}");
    Ok(redirect_html(
        &target,
        "GitHub connection cancelled",
        "Returning to opsbox settings…",
    ))
}

#[derive(Deserialize)]
pub(crate) struct SetupQuery {
    #[serde(default)]
    installation_id: Option<String>,
    #[serde(default)]
    setup_action: Option<String>,
}

/// `GET /api/v1/github/setup` — browser redirect target after installing or updating the App.
pub(crate) async fn setup_callback(Query(q): Query<SetupQuery>) -> Result<Html<String>, HttpError> {
    if let Some(inst) = q.installation_id.as_deref() {
        if inst.is_empty() || inst.len() > 20 || !inst.bytes().all(|b| b.is_ascii_digit()) {
            return Err(HttpError::new(
                ErrorCode::Validation,
                "Invalid installation_id parameter.",
            ));
        }
        let action = q
            .setup_action
            .as_deref()
            .filter(|s| valid_token_param(s, 32))
            .unwrap_or("install");
        let target = format!("/#/settings?github_installation_id={inst}&setup_action={action}");
        return Ok(redirect_html(
            &target,
            "GitHub App installed",
            "Returning to opsbox settings…",
        ));
    }
    Ok(redirect_html(
        "/#/settings",
        "GitHub App updated",
        "Returning to opsbox settings…",
    ))
}

#[derive(Deserialize)]
pub(crate) struct CompleteBody {
    code: String,
    state: String,
}

/// `POST /api/v1/github/complete` — exchange `code` for GitHub App credentials and seal secrets.
pub(crate) async fn complete_manifest(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<CompleteBody>,
) -> Result<Json<Value>, HttpError> {
    if !valid_token_param(&body.code, 256) || !valid_token_param(&body.state, 256) {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "Invalid GitHub setup code or state.",
        ));
    }
    let store = state.store.clone();
    let st = body.state.clone();
    let now = now_secs();
    let pending =
        tokio::task::spawn_blocking(move || store.consume_github_manifest_state(&st, now))
            .await
            .map_err(|_| HttpError::internal())?
            .map_err(|_| HttpError::internal())?;
    if pending.is_none() {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "This GitHub setup link expired or was already used. Start connecting GitHub again.",
        ));
    }

    let gh_client = state.github.client();
    let code = body.code.clone();
    let conv = tokio::task::spawn_blocking(move || gh_client.exchange_manifest_code(&code))
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(map_deploy_err)?;

    let webhook_secret = match conv.webhook_secret.as_deref() {
        Some(s) if !s.is_empty() => Secret::new(s),
        _ => Secret::new(random_hex(32).map_err(map_deploy_err)?),
    };

    let private_key_enc = state
        .priv_client
        .secret_seal(
            &rid,
            Some(&who.user.name),
            CTX_PRIVATE_KEY,
            Secret::new(conv.pem.as_str()),
        )
        .await?;
    let webhook_secret_enc = state
        .priv_client
        .secret_seal(
            &rid,
            Some(&who.user.name),
            CTX_WEBHOOK_SECRET,
            webhook_secret,
        )
        .await?;

    let panel_domain = active_panel_domain(&state);
    let (webhook_url, webhook_active) = match panel_domain {
        Some(ref dom) => (Some(format!("https://{dom}/api/v1/github/webhook")), true),
        None => (None, false),
    };

    // Best-effort check if an installation already exists.
    let gh_client = state.github.client();
    let jwt_opt = sign_app_jwt(conv.id, conv.pem.as_str(), now_secs()).ok();
    let existing_inst = match jwt_opt {
        Some(jwt) => tokio::task::spawn_blocking(move || gh_client.list_installations(&jwt))
            .await
            .ok()
            .and_then(Result::ok)
            .and_then(|list| list.into_iter().next()),
        None => None,
    };

    let new_app = NewGithubApp {
        app_id: conv.id,
        slug: conv.slug.clone(),
        name: conv.name,
        owner_login: conv.owner_login.clone(),
        owner_type: conv.owner_type,
        html_url: conv.html_url,
        private_key_enc,
        webhook_secret_enc,
        webhook_url,
        webhook_active,
        installation_id: existing_inst.as_ref().map(|i| i.id),
        installation_account: existing_inst.as_ref().map(|i| i.account_login.clone()),
        installation_html_url: existing_inst.and_then(|i| i.html_url),
        repository_selection: None,
        created_by: who.user.name.clone(),
        now: now_secs(),
    };

    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.upsert_github_app(&new_app))
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())?;
    state.github.clear_cache().await;

    audit(
        &state,
        &who,
        &client.ip,
        "github.connect",
        Some(&conv.slug),
        json!({ "appId": conv.id, "owner": conv.owner_login }),
    );

    let store = state.store.clone();
    let saved = tokio::task::spawn_blocking(move || store.get_github_app())
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())?;
    Ok(Json(status_json(&state, saved.as_ref(), &[])))
}

#[derive(Deserialize, Default)]
pub(crate) struct InstallationBody {
    #[serde(default, rename = "installationId")]
    installation_id: Option<i64>,
}

/// `POST /api/v1/github/installation` — refresh the GitHub App installation metadata.
pub(crate) async fn sync_installation(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    body: Option<ApiJson<InstallationBody>>,
) -> Result<Json<Value>, HttpError> {
    let store = state.store.clone();
    let app = tokio::task::spawn_blocking(move || store.get_github_app())
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "GitHub is not connected."))?;

    let preferred = body.and_then(|b| b.0.installation_id);
    let updated =
        refresh_installation_from_github(&state, &rid, Some(&who.user.name), &app, preferred)
            .await?;

    audit(
        &state,
        &who,
        &client.ip,
        "github.installation.sync",
        Some(&app.slug),
        json!({ "installationId": updated.as_ref().and_then(|u| u.installation_id) }),
    );

    let store = state.store.clone();
    let deliveries = tokio::task::spawn_blocking(move || {
        store.list_github_webhook_deliveries(5).unwrap_or_default()
    })
    .await
    .unwrap_or_default();
    Ok(Json(status_json(&state, updated.as_ref(), &deliveries)))
}

/// `POST /api/v1/github/webhook/sync` — push the active panel domain webhook URL to GitHub.
pub(crate) async fn sync_webhook(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
) -> Result<Json<Value>, HttpError> {
    let domain = active_panel_domain(&state).ok_or_else(|| {
        HttpError::new(
            ErrorCode::Validation,
            "Set a panel domain with a live HTTPS certificate in Settings first so GitHub can deliver push webhooks.",
        )
    })?;
    let store = state.store.clone();
    let app = tokio::task::spawn_blocking(move || store.get_github_app())
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| HttpError::new(ErrorCode::NotFound, "GitHub is not connected."))?;

    let webhook_url = format!("https://{domain}/api/v1/github/webhook");
    let jwt = open_app_jwt(&state, &rid, Some(&who.user.name), &app).await?;
    let secret = state
        .priv_client
        .secret_open(
            &rid,
            Some(&who.user.name),
            CTX_WEBHOOK_SECRET,
            &app.webhook_secret_enc,
        )
        .await?;
    let gh_client = state.github.client();
    let url_clone = webhook_url.clone();
    tokio::task::spawn_blocking(move || {
        gh_client.update_webhook_config(&jwt, &url_clone, secret.expose())
    })
    .await
    .map_err(|_| HttpError::internal())?
    .map_err(map_deploy_err)?;

    let store = state.store.clone();
    let now = now_secs();
    let url_store = webhook_url.clone();
    tokio::task::spawn_blocking(move || store.update_github_webhook(Some(&url_store), true, now))
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())?;

    audit(
        &state,
        &who,
        &client.ip,
        "github.webhook.sync",
        Some(&app.slug),
        json!({ "webhookUrl": webhook_url }),
    );

    let store = state.store.clone();
    let (updated, deliveries) = tokio::task::spawn_blocking(move || {
        (
            store.get_github_app().ok().flatten(),
            store.list_github_webhook_deliveries(5).unwrap_or_default(),
        )
    })
    .await
    .unwrap_or((None, Vec::new()));
    Ok(Json(status_json(&state, updated.as_ref(), &deliveries)))
}

/// `DELETE /api/v1/github` — disconnect the GitHub App and erase stored credentials.
pub(crate) async fn disconnect(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
) -> Result<Json<Value>, HttpError> {
    let store = state.store.clone();
    let had = tokio::task::spawn_blocking(move || store.delete_github_app())
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())?;
    state.github.clear_cache().await;

    audit(
        &state,
        &who,
        &client.ip,
        "github.disconnect",
        None,
        json!({ "removed": had }),
    );
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize, Default)]
pub(crate) struct ReposQuery {
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    refresh: Option<String>,
}

/// `GET /api/v1/github/repos?q=` — list installed repositories with search and build detection.
pub(crate) async fn list_repos(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    Extension(RequestId(rid)): Extension<RequestId>,
    Query(query): Query<ReposQuery>,
) -> Result<Json<Value>, HttpError> {
    let force_refresh = matches!(query.refresh.as_deref(), Some("1" | "true"));
    let store = state.store.clone();
    let mut app = tokio::task::spawn_blocking(move || store.get_github_app())
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())?;

    if let Some(ref cur) = app
        && (cur.installation_id.is_none() || force_refresh)
        && let Ok(updated) =
            refresh_installation_from_github(&state, &rid, Some(&who.user.name), cur, None).await
    {
        app = updated;
    }

    let Some(app) = app else {
        return Ok(Json(json!({
            "connected": false,
            "installed": false,
            "account": null,
            "installUrl": null,
            "manageUrl": null,
            "repos": [],
        })));
    };

    let inst_url = install_url(&state.github.config.web_base, &app.slug);
    let manage_url = app
        .installation_html_url
        .clone()
        .unwrap_or_else(|| inst_url.clone());
    let Some(inst_id) = app.installation_id else {
        return Ok(Json(json!({
            "connected": true,
            "installed": false,
            "account": app.owner_login,
            "installUrl": inst_url,
            "manageUrl": manage_url,
            "repos": [],
        })));
    };

    let cached = {
        let guard = state.github.cache.read().await;
        if !force_refresh
            && guard.installation_id == Some(inst_id)
            && guard
                .cached_at
                .is_some_and(|at| at.elapsed() < REPO_CACHE_TTL)
        {
            Some(guard.repos.clone())
        } else {
            None
        }
    };

    let repos = if let Some(list) = cached {
        list
    } else {
        let jwt = open_app_jwt(&state, &rid, Some(&who.user.name), &app).await?;
        let gh_client = state.github.client();
        let inst_token =
            tokio::task::spawn_blocking(move || gh_client.create_installation_token(&jwt, inst_id))
                .await
                .map_err(|_| HttpError::internal())?
                .map_err(map_deploy_err)?;

        let gh_client = state.github.client();
        let tok_for_list = inst_token.clone();
        let metas = tokio::task::spawn_blocking(move || {
            gh_client.list_installation_repos(tok_for_list.as_str())
        })
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(map_deploy_err)?;

        let mut build_map = {
            let guard = state.github.cache.read().await;
            guard.build_by_push.clone()
        };

        let mut to_detect = Vec::new();
        for (idx, m) in metas.iter().enumerate() {
            if !build_map.contains_key(&(m.full_name.clone(), m.pushed_at))
                && to_detect.len() < MAX_DETECT_PER_REFRESH
            {
                to_detect.push((idx, m.clone()));
            }
        }

        let detected: Vec<(String, i64, String)> = stream::iter(to_detect)
            .map(|(_, m)| {
                let c = state.github.client();
                let tok = inst_token.clone();
                async move {
                    let full_name = m.full_name.clone();
                    let pushed_at = m.pushed_at;
                    let build = tokio::task::spawn_blocking(move || {
                        c.detect_repo_build(
                            Some(tok.as_str()),
                            &m.owner,
                            &m.repo_name,
                            m.language.as_deref(),
                        )
                    })
                    .await
                    .unwrap_or_else(|_| "Nixpacks".to_owned());
                    (full_name, pushed_at, build)
                }
            })
            .buffer_unordered(6)
            .collect()
            .await;

        for (full_name, pushed_at, build) in detected {
            build_map.insert((full_name, pushed_at), build);
        }

        let mut built = Vec::with_capacity(metas.len());
        for m in metas {
            let build = build_map
                .get(&(m.full_name.clone(), m.pushed_at))
                .cloned()
                .unwrap_or_else(|| {
                    opsbox_deploy::detect_build_method(&[], None, m.language.as_deref())
                });
            built.push(CachedRepo { meta: m, build });
        }

        let mut guard = state.github.cache.write().await;
        guard.cached_at = Some(Instant::now());
        guard.installation_id = Some(inst_id);
        guard.build_by_push = build_map;
        guard.repos = built.clone();
        built
    };

    let q_lower = query
        .q
        .as_deref()
        .map(|s| s.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let now = now_secs();
    let filtered: Vec<Value> = repos
        .iter()
        .filter(|r| {
            q_lower.is_empty()
                || r.meta.full_name.to_ascii_lowercase().contains(&q_lower)
                || r.meta
                    .language
                    .as_deref()
                    .is_some_and(|l| l.to_ascii_lowercase().contains(&q_lower))
                || r.build.to_ascii_lowercase().contains(&q_lower)
        })
        .map(|r| {
            json!({
                "name": r.meta.full_name,
                "vis": if r.meta.private { "private" } else { "public" },
                "lang": r.meta.language.as_deref().unwrap_or("–"),
                "pushed": format_pushed_ago(r.meta.pushed_at, now),
                "pushedAt": r.meta.pushed_at,
                "defaultBranch": r.meta.default_branch,
                "build": r.build,
                "url": r.meta.html_url,
            })
        })
        .collect();

    Ok(Json(json!({
        "connected": true,
        "installed": true,
        "account": app.installation_account.unwrap_or(app.owner_login),
        "installUrl": inst_url,
        "manageUrl": manage_url,
        "repos": filtered,
    })))
}

#[derive(Deserialize)]
pub(crate) struct PublicRepoBody {
    url: String,
}

/// `POST /api/v1/github/public-repo` — validate a public Git repository URL and detect build method.
pub(crate) async fn inspect_public_repo(
    State(state): State<AppState>,
    Deployer(_): Deployer,
    ApiJson(body): ApiJson<PublicRepoBody>,
) -> Result<Json<Value>, HttpError> {
    let target = validate_public_repo_url(&body.url).map_err(map_deploy_err)?;
    match target {
        opsbox_deploy::PublicRepoTarget::GitHub {
            owner,
            repo,
            canonical_url,
        } => {
            let client = state.github.client();
            let info =
                tokio::task::spawn_blocking(move || client.inspect_public_repo(&owner, &repo))
                    .await
                    .map_err(|_| HttpError::internal())?
                    .map_err(map_deploy_err)?;
            Ok(Json(json!({
                "repo": info.meta.full_name,
                "url": canonical_url,
                "provider": "github",
                "vis": "public",
                "lang": info.meta.language.as_deref().unwrap_or("–"),
                "defaultBranch": info.meta.default_branch,
                "build": info.build,
                "detected": true,
            })))
        }
        opsbox_deploy::PublicRepoTarget::GenericGit {
            host,
            display_name,
            canonical_url,
        } => Ok(Json(json!({
            "repo": canonical_url,
            "url": canonical_url,
            "provider": "git",
            "host": host,
            "name": display_name,
            "vis": "public",
            "lang": "Git",
            "defaultBranch": "main",
            "build": "Dockerfile",
            "detected": false,
        }))),
    }
}

/// `GET /api/v1/github/deploy-keys` — list per-repo SSH deploy keys (metadata only).
pub(crate) async fn list_deploy_keys(
    State(state): State<AppState>,
    Deployer(_): Deployer,
) -> Result<Json<Value>, HttpError> {
    let keys = deploy_keys_snapshot(&state).await;
    Ok(Json(json!(keys)))
}

#[derive(Deserialize)]
pub(crate) struct CreateDeployKeyBody {
    repo: String,
}

/// `POST /api/v1/github/deploy-keys` — generate an Ed25519 SSH deploy keypair for one private repo.
pub(crate) async fn create_deploy_key(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    ApiJson(body): ApiJson<CreateDeployKeyBody>,
) -> Result<Json<Value>, HttpError> {
    let normalized = normalize_repo_source(&body.repo).map_err(map_deploy_err)?;
    let id = ulid::Ulid::generate().to_string();
    let short = &id[..8.min(id.len())];
    let comment = format!("opsbox-deploy-{}", short.to_ascii_lowercase());
    let keypair = generate_ed25519_deploy_key(&comment).map_err(map_deploy_err)?;

    let ctx = deploy_key_context(&id);
    let private_key_enc = state
        .priv_client
        .secret_seal(
            &rid,
            Some(&who.user.name),
            &ctx,
            Secret::new(keypair.private_key_pem.as_str()),
        )
        .await?;

    let now = now_secs();
    let record = NewDeployKey {
        id: id.clone(),
        repo: normalized.repo.clone(),
        ssh_url: normalized.ssh_url.clone(),
        public_key: keypair.public_key.clone(),
        fingerprint: keypair.fingerprint.clone(),
        private_key_enc,
        created_by: who.user.name.clone(),
        created_at: now,
    };

    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.create_deploy_key(&record))
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())?;

    audit(
        &state,
        &who,
        &client.ip,
        "github.deploy_key.create",
        Some(&normalized.repo),
        json!({ "id": id, "fingerprint": keypair.fingerprint }),
    );

    Ok(Json(json!({
        "id": id,
        "repo": normalized.repo,
        "sshUrl": normalized.ssh_url,
        "publicKey": keypair.public_key,
        "fingerprint": keypair.fingerprint,
        "createdBy": who.user.name,
        "createdAt": now,
    })))
}

/// `DELETE /api/v1/github/deploy-keys/{id}` — remove a deploy key.
pub(crate) async fn delete_deploy_key(
    State(state): State<AppState>,
    Deployer(who): Deployer,
    client: Client,
    Path(id): Path<String>,
) -> Result<Json<Value>, HttpError> {
    if !valid_token_param(&id, 64) {
        return Err(HttpError::new(
            ErrorCode::Validation,
            "Invalid deploy key id.",
        ));
    }
    let store = state.store.clone();
    let key_id = id.clone();
    let deleted = tokio::task::spawn_blocking(move || store.delete_deploy_key(&key_id))
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())?;
    if !deleted {
        return Err(HttpError::not_found());
    }
    audit(
        &state,
        &who,
        &client.ip,
        "github.deploy_key.delete",
        Some(&id),
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/v1/github/webhook` — receive GitHub App webhook deliveries (HMAC-SHA256 verified).
///
/// Exempt from the panel IP allow-list because GitHub delivers webhooks from its own IPs and
/// authenticates every request with `X-Hub-Signature-256` against the sealed webhook secret.
pub(crate) async fn webhook(
    State(state): State<AppState>,
    Extension(RequestId(rid)): Extension<RequestId>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, HttpError> {
    let store = state.store.clone();
    let app = tokio::task::spawn_blocking(move || store.get_github_app())
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|_| HttpError::internal())?
        .ok_or_else(|| {
            HttpError::new(ErrorCode::NotFound, "No GitHub App webhook is configured.")
        })?;

    let secret = state
        .priv_client
        .secret_open(&rid, None, CTX_WEBHOOK_SECRET, &app.webhook_secret_enc)
        .await?;

    let sig_header = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok());
    verify_github_signature(secret.expose().as_bytes(), &body, sig_header)
        .map_err(map_deploy_err)?;

    let event_name = headers
        .get("x-github-event")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let delivery_id = headers
        .get("x-github-delivery")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.chars().take(64).collect::<String>());

    let parsed = parse_webhook_event(event_name, &body).map_err(map_deploy_err)?;
    let now = now_secs();

    let (response_json, delivery, audit_target, audit_detail) = match parsed {
        WebhookEvent::Ping { hook_id, zen } => (
            json!({ "ok": true, "event": "ping", "hookId": hook_id }),
            NewWebhookDelivery {
                delivery_id: delivery_id.clone(),
                event: "ping".into(),
                repo: None,
                git_ref: None,
                commit_sha: None,
                pusher: None,
                received_at: now,
            },
            None,
            json!({ "deliveryId": delivery_id, "hookId": hook_id, "zen": zen }),
        ),
        WebhookEvent::Push {
            repo_full_name,
            git_ref,
            branch,
            after,
            deleted,
            pusher,
            commit_message,
        } => {
            state.github.clear_cache().await;
            let short_sha = after.chars().take(12).collect::<String>();
            let (queued, triggered_deploys) = if !deleted && let Some(ref br) = branch {
                crate::deploy::trigger_webhook_deploys(
                    &state,
                    &rid,
                    &repo_full_name,
                    br,
                    &after,
                    commit_message.as_deref(),
                    pusher.as_deref(),
                )
                .await
            } else {
                (0, Vec::new())
            };
            let resp = json!({
                "ok": true,
                "event": "push",
                "repo": repo_full_name,
                "ref": git_ref,
                "branch": branch,
                "commit": after,
                "deleted": deleted,
                "queued": queued,
                "deploys": triggered_deploys,
            });
            let deliv = NewWebhookDelivery {
                delivery_id: delivery_id.clone(),
                event: "push".into(),
                repo: Some(repo_full_name.clone()),
                git_ref: Some(git_ref.clone()),
                commit_sha: Some(short_sha),
                pusher: pusher.clone(),
                received_at: now,
            };
            let detail = json!({
                "deliveryId": delivery_id,
                "ref": git_ref,
                "branch": branch,
                "commit": after,
                "deleted": deleted,
                "pusher": pusher,
                "message": commit_message.map(|m| m.chars().take(120).collect::<String>()),
            });
            (resp, deliv, Some(repo_full_name), detail)
        }
        WebhookEvent::Installation {
            action,
            installation_id,
            account_login,
            account_html_url,
            repository_selection,
        } => {
            let store = state.store.clone();
            let act = action.clone();
            let acc = account_login.clone();
            let url = account_html_url.clone();
            let sel = repository_selection.clone();
            tokio::task::spawn_blocking(move || {
                if act == "deleted" {
                    store.update_github_installation(None, None, None, None, now)
                } else {
                    store.update_github_installation(
                        Some(installation_id),
                        acc.as_deref(),
                        url.as_deref(),
                        sel.as_deref(),
                        now,
                    )
                }
            })
            .await
            .map_err(|_| HttpError::internal())?
            .map_err(|_| HttpError::internal())?;
            state.github.clear_cache().await;
            (
                json!({ "ok": true, "event": "installation", "action": action }),
                NewWebhookDelivery {
                    delivery_id: delivery_id.clone(),
                    event: format!("installation.{action}"),
                    repo: None,
                    git_ref: None,
                    commit_sha: None,
                    pusher: account_login.clone(),
                    received_at: now,
                },
                account_login,
                json!({ "deliveryId": delivery_id, "action": action, "installationId": installation_id }),
            )
        }
        WebhookEvent::InstallationRepositories {
            action,
            installation_id,
            repository_selection,
        } => {
            if let Some(ref sel) = repository_selection {
                let store = state.store.clone();
                let sel_clone = sel.clone();
                let acc = app.installation_account.clone();
                let url = app.installation_html_url.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    store.update_github_installation(
                        Some(installation_id),
                        acc.as_deref(),
                        url.as_deref(),
                        Some(&sel_clone),
                        now,
                    )
                })
                .await;
            }
            state.github.clear_cache().await;
            (
                json!({ "ok": true, "event": "installation_repositories", "action": action }),
                NewWebhookDelivery {
                    delivery_id: delivery_id.clone(),
                    event: format!("installation_repositories.{action}"),
                    repo: None,
                    git_ref: None,
                    commit_sha: None,
                    pusher: None,
                    received_at: now,
                },
                None,
                json!({ "deliveryId": delivery_id, "action": action, "installationId": installation_id }),
            )
        }
        WebhookEvent::Other { event } => (
            json!({ "ok": true, "event": event, "ignored": true }),
            NewWebhookDelivery {
                delivery_id: delivery_id.clone(),
                event: event.clone(),
                repo: None,
                git_ref: None,
                commit_sha: None,
                pusher: None,
                received_at: now,
            },
            None,
            json!({ "deliveryId": delivery_id, "event": event }),
        ),
    };

    let audit_action = format!("github.webhook.{}", delivery.event);
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        store.record_github_webhook_delivery(&delivery)?;
        store.append_audit(NewAudit {
            actor: "github-webhook".into(),
            actor_kind: ActorKind::System,
            ip: None,
            action: audit_action,
            target: audit_target,
            result: AuditResult::Ok,
            detail: audit_detail,
        })
    })
    .await
    .map_err(|_| HttpError::internal())?
    .map_err(|_| HttpError::internal())?;

    Ok((StatusCode::OK, Json(response_json)))
}
