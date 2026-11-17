//! Self-update endpoints, daily release checks, and audit outcome syncing (spec 7.15).

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::Json;
use axum::extract::{Extension, State};
use opsbox_auth::Principal;
use opsbox_proto::ErrorCode;
use opsbox_store::{ActorKind, AuditResult, Store};
use opsbox_update::{Channel, CheckResult, SlotLayout, UpdateOutcome};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::json::ApiJson;
use crate::session::{Admin, Client};
use crate::{AppState, HttpError, RequestId};

const KEY_CHANNEL: &str = "opsbox.update.channel";
const KEY_AUTO: &str = "opsbox.update.auto";
const KEY_WINDOW: &str = "opsbox.update.window";

#[derive(Clone)]
pub struct Updater {
    store: Arc<Store>,
    priv_client: opsbox_priv::PrivClient,
    xfer_dir: PathBuf,
    version: &'static str,
    cached: Arc<RwLock<Option<CheckResult>>>,
}

impl Updater {
    pub fn spawn(
        store: Arc<Store>,
        priv_client: opsbox_priv::PrivClient,
        xfer_dir: PathBuf,
        version: &'static str,
    ) -> Self {
        let updater = Self {
            store,
            priv_client,
            xfer_dir,
            version,
            cached: Arc::new(RwLock::new(None)),
        };
        updater.sync_outcome_to_audit();
        let bg = updater.clone();
        if std::env::var_os("OPSBOX_DISABLE_BG_UPDATE").is_none()
            && tokio::runtime::Handle::try_current().is_ok()
        {
            tokio::spawn(async move {
                bg.background_loop().await;
            });
        }
        updater
    }

    pub fn channel(&self) -> Channel {
        self.store
            .get_setting(KEY_CHANNEL)
            .ok()
            .flatten()
            .and_then(|v| v.as_str().and_then(Channel::parse))
            .unwrap_or(Channel::Stable)
    }

    pub fn auto_enabled(&self) -> bool {
        self.store
            .get_setting(KEY_AUTO)
            .ok()
            .flatten()
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    pub fn window_hour(&self) -> u8 {
        self.store
            .get_setting(KEY_WINDOW)
            .ok()
            .flatten()
            .and_then(|v| v.as_u64())
            .and_then(|h| u8::try_from(h).ok())
            .filter(|h| *h < 24)
            .unwrap_or(3)
    }

    pub fn latest_available(&self) -> Option<String> {
        let guard = self.cached.read().ok()?;
        let res = guard.as_ref()?;
        if res.update_available {
            res.latest.as_ref().map(|r| r.version.clone())
        } else {
            None
        }
    }

    pub fn snapshot(&self) -> Value {
        let cached = self.cached.read().ok().and_then(|g| g.clone());
        let outcome = self.sync_outcome_to_audit();
        let state = SlotLayout::default().read_state();
        json!({
            "current": self.version,
            "channel": self.channel().as_str(),
            "auto": self.auto_enabled(),
            "window": self.window_hour(),
            "latest": cached.as_ref().and_then(|c| c.latest.as_ref()).map(|r| json!({
                "version": r.version,
                "tag": r.tag,
                "prerelease": r.prerelease,
                "notesUrl": r.notes_url,
                "publishedAt": r.published_at,
            })),
            "updateAvailable": cached.as_ref().is_some_and(|c| c.update_available),
            "checkedAt": cached.as_ref().map(|c| c.checked_at),
            "previousVersion": state.previous_version,
            "lastOutcome": outcome.map(|o| json!({
                "status": o.status,
                "fromVersion": o.from_version,
                "toVersion": o.to_version,
                "message": o.message,
                "at": o.at,
            })),
        })
    }

    fn sync_outcome_to_audit(&self) -> Option<UpdateOutcome> {
        let layout = SlotLayout::default();
        let mut state = layout.read_state();
        if let Some(ref mut outcome) = state.last_outcome
            && !outcome.audited
            && outcome.status != "pending"
        {
            let action = match outcome.status.as_str() {
                "rolled_back" => "opsbox.update.rolled_back",
                "failed" => "opsbox.update.failed",
                _ => "opsbox.update.verified",
            };
            let res = if outcome.status == "updated" {
                AuditResult::Ok
            } else {
                AuditResult::Failed
            };
            let _ = self.store.append_audit(opsbox_store::NewAudit {
                actor: "opsbox".to_owned(),
                actor_kind: ActorKind::System,
                ip: None,
                action: action.to_owned(),
                target: Some(outcome.to_version.clone()),
                result: res,
                detail: json!({
                    "from": outcome.from_version,
                    "to": outcome.to_version,
                    "message": outcome.message,
                }),
            });
            outcome.audited = true;
            let _ = layout.write_state(&state);
        }
        state.last_outcome
    }

    pub async fn check_now(&self) -> Result<CheckResult, HttpError> {
        let version = self.version;
        let channel = self.channel();
        let res =
            tokio::task::spawn_blocking(move || opsbox_update::check_latest(version, channel))
                .await
                .map_err(|_| HttpError::internal())?
                .map_err(|e| {
                    HttpError::new(ErrorCode::Internal, format!("Update check failed: {e}"))
                })?;
        if let Ok(mut guard) = self.cached.write() {
            *guard = Some(res.clone());
        }
        Ok(res)
    }

    async fn apply_version(
        &self,
        rid: &str,
        actor: &str,
        version: &str,
    ) -> Result<String, HttpError> {
        let clean = version.trim_start_matches('v').to_owned();
        let xfer = self.xfer_dir.clone();
        let ver_clone = clean.clone();
        let staged = tokio::task::spawn_blocking(move || {
            opsbox_update::download_and_verify(&ver_clone, &xfer, None)
        })
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|e| {
            HttpError::new(
                ErrorCode::Internal,
                format!("Update download/verify failed: {e}"),
            )
        })?;

        let staged_str = staged.to_string_lossy().into_owned();
        self.priv_client
            .clone()
            .with_timeout(Duration::from_secs(60))
            .self_update_install(
                rid,
                actor,
                &staged_str,
                &clean,
                opsbox_store::LATEST_VERSION,
            )
            .await?;

        Ok(clean)
    }

    async fn background_loop(self) {
        tokio::time::sleep(Duration::from_secs(15)).await;
        let mut last_check = 0i64;
        let mut last_auto_day = -1i64;
        loop {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));

            if now - last_check >= 24 * 3600 {
                match self.check_now().await {
                    Ok(_) => last_check = now,
                    Err(e) => tracing::debug!(error = ?e, "daily opsbox update check failed"),
                }
            }

            if self.auto_enabled()
                && let Some(target_ver) = self.latest_available()
            {
                let day = now / 86400;
                let hour = ((now % 86400) / 3600) as u8;
                if hour == self.window_hour() && day != last_auto_day {
                    last_auto_day = day;
                    let rid = format!("auto-update-{now}");
                    if let Err(e) = self.apply_version(&rid, "opsbox-auto", &target_ver).await {
                        tracing::warn!(error = ?e, version = %target_ver, "automatic opsbox update failed");
                    }
                }
            }

            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    }
}

fn audit(
    state: &AppState,
    who: &Principal,
    ip: &str,
    action: &str,
    target: Option<&str>,
    detail: Value,
) {
    state.auth.audit(
        &who.user.name,
        ActorKind::User,
        Some(ip),
        action,
        target,
        AuditResult::Ok,
        detail,
    );
}

pub(crate) async fn get_status(State(state): State<AppState>, Admin(_): Admin) -> Json<Value> {
    Json(state.updater.snapshot())
}

pub(crate) async fn check(
    State(state): State<AppState>,
    Admin(_): Admin,
) -> Result<Json<Value>, HttpError> {
    state.updater.check_now().await?;
    Ok(Json(state.updater.snapshot()))
}

#[derive(Deserialize)]
pub(crate) struct ApplyBody {
    version: Option<String>,
}

pub(crate) async fn apply(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    body: Option<ApiJson<ApplyBody>>,
) -> Result<Json<Value>, HttpError> {
    let requested = body.and_then(|ApiJson(b)| b.version);
    let target = match requested {
        Some(v) if !v.trim().is_empty() => v,
        _ => {
            let res = state.updater.check_now().await?;
            if !res.update_available {
                return Err(HttpError::new(
                    ErrorCode::Validation,
                    "opsbox is already up to date.",
                ));
            }
            res.latest
                .map(|r| r.version)
                .ok_or_else(|| HttpError::new(ErrorCode::Validation, "No release found."))?
        }
    };
    let installed = state
        .updater
        .apply_version(&rid, &who.user.name, &target)
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "opsbox.update.start",
        Some(&installed),
        json!({ "from": state.version, "to": installed }),
    );
    Ok(Json(json!({ "ok": true, "version": installed })))
}

#[derive(Deserialize)]
pub(crate) struct RollbackBody {
    version: Option<String>,
}

pub(crate) async fn rollback(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    Extension(RequestId(rid)): Extension<RequestId>,
    body: Option<ApiJson<RollbackBody>>,
) -> Result<Json<Value>, HttpError> {
    let target = body.and_then(|ApiJson(b)| b.version);
    state
        .priv_client
        .self_update_rollback(&rid, &who.user.name, target.as_deref())
        .await?;
    audit(
        &state,
        &who,
        &client.ip,
        "opsbox.update.rollback",
        target.as_deref(),
        json!({}),
    );
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub(crate) struct AutoBody {
    channel: Option<String>,
    auto: Option<bool>,
    window: Option<u8>,
}

pub(crate) async fn set_auto(
    State(state): State<AppState>,
    Admin(who): Admin,
    client: Client,
    ApiJson(body): ApiJson<AutoBody>,
) -> Result<Json<Value>, HttpError> {
    if let Some(ref ch) = body.channel {
        let parsed = Channel::parse(ch).ok_or_else(|| {
            HttpError::new(ErrorCode::Validation, "Channel must be stable or beta.")
        })?;
        state
            .store
            .set_setting(KEY_CHANNEL, &json!(parsed.as_str()))
            .map_err(|_| HttpError::internal())?;
        if let Ok(mut guard) = state.updater.cached.write() {
            *guard = None;
        }
    }
    if let Some(auto) = body.auto {
        state
            .store
            .set_setting(KEY_AUTO, &json!(auto))
            .map_err(|_| HttpError::internal())?;
    }
    if let Some(w) = body.window {
        if w >= 24 {
            return Err(HttpError::new(
                ErrorCode::Validation,
                "Daily window must be between 0 and 23.",
            ));
        }
        state
            .store
            .set_setting(KEY_WINDOW, &json!(w))
            .map_err(|_| HttpError::internal())?;
    }
    audit(
        &state,
        &who,
        &client.ip,
        "opsbox.update.settings",
        None,
        json!({
            "channel": state.updater.channel().as_str(),
            "auto": state.updater.auto_enabled(),
            "window": state.updater.window_hour(),
        }),
    );
    Ok(Json(state.updater.snapshot()))
}
