//! How a request really arrived: the TCP peer, whether the connection was encrypted,
//! and when a forwarding proxy may speak for the client.
//!
//! `X-Forwarded-For`, `X-Forwarded-Proto` and friends are plain headers that any client
//! can send, so they are believed only when the TCP peer is on this machine **and** the
//! request carries the proxy's shared secret ([`PROXY_SECRET_HEADER`]). Everything else
//! has those headers removed before a handler can read them. The result is one
//! [`Transport`] value in the request extensions; rate limits, the allow-list, audit
//! entries and cookie attributes all read it instead of looking at headers.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::http::HeaderMap;
use axum::middleware::Next;
use axum::response::Response;
use opsbox_store::{Store, StoreError};

use crate::session::SecureCookies;

/// Setting that holds the secret `opsbox-proxy` adds to requests it forwards to the panel.
pub const PROXY_SECRET_KEY: &str = "opsbox.proxy.forward_secret";
/// Header the proxy adds (after removing any copy the client sent).
pub const PROXY_SECRET_HEADER: &str = "x-opsbox-proxy-secret";

/// Headers a client can send to claim an address, scheme or host it does not have.
const FORWARDING_HEADERS: [&str; 6] = [
    "x-forwarded-for",
    "x-forwarded-proto",
    "x-forwarded-host",
    "x-forwarded-port",
    "forwarded",
    "x-real-ip",
];

/// Read the proxy secret, creating it on first use. It lives in the database, which only
/// `opsbox` and root can read, so `serve` and the `opsbox web` command agree on it.
pub fn ensure_proxy_secret(store: &Store) -> Result<String, StoreError> {
    if let Some(s) = store
        .get_setting(PROXY_SECRET_KEY)?
        .and_then(|v| v.as_str().map(str::to_owned))
        .filter(|s| s.len() >= 32)
    {
        return Ok(s);
    }
    let secret = opsbox_auth::tokens::new_token()
        .map_err(|e| StoreError::Io(std::io::Error::other(e.to_string())))?;
    store.set_setting(PROXY_SECRET_KEY, &serde_json::json!(secret))?;
    Ok(secret)
}

/// The facts about one request that handlers may rely on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transport {
    /// The caller: the forwarded address from a trusted proxy, else the TCP peer.
    /// `None` only when the router is called without a connection (unit tests).
    pub client_ip: Option<IpAddr>,
    /// The caller reached us over TLS (our own listener, or a trusted proxy that says so),
    /// or the operator asked for `Secure` cookies always.
    pub https: bool,
    /// A trusted proxy forwarded this request.
    pub via_proxy: bool,
}

impl Transport {
    pub fn client_ip_text(&self) -> String {
        self.client_ip
            .map_or_else(|| "unknown".to_owned(), |ip| ip.to_string())
    }
}

/// What the middleware needs from the application state.
#[derive(Clone)]
pub(crate) struct Policy {
    secret: Option<Arc<str>>,
    listener_tls: bool,
    always_secure: bool,
}

impl Policy {
    pub(crate) fn new(
        secret: Option<Arc<str>>,
        listener_tls: bool,
        cookies: SecureCookies,
    ) -> Self {
        Self {
            secret,
            listener_tls,
            always_secure: cookies == SecureCookies::Always,
        }
    }

    /// Decide who the caller is and clean the headers.
    pub(crate) fn resolve(&self, peer: Option<SocketAddr>, headers: &mut HeaderMap) -> Transport {
        let trusted = peer.is_some_and(|p| p.ip().to_canonical().is_loopback())
            && self.secret.as_deref().is_some_and(|secret| {
                headers
                    .get(PROXY_SECRET_HEADER)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|sent| opsbox_auth::tokens::ct_eq(sent, secret))
            });
        // Nothing the client sent may reach a handler under the proxy's name.
        headers.remove(PROXY_SECRET_HEADER);

        let mut client_ip = peer.map(|p| p.ip());
        let mut forwarded_https = false;
        if trusted {
            // The proxy appends the address it saw as the last entry.
            if let Some(ip) = headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit(',').next())
                .and_then(|v| v.trim().parse::<IpAddr>().ok())
            {
                client_ip = Some(ip);
            }
            forwarded_https = headers
                .get("x-forwarded-proto")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("https"));
        } else {
            for name in FORWARDING_HEADERS {
                headers.remove(name);
            }
        }
        Transport {
            client_ip,
            https: self.always_secure || self.listener_tls || forwarded_https,
            via_proxy: trusted,
        }
    }
}

/// Outermost layer: resolve [`Transport`] and strip untrusted forwarding headers.
pub(crate) async fn middleware(
    State(policy): State<Policy>,
    mut req: Request,
    next: Next,
) -> Response {
    let peer = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    let transport = policy.resolve(peer, req.headers_mut());
    req.extensions_mut().insert(transport);
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn policy(listener_tls: bool, cookies: SecureCookies) -> Policy {
        Policy::new(Some(SECRET.into()), listener_tls, cookies)
    }

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    fn peer(s: &str) -> Option<SocketAddr> {
        Some(s.parse().unwrap())
    }

    #[test]
    fn a_client_cannot_claim_an_address_or_scheme() {
        let mut h = headers(&[
            ("x-forwarded-for", "203.0.113.9"),
            ("x-forwarded-proto", "https"),
            ("x-forwarded-host", "evil.example"),
            ("forwarded", "for=203.0.113.9;proto=https"),
            ("x-real-ip", "203.0.113.9"),
        ]);
        let t = policy(false, SecureCookies::Auto).resolve(peer("198.51.100.4:5000"), &mut h);
        assert_eq!(t.client_ip, Some("198.51.100.4".parse().unwrap()));
        assert!(!t.https && !t.via_proxy);
        for name in FORWARDING_HEADERS {
            assert!(!h.contains_key(name), "{name} must be stripped");
        }
    }

    #[test]
    fn a_loopback_peer_without_the_secret_is_not_a_proxy() {
        // For example an SSH tunnel, or another program on this machine.
        for sent in [None, Some("wrong-secret-of-the-same-length!!!!")] {
            let mut pairs = vec![
                ("x-forwarded-for", "203.0.113.9"),
                ("x-forwarded-proto", "https"),
            ];
            if let Some(s) = sent {
                pairs.push((PROXY_SECRET_HEADER, s));
            }
            let mut h = headers(&pairs);
            let t = policy(false, SecureCookies::Auto).resolve(peer("127.0.0.1:5000"), &mut h);
            assert_eq!(t.client_ip, Some("127.0.0.1".parse().unwrap()));
            assert!(!t.https && !t.via_proxy);
            assert!(!h.contains_key("x-forwarded-for") && !h.contains_key(PROXY_SECRET_HEADER));
        }
    }

    #[test]
    fn the_secret_from_a_remote_peer_is_not_enough() {
        let mut h = headers(&[
            (PROXY_SECRET_HEADER, SECRET),
            ("x-forwarded-for", "203.0.113.9"),
            ("x-forwarded-proto", "https"),
        ]);
        let t = policy(false, SecureCookies::Auto).resolve(peer("198.51.100.4:5000"), &mut h);
        assert_eq!(t.client_ip, Some("198.51.100.4".parse().unwrap()));
        assert!(!t.https && !t.via_proxy);
        assert!(!h.contains_key("x-forwarded-for") && !h.contains_key(PROXY_SECRET_HEADER));
    }

    #[test]
    fn the_proxy_with_its_secret_speaks_for_the_client() {
        // A client-supplied first entry is ignored: the proxy's own entry is last.
        let mut h = headers(&[
            (PROXY_SECRET_HEADER, SECRET),
            ("x-forwarded-for", "10.0.0.1, 203.0.113.9"),
            ("x-forwarded-proto", "https"),
        ]);
        let t = policy(false, SecureCookies::Auto).resolve(peer("127.0.0.1:5000"), &mut h);
        assert_eq!(t.client_ip, Some("203.0.113.9".parse().unwrap()));
        assert!(t.https && t.via_proxy);
        assert!(!h.contains_key(PROXY_SECRET_HEADER));
    }

    #[test]
    fn the_proxy_saying_http_is_not_https() {
        let mut h = headers(&[
            (PROXY_SECRET_HEADER, SECRET),
            ("x-forwarded-for", "203.0.113.9"),
            ("x-forwarded-proto", "http"),
        ]);
        let t = policy(false, SecureCookies::Auto).resolve(peer("127.0.0.1:5000"), &mut h);
        assert!(!t.https && t.via_proxy);
    }

    #[test]
    fn our_own_tls_listener_decides_the_scheme_not_headers() {
        let mut h = HeaderMap::new();
        let t = policy(true, SecureCookies::Auto).resolve(peer("198.51.100.4:5000"), &mut h);
        assert!(t.https && !t.via_proxy);
        let t = policy(false, SecureCookies::Always).resolve(peer("127.0.0.1:5000"), &mut h);
        assert!(t.https, "`always` marks cookies Secure on any listener");
    }

    #[test]
    fn without_a_configured_secret_nothing_is_trusted() {
        let p = Policy::new(None, false, SecureCookies::Auto);
        let mut h = headers(&[
            (PROXY_SECRET_HEADER, ""),
            ("x-forwarded-for", "203.0.113.9"),
        ]);
        let t = p.resolve(peer("127.0.0.1:5000"), &mut h);
        assert!(!t.via_proxy);
        assert_eq!(t.client_ip, Some("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn no_connection_means_no_address() {
        let mut h = HeaderMap::new();
        let t = policy(false, SecureCookies::Auto).resolve(None, &mut h);
        assert_eq!(t.client_ip, None);
        assert_eq!(t.client_ip_text(), "unknown");
    }

    #[test]
    fn the_secret_is_created_once_and_kept() {
        let store = Store::open_in_memory().unwrap();
        let a = ensure_proxy_secret(&store).unwrap();
        assert!(a.len() >= 32);
        assert_eq!(ensure_proxy_secret(&store).unwrap(), a);
    }
}
