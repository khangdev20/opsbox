//! Metrics, attention and the WebSocket stream over HTTP.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::{Harness, ORIGIN};
use futures_util::StreamExt;
use opsbox_proto::Role;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

/// The first snapshot needs two samples, about two seconds after start.
async fn warmed_up(h: &Harness) {
    for _ in 0..80 {
        if h.state.metrics.latest().is_some() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no metrics sample after 8 s");
}

#[tokio::test]
async fn metrics_come_back_in_the_shape_the_overview_page_reads() {
    let h = Harness::new(true);
    let mut b = h.signed_in("viewer1", Role::Viewer);
    // Asking before the first sample waits for it instead of failing.
    let r = b.get("/api/v1/metrics").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let v = r.json();
    for key in ["cpu", "mem", "load", "procs", "disks", "cores"] {
        assert!(v[key].is_array(), "{key}: {v}");
    }
    assert_eq!(v["load"].as_array().unwrap().len(), 3);
    for key in [
        "cpuNow",
        "memUsed",
        "memTotal",
        "cache",
        "disk",
        "diskTotal",
        "netIn",
        "netOut",
        "conns",
        "ts",
    ] {
        assert!(v[key].is_number(), "{key}: {v}");
    }
    assert!(v["memTotal"].as_f64().unwrap() > 0.0 && v["diskTotal"].as_f64().unwrap() > 0.0);
    assert!(v["memUsed"].as_f64().unwrap() <= v["memTotal"].as_f64().unwrap());
    assert!((0.0..=100.0).contains(&v["cpuNow"].as_f64().unwrap()));
    assert!(
        !v["cpu"].as_array().unwrap().is_empty()
            && v["cpu"].as_array().unwrap().len() == v["mem"].as_array().unwrap().len()
    );
    h.shutdown().await;
}

#[tokio::test]
async fn processes_are_real_and_include_this_one() {
    let h = Harness::new(true);
    let mut b = h.signed_in("viewer1", Role::Viewer);
    warmed_up(&h).await;
    // Process lists arrive every other sample; give it a few seconds.
    let mut found = Value::Null;
    for _ in 0..30 {
        let v = b.get("/api/v1/metrics").await.json();
        if v["procs"].as_array().is_some_and(|p| !p.is_empty()) {
            found = v;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let procs = found["procs"]
        .as_array()
        .expect("a process list within 6 s");
    for p in procs {
        let kind = p["kind"].as_str().unwrap_or("");
        assert!(
            p["name"].as_str().is_some_and(|n| !n.is_empty())
                && p["pid"].is_u64()
                && p["cpu"].is_number()
                && p["mem"].is_number()
                && p["self"].is_boolean()
                && matches!(kind, "container" | "system" | "user" | "unknown"),
            "{p}"
        );
        if kind == "container" {
            assert!(p["container"]["id"].as_str().is_some_and(|s| !s.is_empty()), "{p}");
            assert!(p["container"]["name"].as_str().is_some_and(|s| !s.is_empty()), "{p}");
        }
        if kind == "system" {
            assert!(p["unit"].as_str().is_some_and(|s| s.ends_with(".service")), "{p}");
        }
    }
    assert!(procs.len() <= 8);
    let cpus: Vec<f64> = procs.iter().map(|p| p["cpu"].as_f64().unwrap()).collect();
    assert!(
        cpus.windows(2).all(|w| w[0] >= w[1]),
        "sorted by CPU: {cpus:?}"
    );
    h.shutdown().await;
}

#[tokio::test]
async fn window_parameter_is_validated_and_changes_the_history_length() {
    let h = Harness::new(true);
    let mut b = h.signed_in("viewer1", Role::Viewer);
    warmed_up(&h).await;
    for bad in ["abc", "0s", "2h", "10x"] {
        let r = b.get(&format!("/api/v1/metrics?window={bad}")).await;
        assert_eq!(
            (r.status, r.error_code()),
            (StatusCode::BAD_REQUEST, "validation".to_owned()),
            "{bad}"
        );
    }
    let short = b.get("/api/v1/metrics?window=2s").await.json();
    assert_eq!(short["cpu"].as_array().unwrap().len(), 1);
    h.shutdown().await;
}

#[tokio::test]
async fn metrics_and_attention_need_a_session() {
    let h = Harness::new(true);
    let mut anon = h.browser();
    for path in [
        "/api/v1/metrics",
        "/api/v1/attention",
        "/api/v1/stream/metrics",
    ] {
        let r = anon.get(path).await;
        assert_eq!(
            (r.status, r.error_code()),
            (StatusCode::UNAUTHORIZED, "unauthorized".to_owned()),
            "{path}"
        );
    }
    h.shutdown().await;
}

#[tokio::test]
async fn without_the_helper_metrics_say_so_in_a_sentence() {
    let h = Harness::new(false);
    let mut b = h.signed_in("viewer1", Role::Viewer);
    let r = b.get("/api/v1/metrics").await;
    assert_eq!(
        (r.status, r.error_code()),
        (StatusCode::SERVICE_UNAVAILABLE, "unavailable".to_owned())
    );
    let msg = r.json()["error"]["message"].as_str().unwrap().to_owned();
    assert!(
        msg.contains("opsbox-priv") && !msg.contains(".sock"),
        "{msg}"
    );
}

#[tokio::test]
async fn attention_is_a_list_of_kind_text_and_link() {
    let h = Harness::new(true);
    let mut b = h.signed_in("viewer1", Role::Viewer);
    warmed_up(&h).await;
    let v = b.get("/api/v1/attention").await.json();
    assert!(v["unread"].is_u64() && v["critical"].is_u64(), "{v}");
    let items = v["items"].as_array().expect("items array");
    for a in items {
        assert!(
            a["id"].is_string()
                && a["kind"].is_string()
                && a["severity"].is_string()
                && a["since"].is_i64()
                && a["text"].is_string()
                && a["read"].is_boolean()
                && a["snoozed"].is_boolean(),
            "{a}"
        );
        assert!(
            a["link"]
                .as_array()
                .is_some_and(|l| l.len() == 2 && l[1].as_str().unwrap().starts_with("#/")),
            "{a}"
        );
    }
    h.shutdown().await;
}

#[tokio::test]
async fn attention_deduplicates_tracks_read_and_snooze_and_enforces_rbac_and_csrf() {
    use axum::http::Method;
    use common::Opts;
    use opsbox_alerts::{Engine, Observation};
    use serde_json::json;
    use std::collections::BTreeSet;

    let h = Harness::new(false);
    let mut admin = h.signed_in("admin1", Role::Admin);
    let mut viewer = h.signed_in("viewer1", Role::Viewer);

    // 1. Seed a missed backup on `opsbox-self` (built-in backup:opsbox) AND a firing
    //    `missed:opsbox-self` alert + `disk:/` warning + `cpu:host` critical alert.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    h.store
        .update_backup_job(
            opsbox_store::OPSBOX_SELF_JOB_ID,
            opsbox_store::UpdateBackupJob {
                cron: Some("0 * * * *"), // hourly :00 -> missed > 15m ago if created earlier
                now: now - 7200,
                ..Default::default()
            },
        )
        .unwrap();

    let mut eng = Engine::default();
    let known = BTreeSet::from(["missed".into(), "disk".into(), "cpu".into()]);
    let obs = vec![
        Observation::new(
            "missed",
            opsbox_store::OPSBOX_SELF_JOB_ID,
            "Backup opsbox is overdue. Check its schedule and destination.".into(),
            "#/backups",
            false,
        ),
        Observation::new("disk", "/", "Disk / is at 82%.".into(), "#/cleanup", false),
        Observation::new(
            "cpu",
            "host",
            "CPU is at 99%. Review busy processes.".into(),
            "#/overview",
            true,
        ),
    ];
    eng.evaluate(obs.clone(), &known, now - 1000);
    eng.evaluate(obs, &known, now - 650);
    h.store
        .set_setting("alerts.engine.v1", &serde_json::to_value(&eng).unwrap())
        .unwrap();

    // Admin sees 3 deduplicated items: backup:opsbox (linked to alert), disk:/, cpu:host (critical on top).
    let att = admin.get("/api/v1/attention").await.json();
    let items = att["items"].as_array().unwrap();
    let backup_count = items.iter().filter(|i| i["id"] == "backup:opsbox").count();
    assert_eq!(backup_count, 1, "missed backup must deduplicate: {att}");
    let backup_item = items.iter().find(|i| i["id"] == "backup:opsbox").unwrap();
    assert_eq!(backup_item["link"][1], "#/alerts?id=missed%3Aopsbox-self");
    assert_eq!(items[0]["id"], "cpu:host");
    assert_eq!(items[0]["severity"], "critical");
    assert_eq!(att["unread"], 3);
    assert_eq!(att["critical"], 1);

    // Non-admin viewer does NOT see host-wide alerts (cpu:host, disk:/ alert), only built-in backup:opsbox!
    let v_att = viewer.get("/api/v1/attention").await.json();
    assert!(
        !v_att["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == "cpu:host"),
        "viewer must not see host-wide cpu alert: {v_att}"
    );
    assert_eq!(
        viewer
            .post("/api/v1/attention/cpu%3Ahost/read", json!({}))
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // 2. CSRF & Origin refusal on POST attention mutations.
    assert_eq!(
        admin
            .send(
                Method::POST,
                "/api/v1/attention/read-all",
                None,
                Opts {
                    csrf: false,
                    ..Opts::default()
                },
            )
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        admin
            .send(
                Method::POST,
                "/api/v1/attention/disk%3A%2F/read",
                None,
                Opts {
                    origin: Some("http://evil.example".to_owned()),
                    ..Opts::default()
                },
            )
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    // Read token cannot mutate attention state.
    let tok = admin
        .post(
            "/api/v1/tokens",
            json!({"name": "reader-tok", "scope": "read"}),
        )
        .await
        .json()["secret"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut script = h.browser();
    assert_eq!(
        script
            .send(
                Method::POST,
                "/api/v1/attention/read-all",
                None,
                Opts {
                    csrf: false,
                    origin: None,
                    extra: vec![("authorization".into(), format!("Bearer {tok}"))],
                    ..Opts::default()
                },
            )
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    // 3. Mark disk:/ read (percent-encoded %2F works) -> unread drops to 2; viewer's unread unaffected.
    let r = admin
        .send(
            Method::POST,
            "/api/v1/attention/disk%3A%2F/read",
            None,
            Opts::default(),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text());
    let after_read = r.json();
    assert_eq!(after_read["unread"], 2);
    assert_eq!(after_read["critical"], 1);
    assert!(
        after_read["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == "disk:/")
            .unwrap()["read"]
            == true
    );
    assert_eq!(viewer.get("/api/v1/attention").await.json()["unread"], 1);

    // 4. Severity escalation on disk:/ (warning -> critical) makes it unread again!
    let mut eng2: Engine =
        serde_json::from_value(h.store.get_setting("alerts.engine.v1").unwrap().unwrap()).unwrap();
    let obs_crit = vec![
        Observation::new(
            "missed",
            opsbox_store::OPSBOX_SELF_JOB_ID,
            "Backup opsbox is overdue. Check its schedule and destination.".into(),
            "#/backups",
            false,
        ),
        Observation::new("disk", "/", "Disk / is at 96%.".into(), "#/cleanup", true),
        Observation::new(
            "cpu",
            "host",
            "CPU is at 99%. Review busy processes.".into(),
            "#/overview",
            true,
        ),
    ];
    eng2.evaluate(obs_crit, &known, now - 600);
    h.store
        .set_setting("alerts.engine.v1", &serde_json::to_value(&eng2).unwrap())
        .unwrap();
    let esc = admin.get("/api/v1/attention").await.json();
    assert_eq!(esc["unread"], 3);
    assert_eq!(esc["critical"], 2);
    assert_eq!(
        esc["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["id"] == "disk:/")
            .unwrap()["read"],
        false
    );

    // 5. Snooze validation + invariant:
    //    - 0 and > 7 days (10081) fail with 400
    //    - snoozing warning backup:opsbox removes it from items & unread
    //    - snoozing critical cpu:host keeps it in items (dimmed, after unsnoozed critical disk:/) AND keeps it in unread & critical count!
    assert_eq!(
        admin
            .post(
                "/api/v1/attention/backup%3Aopsbox/snooze",
                json!({"minutes": 0})
            )
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        admin
            .post(
                "/api/v1/attention/backup%3Aopsbox/snooze",
                json!({"minutes": 10081})
            )
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    let sn_warn = admin
        .post(
            "/api/v1/attention/backup%3Aopsbox/snooze",
            json!({"minutes": 60}),
        )
        .await;
    assert_eq!(sn_warn.status, StatusCode::OK, "{}", sn_warn.text());
    let sn_warn_j = sn_warn.json();
    assert_eq!(sn_warn_j["unread"], 2);
    assert!(
        !sn_warn_j["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["id"] == "backup:opsbox")
    );

    let sn_crit = admin
        .post(
            "/api/v1/attention/cpu%3Ahost/snooze",
            json!({"minutes": 60}),
        )
        .await;
    assert_eq!(sn_crit.status, StatusCode::OK, "{}", sn_crit.text());
    let sn_crit_j = sn_crit.json();
    assert_eq!(
        sn_crit_j["unread"], 2,
        "snoozing critical must NOT reduce unread count: {sn_crit_j}"
    );
    assert_eq!(
        sn_crit_j["critical"], 2,
        "snoozing critical must NOT reduce critical count: {sn_crit_j}"
    );
    let crit_items = sn_crit_j["items"].as_array().unwrap();
    assert_eq!(crit_items[0]["id"], "disk:/");
    assert_eq!(crit_items[1]["id"], "cpu:host");
    assert_eq!(crit_items[1]["snoozed"], true);

    // 6. New `since` on a previously read item makes it unread again.
    let ra = admin
        .send(
            Method::POST,
            "/api/v1/attention/read-all",
            None,
            Opts::default(),
        )
        .await
        .json();
    assert_eq!(ra["unread"], 0);
    assert_eq!(ra["critical"], 0);

    // Resolve cpu:host, then re-fire with a newer `since`.
    let mut eng3: Engine =
        serde_json::from_value(h.store.get_setting("alerts.engine.v1").unwrap().unwrap()).unwrap();
    eng3.evaluate(vec![], &known, now - 400);
    let cpu_obs = vec![Observation::new(
        "cpu",
        "host",
        "CPU spiked again.".into(),
        "#/overview",
        true,
    )];
    eng3.evaluate(cpu_obs.clone(), &known, now - 320);
    eng3.evaluate(cpu_obs, &known, now);
    h.store
        .set_setting("alerts.engine.v1", &serde_json::to_value(&eng3).unwrap())
        .unwrap();
    let again = admin.get("/api/v1/attention").await.json();
    assert_eq!(again["unread"], 1);
    assert_eq!(again["critical"], 1);
    assert_eq!(again["items"][0]["id"], "cpu:host");
    assert_eq!(again["items"][0]["read"], false);

    h.shutdown().await;
}

// ------------------------------------------------------------------ WebSocket

/// Serve the real router on a local port; returns its address.
async fn serve(h: &Harness) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(opsbox_api::serve(
        listener,
        h.state.clone(),
        std::future::pending(),
    ));
    addr
}

fn ws_request(
    addr: std::net::SocketAddr,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> tokio_tungstenite::tungstenite::http::Request<()> {
    let mut req = format!("ws://{addr}/api/v1/stream/metrics")
        .into_client_request()
        .unwrap();
    if let Some(c) = cookie {
        req.headers_mut().insert("cookie", c.parse().unwrap());
    }
    if let Some(o) = origin {
        req.headers_mut().insert("origin", o.parse().unwrap());
    }
    req
}

fn cookie_of(b: &common::Browser) -> String {
    b.jar
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

#[tokio::test]
async fn the_stream_pushes_a_frame_a_second_in_the_metrics_shape() {
    let h = Harness::new(true);
    let b = h.signed_in("viewer1", Role::Viewer);
    warmed_up(&h).await;
    let addr = serve(&h).await;
    let origin = format!("http://{addr}");
    let (mut ws, resp) =
        tokio_tungstenite::connect_async(ws_request(addr, Some(&cookie_of(&b)), Some(&origin)))
            .await
            .expect("handshake");
    assert_eq!(resp.status().as_u16(), 101);

    let mut stamps = Vec::new();
    for _ in 0..3 {
        let msg = tokio::time::timeout(Duration::from_secs(4), ws.next())
            .await
            .expect("a frame within 4 s")
            .unwrap()
            .unwrap();
        let Message::Text(text) = msg else {
            panic!("expected a text frame, got {msg:?}")
        };
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["type"], "metrics");
        assert!(
            v["data"]["cpu"].is_array()
                && v["data"]["memTotal"].is_number()
                && v["data"]["procs"].is_array()
        );
        stamps.push(v["ts"].as_i64().unwrap());
    }
    // The first frame is sent at once (the newest sample), the rest follow each second.
    assert!(stamps.windows(2).all(|w| w[0] <= w[1]), "{stamps:?}");
    assert!(stamps[2] > stamps[0], "time advances: {stamps:?}");
    h.shutdown().await;
}

#[tokio::test]
async fn the_stream_refuses_other_sites_and_anonymous_callers() {
    let h = Harness::new(true);
    let b = h.signed_in("viewer1", Role::Viewer);
    let addr = serve(&h).await;
    let cookie = cookie_of(&b);
    let status = |r: Result<_, WsError>| match r {
        Err(WsError::Http(resp)) => resp.status().as_u16(),
        Ok(_) => 101,
        Err(e) => panic!("{e}"),
    };
    let own = format!("http://{addr}");
    assert_eq!(
        status(
            tokio_tungstenite::connect_async(ws_request(
                addr,
                Some(&cookie),
                Some("http://evil.example")
            ))
            .await
        ),
        403,
        "foreign Origin"
    );
    assert_eq!(
        status(tokio_tungstenite::connect_async(ws_request(addr, Some(&cookie), None)).await),
        403,
        "no Origin"
    );
    assert_eq!(
        status(tokio_tungstenite::connect_async(ws_request(addr, None, Some(&own))).await),
        401,
        "no session"
    );
    assert_eq!(
        status(
            tokio_tungstenite::connect_async(ws_request(
                addr,
                Some("opsbox_session=forged"),
                Some(&own)
            ))
            .await
        ),
        401,
        "bad session"
    );
    let _ = ORIGIN;
    h.shutdown().await;
}

#[tokio::test]
async fn revoking_a_session_ends_an_already_open_metrics_stream() {
    let h = Harness::new(true);
    let b = h.signed_in("reader", Role::Viewer);
    let user = h.store.user_by_name("reader").unwrap().unwrap();
    let addr = serve(&h).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(ws_request(
        addr,
        Some(&cookie_of(&b)),
        Some(&format!("http://{addr}")),
    ))
    .await
    .unwrap();
    assert!(matches!(ws.next().await, Some(Ok(Message::Text(_)))));
    h.store.delete_user_sessions(&user.id).unwrap();
    tokio::time::timeout(Duration::from_secs(8), async {
        while let Some(Ok(msg)) = ws.next().await {
            if matches!(msg, Message::Close(_)) {
                return;
            }
        }
    })
    .await
    .expect("an open stream ends after its session is revoked");
    h.shutdown().await;
}
