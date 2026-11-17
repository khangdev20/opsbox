# Status

Last completed prompt: `docs/prompts/archive/08-github-connection.md` (GitHub connection)

## What works
- `crates/opsbox-deploy` + `crates/opsbox-store` (`0005_github_sources.sql`) + `opsbox-priv` (`ops/sealed.rs`): pure-Rust GitHub App manifest handshake, PKCS#1/PKCS#8 RS256 App JWT signing (`ring`), installation repository listing with search & remote build-method detection (`Dockerfile`, `Docker Compose`, `Nixpacks · <runtime>`, `static site`, `nothing to build`), constant-time HMAC-SHA256 webhook verification (`hmac`/`sha2`), public HTTPS repo URL validation/inspection (blocking private/loopback/local hosts), and per-repo Ed25519 SSH deploy keypair generation.
- Privilege split: GitHub App private keys, webhook secrets, and SSH deploy private keys are sealed/opened solely via `opsbox-priv` (`PrivOp::SecretSeal` / `SecretOpen` under `deploy:*` contexts, keeping `totp:*` refused) and stored as `enc:v1:...` in SQLite.
- `crates/opsbox-api` (`github.rs`, `tokens.rs`, `panel.rs`, `session.rs`): `/api/v1/github` (`GET`/`DELETE`), `/api/v1/github/{manifest,callback,setup,complete,installation,webhook,webhook/sync,repos,public-repo,deploy-keys,deploy-keys/{id}}`, `GET /api/v1/settings` `github` & `deployKeys`, automatic webhook URL sync when a panel domain cert goes live, and `/api/v1/github/webhook` reachable through `opsbox-proxy` (exempt from panel IP allow-list, gated by HMAC-SHA256).
- `web/js` (`api.js`, `pages/settings.js`, `pages/newservice.js`, `pages/configure.js`): Settings GitHub section wired live (connect via manifest flow, choose/manage access link, webhook status & panel domain requirement notice, disconnect, deploy keys list) with its sample-data badge removed; New Service wired live (`getRepos` search & detected build method, Deploy key tab, Public repo URL tab) with no sample-data badge.

## Partial / Found, not fixed
- Linux-only runtime/CI checks (`tools/check-idle-memory.py`, `musl` static build, `cargo-deny`, live GitHub App manifest round-trip against `github.com` over a public panel domain) cannot run natively on macOS and remain for Linux CI / `opsbox-vm`.

## Next
- Verify GitHub App manifest flow, repo picker, webhook delivery through the panel domain, public repo URL inspection, and deploy key generation in `opsbox-vm`.
- Next prompt: `docs/prompts/09-deploy-pipeline.md`.
