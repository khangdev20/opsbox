# 05. Proxy and panel domain (Phase 4, panel first)

Spec: 7.7, 7.18; DECISIONS on proxy. Crates: `opsbox-proxy`, `opsbox-api`.

Goal: the user types `panel.example.com` in Settings and gets the panel on HTTPS with a real certificate, without touching DNS tools beyond adding one record.

Do:
- Decide in-process Rust proxy vs managed Caddy (spec open question 1). Prefer the simplest that stays one binary; write the decision and reasons in DECISIONS.md.
- `opsbox proxy` unit on :80/:443 (`CAP_NET_BIND_SERVICE`), routing table pushed by `serve`. For now one route: panel domain to `serve`.
- ACME HTTP-01 with Let's Encrypt (staging in tests), renew 30 days before expiry, keys encrypted at rest.
- DNS check (hickory-resolver) comparing A/AAAA with the server's public IPs; the UI shows "points here" / "waiting for DNS" and the exact record to add.
- Settings: "Panel address" field with Check DNS, then Issue certificate. When live, offer to close the raw :8443 listener.
- `opsbox web domain <name>` CLI.

Out of scope: app domains (prompt 10).
