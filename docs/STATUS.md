# Status

Last completed prompt: `docs/prompts/archive/16c-adopt-existing-web.md` (tag `v0.1.0-alpha.19`).

## What works (16c)
- Read-only privileged discovery: holder/image/Compose labels, nginx/Apache/Caddy/Traefik names, standard/mounted/container certificate stores (names/expiry only), local Host probes, manual names and Everything else.
- Compose `!override` preserves originals and other published ports. Confirmed plain Docker recreation snapshots writable config, warms private listeners, masks secrets in its generated command diff, retains the stopped/renamed original and restores its restart policy on Undo.
- nginx/Apache Listen rewrites, IPv6 duplicate collapse/flags, Caddy ports/loopback bind, Traefik static YAML/TOML structural validation, backups and reload. Unsupported layouts refuse.
- Separate persisted adopted routes; bounded non-consuming TLS 1.2/1.3 SNI peeks, raw TCP/HTTP forwarding, limits/timeouts and payload-free logs. Panel/app hosts win. Sealed certificate import or issuance precedes TLS termination; response regressions restore passthrough.
- Durable root cutover/Undo receipts, automatic rollback, baseline status classes, immediate bind (≤1s draining-worker handoff), shared 20s verification, live task steps and audits. Interactive-admin API, REAL map, Overview/Network dialog, adopted-domain actions, installer/status hint and API/spec docs.

## Verified
- `cargo fmt --all --check`; API generator check; warning-level sh shellcheck; modified JS and check-*-ui.mjs syntax; all three UI contract scripts; locked workspace clippy with warnings denied; locked release build.
- Adoption fixtures/state-machine tests, TLS versions/SNI/missing SNI/record and TCP fragmentation, bidirectional byte preservation, HTTP-01/Host preservation, routing precedence, command masking and HTTP authorization/task tests pass.
- All 596 workspace tests pass (`cargo test --workspace --locked`), including adoption and capacity suites.
- `opsbox-vm` (Ubuntu 24.04, Docker 29): installed release binary; real plain Docker, Compose override and nginx.service moves + Undo; old.test HTTP/TLS, new.test managed-container route fixture, unknown/missing SNI, imported certificate, safe refusal. Latest successful loop: 0 / 1 / 0 failed requests respectively at 200ms intervals. Fixtures/routes/hosts restored. See `tools/check-adoption-vm.py`.

## Partial / Found, not fixed
- Port-qualified Apache `<VirtualHost *:80>`/`:443` blocks are intentionally refused so only `Listen` directives are rewritten (`docs/DECISIONS.md`); wildcard/no-port VirtualHost layouts remain supported.
- Caddy/Traefik live cutovers and an entire service-deploy API workflow on new.test were not exercised in the VM (the real proxy app-route fixture was). Their parsing/rewriting and existing deployment paths have automated coverage.
- Earlier open items remain: live DB/R2/MinIO/SFTP/alert-channel checks, pgweb path prefix, Capacity override click-through and idle browser CPU profiling.
- Existing `design/*`, staged 16d/16e prompts, and concurrent unrelated panel edits are preserved outside this change.

## Next
- Next prompts: `docs/prompts/16d-align-panel-with-design.md`, `16e-transport-and-response-security.md`, then `17-alpha-release.md`; finish the earlier live integration checks.
- VM: `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target ~/.cargo/bin/cargo build --release --locked`; `sudo sh install.sh --dev --binary ~/target/release/opsbox`; `sudo python3 tools/check-adoption-vm.py` (requires the 03c nginx `web` fixture and Compose v2).
- UI: `ssh -L 8443:127.0.0.1:8443 ubuntu@192.168.252.2`; open Settings > Network, review Move behind opsbox, and check live steps/Undo. A wrong typed recreation confirmation must leave the existing site online.
