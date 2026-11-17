//! Sign-in, setup, lockout and session behaviour, end to end against a real `priv`
//! server and an in-memory database. Time for the auth logic comes from a clock the
//! tests move; TOTP codes use the real clock because `priv` does.

use std::os::unix::fs::MetadataExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use opsbox_auth::password::HashParams;
use opsbox_auth::{Auth, AuthConfig, AuthError, LoginOutcome, LoginRequest, SessionKindView};
use opsbox_priv::{PrivClient, PrivConfig, PrivServer};
use opsbox_proto::Role;
use opsbox_store::Store;
use tokio::sync::oneshot;

const PASSWORD: &str = "correct horse battery staple";
const IP: &str = "203.0.113.5";

struct Env {
    auth: Auth,
    store: Arc<Store>,
    offset: Arc<AtomicI64>,
    _dir: tempfile::TempDir,
    stop: Option<oneshot::Sender<()>>,
}

impl Env {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let probe = dir.path().join("probe");
        std::fs::write(&probe, b"").unwrap();
        let uid = std::fs::metadata(&probe).unwrap().uid();
        let socket = dir.path().join("p.sock");
        let mut cfg = PrivConfig::new(&socket, vec![uid]);
        cfg.master_key_path = dir.path().join("master.key");
        let server = PrivServer::bind(cfg).unwrap();
        let (stop, rx) = oneshot::channel::<()>();
        tokio::spawn(server.run(async {
            let _ = rx.await;
        }));

        let store = Arc::new(Store::open_in_memory().unwrap());
        let offset = Arc::new(AtomicI64::new(0));
        let off = offset.clone();
        let clock: Arc<dyn Fn() -> i64 + Send + Sync> =
            Arc::new(move || real_now() + off.load(Ordering::SeqCst));
        let auth = Auth::with_clock(
            store.clone(),
            PrivClient::new(&socket),
            AuthConfig {
                hash_params: HashParams {
                    memory_kib: 8,
                    iterations: 1,
                    parallelism: 1,
                },
                ..AuthConfig::default()
            },
            clock,
        );
        Self {
            auth,
            store,
            offset,
            _dir: dir,
            stop: Some(stop),
        }
    }

    fn advance(&self, secs: i64) {
        self.offset.fetch_add(secs, Ordering::SeqCst);
    }

    /// Run the whole first-run flow and return the admin's TOTP secret (base32).
    async fn setup_admin(&self) -> String {
        let token = self.auth.issue_setup_token().unwrap();
        let session = self.auth.claim_setup(&token, IP).unwrap();
        let enrol = self
            .auth
            .create_admin("r", &session, "admin", PASSWORD, "testhost", IP)
            .await
            .unwrap();
        let code = code_now(&enrol.secret_base32);
        self.auth
            .finish_setup("r", &session, &code, IP, None)
            .await
            .unwrap();
        enrol.secret_base32
    }

    fn req(&self, user: &str, password: &str, code: &str) -> LoginRequest {
        LoginRequest {
            username: user.into(),
            password: password.into(),
            code: code.into(),
            ip: Some(IP.into()),
            user_agent: Some("tests".into()),
        }
    }

    /// A signed-in session made directly in the store. A real sign-in needs a code
    /// for a fresh 30 s step, and the real clock cannot be moved, so tests that only
    /// need *a session* create one this way.
    fn session_for(&self, name: &str) -> String {
        let user = self.store.user_by_name(name).unwrap().unwrap();
        let token = opsbox_auth::tokens::new_token().unwrap();
        let now = real_now() + self.offset.load(Ordering::SeqCst);
        self.store
            .create_session(
                &user.id,
                &opsbox_auth::tokens::hash_token(&token),
                opsbox_store::SessionKind::Web,
                None,
                None,
                now,
            )
            .unwrap();
        token
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        if let Some(s) = self.stop.take() {
            let _ = s.send(());
        }
    }
}

fn real_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn decode_base32(s: &str) -> Vec<u8> {
    let (mut buf, mut bits, mut out) = (0u32, 0u32, Vec::new());
    for c in s.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'2'..=b'7' => c - b'2' + 26,
            _ => panic!("not base32"),
        };
        buf = (buf << 5) | u32::from(v);
        bits += 5;
        if bits >= 8 {
            out.push((buf >> (bits - 8)) as u8);
            bits -= 8;
            buf &= (1 << bits) - 1;
        }
    }
    out
}

fn code_for(secret_b32: &str, step: u64) -> String {
    opsbox_priv::code_at(&decode_base32(secret_b32), step)
}
fn code_now(secret_b32: &str) -> String {
    code_for(secret_b32, real_now() as u64 / 30)
}

// ------------------------------------------------------------------ setup

#[tokio::test]
async fn first_run_setup_creates_an_enrolled_admin_and_then_closes() {
    let env = Env::new().await;
    assert!(env.auth.setup_required().unwrap());

    let token = env.auth.issue_setup_token().unwrap();
    assert_eq!(token.len(), 14);

    // A wrong token is refused with a helpful sentence, and does not use up the real one.
    let err = env.auth.claim_setup("AAAA-BBBB-CCCC", IP).unwrap_err();
    assert!(
        matches!(err, AuthError::Invalid { field: "token", .. }),
        "{err:?}"
    );
    let session = env
        .auth
        .claim_setup(&token.to_lowercase(), IP)
        .expect("tokens are typed loosely");
    // Single use: the same token cannot be claimed again.
    assert!(env.auth.claim_setup(&token, IP).is_err());

    // Password rules apply to the first account too.
    let weak = env
        .auth
        .create_admin("r", &session, "admin", "short", "h", IP)
        .await
        .unwrap_err();
    assert!(matches!(
        weak,
        AuthError::Invalid {
            field: "password",
            ..
        }
    ));
    let bad_name = env
        .auth
        .create_admin("r", &session, "Not Valid!", PASSWORD, "h", IP)
        .await
        .unwrap_err();
    assert!(matches!(
        bad_name,
        AuthError::Invalid {
            field: "username",
            ..
        }
    ));
    // A made-up setup session is useless.
    assert!(
        env.auth
            .create_admin("r", "forged", "admin", PASSWORD, "h", IP)
            .await
            .is_err()
    );

    let enrol = env
        .auth
        .create_admin("r", &session, "admin", PASSWORD, "my host", IP)
        .await
        .unwrap();
    assert!(
        enrol
            .otpauth_uri
            .starts_with("otpauth://totp/opsbox%20my%20host:admin?secret="),
        "{}",
        enrol.otpauth_uri
    );
    assert!(
        enrol
            .otpauth_uri
            .contains("&issuer=opsbox%20my%20host&algorithm=SHA1&digits=6&period=30")
    );
    assert!(
        env.auth.setup_required().unwrap(),
        "not finished until the first code is confirmed"
    );

    // A wrong first code does not finish setup.
    let bad = env
        .auth
        .finish_setup("r", &session, "000000", IP, None)
        .await
        .unwrap_err();
    assert!(matches!(bad, AuthError::Invalid { field: "code", .. }));

    let issued = env
        .auth
        .finish_setup(
            "r",
            &session,
            &code_now(&enrol.secret_base32),
            IP,
            Some("ua"),
        )
        .await
        .unwrap();
    assert_eq!(issued.user.role, Role::Admin);
    assert!(
        env.auth.authenticate(&issued.token).unwrap().is_some(),
        "setup signs the admin in"
    );

    // The setup endpoints are gone for good.
    assert!(!env.auth.setup_required().unwrap());
    assert!(matches!(
        env.auth.issue_setup_token(),
        Err(AuthError::NotFound)
    ));
    assert!(matches!(
        env.auth.claim_setup(&token, IP),
        Err(AuthError::NotFound)
    ));
    assert!(matches!(
        env.auth
            .create_admin("r", &session, "evil", PASSWORD, "h", IP)
            .await,
        Err(AuthError::NotFound)
    ));
    assert_eq!(env.store.list_users().unwrap().len(), 1);
}

#[tokio::test]
async fn setup_tokens_expire_and_a_new_one_replaces_the_old() {
    let env = Env::new().await;
    let old = env.auth.issue_setup_token().unwrap();
    let new = env.auth.issue_setup_token().unwrap();
    assert!(
        env.auth.claim_setup(&old, IP).is_err(),
        "issuing a new token cancels the old one"
    );
    env.advance(3601);
    assert!(
        env.auth.claim_setup(&new, IP).is_err(),
        "valid for one hour only"
    );
}

#[tokio::test]
async fn ensure_setup_token_only_creates_one_when_needed() {
    let env = Env::new().await;
    assert!(env.auth.ensure_setup_token().unwrap().is_some());
    assert!(
        env.auth.ensure_setup_token().unwrap().is_none(),
        "a live token already exists"
    );
    env.advance(3601);
    assert!(
        env.auth.ensure_setup_token().unwrap().is_some(),
        "an expired one is replaced"
    );
    env.setup_admin().await;
    assert!(
        env.auth.ensure_setup_token().unwrap().is_none(),
        "never once an admin exists"
    );
}

#[tokio::test]
async fn abandoned_setup_can_be_restarted() {
    let env = Env::new().await;
    let t1 = env.auth.issue_setup_token().unwrap();
    let s1 = env.auth.claim_setup(&t1, IP).unwrap();
    env.auth
        .create_admin("r", &s1, "first", PASSWORD, "h", IP)
        .await
        .unwrap();
    // The person walks away, then starts over with a new token and another name.
    let t2 = env.auth.issue_setup_token().unwrap();
    let s2 = env.auth.claim_setup(&t2, IP).unwrap();
    let e = env
        .auth
        .create_admin("r", &s2, "second", PASSWORD, "h", IP)
        .await
        .unwrap();
    env.auth
        .finish_setup("r", &s2, &code_now(&e.secret_base32), IP, None)
        .await
        .unwrap();
    let names: Vec<_> = env
        .store
        .list_users()
        .unwrap()
        .into_iter()
        .map(|u| u.name)
        .collect();
    assert_eq!(names, ["second"]);
}

#[tokio::test]
async fn guessing_setup_tokens_is_rate_limited() {
    let env = Env::new().await;
    env.auth.issue_setup_token().unwrap();
    for _ in 0..5 {
        assert!(matches!(
            env.auth.claim_setup("AAAA-AAAA-AAAA", IP),
            Err(AuthError::Invalid { .. })
        ));
    }
    assert!(matches!(
        env.auth.claim_setup("AAAA-AAAA-AAAA", IP),
        Err(AuthError::Locked(_))
    ));
}

// ------------------------------------------------------------------ login

#[tokio::test]
async fn login_needs_password_and_a_fresh_code() {
    let env = Env::new().await;
    let secret = env.setup_admin().await;
    env.advance(0);

    // Wrong password, wrong code and unknown user all give the same answer.
    for (u, p, c) in [
        ("admin", "wrong password here", code_now(&secret)),
        ("admin", PASSWORD, "000000".into()),
        ("nobody", PASSWORD, code_now(&secret)),
    ] {
        let e = env.auth.login("r", env.req(u, p, &c)).await.unwrap_err();
        assert!(matches!(e, AuthError::InvalidCredentials), "{u}/{p}: {e:?}");
        assert_eq!(e.to_string(), "Wrong username, password or code.");
    }

    // The code from setup was already used for this 30 s step, so a login in the same
    // step is refused. The next step's code is accepted (+-1 step tolerance).
    let step = real_now() as u64 / 30;
    let used = code_for(&secret, step);
    assert!(matches!(
        env.auth.login("r", env.req("admin", PASSWORD, &used)).await,
        Err(AuthError::InvalidCredentials)
    ));
    let next = code_for(&secret, step + 1);
    let issued = match env
        .auth
        .login("r", env.req("ADMIN", PASSWORD, &next))
        .await
        .unwrap()
    {
        LoginOutcome::Session(i) => i,
        LoginOutcome::EnrollRequired(_) => panic!("2FA is enrolled"),
    };
    let who = env.auth.authenticate(&issued.token).unwrap().unwrap();
    assert_eq!(
        (who.user.name.as_str(), who.kind),
        ("admin", SessionKindView::Web)
    );
    assert_eq!(issued.csrf, opsbox_auth::tokens::csrf_token(&issued.token));

    // Replaying that exact code fails, even though it is still inside the window.
    assert!(matches!(
        env.auth.login("r", env.req("admin", PASSWORD, &next)).await,
        Err(AuthError::InvalidCredentials)
    ));
    // So does the previous step's code, which is now older than the last one used.
    assert!(matches!(
        env.auth.login("r", env.req("admin", PASSWORD, &used)).await,
        Err(AuthError::InvalidCredentials)
    ));
}

#[tokio::test]
async fn one_code_cannot_win_two_simultaneous_logins() {
    let env = Env::new().await;
    let secret = env.setup_admin().await;
    let code = code_for(&secret, real_now() as u64 / 30 + 1);
    let (a, b) = tokio::join!(
        env.auth.login("r1", env.req("admin", PASSWORD, &code)),
        env.auth.login("r2", env.req("admin", PASSWORD, &code)),
    );
    let wins = [a.is_ok(), b.is_ok()].into_iter().filter(|w| *w).count();
    assert_eq!(wins, 1, "exactly one sign-in may use a given code");
}

#[tokio::test]
async fn five_failures_lock_the_account_with_a_growing_backoff() {
    let env = Env::new().await;
    let secret = env.setup_admin().await;

    for i in 0..5 {
        let e = env
            .auth
            .login("r", env.req("admin", "wrong password here", "000000"))
            .await
            .unwrap_err();
        assert!(
            matches!(e, AuthError::InvalidCredentials),
            "attempt {i}: {e:?}"
        );
    }
    // Now even the right password and code are refused, with a wait time.
    let code = code_for(&secret, real_now() as u64 / 30 + 1);
    match env.auth.login("r", env.req("admin", PASSWORD, &code)).await {
        Err(AuthError::Locked(s)) => assert!((1..=30).contains(&s), "{s}"),
        other => panic!("expected a lockout, got {other:?}"),
    }
    // Attempts made while locked do not extend the lock.
    env.advance(31);
    let issued = env.auth.login("r", env.req("admin", PASSWORD, &code)).await;
    assert!(
        matches!(issued, Ok(LoginOutcome::Session(_))),
        "lock expired: {issued:?}"
    );
}

#[tokio::test]
async fn a_sixth_failure_doubles_the_wait() {
    let env = Env::new().await;
    env.setup_admin().await;
    for _ in 0..5 {
        let _ = env
            .auth
            .login("r", env.req("admin", "nope nope nope nope", "000000"))
            .await;
    }
    env.advance(31); // the 30 s lock is over...
    let _ = env
        .auth
        .login("r", env.req("admin", "nope nope nope nope", "000000"))
        .await; // ...failure number six
    match env
        .auth
        .login("r", env.req("admin", PASSWORD, "000000"))
        .await
    {
        Err(AuthError::Locked(s)) => {
            assert!((31..=60).contains(&s), "second lock lasts 60 s, got {s}")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn failures_age_out_of_the_window() {
    let env = Env::new().await;
    let secret = env.setup_admin().await;
    for _ in 0..4 {
        let _ = env
            .auth
            .login("r", env.req("admin", "nope nope nope nope", "000000"))
            .await;
    }
    env.advance(16 * 60);
    let _ = env
        .auth
        .login("r", env.req("admin", "nope nope nope nope", "000000"))
        .await; // only 1 in the window now
    let code = code_for(&secret, real_now() as u64 / 30 + 1);
    assert!(matches!(
        env.auth.login("r", env.req("admin", PASSWORD, &code)).await,
        Ok(LoginOutcome::Session(_))
    ));
}

#[tokio::test]
async fn an_address_is_locked_across_user_names() {
    let env = Env::new().await;
    let secret = env.setup_admin().await;
    for i in 0..5 {
        let _ = env
            .auth
            .login(
                "r",
                env.req(&format!("guess{i}"), "whatever password", "000000"),
            )
            .await;
    }
    // A different name from the same address is locked too, even the real admin.
    let code = code_for(&secret, real_now() as u64 / 30 + 1);
    assert!(matches!(
        env.auth.login("r", env.req("admin", PASSWORD, &code)).await,
        Err(AuthError::Locked(_))
    ));
    // Another address is unaffected.
    let mut req = env.req("admin", PASSWORD, &code);
    req.ip = Some("198.51.100.9".into());
    assert!(matches!(
        env.auth.login("r", req).await,
        Ok(LoginOutcome::Session(_))
    ));
}

#[tokio::test]
async fn unlock_command_clears_a_lockout() {
    let env = Env::new().await;
    let secret = env.setup_admin().await;
    for _ in 0..5 {
        let _ = env
            .auth
            .login("r", env.req("admin", "nope nope nope nope", "000000"))
            .await;
    }
    // The address stays locked too, so test from another one after unlocking the user.
    env.auth.unlock("admin", "cli:root").unwrap();
    let code = code_for(&secret, real_now() as u64 / 30 + 1);
    let mut req = env.req("admin", PASSWORD, &code);
    req.ip = Some("198.51.100.77".into());
    assert!(matches!(
        env.auth.login("r", req).await,
        Ok(LoginOutcome::Session(_))
    ));
    assert!(env.auth.unlock("ghost", "cli:root").is_err());
}

// --------------------------------------------------------------- sessions

#[tokio::test]
async fn sessions_expire_when_idle_and_after_seven_days() {
    let env = Env::new().await;
    env.setup_admin().await;
    let a = env.session_for("admin");
    assert!(env.auth.authenticate(&a).unwrap().is_some());

    env.advance(12 * 3600 + 1);
    assert!(
        env.auth.authenticate(&a).unwrap().is_none(),
        "idle for more than 12 h"
    );
    assert!(
        env.auth.authenticate(&a).unwrap().is_none(),
        "and it stays gone"
    );

    // Activity keeps a session alive, but not past seven days in total.
    let b = env.session_for("admin");
    // Used every 11 hours (inside the 12 h idle limit) for 165 hours...
    for n in 1..=15 {
        env.advance(11 * 3600);
        assert!(
            env.auth.authenticate(&b).unwrap().is_some(),
            "still active after {} h",
            n * 11
        );
    }
    // ...but 176 hours after sign-in the seven-day limit (168 h) ends it regardless.
    env.advance(11 * 3600);
    assert!(
        env.auth.authenticate(&b).unwrap().is_none(),
        "absolute limit of seven days"
    );
}

#[tokio::test]
async fn logout_and_sign_out_everywhere() {
    let env = Env::new().await;
    env.setup_admin().await;
    let (a, b) = (env.session_for("admin"), env.session_for("admin"));
    let who = env.auth.authenticate(&a).unwrap().unwrap();
    env.auth.logout(&who, Some(IP)).unwrap();
    assert!(env.auth.authenticate(&a).unwrap().is_none());
    assert!(
        env.auth.authenticate(&b).unwrap().is_some(),
        "other sessions are untouched"
    );

    let who = env.auth.authenticate(&b).unwrap().unwrap();
    env.auth.revoke_all(&who.user, Some(IP)).unwrap();
    assert!(env.auth.authenticate(&b).unwrap().is_none());
    assert!(env.auth.authenticate("not-a-token").unwrap().is_none());
}

#[tokio::test]
async fn changing_the_password_ends_every_session() {
    let env = Env::new().await;
    let secret = env.setup_admin().await;
    let a = env.session_for("admin");
    env.auth
        .set_password("admin", "an entirely new passphrase", "cli:root")
        .await
        .unwrap();
    assert!(env.auth.authenticate(&a).unwrap().is_none());
    let weak = env
        .auth
        .set_password("admin", "short", "cli:root")
        .await
        .unwrap_err();
    assert!(matches!(
        weak,
        AuthError::Invalid {
            field: "password",
            ..
        }
    ));
    // The old password no longer works; the new one does.
    let code = code_for(&secret, real_now() as u64 / 30 + 1);
    assert!(
        env.auth
            .login("r", env.req("admin", PASSWORD, &code))
            .await
            .is_err()
    );
    let mut req = env.req("admin", "an entirely new passphrase", &code);
    req.ip = Some("198.51.100.50".into());
    assert!(matches!(
        env.auth.login("r", req).await,
        Ok(LoginOutcome::Session(_))
    ));
}

// ------------------------------------------------------------ 2FA recovery

#[tokio::test]
async fn reset_2fa_leads_to_enrolment_on_the_next_sign_in() {
    let env = Env::new().await;
    let secret = env.setup_admin().await;
    let old = env.session_for("admin");
    env.auth.reset_2fa("admin", "cli:root").unwrap();
    assert!(
        env.auth.authenticate(&old).unwrap().is_none(),
        "reset ends all sessions"
    );

    // Password alone now gives only an enrolment session.
    let issued = match env
        .auth
        .login("r", env.req("admin", PASSWORD, ""))
        .await
        .unwrap()
    {
        LoginOutcome::EnrollRequired(i) => i,
        LoginOutcome::Session(_) => panic!("must not be a full session without 2FA"),
    };
    let who = env.auth.authenticate(&issued.token).unwrap().unwrap();
    assert_eq!(who.kind, SessionKindView::Enroll);
    // The wrong password still gets nothing.
    assert!(
        env.auth
            .login("r", env.req("admin", "wrong password here", ""))
            .await
            .is_err()
    );

    // Enrol with a new secret, then the session is upgraded to a full one.
    let enrol = env.auth.begin_totp("r", &who.user, "h").await.unwrap();
    assert_ne!(enrol.secret_base32, secret);
    assert!(
        env.auth
            .confirm_totp("r", &who.user, "000000", IP)
            .await
            .is_err()
    );
    env.auth
        .confirm_totp("r", &who.user, &code_now(&enrol.secret_base32), IP)
        .await
        .unwrap();
    let full = env
        .auth
        .upgrade_enroll_session(&who, Some(IP), None)
        .unwrap();
    assert!(
        env.auth.authenticate(&issued.token).unwrap().is_none(),
        "the enrolment session is spent"
    );
    assert_eq!(
        env.auth.authenticate(&full.token).unwrap().unwrap().kind,
        SessionKindView::Web
    );
    // And the old authenticator no longer works.
    let old_code = code_for(&secret, real_now() as u64 / 30 + 1);
    let mut req = env.req("admin", PASSWORD, &old_code);
    req.ip = Some("198.51.100.60".into());
    assert!(env.auth.login("r", req).await.is_err());
}

#[tokio::test]
async fn enrolment_sessions_are_short() {
    let env = Env::new().await;
    env.setup_admin().await;
    env.auth.reset_2fa("admin", "cli:root").unwrap();
    let LoginOutcome::EnrollRequired(i) = env
        .auth
        .login("r", env.req("admin", PASSWORD, ""))
        .await
        .unwrap()
    else {
        panic!()
    };
    env.advance(16 * 60);
    assert!(env.auth.authenticate(&i.token).unwrap().is_none());
}

// ------------------------------------------------------------------ audit

#[tokio::test]
async fn every_sign_in_and_setup_step_is_in_a_valid_audit_chain() {
    let env = Env::new().await;
    let secret = env.setup_admin().await;
    let _ = env
        .auth
        .login("r", env.req("admin", "nope nope nope nope", "000000"))
        .await;
    let code = code_for(&secret, real_now() as u64 / 30 + 1);
    let LoginOutcome::Session(i) = env
        .auth
        .login("r", env.req("admin", PASSWORD, &code))
        .await
        .unwrap()
    else {
        panic!()
    };
    let who = env.auth.authenticate(&i.token).unwrap().unwrap();
    env.auth.logout(&who, Some(IP)).unwrap();

    let entries = env.store.recent_audit(100).unwrap();
    let actions: Vec<&str> = entries.iter().rev().map(|e| e.action.as_str()).collect();
    for expected in [
        "setup.token.issue",
        "setup.claim",
        "setup.admin",
        "auth.2fa.enable",
        "setup.complete",
        "auth.login",
        "auth.logout",
    ] {
        assert!(
            actions.contains(&expected),
            "{expected} missing from {actions:?}"
        );
    }
    let denied = entries
        .iter()
        .find(|e| e.action == "auth.login" && e.result == "denied")
        .expect("failed login is logged");
    assert_eq!(denied.actor, "admin");
    assert!(denied.detail.contains("bad password"), "{}", denied.detail);
    // Nothing secret ever lands in the log.
    let all = entries.iter().map(|e| format!("{e:?}")).collect::<String>();
    assert!(!all.contains(PASSWORD) && !all.contains(&secret) && !all.contains(&i.token));
    assert_eq!(
        env.store.verify_audit().unwrap().entries as usize,
        entries.len()
    );
}

#[tokio::test]
async fn unknown_names_are_logged_without_echoing_junk() {
    let env = Env::new().await;
    env.setup_admin().await;
    // Someone types their password into the username box.
    let _ = env
        .auth
        .login(
            "r",
            env.req("My Secret Pass 123!", "x y z x y z x y z", "000000"),
        )
        .await;
    let _ = env
        .auth
        .login("r", env.req("ghost", "x y z x y z x y z", "000000"))
        .await;
    let entries = env.store.recent_audit(10).unwrap();
    let all = entries.iter().map(|e| format!("{e:?}")).collect::<String>();
    assert!(
        !all.contains("Secret"),
        "the typed text must not be logged: {all}"
    );
    assert!(
        all.contains("ghost"),
        "well-formed unknown names are logged"
    );
}

#[tokio::test]
async fn the_helper_being_down_is_reported_not_treated_as_a_bad_code() {
    let env = Env::new().await;
    let secret = env.setup_admin().await;
    // Point a second Auth at a socket nobody listens on.
    let dead = Auth::new(
        env.store.clone(),
        PrivClient::new("/nonexistent/priv.sock"),
        AuthConfig {
            hash_params: HashParams {
                memory_kib: 8,
                iterations: 1,
                parallelism: 1,
            },
            ..AuthConfig::default()
        },
    );
    let code = code_for(&secret, real_now() as u64 / 30 + 1);
    let e = dead
        .login("r", env.req("admin", PASSWORD, &code))
        .await
        .unwrap_err();
    assert!(matches!(e, AuthError::HelperUnavailable), "{e:?}");
    // It must not count as a failed guess either.
    assert_eq!(env.store.failures_for_user("admin", 0).unwrap().0, 0);
}
