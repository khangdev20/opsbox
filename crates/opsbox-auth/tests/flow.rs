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

#[tokio::test]
async fn losing_the_second_factor_never_reopens_first_run_setup() {
    // Regression: reset-2fa leaves the only admin "not enrolled", which used to make
    // setup look necessary again, letting a setup token replace that admin.
    let env = Env::new().await;
    env.setup_admin().await;
    env.auth.reset_2fa("admin", "cli:root").unwrap();

    assert!(!env.auth.setup_required().unwrap());
    assert!(matches!(
        env.auth.issue_setup_token(),
        Err(AuthError::NotFound)
    ));
    assert!(matches!(env.auth.ensure_setup_token(), Ok(None)));
    assert!(matches!(
        env.auth.claim_setup("AAAA-BBBB-CCCC", IP),
        Err(AuthError::NotFound)
    ));
    assert!(matches!(
        env.auth
            .create_admin("r", "anything", "evil", PASSWORD, "h", IP)
            .await,
        Err(AuthError::NotFound)
    ));
    assert_eq!(
        env.store.list_users().unwrap().len(),
        1,
        "the admin was not replaced"
    );
}

// ---- invitations ---------------------------------------------------------------------

#[tokio::test]
async fn an_invitation_leads_from_a_link_to_a_signed_in_second_person() {
    let env = Env::new().await;
    env.setup_admin().await;
    let admin = env.store.user_by_name("admin").unwrap().unwrap();

    let (token, expires) = env
        .auth
        .invite_user(&admin, " Dana ", Role::Deployer, Some(IP))
        .unwrap();
    assert!(expires > real_now() + 2 * 24 * 3600, "valid for days");
    let pending = env.store.user_by_name("dana").unwrap().unwrap();
    assert_eq!(
        (pending.role, pending.password_hash.as_str()),
        (Role::Deployer, "")
    );
    // Until the link is used the account cannot sign in, whatever is typed.
    for guess in ["", "x", PASSWORD] {
        assert!(
            env.auth
                .login("r", env.req("dana", guess, "000000"))
                .await
                .is_err()
        );
    }

    let info = env.auth.invite_info(&token, IP).unwrap();
    assert_eq!((info.name.as_str(), info.role), ("dana", Role::Deployer));
    // Looking does not use it up; a weak password is refused and the link still works.
    let weak = env
        .auth
        .claim_invite(&token, "short", IP, None)
        .await
        .unwrap_err();
    assert!(
        matches!(
            weak,
            AuthError::Invalid {
                field: "password",
                ..
            }
        ),
        "{weak:?}"
    );
    assert!(env.auth.invite_info(&token, IP).is_ok());

    let issued = env
        .auth
        .claim_invite(&token, "a long enough passphrase", IP, None)
        .await
        .unwrap();
    // The result is an enrolment session, which can only finish two-factor setup.
    let who = env.auth.authenticate(&issued.token).unwrap().unwrap();
    assert!(matches!(who.kind, SessionKindView::Enroll));
    assert_eq!(who.user.name, "dana");
    // Single use.
    assert!(
        env.auth
            .claim_invite(&token, "another long passphrase", IP, None)
            .await
            .is_err()
    );
    assert!(env.auth.invite_info(&token, IP).is_err());
    // Now the password works and, as for the first admin, leads to enrolment.
    let out = env
        .auth
        .login("r", env.req("dana", "a long enough passphrase", ""))
        .await
        .unwrap();
    assert!(matches!(out, LoginOutcome::EnrollRequired(_)), "{out:?}");
}

#[tokio::test]
async fn invitations_expire_and_can_be_replaced() {
    let env = Env::new().await;
    env.setup_admin().await;
    let admin = env.store.user_by_name("admin").unwrap().unwrap();
    let (old, _) = env
        .auth
        .invite_user(&admin, "erin", Role::Viewer, None)
        .unwrap();
    let (fresh, _) = env
        .auth
        .invite_user(&admin, "erin", Role::Viewer, None)
        .unwrap();
    assert!(
        env.auth.invite_info(&old, IP).is_err(),
        "a new link replaces the old one"
    );
    assert!(env.auth.invite_info(&fresh, IP).is_ok());
    // Asking again with another role changes the role of the waiting account.
    let (admin_link, _) = env
        .auth
        .invite_user(&admin, "erin", Role::Admin, None)
        .unwrap();
    assert_eq!(
        env.auth.invite_info(&admin_link, IP).unwrap().role,
        Role::Admin
    );
    assert!(env.auth.invite_info(&fresh, IP).is_err());
    env.advance(4 * 24 * 3600);
    let err = env.auth.invite_info(&admin_link, IP).unwrap_err();
    assert!(err.to_string().contains("expired"), "{err}");
    assert!(
        env.auth
            .claim_invite(&admin_link, "a long enough passphrase", IP, None)
            .await
            .is_err()
    );
    // Housekeeping removes the account that never got past its invitation.
    env.auth.purge_expired().unwrap();
    assert!(env.store.user_by_name("erin").unwrap().is_none());
}

#[tokio::test]
async fn names_in_use_cannot_be_invited_and_guessing_links_is_limited() {
    let env = Env::new().await;
    env.setup_admin().await;
    let admin = env.store.user_by_name("admin").unwrap().unwrap();
    let err = env
        .auth
        .invite_user(&admin, "admin", Role::Viewer, None)
        .unwrap_err();
    assert!(err.to_string().contains("already taken"), "{err}");
    assert!(
        env.auth
            .invite_user(&admin, "A", Role::Viewer, None)
            .is_err(),
        "names need 3+ characters"
    );
    // Wrong links from one address run into the same lockout as wrong passwords.
    let mut locked = false;
    for i in 0..12 {
        match env.auth.invite_info(&format!("guess-{i}"), "198.51.100.9") {
            Err(AuthError::Locked(_)) => {
                locked = true;
                break;
            }
            Err(_) => {}
            Ok(_) => panic!("a guess worked"),
        }
    }
    assert!(locked, "repeated bad links lock that address out");
    // Another address is not affected.
    let (token, _) = env
        .auth
        .invite_user(&admin, "frank", Role::Viewer, None)
        .unwrap();
    assert!(env.auth.invite_info(&token, IP).is_ok());
}

#[tokio::test]
async fn removing_accounts_has_safeguards_and_is_audited() {
    let env = Env::new().await;
    env.setup_admin().await;
    let admin = env.store.user_by_name("admin").unwrap().unwrap();
    assert!(
        env.auth.remove_user(&admin, "admin", None).is_err(),
        "not yourself"
    );
    let (_t, _) = env
        .auth
        .invite_user(&admin, "gina", Role::Viewer, None)
        .unwrap();
    env.auth.remove_user(&admin, "gina", Some(IP)).unwrap();
    assert!(env.store.user_by_name("gina").unwrap().is_none());
    assert!(matches!(
        env.auth.remove_user(&admin, "gina", None),
        Err(AuthError::NotFound)
    ));
    // A second enrolled administrator can remove the first, but nobody can remove the last.
    let other = env
        .store
        .create_user("other", Role::Admin, "$argon2id$x", 0)
        .unwrap();
    env.store
        .set_pending_totp(&other.id, "enc:v1:x", 0)
        .unwrap();
    env.store.enable_totp(&other.id, 1, 0).unwrap();
    let err = env.auth.remove_user(&other, "admin", None);
    assert!(err.is_ok(), "{err:?}");
    let last = env.auth.remove_user(&admin, "other", None).unwrap_err();
    assert!(
        last.to_string().contains("yourself") || last.to_string().contains("only administrator"),
        "{last}"
    );
    let actions: Vec<_> = env
        .store
        .recent_audit(50)
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect();
    for a in ["user.invite", "user.remove"] {
        assert!(actions.iter().any(|x| x == a), "{a} in {actions:?}");
    }
    let entries = env.store.recent_audit(500).unwrap().len();
    assert_eq!(
        env.store.verify_audit().unwrap().entries as usize,
        entries,
        "the chain verifies end to end"
    );
}

// ---- API tokens ----------------------------------------------------------------------

#[tokio::test]
async fn tokens_are_shown_once_stored_as_a_hash_and_act_with_their_scope() {
    use opsbox_store::TokenScope;
    let env = Env::new().await;
    env.setup_admin().await;
    let admin = env.store.user_by_name("admin").unwrap().unwrap();

    let (token, secret) = env
        .auth
        .create_token(&admin, " CI-Deploy ", TokenScope::Deploy, Some(IP))
        .unwrap();
    assert_eq!(token.name, "ci-deploy");
    assert!(secret.starts_with("ops_") && secret.len() > 40, "{secret}");
    // Only the hash is kept: the secret is nowhere in what the database holds.
    let stored = env
        .store
        .api_token_by_hash(&opsbox_auth::tokens::hash_token(&secret))
        .unwrap()
        .unwrap();
    assert_eq!(stored.id, token.id);
    assert!(env.store.api_token_by_hash(&secret).unwrap().is_none());
    let listed = env.auth.list_tokens().unwrap();
    assert!(listed.iter().all(|t| !format!("{t:?}").contains(&secret)));

    let who = env.auth.authenticate_token(&secret).unwrap().unwrap();
    assert!(who.via_token);
    assert_eq!(
        (who.user.role, who.user.name.as_str()),
        (Role::Deployer, "token:ci-deploy")
    );
    assert!(env.auth.list_tokens().unwrap()[0].last_used_at.is_some());

    let (_t, read_secret) = env
        .auth
        .create_token(&admin, "metrics", TokenScope::Read, None)
        .unwrap();
    assert_eq!(
        env.auth
            .authenticate_token(&read_secret)
            .unwrap()
            .unwrap()
            .user
            .role,
        Role::Viewer
    );
    let (_t, full) = env
        .auth
        .create_token(&admin, "everything", TokenScope::Full, None)
        .unwrap();
    assert_eq!(
        env.auth
            .authenticate_token(&full)
            .unwrap()
            .unwrap()
            .user
            .role,
        Role::Admin
    );

    // Not a token, or not a live one.
    for bad in [
        "",
        "ops_",
        "ops_nope",
        "Bearer x",
        &secret[..secret.len() - 1],
        &"ops_".repeat(40),
    ] {
        assert!(
            env.auth.authenticate_token(bad).unwrap().is_none(),
            "{bad:?}"
        );
    }
    // Names are checked and unique, ignoring case.
    for bad in ["", "ab", "Has Space", "-dash", "9lives", &"x".repeat(33)] {
        assert!(
            env.auth
                .create_token(&admin, bad, TokenScope::Read, None)
                .is_err(),
            "{bad:?}"
        );
    }
    let dup = env
        .auth
        .create_token(&admin, "METRICS", TokenScope::Read, None)
        .unwrap_err();
    assert!(dup.to_string().contains("already exists"), "{dup}");

    // Revoking works at once and is audited; revoking again says so.
    env.auth.revoke_token(&admin, &token.id, Some(IP)).unwrap();
    assert!(env.auth.authenticate_token(&secret).unwrap().is_none());
    assert!(matches!(
        env.auth.revoke_token(&admin, &token.id, None),
        Err(AuthError::NotFound)
    ));
    let actions: Vec<_> = env
        .store
        .recent_audit(50)
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect();
    for a in ["token.create", "token.revoke"] {
        assert!(actions.iter().any(|x| x == a), "{a}");
    }
    // A token is not a session cookie: the same string does not authenticate as one.
    assert!(env.auth.authenticate(&read_secret).unwrap().is_none());

    // Service-scoped deploy token enforces allows_service + stream_active.
    let (_svc_tok, svc_secret) = env
        .auth
        .create_token(
            &admin,
            "ci-web",
            TokenScope::DeployService("web".into()),
            Some(IP),
        )
        .unwrap();
    let svc_who = env.auth.authenticate_token(&svc_secret).unwrap().unwrap();
    assert_eq!(svc_who.user.role, Role::Deployer);
    assert!(svc_who.allows_service("web"));
    assert!(!svc_who.allows_service("api"));
    assert!(env.auth.stream_active(&svc_who).unwrap());
}
