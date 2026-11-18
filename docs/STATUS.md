# Status

Last completed prompt: `docs/prompts/archive/03c-proxy-port-conflict.md` (tag `v0.1.0-alpha.16`).

## What works
- Busy/inaccessible proxy ports fall back to 8080/8445, then 8080–8099 / 8445–8464 on the same bind IP. Control status stays running with `port_conflict`, actual ports and best-effort PID/process/unit/container holders.
- Primary ports retry every 30 seconds and promote without restarting or interrupting accepted requests. Redirects use actual serving ports. Other bind/configuration errors still fail.
- Typed priv ops identify holders through TCP/TCP6 inodes and process FDs, recheck Docker publications/cgroups, take over confirmed holders, keep current ports, and Undo. Docker restart policy/systemd enablement survives in root-only receipts; API changes are audited. No containers or volumes are removed.
- Watchdog conflicts are warnings: no failure count, restart, safe mode or update rollback. The persistent alerts engine raises a warning. Installer/update/status accept conflicts and point real failures at the affected units.
- Overview banner and Settings > Network show holders, confirmed takeover, persistent Undo and “Keep it and use other ports”; REAL/PAGE_USES and generated API docs are wired.
- Previous database, deploy, backup and alert functionality remains in place. No new external crates/licenses; priv reuses the locked pure-Rust TOML parser.

## Verified
- `cargo fmt --all --check`; `python3 tools/generate-api.py --check`; `shellcheck --shell=sh --severity=warning install.sh`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`; `cargo test --workspace --locked` (563 passed); `cargo build --release --locked`.
- Real-socket test: busy primary + preferred fallback, HTTP forwarding, holder reporting, both listeners promoting, and an in-flight request completing after fallback closes.
- API + real priv + scripted Docker/systemd: confirmation/RBAC/ownership refusals, takeover/Undo, saved ports and audit chain. Procfs v4/v6 fixtures and watchdog post-update/crash policy tests pass. `node tools/check-network-ui.mjs` checks API wiring/CSRF, escaped names and action visibility.
- `opsbox-vm` (Ubuntu 24.04 arm64): release build and full installer with dummy :80 listener passed; warning + exit 0, proxy promotion and watchdog `failed=false`.
- VM with real nginx container `web` on 80/443: installer named the container and exited 0; typed takeover set restart=no and promoted within 30s with unchanged proxy PID. Undo restored running + original `always` policy and fallback ports. Unconfirmed takeover was refused without stopping it.
- VM Keep ports saved/applied 8080/8445 live and ended retries; test config restored afterward. Final VM: web running, proxy on fallback, watchdog warning with only the conflict event and no failures. Temporary build swap removed.

## Partial / Found, not fixed
- VM host actions were checked through typed priv frames; HTTP auth/audit and UI rendering/wiring were checked automatically. Browser click-through remains a manual verification step.
- Database capacity checks await prompt 15b (`capacity::admit` does not exist yet). Earlier real DB Docker provisioning/restore/pooler/major-upgrade, PgBouncer image, PostGIS arm64 and pgweb path-prefix checks remain pending; prior PostgreSQL SQL isolation checks passed.
- Major DB upgrades still need a configured restic destination and have no dump disk-space pre-check. pgvector/PostGIS restores still infer plain postgres; extensions do not automatically carry into a new app database.
- Earlier live R2/MinIO/SFTP/alert-channel checks remain pending. cargo-deny is not installed on this Mac; CI retains license/advisory and musl builds (existing pinned cargo-deny/CVSS-4 and idle-memory HTTP probe notes still apply).
- Unrelated staged architecture/roadmap/15b prompt/README work and concurrent design edits are preserved outside this commit.

## Next
- Prompt `15b` capacity guard, then `17` alpha release; finish the earlier live integration checks.
- VM recheck: `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target cargo build --release`; `sudo sh install.sh --dev --binary ~/target/release/opsbox`; `sudo opsbox status`.
- Open the panel with `ssh -L 8443:127.0.0.1:8443 ubuntu@192.168.252.2`, then https://localhost:8443. Network should name web; cancel takeover first (it must keep running), then confirm takeover and Undo.
