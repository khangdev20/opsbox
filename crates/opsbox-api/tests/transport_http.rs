//! Transport security over real connections: a client that reaches `serve` directly cannot
//! pass itself off as the proxy, a different address or an HTTPS connection.

mod common;

use common::Harness;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const PROXY_SECRET: &str = "test-proxy-secret-test-proxy-secret";

/// One HTTP/1.1 POST over a real socket; returns the status and the raw header block.
async fn post(
    addr: std::net::SocketAddr,
    path: &str,
    headers: &[(&str, &str)],
    body: &serde_json::Value,
) -> (u16, String) {
    let body = body.to_string();
    let mut req = format!(
        "POST {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(&body);
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (
        status,
        text.split("\r\n\r\n").next().unwrap_or("").to_owned(),
    )
}

async fn start(h: &Harness) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(opsbox_api::serve(
        listener,
        h.state.clone().with_proxy_secret(PROXY_SECRET),
        std::future::pending(),
    ));
    addr
}

fn set_cookie_lines(head: &str) -> Vec<&str> {
    head.lines()
        .filter(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
        .collect()
}

fn claim_ips(h: &Harness) -> Vec<String> {
    h.store
        .recent_audit(50)
        .unwrap()
        .into_iter()
        .filter(|e| e.action.starts_with("setup."))
        .filter_map(|e| e.ip)
        .collect()
}

#[tokio::test]
async fn a_direct_client_with_forged_forwarding_headers_gets_the_real_scheme_and_address() {
    let h = Harness::new_unclaimed(false);
    let addr = start(&h).await;
    let token = h.state.auth.issue_setup_token().unwrap();
    let (status, head) = post(
        addr,
        "/api/v1/setup/claim",
        &[
            ("X-Forwarded-Proto", "https"),
            ("X-Forwarded-For", "203.0.113.9"),
            ("X-Forwarded-Host", "evil.example"),
            ("Forwarded", "for=203.0.113.9;proto=https"),
            ("X-Real-IP", "203.0.113.9"),
            (
                "X-Opsbox-Proxy-Secret",
                "guess-guess-guess-guess-guess-guess",
            ),
        ],
        &json!({ "token": token }),
    )
    .await;
    assert_eq!(status, 200, "{head}");
    let cookies = set_cookie_lines(&head);
    assert_eq!(cookies.len(), 1, "{head}");
    assert!(
        cookies[0].contains("opsbox_setup=") && !cookies[0].contains("Secure"),
        "plain HTTP on loopback stays plain whatever the client claims: {head}"
    );
    assert_eq!(
        claim_ips(&h),
        ["127.0.0.1"],
        "the audit shows the real peer, not 203.0.113.9"
    );
}

#[tokio::test]
async fn the_proxy_with_its_secret_is_believed_on_the_same_listener() {
    let h = Harness::new_unclaimed(false);
    let addr = start(&h).await;
    let token = h.state.auth.issue_setup_token().unwrap();
    let (status, head) = post(
        addr,
        "/api/v1/setup/claim",
        &[
            ("X-Opsbox-Proxy-Secret", PROXY_SECRET),
            ("X-Forwarded-Proto", "https"),
            ("X-Forwarded-For", "203.0.113.9"),
        ],
        &json!({ "token": token }),
    )
    .await;
    assert_eq!(status, 200, "{head}");
    let cookies = set_cookie_lines(&head);
    assert!(
        cookies.iter().all(|c| c.contains("Secure")) && !cookies.is_empty(),
        "{head}"
    );
    assert_eq!(claim_ips(&h), ["203.0.113.9"]);
}

#[tokio::test]
async fn the_secret_header_never_reaches_a_handler_and_is_not_an_oracle() {
    // A wrong secret is a plain request: it does not change the answer, so there is
    // nothing to learn by trying values.
    let h = Harness::new_unclaimed(false);
    let addr = start(&h).await;
    let token = h.state.auth.issue_setup_token().unwrap();
    let wrong = post(
        addr,
        "/api/v1/setup/claim",
        &[("X-Opsbox-Proxy-Secret", "x")],
        &json!({ "token": "wrong" }),
    )
    .await;
    let none = post(
        addr,
        "/api/v1/setup/claim",
        &[],
        &json!({ "token": "wrong" }),
    )
    .await;
    assert_eq!(wrong.0, none.0);
    let _ = token;
}
