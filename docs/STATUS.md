# Status

Last completed prompt: `docs/prompts/archive/16i-export-config.md` (export configuration). Not committed, not tagged (per session request). Prior: `16h` (grow the filesystem), `16g`, `16f` (tag `v0.1.0-alpha.20`), plus unreleased `03f` / `16e` / `03e` / `03d` / `16g` / `16h` work still on `dev`.

## What works (16i)
- **`POST /api/v1/config/export {password, passphrase?}`** (interactive admin, no tokens, account password again): one JSON document, `"format": "opsbox-config/1"`, schema in `docs/schemas/config-export-v1.json`. Services (source, build, ports, domains, limits, health path, exposure, owner, variables), template instances, compose projects, databases (engine, version, volume, pooler, linked apps; no data), backup destinations (metadata) and jobs, alert rules / quiet hours / channel ids, panel domain, local-only and allowed addresses, update settings, firewall rules, users with roles. Code: `crates/opsbox-api/src/export.rs`.
- **Secrets**: out by default and listed under `secrets.needsReentry`. With a passphrase (8+ characters) the secret variables, backup destination credentials and alert channel settings are sealed in `secrets.ciphertext` (Argon2id 32 MiB x3, XChaCha20-Poly1305, parameters in the file). `export::decrypt_secrets` opens it (wrong passphrase, changed or damaged file fail cleanly) for the import in 16q. Never exported: password hashes, two-factor secrets, sessions, tokens, GitHub App, deploy-key private keys, certificates, scheduled jobs.
- **Audit** `config.export` (ok with `{file, includeSecrets, encrypted, services, secretsExported}`; denied on a wrong password; never the passphrase). File name `opsbox-config-<host>-<YYYY-MM-DD>.json` (`Content-Disposition`).
- **Settings → Export and uninstall**: the Export button opens a dialog (password, optional passphrase typed twice) and saves the file; `exportConfig` is in `REAL`; the "sample data" badge now sits only on Uninstall (16j).
- Tests: `crates/opsbox-api/tests/export_http.rs` (13): schema validation (small in-test validator), no secret anywhere in the bytes without a passphrase, sealed round trip, wrong passphrase / tampered / damaged / oversized-KDF files, audit rows both ways and for a denied attempt, roles, file name, every `ServiceRecord` field exported or excluded (reads the struct), exported fields present in the output. `cargo test -p opsbox-api`, `cargo fmt --check` and clippy (`-D warnings`) on `opsbox-api` and `opsbox-store` pass. Store gained `list_all_compose_stacks`. Decisions in `docs/DECISIONS.md` (16i).

## Partial / Found, not fixed
- **Not verified on `opsbox-vm`** and not at 390 px (the dialog is the shared form dialog). Not run: `cargo test --workspace`, `cargo deny` (no tool here).
- It is a `POST`, not the `GET` the prompt names (password and passphrase must not be in a URL); see DECISIONS.
- Database sizes and extensions are not in the file (read live, not stored). The firewall is `null` plus `unavailable` when `priv` cannot read it.
- Import is 16q; the file format is ready for it. Uninstall is 16j.
- `docs/API.md` documents validation as `422` in places but the server answers `400`; the 16i text says `400`.
- Carried over: 16h not run on a real disk; 16g capacity bell wiring, PSI sparkline, screenshots; 03f / 16e / 03e / 03d pending commit/release, TLS VM checks, database dialogs close before the request, streaming priv call logging.

## Next
- Commit 16h and 16i (with the pending stack if intended), push `dev`, wait for CI, tag when appropriate.
- Then `16j-uninstall.md`.
- VM check for 16i: build and reinstall as usual; Settings → Export and uninstall → Export configuration; enter the password and leave the passphrase empty. Open the file: `"secrets":{"included":false,...}`, `grep -c` for a known secret returns 0, `needsReentry` names them. Export again with a passphrase: the file has `ciphertext` and still no plain secret. The audit log (Security) shows two `config.export` rows with the host-and-date file name. Must fail safely: a wrong password returns 403, saves no file and shows a denied audit row.
