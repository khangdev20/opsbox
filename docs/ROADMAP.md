# Roadmap

## Done: UI on mock data (13 + 5 screens)
Overview, Apps (graph, deploys, rollback, variables), New service (GitHub, templates, Compose), Containers, System (updates, services, logs, jobs, firewall), Backups, Alerts, Cleanup, Terminal, Files (Monaco), Security, Access (SSH keys, Linux users), Settings, Login, Setup, Safe mode. Dark and light themes.

## Decided: desktop app + optional per-instance web panel
See docs/ARCHITECTURE.md. Design still needs: server list and switcher, add server (pairing), network rules, web-panel toggle per instance.

## Done: backend phase 0, skeleton
Cargo workspace, `opsbox-proto`, SQLite store with migrations and a hash-chained audit log, the `priv` socket server with peer-credential checks, the axum server with the embedded UI, `status`, install script and systemd units, CI. Only `GET /api/v1/host` is real; see `docs/DECISIONS.md` and `docs/CONFIG.md`.

## Next: backend phase 1 (not started)
Agent binary (also serves `web/` when the web panel is on); auth with TOTP, sessions, lockout; metrics; PTY terminal over WebSocket; files; Docker; system (updates, systemd, journal, cron, firewall); watchdog and safe mode; install script.

## Later
Dockerfile deploys and proxy with TLS; backups and alerts; templates and Compose; Nixpacks, previews; multi-server.

## Before trusting it with company data
Independent security review, SECURITY.md, threat model, signed releases.
