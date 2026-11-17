# Roadmap

## Done: UI on mock data (13 + 5 screens)
Overview, Apps (graph, deploys, rollback, variables), New service (GitHub, templates, Compose), Containers, System (updates, services, logs, jobs, firewall), Backups, Alerts, Cleanup, Terminal, Files (Monaco), Security, Access (SSH keys, Linux users), Settings, Login, Setup, Safe mode. Dark and light themes.

## Decided: desktop app + optional per-instance web panel
See docs/ARCHITECTURE.md. Design still needs: server list and switcher, add server (pairing), network rules, web-panel toggle per instance.

## Done: backend phase 0, skeleton
Cargo workspace, `opsbox-proto`, SQLite store with migrations and a hash-chained audit log, the `priv` socket server with peer-credential checks, the axum server with the embedded UI, `status`, install script and systemd units, CI. At that milestone only `GET /api/v1/host` was real; see `docs/DECISIONS.md` and `docs/CONFIG.md`.

## Implemented: backend Phase 1, own the server
Auth/setup/TOTP, sessions/lockout/audit, real metrics, system updates and systemd D-Bus, journal/jobs/firewall/emergency SSH/reboot, files/Monaco, PTY/xterm, cleanup, security, access and settings/scoped tokens. Hardening includes rate/body/deadline/stream limits, doctor, generated route documentation and an idle-memory CI budget. See [PHASE1_STATUS.md](PHASE1_STATUS.md) for evidence and remaining validation.

## Next phases
2. Watchdog, safe mode, signed self-update/rollback.
3. Containers, GitHub deployments, templates and Compose.
4. Proxy, panel domains and TLS.
5. Backups and alerts.
6. Desktop, WireGuard, pairing and multi-server.

## Before trusting it with company data
Independent security review, SECURITY.md, threat model, signed releases.
