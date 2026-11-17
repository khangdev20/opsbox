# Architecture

Decided 2026-11-17, revised the same day: the web panel is the product; the desktop app is a separate, later upgrade.

## Who it is for

Small users: solo developers and small teams who want a Railway/Render-like experience on one server they rent, with a fixed monthly bill instead of usage-based pricing, and no configuration to learn. Every decision favours plug-and-play over flexibility: good defaults, one screen per task, plain language.

## 1. opsbox (the product): agent + web panel, one instance

- Installed on a server with one command. The same binary is the agent and serves the web UI.
- **Manages only the instance it is installed on.** No server switcher, no cross-server features.
- The web panel is **on by default** and is the main way in, from any browser.
- Everything that touches the machine (Docker, files, PTY terminal, systemd, journal, timers, firewall, updates, backups, metrics) lives in the agent, split into an unprivileged `serve` and a root `priv` helper (see BACKEND_SPEC.md section 2).
- A watchdog restarts it, rolls back a bad update and serves safe mode.

### Reaching the panel without SSH (first run)

1. Install prints a setup URL and a single-use setup token to the console and the provider's boot log.
2. Until it is claimed, the panel listens on the server's public address on port 8443 with a self-signed certificate. Nothing works without the setup token.
3. The claim flow creates the admin account with mandatory 2FA, then offers one step to attach a domain (`panel.example.com`) with an automatic Let's Encrypt certificate and to restrict access by IP if wanted.
4. Hardening defaults after claim: lockout, audit log, session limits, security checks that nag until a domain with HTTPS is set.
5. Users who prefer it can choose localhost-only at install (`--local-only`) and use an SSH tunnel; this is the current Phase 0/1 behaviour and stays available.

## 2. opsbox Desktop (separate product, later): many instances

An upgrade for larger systems, built after opsbox is solid. Not part of the opsbox MVP.

- Tauri app (macOS, Windows, Linux) reusing `web/` as its UI, with a server switcher.
- Manages many instances, each running the same opsbox agent.
- Runs one app or a set of microservices across several instances: replicas, load balancing, adding machines to scale.
- Private network between devices and servers (WireGuard), pairing, and network rules similar to a cloud security group.
- Cross-server scheduling and load balancing should be built on an existing orchestrator (for example Docker Swarm or Nomad) rather than written from scratch. It gets its own spec and roadmap.

## Consequences

- `web/js/api.js` stays the single seam: `http` mode for the web panel, a future `tauri` mode for Desktop.
- The agent must not assume Desktop exists. Anything Desktop needs later (device auth, WireGuard) is added behind a feature flag without changing the single-instance experience.
- The setup token in the boot log is the first-run path. Desktop, when it exists, will add pairing as a second path.
- Safe mode: watchdog on the instance, reachable on its own port, plus emergency SSH.
