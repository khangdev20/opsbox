use std::sync::Arc;

use opsbox_priv::{ClientError, PrivClient};
use opsbox_proto::Role;
use opsbox_store::{
    ActorKind, AuditResult, NewAudit, Session, SessionKind, Store, StoreError, User,
};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tracing::{error, warn};

use crate::lockout::{self, WINDOW_SECS};
use crate::password::{HashParams, Hasher, check_new_password, check_username};
use crate::setup_token;
use crate::tokens::{self, csrf_token, hash_token, new_token};

const SETUP_TOKEN_KEY: &str = "setup.token";
const SETUP_SESSION_KEY: &str = "setup.session";
const SETUP_TOKEN_TTL: i64 = 60 * 60;
const SETUP_SESSION_TTL: i64 = 30 * 60;
/// Sessions that can only finish two-factor enrolment are short.
const ENROLL_TTL: i64 = 15 * 60;
/// `last_seen` is refreshed at most this often, so reads do not write.
const TOUCH_EVERY: i64 = 60;

type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub hash_params: HashParams,
    /// Sign-ins hashing at once; each needs `hash_params.memory_kib` of memory.
    pub max_parallel_hashes: usize,
    pub idle_secs: i64,
    pub absolute_secs: i64,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            hash_params: HashParams::default(),
            max_parallel_hashes: 2,
            idle_secs: 12 * 3600,
            absolute_secs: 7 * 24 * 3600,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Wrong username, password or code.")]
    InvalidCredentials,
    #[error("Too many attempts. Try again in {0} seconds.")]
    Locked(u64),
    #[error("{message}")]
    Invalid {
        field: &'static str,
        message: String,
    },
    #[error("There is nothing at this address.")]
    NotFound,
    #[error("The privileged helper is not running. Start opsbox-priv and try again.")]
    HelperUnavailable,
    #[error("internal error: {0}")]
    Internal(String),
}

impl From<StoreError> for AuthError {
    fn from(e: StoreError) -> Self {
        error!(error = %e, "store error");
        Self::Internal("database error".into())
    }
}

impl From<ClientError> for AuthError {
    fn from(e: ClientError) -> Self {
        error!(error = %e, "privileged helper call failed");
        match e {
            ClientError::Connect { .. } | ClientError::Timeout => Self::HelperUnavailable,
            other => Self::Internal(other.to_string()),
        }
    }
}

impl From<getrandom::Error> for AuthError {
    fn from(_: getrandom::Error) -> Self {
        Self::Internal("random number generator failed".into())
    }
}

fn invalid(field: &'static str, message: impl Into<String>) -> AuthError {
    AuthError::Invalid {
        field,
        message: message.into(),
    }
}

pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub code: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

/// A freshly created session: the secret token (for the cookie) and its CSRF token.
#[derive(Debug)]
pub struct Issued {
    pub token: String,
    pub csrf: String,
    pub user: User,
}

#[derive(Debug)]
pub enum LoginOutcome {
    /// Signed in.
    Session(Issued),
    /// The password was right but the account has no second factor yet.
    EnrollRequired(Issued),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKindView {
    Web,
    Enroll,
}

/// Who is making a request.
#[derive(Debug, Clone)]
pub struct Principal {
    pub user: User,
    pub session_id: String,
    pub kind: SessionKindView,
}

/// What the person needs to set up their authenticator app.
#[derive(Debug)]
pub struct Enrolment {
    pub secret_base32: String,
    pub otpauth_uri: String,
}

struct Inner {
    store: Arc<Store>,
    priv_client: PrivClient,
    hasher: Arc<Hasher>,
    slots: Semaphore,
    dummy_hash: String,
    cfg: AuthConfig,
    clock: Clock,
}

#[derive(Clone)]
pub struct Auth {
    inner: Arc<Inner>,
}

fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
    })
}

impl Auth {
    pub fn new(store: Arc<Store>, priv_client: PrivClient, cfg: AuthConfig) -> Self {
        Self::with_clock(store, priv_client, cfg, system_clock())
    }

    /// For tests: time comes from `clock` (unix seconds).
    pub fn with_clock(
        store: Arc<Store>,
        priv_client: PrivClient,
        cfg: AuthConfig,
        clock: Clock,
    ) -> Self {
        let hasher = Arc::new(Hasher::new(cfg.hash_params));
        // Unknown user names are checked against this so they cost the same time as
        // real ones; otherwise response time reveals which accounts exist.
        let dummy_hash = hasher
            .hash("opsbox-timing-equaliser")
            .expect("hashing works");
        Self {
            inner: Arc::new(Inner {
                slots: Semaphore::new(cfg.max_parallel_hashes.max(1)),
                store,
                priv_client,
                hasher,
                dummy_hash,
                cfg,
                clock,
            }),
        }
    }

    fn now(&self) -> i64 {
        (self.inner.clock)()
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.inner.store
    }

    // ------------------------------------------------------------- hashing

    async fn verify_password(&self, password: &str, stored: &str) -> bool {
        let Ok(_permit) = self.inner.slots.acquire().await else {
            return false;
        };
        let (hasher, password, stored) = (
            self.inner.hasher.clone(),
            password.to_owned(),
            stored.to_owned(),
        );
        tokio::task::spawn_blocking(move || hasher.verify(&password, &stored))
            .await
            .unwrap_or(false)
    }

    /// Hash a password on a blocking thread. Callers validate it first.
    pub async fn hash_password(&self, password: &str) -> Result<String, AuthError> {
        let _permit = self
            .inner
            .slots
            .acquire()
            .await
            .map_err(|_| AuthError::Internal("shutting down".into()))?;
        let (hasher, password) = (self.inner.hasher.clone(), password.to_owned());
        tokio::task::spawn_blocking(move || hasher.hash(&password))
            .await
            .map_err(|_| AuthError::Internal("hashing crashed".into()))?
            .map_err(AuthError::Internal)
    }

    // --------------------------------------------------------------- audit

    #[allow(clippy::too_many_arguments)]
    pub fn audit(
        &self,
        actor: &str,
        kind: ActorKind,
        ip: Option<&str>,
        action: &str,
        target: Option<&str>,
        result: AuditResult,
        detail: Value,
    ) {
        let entry = NewAudit {
            actor: actor.to_owned(),
            actor_kind: kind,
            ip: ip.map(str::to_owned),
            action: action.to_owned(),
            target: target.map(str::to_owned),
            result,
            detail,
        };
        if let Err(e) = self.inner.store.append_audit(entry) {
            // Never let a failing log hide the action, but make the failure loud.
            error!(error = %e, action, "could not write the audit log");
        }
    }

    fn audit_user(
        &self,
        user: &User,
        ip: Option<&str>,
        action: &str,
        result: AuditResult,
        detail: Value,
    ) {
        self.audit(
            &user.name,
            ActorKind::User,
            ip,
            action,
            Some(&user.name),
            result,
            detail,
        );
    }

    // ------------------------------------------------------------- lockout

    fn check_locked(&self, name: &str, ip: &str) -> Result<(), AuthError> {
        let now = self.now();
        let since = now - WINDOW_SECS;
        let by_user = self.inner.store.failures_for_user(name, since)?;
        let by_ip = self.inner.store.failures_for_ip(ip, since)?;
        let left = [by_user, by_ip]
            .into_iter()
            .filter_map(|(n, newest)| lockout::remaining(n, newest, now))
            .max();
        match left {
            Some(secs) => Err(AuthError::Locked(u64::try_from(secs).unwrap_or(1))),
            None => Ok(()),
        }
    }

    /// Count a failed attempt; returns `true` if it tipped the user into a lockout.
    fn record_failure(&self, name: &str, ip: &str, user: Option<&User>) -> bool {
        let now = self.now();
        if let Err(e) = self.inner.store.record_login_failure(name, ip, now) {
            error!(error = %e, "could not record a sign-in failure");
            return false;
        }
        let Ok((n, _)) = self.inner.store.failures_for_user(name, now - WINDOW_SECS) else {
            return false;
        };
        if let Some(u) = user {
            let until = (lockout::lock_secs(n) > 0).then(|| now + lockout::lock_secs(n));
            let _ = self.inner.store.set_lock(&u.id, until, n);
        }
        n == lockout::THRESHOLD
    }

    // --------------------------------------------------------------- login

    pub async fn login(
        &self,
        request_id: &str,
        req: LoginRequest,
    ) -> Result<LoginOutcome, AuthError> {
        let name = req.username.trim().to_lowercase();
        let ip = req.ip.as_deref().unwrap_or("unknown").to_owned();
        if name.is_empty() || req.password.is_empty() {
            return Err(invalid("username", "Enter a username and password."));
        }
        self.check_locked(&name, &ip)?;

        let user = self.inner.store.user_by_name(&name)?;
        let password_ok = match &user {
            Some(u) => self.verify_password(&req.password, &u.password_hash).await,
            None => {
                self.verify_password(&req.password, &self.inner.dummy_hash)
                    .await;
                false
            }
        };

        let Some(user) = user.filter(|_| password_ok) else {
            // `user` was moved by the filter above; look it up again for the audit entry.
            let known = self.inner.store.user_by_name(&name)?;
            return Err(self.failed_login(
                &name,
                &ip,
                known.as_ref(),
                "bad password or unknown user",
            ));
        };

        // No second factor yet (new reset, or abandoned enrolment): hand out a short
        // session that can only finish enrolment. The password was right.
        if !user.totp_enabled {
            let issued = self.issue(
                &user,
                SessionKind::Enroll,
                req.ip.as_deref(),
                req.user_agent.as_deref(),
            )?;
            self.audit_user(
                &user,
                req.ip.as_deref(),
                "auth.login",
                AuditResult::Ok,
                json!({"needs_2fa_enrolment": true}),
            );
            return Ok(LoginOutcome::EnrollRequired(issued));
        }

        let sealed = user
            .totp_secret
            .clone()
            .ok_or_else(|| AuthError::Internal("two-factor is on but has no secret".into()))?;
        let step = self
            .inner
            .priv_client
            .totp_verify(
                request_id,
                &sealed,
                &format!("totp:{}", user.id),
                req.code.trim(),
                user.totp_last_step,
            )
            .await?;
        let accepted = match step {
            // Only one of two simultaneous sign-ins with the same code can win.
            Some(step) => self.inner.store.advance_totp_step(&user.id, step)?,
            None => false,
        };
        if !accepted {
            return Err(self.failed_login(&name, &ip, Some(&user), "bad code"));
        }

        self.inner.store.clear_failures_for_user(&name)?;
        self.inner.store.set_lock(&user.id, None, 0)?;
        let issued = self.issue(
            &user,
            SessionKind::Web,
            req.ip.as_deref(),
            req.user_agent.as_deref(),
        )?;
        self.audit_user(
            &user,
            req.ip.as_deref(),
            "auth.login",
            AuditResult::Ok,
            json!({}),
        );
        Ok(LoginOutcome::Session(issued))
    }

    fn failed_login(&self, name: &str, ip: &str, user: Option<&User>, why: &str) -> AuthError {
        let locked_now = self.record_failure(name, ip, user);
        // The audit entry says why; the caller is told only that something was wrong.
        let attempted = (check_username(name).is_ok()).then_some(name);
        self.audit(
            user.map_or("unknown", |u| u.name.as_str()),
            ActorKind::User,
            Some(ip),
            "auth.login",
            attempted,
            AuditResult::Denied,
            json!({ "reason": why }),
        );
        if locked_now {
            warn!(user = name, ip, "sign-in lockout started");
            self.audit(
                "opsbox",
                ActorKind::System,
                Some(ip),
                "auth.lockout",
                attempted,
                AuditResult::Ok,
                json!({}),
            );
        }
        AuthError::InvalidCredentials
    }

    fn issue(
        &self,
        user: &User,
        kind: SessionKind,
        ip: Option<&str>,
        ua: Option<&str>,
    ) -> Result<Issued, AuthError> {
        let token = new_token()?;
        let ua = ua.map(|u| u.chars().take(200).collect::<String>());
        self.inner.store.create_session(
            &user.id,
            &hash_token(&token),
            kind,
            ip,
            ua.as_deref(),
            self.now(),
        )?;
        Ok(Issued {
            csrf: csrf_token(&token),
            token,
            user: user.clone(),
        })
    }

    // ------------------------------------------------------------ sessions

    /// The signed-in user for a session token, or `None` if it is unknown or expired.
    pub fn authenticate(&self, token: &str) -> Result<Option<Principal>, AuthError> {
        let now = self.now();
        let Some((session, user)) = self.inner.store.session_by_token_hash(&hash_token(token))?
        else {
            return Ok(None);
        };
        let (idle, absolute) = match session.kind {
            SessionKind::Web => (self.inner.cfg.idle_secs, self.inner.cfg.absolute_secs),
            SessionKind::Enroll => (ENROLL_TTL, ENROLL_TTL),
        };
        if now - session.last_seen > idle || now - session.created_at > absolute {
            self.inner.store.delete_session(&session.id)?;
            return Ok(None);
        }
        if now - session.last_seen >= TOUCH_EVERY {
            self.inner.store.touch_session(&session.id, now)?;
        }
        Ok(Some(principal(session, user)))
    }

    pub fn logout(&self, who: &Principal, ip: Option<&str>) -> Result<(), AuthError> {
        self.inner.store.delete_session(&who.session_id)?;
        self.audit_user(&who.user, ip, "auth.logout", AuditResult::Ok, json!({}));
        Ok(())
    }

    /// Sign the user out on every device. Returns how many sessions ended.
    pub fn revoke_all(&self, user: &User, ip: Option<&str>) -> Result<usize, AuthError> {
        let n = self.inner.store.delete_user_sessions(&user.id)?;
        self.audit_user(
            user,
            ip,
            "auth.sessions.revoke_all",
            AuditResult::Ok,
            json!({ "sessions": n }),
        );
        Ok(n)
    }

    /// Housekeeping: drop expired sessions and old failure rows.
    pub fn purge_expired(&self) -> Result<(), AuthError> {
        let now = self.now();
        self.inner.store.purge_sessions(
            now - self.inner.cfg.idle_secs,
            now - self.inner.cfg.absolute_secs,
        )?;
        self.inner.store.prune_failures(now - 24 * 3600)?;
        Ok(())
    }

    // ------------------------------------------------------ 2FA enrolment

    /// Start (or restart) two-factor enrolment: a new secret is stored, not yet active.
    pub async fn begin_totp(
        &self,
        request_id: &str,
        user: &User,
        host: &str,
    ) -> Result<Enrolment, AuthError> {
        let (sealed, secret) = self
            .inner
            .priv_client
            .totp_new_secret(request_id, &format!("totp:{}", user.id))
            .await?;
        self.inner
            .store
            .set_pending_totp(&user.id, &sealed, self.now())?;
        let secret = secret.expose().to_owned();
        let issuer = format!("opsbox {host}");
        let uri = format!(
            "otpauth://totp/{}:{}?secret={secret}&issuer={}&algorithm=SHA1&digits=6&period=30",
            pct(&issuer),
            pct(&user.name),
            pct(&issuer)
        );
        Ok(Enrolment {
            secret_base32: secret,
            otpauth_uri: uri,
        })
    }

    /// Check the first code from the authenticator app and switch two-factor on.
    pub async fn confirm_totp(
        &self,
        request_id: &str,
        user: &User,
        code: &str,
        ip: &str,
    ) -> Result<(), AuthError> {
        self.check_locked(&user.name, ip)?;
        let fresh = self
            .inner
            .store
            .user_by_id(&user.id)?
            .ok_or(AuthError::NotFound)?;
        let Some(sealed) = fresh.totp_secret.clone().filter(|_| !fresh.totp_enabled) else {
            return Err(invalid(
                "code",
                "Two-factor setup has not been started for this account.",
            ));
        };
        let step = self
            .inner
            .priv_client
            .totp_verify(
                request_id,
                &sealed,
                &format!("totp:{}", fresh.id),
                code.trim(),
                0,
            )
            .await?;
        let Some(step) = step else {
            self.record_failure(&fresh.name, ip, Some(&fresh));
            self.audit_user(
                &fresh,
                Some(ip),
                "auth.2fa.confirm",
                AuditResult::Denied,
                json!({}),
            );
            return Err(invalid(
                "code",
                "That code is not right. Check the clock on your phone and try the next code.",
            ));
        };
        self.inner.store.enable_totp(&fresh.id, step, self.now())?;
        self.inner.store.clear_failures_for_user(&fresh.name)?;
        self.audit_user(
            &fresh,
            Some(ip),
            "auth.2fa.enable",
            AuditResult::Ok,
            json!({}),
        );
        Ok(())
    }

    /// An enrolment session becomes a normal one once two-factor is on.
    pub fn upgrade_enroll_session(
        &self,
        who: &Principal,
        ip: Option<&str>,
        ua: Option<&str>,
    ) -> Result<Issued, AuthError> {
        self.inner.store.delete_session(&who.session_id)?;
        let user = self
            .inner
            .store
            .user_by_id(&who.user.id)?
            .ok_or(AuthError::NotFound)?;
        self.issue(&user, SessionKind::Web, ip, ua)
    }

    // --------------------------------------------------------------- setup

    /// First run: no admin has finished two-factor enrolment yet.
    pub fn setup_required(&self) -> Result<bool, AuthError> {
        Ok(self.inner.store.enrolled_admin_count()? == 0)
    }

    /// Make a new single-use token, valid one hour. Any earlier token or setup
    /// session stops working. Fails with `NotFound` once an admin exists.
    pub fn issue_setup_token(&self) -> Result<String, AuthError> {
        if !self.setup_required()? {
            return Err(AuthError::NotFound);
        }
        let token = setup_token::generate()?;
        self.inner.store.set_setting(
            SETUP_TOKEN_KEY,
            &json!({ "hash": hash_token(&token), "expires": self.now() + SETUP_TOKEN_TTL }),
        )?;
        self.inner.store.delete_setting(SETUP_SESSION_KEY)?;
        self.audit(
            "opsbox",
            ActorKind::System,
            None,
            "setup.token.issue",
            None,
            AuditResult::Ok,
            json!({}),
        );
        Ok(token)
    }

    /// At startup: if setup is needed and no live token exists, make one.
    pub fn ensure_setup_token(&self) -> Result<Option<String>, AuthError> {
        if !self.setup_required()? {
            return Ok(None);
        }
        let live = self
            .inner
            .store
            .get_setting(SETUP_TOKEN_KEY)?
            .and_then(|v| v["expires"].as_i64())
            .is_some_and(|exp| exp > self.now());
        if live {
            Ok(None)
        } else {
            self.issue_setup_token().map(Some)
        }
    }

    /// Exchange the setup token for a setup session (a secret for the next two steps).
    pub fn claim_setup(&self, token: &str, ip: &str) -> Result<String, AuthError> {
        if !self.setup_required()? {
            return Err(AuthError::NotFound);
        }
        self.check_locked("setup-claim", ip)?;
        let wrong = || {
            invalid(
                "token",
                "That token is not right, or it has expired. Run `sudo opsbox setup-token` on the server for a new one.",
            )
        };
        let stored = self.inner.store.get_setting(SETUP_TOKEN_KEY)?;
        let matches = (|| {
            let normalized = setup_token::normalize(token)?;
            let stored = stored.as_ref()?;
            let live = stored["expires"].as_i64()? > self.now();
            (live && tokens::ct_eq(&hash_token(&normalized), stored["hash"].as_str()?))
                .then_some(())
        })()
        .is_some();
        if !matches {
            self.record_failure("setup-claim", ip, None);
            self.audit(
                "unknown",
                ActorKind::User,
                Some(ip),
                "setup.claim",
                None,
                AuditResult::Denied,
                json!({}),
            );
            return Err(wrong());
        }
        // Single use: the token is gone the moment it is accepted.
        self.inner.store.delete_setting(SETUP_TOKEN_KEY)?;
        let session = new_token()?;
        self.inner.store.set_setting(
            SETUP_SESSION_KEY,
            &json!({ "hash": hash_token(&session), "expires": self.now() + SETUP_SESSION_TTL }),
        )?;
        self.inner.store.clear_failures_for_user("setup-claim")?;
        self.audit(
            "setup",
            ActorKind::User,
            Some(ip),
            "setup.claim",
            None,
            AuditResult::Ok,
            json!({}),
        );
        Ok(session)
    }

    fn check_setup_session(&self, session: &str) -> Result<(), AuthError> {
        if !self.setup_required()? {
            return Err(AuthError::NotFound);
        }
        let stored = self.inner.store.get_setting(SETUP_SESSION_KEY)?;
        let ok = stored.as_ref().is_some_and(|s| {
            s["expires"].as_i64().is_some_and(|e| e > self.now())
                && s["hash"]
                    .as_str()
                    .is_some_and(|h| tokens::ct_eq(h, &hash_token(session)))
        });
        if ok {
            Ok(())
        } else {
            Err(invalid(
                "token",
                "The setup session has expired. Start again with a new token.",
            ))
        }
    }

    /// Step 2: create the admin account and start two-factor enrolment for it.
    pub async fn create_admin(
        &self,
        request_id: &str,
        setup_session: &str,
        username: &str,
        password: &str,
        host: &str,
        ip: &str,
    ) -> Result<Enrolment, AuthError> {
        self.check_setup_session(setup_session)?;
        let name = username.trim().to_lowercase();
        check_username(&name).map_err(|m| invalid("username", m))?;
        check_new_password(password, &name).map_err(|m| invalid("password", m))?;
        let hash = self.hash_password(password).await?;
        // An earlier attempt that never finished enrolment is replaced.
        self.inner.store.delete_unenrolled_admins()?;
        let user = self
            .inner
            .store
            .create_user(&name, Role::Admin, &hash, self.now())?;
        let enrolment = self.begin_totp(request_id, &user, host).await?;
        let mut stored = self
            .inner
            .store
            .get_setting(SETUP_SESSION_KEY)?
            .unwrap_or_else(|| json!({}));
        stored["user_id"] = json!(user.id);
        self.inner.store.set_setting(SETUP_SESSION_KEY, &stored)?;
        self.audit(
            "setup",
            ActorKind::User,
            Some(ip),
            "setup.admin",
            Some(&name),
            AuditResult::Ok,
            json!({}),
        );
        Ok(enrolment)
    }

    /// Step 3: confirm the first code; the admin is signed in and setup closes.
    pub async fn finish_setup(
        &self,
        request_id: &str,
        setup_session: &str,
        code: &str,
        ip: &str,
        ua: Option<&str>,
    ) -> Result<Issued, AuthError> {
        self.check_setup_session(setup_session)?;
        let stored = self
            .inner
            .store
            .get_setting(SETUP_SESSION_KEY)?
            .unwrap_or_default();
        let user_id = stored["user_id"]
            .as_str()
            .ok_or_else(|| invalid("code", "Create the admin account first."))?;
        let user = self
            .inner
            .store
            .user_by_id(user_id)?
            .ok_or_else(|| invalid("code", "Create the admin account first."))?;
        self.confirm_totp(request_id, &user, code, ip).await?;
        self.inner.store.delete_setting(SETUP_SESSION_KEY)?;
        self.inner.store.delete_setting(SETUP_TOKEN_KEY)?;
        let issued = self.issue(&user, SessionKind::Web, Some(ip), ua)?;
        self.audit_user(
            &user,
            Some(ip),
            "setup.complete",
            AuditResult::Ok,
            json!({}),
        );
        Ok(issued)
    }

    // -------------------------------------------------- recovery (CLI use)

    /// `opsbox user reset-2fa`: forget the second factor and end all sessions. The
    /// next sign-in (password only) leads straight to enrolment.
    pub fn reset_2fa(&self, name: &str, actor: &str) -> Result<(), AuthError> {
        let user = self
            .inner
            .store
            .user_by_name(&name.to_lowercase())?
            .ok_or_else(|| invalid("name", format!("No such user: {name}")))?;
        self.inner.store.clear_totp(&user.id, self.now())?;
        self.inner.store.delete_user_sessions(&user.id)?;
        self.audit(
            actor,
            ActorKind::System,
            None,
            "user.reset_2fa",
            Some(&user.name),
            AuditResult::Ok,
            json!({}),
        );
        Ok(())
    }

    /// `opsbox user unlock`.
    pub fn unlock(&self, name: &str, actor: &str) -> Result<(), AuthError> {
        let user = self
            .inner
            .store
            .user_by_name(&name.to_lowercase())?
            .ok_or_else(|| invalid("name", format!("No such user: {name}")))?;
        self.inner.store.set_lock(&user.id, None, 0)?;
        self.inner.store.clear_failures_for_user(&user.name)?;
        self.audit(
            actor,
            ActorKind::System,
            None,
            "user.unlock",
            Some(&user.name),
            AuditResult::Ok,
            json!({}),
        );
        Ok(())
    }

    /// `opsbox user password`: set a new password and end all sessions.
    pub async fn set_password(
        &self,
        name: &str,
        new_password: &str,
        actor: &str,
    ) -> Result<(), AuthError> {
        let user = self
            .inner
            .store
            .user_by_name(&name.to_lowercase())?
            .ok_or_else(|| invalid("name", format!("No such user: {name}")))?;
        check_new_password(new_password, &user.name).map_err(|m| invalid("password", m))?;
        let hash = self.hash_password(new_password).await?;
        self.inner
            .store
            .set_password_hash(&user.id, &hash, self.now())?;
        self.inner.store.delete_user_sessions(&user.id)?;
        self.audit(
            actor,
            ActorKind::System,
            None,
            "user.password",
            Some(&user.name),
            AuditResult::Ok,
            json!({}),
        );
        Ok(())
    }
}

fn principal(session: Session, user: User) -> Principal {
    Principal {
        user,
        session_id: session.id,
        kind: match session.kind {
            SessionKind::Web => SessionKindView::Web,
            SessionKind::Enroll => SessionKindView::Enroll,
        },
    }
}

/// Percent-encode for a URI path or query component (RFC 3986 unreserved stay).
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod unit {
    use super::pct;

    #[test]
    fn uri_components_are_escaped() {
        assert_eq!(pct("opsbox my-host"), "opsbox%20my-host");
        assert_eq!(pct("a:b/c?d&e=f"), "a%3Ab%2Fc%3Fd%26e%3Df");
        assert_eq!(pct("ünï"), "%C3%BCn%C3%AF");
    }
}
