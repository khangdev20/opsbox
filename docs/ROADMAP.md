# Roadmap

## Done: UI on mock data (13 + 5 screens)
Overview, Apps (graph, deploys, rollback, variables), New service (GitHub, templates, Compose), Containers, System (updates, services, logs, jobs, firewall), Backups, Alerts, Cleanup, Terminal, Files (Monaco), Security, Access (SSH keys, Linux users), Settings, Login, Setup, Safe mode. Dark and light themes.

## Next: backend phase 1 (not started)
Single binary embedding `web/`; auth with TOTP, sessions, lockout; metrics; PTY terminal over WebSocket; files; Docker; system (updates, systemd, journal, cron, firewall); watchdog and safe mode; install script.

## Later
Dockerfile deploys and proxy with TLS; backups and alerts; templates and Compose; Nixpacks, previews; multi-server.

## Before trusting it with company data
Independent security review, SECURITY.md, threat model, signed releases.
