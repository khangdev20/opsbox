# Status

Last completed prompt: `docs/prompts/archive/03f-form-state-and-helper-errors.md` (forms keep typed values; helper errors say what happened). Not committed, not tagged yet. Prior: `16e`, `03e`, `03d` (not yet released), `16f` (tag `v0.1.0-alpha.20`).

## What works (03f)
- **Forms:** a failed submit no longer rebuilds anything. Templates (`paint` only on picking another template; `paintBusy` for button/labels), New app and the memory dialog (`editMemory` now uses `run`, so the dialog stays open) keep every field, the override box and the confirmation sentence; the error shows in `role="alert"` and takes focus (`showFormError` in `ui.js`). `bindCapacity` returns `repaint` besides `refresh`.
- **Helper errors:** Connect / Timeout / Forbidden have separate messages (`error.rs`); one log line per failed call with request id, op and elapsed ms (`client.rs`). Same wording in `AuthError::HelperUnavailable`.
- **Version skew:** `/readyz` is 503 with `privileged_helper: "version mismatch"` and `checks.versions` when `priv` and `serve` differ; `opsbox status` prints `MISMATCH` and is unhealthy. `opsbox update` already restarts priv, proxy and serve together.
- **Banner:** persistent panel banner when `/readyz` says the helper is down or mismatched, with the restart command and a Copy button; clears on its own (10 s poll).
- **Unit:** `opsbox-priv.service` has `Restart=always`, `RestartSec=2`, `OOMScoreAdjust=-900` (test `opsbox-priv/tests/unit_file.rs`).
- **Tests:** `error.rs` (5 unit), `http.rs` (`/readyz` ready, mismatch, down), `status.rs` (mismatch), `unit_file.rs`; `tools/check-forms-ui.mjs` (headless Chrome: refused submit on all three forms, banner down/mismatch/clear; it fails against the old `templates.js`). Not added to CI (needs Chrome).
- Verified here: `cargo test -p opsbox-api -p opsbox-priv -p opsbox-auth`, `-p opsbox status`, clippy `-D warnings`, `cargo fmt`, `generate-api.py --check`, `check-forms-ui.mjs` and the other `tools/check-*-ui.mjs`.
- Priv calls reachable from the capacity-guarded POSTs: all but `deploy_ensure_dir`, `deploy_image_exists`, `secret_seal/open` and `capacity` already use long timeouts (`patient()` or inside `client.rs`); those four are quick reads and stay at 10 s. `deploy_prune_images` was the one that needed more: now 120 s.

## Earlier (16e, uncommitted with 03e/03d): TLS everywhere, safe cookies
- Config refuses plain HTTP off loopback; `opsbox status` prints scheme, TLS version and fingerprint; `transport.rs` decides client IP/https; `__Host-` session cookie over TLS. Details in `docs/DECISIONS.md`, `docs/THREAT_MODEL.md`, `scripts/check-transport.sh`.
- Not verified on Linux/`opsbox-vm`; fingerprint not yet in the setup-token output; sign-in once after updating (cookie rename).

## Earlier work, still pending release
- 03e: update check reasons (`ReleaseChecker`, `reason` in `/updates/self`, "Check now"). 03d: `opsbox-version` crate, release stamping, channel order and migration of old builds. Details in `docs/DECISIONS.md` and `docs/RELEASING.md`.

## Partial / Found, not fixed
- **Not verified on `opsbox-vm` or any Linux host.** The prompt's checks (https and fingerprint in `status`, `curl http://VM:8443` fails, `check-transport.sh`, a `tcpdump -i any -A port 8443` capture during login with nothing readable) are not run. Record results here.
- **Not done from the prompt:** the fingerprint is not yet in the setup-token output and the installer summary (only `status`); HSTS and a fuller CSP are 16r; secrets in responses are 16o. Port 80 on the proxy redirecting only (308) was not re-checked.
- **Session cookie rename:** everyone signs in once after the update to this build over HTTPS. Not tested on a server upgraded from the previous alpha.
- `opsbox web` writing a route before `serve` ran once creates the secret; two processes creating it at the same moment could race (the later write wins; `serve` re-reads on restart).
- `opsbox update` (CLI) does not read the panel's channel setting; `web/js/api.js` mock still says `current: '0.1.0'`.
- Earlier open items: screenshots use demo data, live DB/R2/MinIO/SFTP/alert checks, pgweb path prefix, Capacity override click-through, idle browser CPU profiling, Caddy/Traefik live cutovers, the `capacity_http` queued-workloads fixture issue (see DECISIONS).

## Found, not fixed (03f)
- Database page dialogs (extension, pooler) still close before the request runs, so a refused request loses what was typed there; convert them to `run` like `editMemory`.
- The prompt's capacity reserve check (opsbox's 128 MB covering `serve` and `priv`, measured RSS in DECISIONS) is not done.
- Streaming priv calls (`deploy_build`, `docker_logs_follow`, `pty_open`) do not use `call`, so their failures log only the mapped error, without op/elapsed.
- Existing installs get `OOMScoreAdjust` only after reinstall (`install.sh`); the update path does not rewrite unit files.
- Not run on `opsbox-vm` (see below).

## Next
- Commit, push, wait for CI, tag `v0.1.0-alpha.21`; VM verification for 03d, 03e and 16e.
- Then `16g-capacity-page.md` and the rest of the 16* series, then `17-alpha-release.md`.
- VM: `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target ~/.cargo/bin/cargo build --release --locked`; `sudo sh install.sh --dev --binary ~/target/release/opsbox`; UI via `ssh -L 8443:127.0.0.1:8443 ubuntu@192.168.252.2`.
