# Status

Last completed prompt: `docs/prompts/archive/06-watchdog-safe-mode.md` (Watchdog and safe mode)

## What works
- `crates/opsbox-watchdog` + `opsbox watchdog` (`opsbox-watchdog.service`, root, `MemoryMax=32M`, `Nice=-5`, `Before=` all opsbox services): probes `serve` `/healthz`, `priv` ping, `proxy` ping, `/` and `/var/lib/opsbox` disk space every 10s, and SQLite `PRAGMA quick_check` on start via `ReadOnlyStore`.
- Auto-recovery & rollback policy: restarts failed units; frees opsbox space first on 3 failures in 10 min with critical disk; takes over post-update verification/schema-checked rollback from `opsbox-rollback-check` when active; enters safe-mode alert state and backs off restarts to 60s.
- Standalone HTTPS Safe Mode (`:8444`, shared panel TLS cert, zero shared code with `serve` beyond `opsbox-proto` & read-only `opsbox-store`): admin password+TOTP (offloading 64 MiB Argon2id via stdin to a 128M transient systemd scope when constrained) or single-use `opsbox recovery-code`; actions for logs, free disk space, restart, conditional rollback, 1h emergency SSH, and reboot.
- Panel recovery screen & Security check: `PrivOp::WatchdogStatus`, `GET /api/v1/recovery`, Security `"watchdog"` check, `web/js/pages/recovery.js` wired cleanly without `"sample data"` badge.

## Partial / Found, not fixed
- Pre-existing `cargo::unused_dependencies` warning for `futures-util` in `crates/opsbox-priv/Cargo.toml` on non-Linux hosts (`futures-util` is only used inside `#[cfg(target_os = "linux")]`).
- Linux-only checks (`tools/check-idle-memory.py`, `musl` target build, systemd `MemoryMax=32M` + `systemd-run` Argon2 offload, live restart/rollback in VM) cannot run natively on macOS and remain for Linux CI / `opsbox-vm`.

## Next
- Verify `opsbox-watchdog.service`, Safe Mode `:8444`, `opsbox recovery-code`, and failure/rollback recovery in `opsbox-vm`.
- Next prompt: `docs/prompts/07-docker-containers.md`.
