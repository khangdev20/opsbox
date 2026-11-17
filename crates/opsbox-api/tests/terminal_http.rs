//! The terminal over HTTP and WebSocket: roles, Origin, the stream itself, the audit trail.

mod common;

use std::os::unix::fs::MetadataExt;
use std::time::Duration;

use common::Harness;
use futures_util::{SinkExt, StreamExt};
use opsbox_proto::Role;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn serve(h: &Harness, idle: Duration) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(opsbox_api::serve(
        listener,
        h.state.clone().with_terminal_idle(idle),
        std::future::pending(),
    ));
    addr
}

fn cookie_of(b: &common::Browser) -> String {
    b.jar
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn request(
    addr: std::net::SocketAddr,
    query: &str,
    cookie: Option<&str>,
    origin: Option<&str>,
) -> tokio_tungstenite::tungstenite::http::Request<()> {
    let mut req = format!("ws://{addr}/api/v1/stream/terminal?{query}")
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

/// The account the test process runs as, as the terminal page would list it.
async fn own_user(b: &mut common::Browser, h: &Harness) -> String {
    let probe = h.root.join("uid-probe");
    std::fs::write(&probe, b"").unwrap();
    let uid = u64::from(std::fs::metadata(&probe).unwrap().uid());
    let list = b.get("/api/v1/terminal/users").await;
    assert_eq!(list.status, 200, "{}", list.text());
    list.json()["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["uid"].as_u64() == Some(uid))
        .unwrap_or_else(|| panic!("the test user (uid {uid}) is offered: {}", list.text()))["name"]
        .as_str()
        .unwrap()
        .to_owned()
}

async fn open(addr: std::net::SocketAddr, b: &common::Browser, user: &str) -> Ws {
    let origin = format!("http://{addr}");
    // `confirm=root` makes the test independent of whether CI runs as root.
    let q = format!("user={user}&cols=90&rows=30&confirm=root");
    let (ws, resp) =
        tokio_tungstenite::connect_async(request(addr, &q, Some(&cookie_of(b)), Some(&origin)))
            .await
            .expect("handshake");
    assert_eq!(resp.status().as_u16(), 101);
    ws
}

/// Collect shell output (binary frames) until `needle`; returns the first text frame
/// that arrived instead (for example `exit`) when the stream ends earlier.
async fn read_until(ws: &mut Ws, needle: &str) -> Result<String, Value> {
    let mut seen = String::new();
    let step = async {
        while let Some(m) = ws.next().await {
            match m.unwrap() {
                Message::Binary(b) => {
                    seen.push_str(&String::from_utf8_lossy(&b));
                    if seen.contains(needle) {
                        return Ok(());
                    }
                }
                Message::Text(t) => return Err(serde_json::from_str::<Value>(&t).unwrap()),
                Message::Close(_) => return Err(json!({"type": "closed"})),
                _ => {}
            }
        }
        Err(json!({"type": "closed"}))
    };
    tokio::time::timeout(Duration::from_secs(20), step)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {needle:?}; saw {seen:?}"))?;
    Ok(seen)
}

async fn first_text(ws: &mut Ws) -> Value {
    loop {
        let m = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("a message")
            .expect("not closed")
            .unwrap();
        if let Message::Text(t) = m {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

fn audit_actions(h: &Harness) -> Vec<(String, Value)> {
    h.store
        .recent_audit(50)
        .unwrap()
        .into_iter()
        .filter(|e| e.action.starts_with("terminal."))
        .map(|e| {
            (
                e.action,
                serde_json::from_str(&e.detail.to_string()).unwrap(),
            )
        })
        .collect()
}

async fn wait_for_audit(h: &Harness, action: &str) -> Value {
    for _ in 0..100 {
        if let Some((_, d)) = audit_actions(h).into_iter().find(|(a, _)| a == action) {
            return d;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("no {action} in the audit log: {:?}", audit_actions(h));
}

#[tokio::test]
async fn only_admins_get_a_terminal_and_other_sites_get_nothing() {
    let h = Harness::new(true);
    let addr = serve(&h, Duration::from_secs(60)).await;
    let origin = format!("http://{addr}");
    let status = |r: Result<_, WsError>| match r {
        Err(WsError::Http(resp)) => resp.status().as_u16(),
        Ok(_) => 101,
        Err(e) => panic!("{e}"),
    };

    for role in [Role::Viewer, Role::Deployer] {
        let mut b = h.signed_in(&format!("{role:?}"), role);
        assert_eq!(b.get("/api/v1/terminal/users").await.status, 403);
        let c = cookie_of(&b);
        assert_eq!(
            status(
                tokio_tungstenite::connect_async(request(addr, "", Some(&c), Some(&origin))).await
            ),
            403,
            "{role:?}"
        );
    }
    let mut admin = h.signed_in("boss", Role::Admin);
    let c = cookie_of(&admin);
    assert_eq!(
        status(
            tokio_tungstenite::connect_async(request(
                addr,
                "",
                Some(&c),
                Some("http://evil.example")
            ))
            .await
        ),
        403,
        "foreign Origin"
    );
    assert_eq!(
        status(tokio_tungstenite::connect_async(request(addr, "", Some(&c), None)).await),
        403,
        "no Origin"
    );
    assert_eq!(
        status(tokio_tungstenite::connect_async(request(addr, "", None, Some(&origin))).await),
        401,
        "no session"
    );
    // Nothing started, so nothing is in the audit trail.
    assert!(audit_actions(&h).is_empty());
    let _ = admin.get("/healthz").await;
    h.shutdown().await;
}

#[tokio::test]
async fn bad_choices_are_explained_inside_the_socket_and_no_shell_starts() {
    let h = Harness::new(true);
    let mut admin = h.signed_in("boss", Role::Admin);
    let addr = serve(&h, Duration::from_secs(60)).await;
    let origin = format!("http://{addr}");
    let c = cookie_of(&admin);
    // The upgrade succeeds; the first message is an `error` saying why, then the socket closes.
    let refusal = |q: &'static str| {
        let (c, origin) = (c.clone(), origin.clone());
        async move {
            let (mut ws, resp) =
                tokio_tungstenite::connect_async(request(addr, q, Some(&c), Some(&origin)))
                    .await
                    .unwrap();
            assert_eq!(resp.status().as_u16(), 101);
            let v = first_text(&mut ws).await;
            assert_eq!(v["type"], "error", "{q}: {v}");
            let closed = async { while ws.next().await.is_some_and(|m| m.is_ok()) {} };
            tokio::time::timeout(Duration::from_secs(5), closed)
                .await
                .expect("closed after the error");
            (
                v["code"].as_str().unwrap().to_owned(),
                v["message"].as_str().unwrap().to_owned(),
            )
        }
    };
    // Root needs an explicit confirmation.
    let (code, msg) = refusal("user=root").await;
    assert_eq!(code, "confirmation_required");
    assert!(msg.contains("root"), "{msg}");
    // Accounts that are not offered (no login shell, not there, hostile names).
    for q in [
        "user=daemon",
        "user=no-such-person",
        "user=-r",
        "user=a%3Bb",
    ] {
        let (code, msg) = refusal(q).await;
        assert_eq!(code, "validation", "{q}");
        assert!(msg.contains("cannot be opened"), "{q}: {msg}");
    }
    assert!(audit_actions(&h).is_empty(), "nothing started");
    let _ = own_user(&mut admin, &h).await;
    h.shutdown().await;
}

#[tokio::test]
async fn a_missing_helper_is_explained_too() {
    let h = Harness::new(false);
    let admin = h.signed_in("boss", Role::Admin);
    let addr = serve(&h, Duration::from_secs(60)).await;
    let origin = format!("http://{addr}");
    let (mut ws, _) = tokio_tungstenite::connect_async(request(
        addr,
        "user=x",
        Some(&cookie_of(&admin)),
        Some(&origin),
    ))
    .await
    .unwrap();
    let v = first_text(&mut ws).await;
    assert_eq!(v["type"], "error");
    assert_eq!(v["code"], "unavailable");
    h.shutdown().await;
}

#[tokio::test]
async fn a_real_shell_over_the_websocket_with_resize_exit_and_audit() {
    let h = Harness::new(true);
    let mut admin = h.signed_in("boss", Role::Admin);
    let me = own_user(&mut admin, &h).await;
    let addr = serve(&h, Duration::from_secs(60)).await;
    let mut ws = open(addr, &admin, &me).await;

    let ready = first_text(&mut ws).await;
    assert_eq!(ready["type"], "ready");
    assert_eq!(ready["user"], me.as_str());

    // Keystrokes go in as binary frames; the sum is computed by the shell.
    ws.send(Message::binary(b"echo sum=$((40+2))\r".to_vec()))
        .await
        .unwrap();
    read_until(&mut ws, "sum=42").await.unwrap();

    // The size is the one asked for at the start, and a resize message changes it.
    ws.send(Message::binary(b"stty size\r".to_vec()))
        .await
        .unwrap();
    read_until(&mut ws, "30 90").await.unwrap();
    ws.send(Message::text(
        json!({"type": "resize", "cols": 120, "rows": 40}).to_string(),
    ))
    .await
    .unwrap();
    ws.send(Message::binary(b"stty size\r".to_vec()))
        .await
        .unwrap();
    read_until(&mut ws, "40 120").await.unwrap();

    // Junk in text frames is ignored, not fatal.
    ws.send(Message::text("{not json")).await.unwrap();
    ws.send(Message::text(
        json!({"type": "resize", "cols": "x"}).to_string(),
    ))
    .await
    .unwrap();
    ws.send(Message::binary(b"echo still=alive\r".to_vec()))
        .await
        .unwrap();
    read_until(&mut ws, "still=alive").await.unwrap();

    ws.send(Message::binary(b"exit 3\r".to_vec()))
        .await
        .unwrap();
    let end = read_until(&mut ws, "never-appears").await.unwrap_err();
    assert_eq!(end, json!({"type": "exit", "code": 3}));

    let start = wait_for_audit(&h, "terminal.start").await;
    assert_eq!(start["ip"], "127.0.0.1");
    let end = wait_for_audit(&h, "terminal.end").await;
    assert_eq!(end["reason"], "shell_exited");
    // What was typed is never recorded.
    let all = serde_json::to_string(&audit_actions(&h)).unwrap();
    assert!(!all.contains("sum=") && !all.contains("stty"), "{all}");
    let entries = h.store.recent_audit(50).unwrap();
    let e = entries
        .iter()
        .find(|e| e.action == "terminal.start")
        .unwrap();
    assert_eq!(e.actor, "boss");
    assert_eq!(e.target.as_deref(), Some(me.as_str()));
    h.shutdown().await;
}

#[tokio::test]
async fn closing_the_tab_ends_the_session_and_says_so_in_the_audit_log() {
    let h = Harness::new(true);
    let mut admin = h.signed_in("boss", Role::Admin);
    let me = own_user(&mut admin, &h).await;
    let addr = serve(&h, Duration::from_secs(60)).await;
    let mut ws = open(addr, &admin, &me).await;
    first_text(&mut ws).await;
    ws.send(Message::binary(b"echo up\r".to_vec()))
        .await
        .unwrap();
    read_until(&mut ws, "up").await.unwrap();
    drop(ws);
    let end = wait_for_audit(&h, "terminal.end").await;
    assert_eq!(end["reason"], "closed");
    h.shutdown().await;
}

#[tokio::test]
async fn an_untouched_terminal_is_closed_after_the_idle_timeout() {
    let h = Harness::new(true);
    let mut admin = h.signed_in("boss", Role::Admin);
    let me = own_user(&mut admin, &h).await;
    let addr = serve(&h, Duration::from_secs(1)).await;
    let mut ws = open(addr, &admin, &me).await;
    first_text(&mut ws).await;
    // Wait for the server to hang up on us.
    let closed = async { while ws.next().await.is_some_and(|m| m.is_ok()) {} };
    tokio::time::timeout(Duration::from_secs(15), closed)
        .await
        .expect("the idle terminal is closed");
    let end = wait_for_audit(&h, "terminal.end").await;
    assert_eq!(end["reason"], "idle");
    h.shutdown().await;
}

#[tokio::test]
async fn output_is_flow_controlled_a_flood_does_not_stall_the_server() {
    // A program that prints a lot while the browser reads slowly: the session stays
    // responsive and the shell can still be told to stop.
    let h = Harness::new(true);
    let mut admin = h.signed_in("boss", Role::Admin);
    let me = own_user(&mut admin, &h).await;
    let addr = serve(&h, Duration::from_secs(60)).await;
    let mut ws = open(addr, &admin, &me).await;
    first_text(&mut ws).await;
    ws.send(Message::binary(b"i=0; while [ $i -lt 3000 ]; do echo flood-line-$i-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; i=$((i+1)); done; echo flood-done\r".to_vec()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await; // let the output pile up unread
    read_until(&mut ws, "flood-done").await.unwrap();
    // Other requests are served normally meanwhile.
    assert_eq!(admin.get("/healthz").await.status, 200);
    h.shutdown().await;
}
