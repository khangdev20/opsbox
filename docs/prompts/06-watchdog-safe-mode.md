# 06. Watchdog and safe mode (Phase 2)

Spec: 7.16, 9 (`recovery-code`, `user` commands). Crate: `opsbox-watchdog`.

Do:
- `opsbox-watchdog.service` (root, MemoryMax 32M): every 10 s check serve `/healthz` over its socket, priv ping, proxy health, free disk on `/` and `/var/lib/opsbox`.
- Policy: restart a failed unit; after 3 failures in 10 min, free opsbox's own space if disk is the cause; if the failures began after an update, roll back to the previous version slot (prompt 03) and take over post-update health checks from the transient unit used there.
- Actions in safe mode also include "Roll back opsbox" when a previous slot exists.
- Safe mode: a tiny HTTPS server inside the watchdog on :8444 (same TLS cert as the panel), its own minimal HTML. Sign-in with an existing account (password + TOTP, read-only DB access) or a one-time code from `opsbox recovery-code`. Actions: status and last logs, free disk space, restart opsbox, open SSH for 1 hour, reboot.
- Wire the existing Recovery screen (`web/js/pages/recovery.js`) to the real data where the watchdog serves it.
- No shared code with serve beyond `opsbox-proto` and read-only `opsbox-store`.

Verify: stop and corrupt-start serve (e.g. rename the binary), watch the watchdog restart or report it; open safe mode on :8444.
