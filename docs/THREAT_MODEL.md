# Threat model

Short, one section per area. Extend it when a prompt changes what is trusted.

## Transport (prompt 16e)

What we defend against: someone on the network reading or changing traffic, a cookie that leaks or is planted, and a client that lies about where it came from.

| Threat | Control | Where it is pinned |
|---|---|---|
| Passwords and cookies on the wire | `serve` never listens on a non-loopback address without TLS: config load refuses `tls = false` there, in every build. TLS 1.2 minimum, 1.3 preferred, rustls default suites, ALPN `h2` and `http/1.1`. | `config.rs` tests; `tls.rs` tests; `scripts/check-transport.sh` |
| Fake certificate on first connect | `opsbox status` prints the SHA-256 fingerprint of the certificate the listener really presents (a handshake, not a file). | `status.rs` tests |
| Cookie sent over HTTP | `Secure` is set whenever the request arrived over TLS, decided by our listener or the trusted proxy, never by a client header. There is no "never" setting. | `session.rs`, `auth_http.rs` |
| Cookie planted from a sibling site | Over HTTPS the session cookie is `__Host-opsbox_session`; a plain-named cookie is ignored there. | `auth_http.rs` |
| Spoofed `X-Forwarded-For` / `-Proto` (bypass of the allowed-address list, rate limits, audit, `Secure`) | Honoured only from a loopback peer that sends the proxy's shared secret; otherwise removed before handlers run. | `transport.rs`, `transport_http.rs`, `security_http.rs` |
| Another program on the same machine posing as the proxy | Loopback alone is not enough: it must also know the secret, which is stored in the database (mode 0600) and added by the proxy to the panel route only. | `transport.rs` tests |
| Session kept after sign-out | Sign-out deletes the session on the server; a copied cookie stops working. | `auth_http.rs` |

Not covered here: response caching and default headers (16r), secrets in responses and logs (16o). In scope for no one: encrypting JSON inside TLS, mutual TLS, a WAF.

Known limit: the proxy secret is in the database and in the proxy's route table in memory. Anyone who can read either already controls opsbox.
