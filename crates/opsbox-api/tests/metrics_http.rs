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
        assert!(
            p["name"].as_str().is_some_and(|n| !n.is_empty())
                && p["pid"].is_u64()
                && p["cpu"].is_number()
                && p["mem"].is_number()
                && p["self"].is_boolean(),
            "{p}"
        );
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
    let items = v.as_array().expect("a list");
    for a in items {
        assert!(a["kind"].is_string() && a["text"].is_string(), "{a}");
        assert!(
            a["link"]
                .as_array()
                .is_some_and(|l| l.len() == 2 && l[1].as_str().unwrap().starts_with("#/")),
            "{a}"
        );
    }
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
