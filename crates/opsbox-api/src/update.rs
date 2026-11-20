//! Self-update endpoints, daily release checks, and audit outcome syncing (spec 7.15).

use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::Json;
use axum::extract::{Extension, State};
use opsbox_auth::Principal;
use opsbox_proto::ErrorCode;
use opsbox_store::{ActorKind, AuditResult, Store};
use opsbox_update::{
    Channel, CheckResult, NoOfferReason, ReleaseChecker, SlotLayout, UpdateOutcome,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::json::ApiJson;
use crate::session::{Admin, Client};
use crate::{AppState, HttpError, RequestId};

const KEY_CHANNEL: &str = "opsbox.update.channel";
const KEY_AUTO: &str = "opsbox.update.auto";
const KEY_WINDOW: &str = "opsbox.update.window";
const KEY_AUDITED_OUTCOME: &str = "opsbox.update.last_audited_outcome";

/// Shown with the one-time offer to an install that predates version tagging.
const MIGRATION_NOTE: &str =
    "Your install predates version tagging; update to get accurate versions.";

/// Where the update channel in use was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelSource {
    /// Set in the panel (Settings > opsbox updates).
    Setting,
    /// `[update] channel` in the config file, written by the installer.
    Config,
    /// Nothing chosen: beta for a pre-release build, stable otherwise.
    Default,
}

impl ChannelSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Setting => "setting",
            Self::Config => "config",
            Self::Default => "default",
        }
    }
}

#[derive(Clone)]
pub struct Updater {
    store: Arc<Store>,
    priv_client: opsbox_priv::PrivClient,
    xfer_dir: PathBuf,
    version: &'static str,
    /// `[update] channel` from the config file; the panel setting wins over it.
    config_channel: Option<Channel>,
    checker: Arc<ReleaseChecker>,
    cached: Arc<RwLock<Option<CheckResult>>>,
    first_seen: Arc<RwLock<Option<(String, i64)>>>,
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
            config_channel: None,
            checker: Arc::new(ReleaseChecker::default()),
            cached: Arc::new(RwLock::new(None)),
            first_seen: Arc::new(RwLock::new(None)),
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

    /// Replace the release lookup (tests script GitHub's answers).
    pub fn with_checker(mut self, checker: Arc<ReleaseChecker>) -> Self {
        self.checker = checker;
        self
    }

    pub fn with_config_channel(mut self, channel: Option<Channel>) -> Self {
        self.config_channel = channel;
        self
    }

    /// The channel in use and where it came from: the panel setting, the config file
    /// (written by `install.sh --channel beta`), or the default for this version.
    pub fn channel_with_source(&self) -> (Channel, ChannelSource) {
        let stored = self
            .store
            .get_setting(KEY_CHANNEL)
            .ok()
            .flatten()
            .and_then(|v| v.as_str().and_then(Channel::parse));
        if let Some(ch) = stored {
            (ch, ChannelSource::Setting)
        } else if let Some(ch) = self.config_channel {
            (ch, ChannelSource::Config)
        } else {
            (
                opsbox_update::default_channel(self.version),
                ChannelSource::Default,
            )
        }
    }

    pub fn channel(&self) -> Channel {
        self.channel_with_source().0
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

    pub fn latest_available_with_since(&self) -> Option<(String, i64)> {
        let guard = self.cached.read().ok()?;
        let res = guard.as_ref()?;
        if !res.update_available {
            return None;
        }
        let rel = res.latest.as_ref()?;
        if let Some(ref pub_at) = rel.published_at
            && let Ok(dt) =
                time::OffsetDateTime::parse(pub_at, &time::format_description::well_known::Rfc3339)
        {
            return Some((rel.version.clone(), dt.unix_timestamp().max(1)));
        }
        let seen = self
            .first_seen
            .read()
            .ok()
            .and_then(|g| g.clone())
            .filter(|(v, _)| v == &rel.version)
            .map_or_else(|| res.checked_at.max(1), |(_, ts)| ts.max(1));
        Some((rel.version.clone(), seen))
    }

    pub fn snapshot(&self) -> Value {
        let cached = self.cached.read().ok().and_then(|g| g.clone());
        let outcome = self.sync_outcome_to_audit();
        let state = SlotLayout::default().read_state();
        let (channel, source) = self.channel_with_source();
        json!({
            "current": self.version,
            "channel": channel.as_str(),
            "channelSource": source.as_str(),
            "devBuild": opsbox_version::is_dev(self.version),
            "migration": cached.as_ref().is_some_and(|c| c.migration),
            "migrationNote": cached.as_ref().filter(|c| c.migration).map(|_| MIGRATION_NOTE),
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
            "reason": cached.as_ref().and_then(|c| c.reason),
            "reasonDetail": cached.as_ref().and_then(|c| c.detail.as_deref()),
            "retryAt": cached.as_ref().and_then(|c| c.retry_at),
            "checkedChannel": cached.as_ref().map(|c| c.channel.as_str()),
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
            && outcome.status != "pending"
        {
            let outcome_key = format!("{}:{}:{}", outcome.status, outcome.to_version, outcome.at);
            let already_audited = outcome.audited
                || self
                    .store
                    .get_setting(KEY_AUDITED_OUTCOME)
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_str().map(|s| s == outcome_key))
                    .unwrap_or(false);

            if !already_audited {
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
                let _ = self
                    .store
                    .set_setting(KEY_AUDITED_OUTCOME, &json!(outcome_key));
                outcome.audited = true;
                let _ = layout.write_state(&state);
            }
        }
        state.last_outcome
    }

    /// Look up the newest release. `force` ("Check now") skips the one-hour cache. A lookup
    /// that fails (offline, rate limited) is a result with a `reason`, and never replaces an
    /// offer for the same channel that an earlier check found.
    pub async fn check_now(&self, force: bool) -> Result<CheckResult, HttpError> {
        let version = self.version;
        let (channel, source) = self.channel_with_source();
        let checker = self.checker.clone();
        let res = tokio::task::spawn_blocking(move || {
            let chosen = (source != ChannelSource::Default).then_some(channel);
            checker.check(version, chosen, force)
        })
        .await
        .map_err(|_| HttpError::internal())?
        .map_err(|e| HttpError::new(ErrorCode::Internal, format!("Update check failed: {e}")))?;
        let failed = matches!(
            res.reason,
            Some(NoOfferReason::NetworkError | NoOfferReason::RateLimited)
        );
        if failed
            && self
                .cached
                .read()
                .ok()
                .and_then(|g| g.clone())
                .is_some_and(|prev| prev.update_available && prev.channel == res.channel)
        {
            return Ok(res);
        }
        if let Ok(mut seen) = self.first_seen.write() {
            if res.update_available
                && let Some(ref rel) = res.latest
            {
                let same = seen.as_ref().is_some_and(|(v, _)| v == &rel.version);
                if !same {
                    *seen = Some((rel.version.clone(), res.checked_at.max(1)));
                }
            } else {
                *seen = None;
            }
        }
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
        let (staged, sha256, minisign) = tokio::task::spawn_blocking(move || {
            opsbox_update::download_for_install(&ver_clone, &xfer)
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
                opsbox_proto::UpdateVerification { sha256, minisign },
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
                match self.check_now(false).await {
                    // A failed lookup is tried again within the hour, not tomorrow.
                    Ok(res)
                        if !matches!(
                            res.reason,
                            Some(NoOfferReason::NetworkError | NoOfferReason::RateLimited)
                        ) =>
                    {
                        last_check = now;
                    }
                    Ok(res) => {
                        tracing::debug!(reason = ?res.reason, "daily opsbox update check found nothing")
                    }
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

/// Why `apply` with no version has nothing to install, in words for the person clicking.
fn nothing_to_apply(res: &CheckResult) -> String {
    match res.reason {
        Some(NoOfferReason::DevBuild) => "This is a development build, so it is not updated automatically. Pick a version to install.".to_owned(),
        Some(NoOfferReason::NoReleasesOnChannel) => format!(
            "There is no release on the {} channel yet.",
            res.channel.as_str()
        ),
        Some(NoOfferReason::NetworkError) => {
            "opsbox could not reach GitHub to look for a release. Check the server's internet connection and try again.".to_owned()
        }
        Some(NoOfferReason::RateLimited) => {
            "GitHub is limiting update checks right now. Try again in a little while.".to_owned()
        }
        Some(NoOfferReason::UpToDate) | None => "opsbox is already up to date.".to_owned(),
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
    let actor = who.user.actor_label();
    state.auth.audit(
        &actor,
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
    // "Check now": always ask GitHub, whatever the cache holds.
    state.updater.check_now(true).await?;
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
    crate::session::deny_token(&who)?;
    let requested = body.and_then(|ApiJson(b)| b.version);
    let target = match requested {
        Some(v) if !v.trim().is_empty() => v,
        _ => {
            let res = state.updater.check_now(false).await?;
            if !res.update_available {
                return Err(HttpError::new(
                    ErrorCode::Validation,
                    nothing_to_apply(&res),
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
    crate::session::deny_token(&who)?;
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
    crate::session::deny_token(&who)?;
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
        // Answer for the new channel at once; the release list is usually still cached.
        let _ = state.updater.check_now(false).await;
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
