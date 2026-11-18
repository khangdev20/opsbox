# Status

Last completed prompt: `docs/prompts/archive/16e-transport-security.md` (TLS everywhere, safe cookies). Not committed, not tagged yet. Prior: `03e`, `03d` (committed, not yet released), `16f` (tag `v0.1.0-alpha.20`).

## What works (16e)
- **Config refuses plain HTTP off loopback.** New `[api] tls` (default `true`). `tls = false` with any non-loopback `listen` (file, `--listen`, env) is an error at load for every command; no override in any build. `serve` uses plain `opsbox_api::serve` only for `tls = false` on loopback. `secure_cookies = "never"` is gone (parse error); `auto` and `always` remain.
- **`opsbox status`** handshakes with the listener and prints `https://addr`, the TLS version and the certificate SHA-256 (`AA:BB:..`, as browsers show it), or `http://127.0.0.1:..` for a plain loopback listener. Plain HTTP answering on a public address shows as degraded.
- **TLS settings** explicit: TLS 1.3 then 1.2 only, rustls default suites, ALPN `h2` and `http/1.1` (`tls::server_config`, tested with real handshakes).
- **Who is calling** (`opsbox-api/src/transport.rs`, outermost middleware): `X-Forwarded-*`, `Forwarded`, `X-Real-IP` are believed only from a loopback peer that sends `X-Opsbox-Proxy-Secret`; otherwise deleted before handlers. One `Transport` value (client IP, https) feeds rate limits, the allowed-address list, audit and cookies. The proxy adds the secret on the panel route only and strips any client copy everywhere. Secret: settings key `opsbox.proxy.forward_secret`, created by `serve`/`opsbox web`.
- **Cookies:** `Secure` whenever the request arrived over TLS (own listener, trusted proxy, or `always`). Over HTTPS the session cookie is `__Host-opsbox_session` (`Path=/`, no Domain, HttpOnly, SameSite=Strict); the plain name is ignored there. `tools.rs` strips both names from tool cookies. Sign-out deletes the session server side (tested; copied cookie fails).
- **Tests:** `transport.rs` (9 unit), `session.rs` cookie attributes, `tls.rs` (versions, ALPN, probe), `config.rs` (rejection matrix), `status.rs` (https+fingerprint, plain, public-plain), `auth_http.rs` (forged headers, trusted proxy, TLS listener), `transport_http.rs` (real sockets, forged headers and audit peer), `security_http.rs`, `github_http.rs` and `e2e_proxy.rs` (real proxy to real serve with the secret).
- **`scripts/check-transport.sh <url>`**: scheme, TLS version, fingerprint, legacy-protocol refusal, plain-HTTP refusal, security headers, and cookie attributes of a login (`OPSBOX_USER/PASSWORD/CODE`). Run here against a real local `opsbox serve` (TLS and plain loopback); the refused config was checked with the binary.
- Docs: `docs/CONFIG.md`, `docs/API.md`, `docs/VERIFY.md`, new `docs/THREAT_MODEL.md`, `docs/DECISIONS.md`.
- Verified here: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test -p opsbox-api -p opsbox -p opsbox-proxy` (all pass except the known `capacity_http` fixture failure), `shellcheck scripts/check-transport.sh`, `tools/generate-api.py --check`.

## Earlier work, still pending release
- 03e: update check reasons (`ReleaseChecker`, `reason` in `/updates/self`, "Check now"). 03d: `opsbox-version` crate, release stamping, channel order and migration of old builds. Details in `docs/DECISIONS.md` and `docs/RELEASING.md`.

## Partial / Found, not fixed
- **Not verified on `opsbox-vm` or any Linux host.** The prompt's checks (https and fingerprint in `status`, `curl http://VM:8443` fails, `check-transport.sh`, a `tcpdump -i any -A port 8443` capture during login with nothing readable) are not run. Record results here.
- **Not done from the prompt:** the fingerprint is not yet in the setup-token output and the installer summary (only `status`); HSTS and a fuller CSP are 16r; secrets in responses are 16o. Port 80 on the proxy redirecting only (308) was not re-checked.
- **Session cookie rename:** everyone signs in once after the update to this build over HTTPS. Not tested on a server upgraded from the previous alpha.
- `opsbox web` writing a route before `serve` ran once creates the secret; two processes creating it at the same moment could race (the later write wins; `serve` re-reads on restart).
- `opsbox update` (CLI) does not read the panel's channel setting; `web/js/api.js` mock still says `current: '0.1.0'`.
- Earlier open items: screenshots use demo data, live DB/R2/MinIO/SFTP/alert checks, pgweb path prefix, Capacity override click-through, idle browser CPU profiling, Caddy/Traefik live cutovers, the `capacity_http` queued-workloads fixture issue (see DECISIONS).

## Next
- Commit, push, wait for CI, tag `v0.1.0-alpha.21`; VM verification for 03d, 03e and 16e.
- Then `16g-capacity-page.md` and the rest of the 16* series, then `17-alpha-release.md`.
- VM: `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target ~/.cargo/bin/cargo build --release --locked`; `sudo sh install.sh --dev --binary ~/target/release/opsbox`; UI via `ssh -L 8443:127.0.0.1:8443 ubuntu@192.168.252.2`.
