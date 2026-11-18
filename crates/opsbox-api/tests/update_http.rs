//! Settings > opsbox updates: why nothing is offered, "last checked", the one-hour cache and
//! "Check now". GitHub is scripted; nothing here touches the network or the machine.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::http::{Method, StatusCode};
use common::{Browser, Harness, Opts};
use opsbox_api::router;
use opsbox_proto::Role;
use opsbox_update::{HttpReply, ReleaseChecker, ReleaseFetcher};
use serde_json::{Value, json};

/// The harness runs version `9.9.9-test`, a pre-release, so the default channel is beta.
const RELEASES: &str = r#"[
    {"tag_name": "v9.9.9-test", "draft": false, "prerelease": true, "html_url": "https://example.com/r"},
    {"tag_name": "v9.9.0-alpha.1", "draft": false, "prerelease": true, "html_url": "https://example.com/o"}
]"#;
const NEWER: &str = r#"[
    {"tag_name": "v9.9.10-test", "draft": false, "prerelease": true, "html_url": "https://example.com/n"}
]"#;

/// GitHub as a function of how many requests it has seen.
struct Github {
    calls: AtomicUsize,
    answer: Box<dyn Fn(usize) -> Result<HttpReply, String> + Send + Sync>,
}

impl ReleaseFetcher for Github {
    fn get(&self, _url: &str) -> Result<HttpReply, String> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        (self.answer)(n)
    }
}

fn ok(body: &str) -> Result<HttpReply, String> {
    Ok(HttpReply {
        status: 200,
        body: body.to_owned(),
        ..HttpReply::default()
    })
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// An admin's browser on an app whose updater talks to `answer`.
fn admin(
    h: &Harness,
    answer: impl Fn(usize) -> Result<HttpReply, String> + Send + Sync + 'static,
) -> (Browser, Arc<Github>) {
    let gh = Arc::new(Github {
        calls: AtomicUsize::new(0),
        answer: Box::new(answer),
    });
    let checker = Arc::new(ReleaseChecker::new(gh.clone(), Arc::new(now)));
    let mut state = h.state.clone();
    state.updater = state.updater.clone().with_checker(checker);
    let mut b = h.signed_in("root", Role::Admin);
    b.app = router(state);
    (b, gh)
}

async fn check(b: &mut Browser) -> Value {
    let r = b
        .send(Method::POST, "/api/v1/updates/check", None, Opts::default())
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    r.json()
}

async fn set_channel(b: &mut Browser, channel: &str) -> Value {
    let r = b
        .send(
            Method::PUT,
            "/api/v1/updates/auto",
            Some(json!({ "channel": channel })),
            Opts::default(),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    r.json()
}

#[tokio::test]
async fn no_release_on_the_channel_is_named() {
    let h = Harness::new(false);
    let (mut b, _) = admin(&h, |_| ok(RELEASES));
    let snap = set_channel(&mut b, "stable").await;
    assert_eq!(snap["reason"], "no_releases_on_channel");
    assert_eq!(snap["channel"], "stable");
    assert_eq!(snap["checkedChannel"], "stable");
    assert_eq!(snap["updateAvailable"], false);
    assert!(snap["latest"].is_null());
    assert!(snap["checkedAt"].as_i64().unwrap() > 0);

    // Back on beta the same cached list answers: the running version is the newest.
    let snap = set_channel(&mut b, "beta").await;
    assert_eq!(snap["reason"], "up_to_date");
    assert_eq!(snap["latest"]["version"], "9.9.9-test");
}

#[tokio::test]
async fn newest_equal_or_older_is_up_to_date_and_newer_has_no_reason() {
    let h = Harness::new(false);
    let (mut b, _) = admin(&h, |_| ok(RELEASES));
    let snap = check(&mut b).await;
    assert_eq!(snap["reason"], "up_to_date");
    assert_eq!(snap["updateAvailable"], false);
    assert!(snap["checkedAt"].as_i64().unwrap() > 0);

    let h = Harness::new(false);
    let (mut b, _) = admin(&h, |_| ok(NEWER));
    let snap = check(&mut b).await;
    assert!(snap["reason"].is_null());
    assert_eq!(snap["updateAvailable"], true);
    assert_eq!(snap["latest"]["version"], "9.9.10-test");
}

#[tokio::test]
async fn a_network_error_is_named_with_what_went_wrong() {
    let h = Harness::new(false);
    let (mut b, _) = admin(&h, |_| Err("connection refused".to_owned()));
    let snap = check(&mut b).await;
    assert_eq!(snap["reason"], "network_error");
    assert!(snap["reasonDetail"].as_str().unwrap().contains("refused"));
    assert_eq!(snap["updateAvailable"], false);
    assert!(snap["checkedAt"].as_i64().unwrap() > 0);

    // Applying says why too, instead of "up to date".
    let r = b
        .send(Method::POST, "/api/v1/updates/apply", None, Opts::default())
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.text().contains("could not reach GitHub"), "{}", r.text());
}

#[tokio::test]
async fn rate_limiting_is_named_with_the_time_to_retry() {
    let h = Harness::new(false);
    let reset = now() + 1800;
    let (mut b, gh) = admin(&h, move |_| {
        Ok(HttpReply {
            status: 403,
            body: r#"{"message":"API rate limit exceeded"}"#.to_owned(),
            rate_limit_remaining: Some(0),
            rate_limit_reset: Some(reset),
            retry_after: None,
        })
    });
    let snap = check(&mut b).await;
    assert_eq!(snap["reason"], "rate_limited");
    assert_eq!(snap["retryAt"], reset);

    // Still backing off: "Check now" does not hit GitHub again.
    let snap = check(&mut b).await;
    assert_eq!(snap["reason"], "rate_limited");
    assert_eq!(gh.calls.load(Ordering::SeqCst), 1);

    let r = b
        .send(Method::POST, "/api/v1/updates/apply", None, Opts::default())
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(r.text().contains("limiting"), "{}", r.text());
}

#[tokio::test]
async fn the_release_list_is_cached_and_check_now_bypasses_it() {
    let h = Harness::new(false);
    // The first answer is old; every later one has a newer release.
    let (mut b, gh) = admin(&h, |n| ok(if n == 0 { RELEASES } else { NEWER }));

    // Switching channel does an ordinary check: one request, then the cache.
    let snap = set_channel(&mut b, "beta").await;
    assert_eq!(snap["reason"], "up_to_date");
    set_channel(&mut b, "stable").await;
    set_channel(&mut b, "beta").await;
    assert_eq!(gh.calls.load(Ordering::SeqCst), 1);

    // "Check now" asks GitHub again and sees the new release.
    let snap = check(&mut b).await;
    assert_eq!(gh.calls.load(Ordering::SeqCst), 2);
    assert_eq!(snap["updateAvailable"], true);
    assert_eq!(snap["latest"]["version"], "9.9.10-test");
}

#[tokio::test]
async fn a_failed_check_keeps_an_offer_already_found() {
    let h = Harness::new(false);
    let (mut b, _) = admin(&h, |n| {
        if n == 0 {
            ok(NEWER)
        } else {
            Err("timed out".to_owned())
        }
    });
    assert_eq!(check(&mut b).await["updateAvailable"], true);
    // GitHub becomes unreachable: the earlier offer stands, not "up to date".
    let snap = check(&mut b).await;
    assert_eq!(snap["updateAvailable"], true);
    assert_eq!(snap["latest"]["version"], "9.9.10-test");
}

#[tokio::test]
async fn update_status_needs_an_admin() {
    let h = Harness::new(false);
    let mut viewer = h.signed_in("view", Role::Viewer);
    let r = viewer
        .send(Method::POST, "/api/v1/updates/check", None, Opts::default())
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
}
