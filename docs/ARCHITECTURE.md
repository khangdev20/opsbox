# Architecture (decided 2026-11-17)

opsbox ships as two front doors over one agent.

## 1. Agent (on every instance)
Single Rust binary, systemd service. Owns everything that touches the machine:
Docker, files, PTY terminal, systemd, journal, cron, firewall, updates, backups, metrics.
Watchdog restarts it and rolls back a failed update.

## 2. Desktop app (main way in, many instances)
Tauri app (macOS, Windows, Linux) reusing `web/` as its UI.
- Lists all instances, switch between them.
- Talks to agents only over a private WireGuard network it sets up; agents never listen on the public internet.
- Add server = pairing code + one install command (paste into cloud-init user data or run once).
- Network rules screen, like an AWS security group: which device or server may reach which instance and port.

## 3. Web panel (optional, one instance only)
Off by default. When turned on for an instance it serves the same UI from that instance's agent and manages only that instance (no server switcher). Bound to localhost or the private network unless explicitly exposed.

## Consequences
- `web/js/api.js` stays the single seam: one implementation calls the agent over HTTP (web panel), another over Tauri commands (desktop).
- First-run claim token in the boot log stays for the web panel; the desktop app uses pairing instead.
- Safe mode: watchdog on the instance plus emergency SSH, reachable from the desktop app.
