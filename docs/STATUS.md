# Status

Last completed prompt: `16b` (`v0.1.0-alpha.18`). Prompt `16c` is implemented with two unresolved scope decisions; it is not archived or tagged complete.

## What works (16c)
- Read-only privileged discovery: holder/image/Compose labels, nginx/Apache/Caddy/Traefik names, standard/mounted/container certificate stores (names/expiry only), local Host probes, manual names and Everything else.
- Compose `!override` preserves originals and other published ports. Confirmed plain Docker recreation snapshots writable config, warms private listeners, masks secrets in its generated command diff, retains the stopped/renamed original and restores its restart policy on Undo.
- nginx/Apache Listen rewrites, IPv6 duplicate collapse/flags, Caddy ports/loopback bind, Traefik static YAML/TOML structural validation, backups and reload. Unsupported layouts refuse.
- Separate persisted adopted routes; bounded non-consuming TLS 1.2/1.3 SNI peeks, raw TCP/HTTP forwarding, limits/timeouts and payload-free logs. Panel/app hosts win. Sealed certificate import or issuance precedes TLS termination; response regressions restore passthrough.
- Durable root cutover/Undo receipts, automatic rollback, baseline status classes, immediate bind (≤1s draining-worker handoff), shared 20s verification, live task steps and audits. Interactive-admin API, REAL map, Overview/Network dialog, adopted-domain actions, installer/status hint and API/spec docs.

## Verified
- `cargo fmt --all --check`; API generator check; warning-level sh shellcheck; modified JS and check-*-ui.mjs syntax; all three UI contract scripts; locked workspace clippy with warnings denied; locked release build.
- Adoption fixtures/state-machine tests, TLS versions/SNI/missing SNI/record and TCP fragmentation, bidirectional byte preservation, HTTP-01/Host preservation, routing precedence, command masking and HTTP authorization/task tests pass.
- All workspace tests pass when the pre-existing Capacity failure below is excluded. The required unfiltered command was run and fails at that same test; reproduced on unchanged `450e0bf` in an isolated worktree.
- `opsbox-vm` (Ubuntu 24.04, Docker 29): installed release binary; real plain Docker, Compose override and nginx.service moves + Undo; old.test HTTP/TLS, new.test managed-container route fixture, unknown/missing SNI, imported certificate, safe refusal. Latest successful loop: 0 / 1 / 0 failed requests respectively at 200ms intervals. Fixtures/routes/hosts restored. See `tools/check-adoption-vm.py`.

## Partial / Found, not fixed
- Awaiting Apache choice: port-qualified `<VirtualHost *:80>`/`:443` blocks cannot follow a Listen-only move. Current code refuses those layouts to obey the instruction not to change other directives; wildcard/no-port VirtualHost layouts remain supported.
- Awaiting permission for an unrelated test-fixture fix: `capacity_http::queued_workloads_block_database_resize_pooler_and_memory_adding_extensions` expects structured pooler admission but its database is stopped, producing “Start this database first.” Conventions say to record unrelated problems rather than fix them.
- Caddy/Traefik live cutovers and an entire service-deploy API workflow on new.test were not exercised in the VM (the real proxy app-route fixture was). Their parsing/rewriting and existing deployment paths have automated coverage.
- Earlier open items remain: live DB/R2/MinIO/SFTP/alert-channel checks, pgweb path prefix, Capacity override click-through and idle browser CPU profiling.
- Existing `design/*`, staged 16d/16e prompts, and concurrent unrelated panel edits are preserved outside this change.

## Next
- Resolve the two decisions above, rerun the unfiltered suite, then finish 16c archive/README checkmark and `v0.1.0-alpha.19`.
- VM: `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target ~/.cargo/bin/cargo build --release --locked`; `sudo sh install.sh --dev --binary ~/target/release/opsbox`; `sudo python3 tools/check-adoption-vm.py` (requires the 03c nginx `web` fixture and Compose v2).
- UI: `ssh -L 8443:127.0.0.1:8443 ubuntu@192.168.252.2`; open Settings > Network, review Move behind opsbox, and check live steps/Undo. A wrong typed recreation confirmation must leave the existing site online.
