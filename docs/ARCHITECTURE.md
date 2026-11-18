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

## 3. opsbox Cloud (separate site, later): a gateway, not a host

Decided 2026-11-18. An official site that saves the user from creating a server and pasting the install command by hand. It is a convenience gateway only.

- The user picks a provider they already use (Hetzner, DigitalOcean, Vultr, Linode, AWS, ...), a region and a size. The site creates the instance **in the user's own provider account**. The provider bills the user directly; opsbox charges a small fee for the service.
- The instance boots with cloud-init that runs the normal `install.sh` unattended (signed release, same as a manual install). Nothing on the server differs from a self-installed opsbox.
- When the panel is up, the site hands the user the panel link and a one-time claim link. The user sets their own admin password and 2FA in the panel. Done: from then on the server has no tie to the site, and it keeps working if the site goes away.
- The site keeps no credentials:
  - Provider access: prefer the provider's OAuth with the narrowest scope; otherwise an API key used only for the create call, held in memory for that request and never stored or logged. The user can revoke it right after.
  - Panel access: the claim token is generated per instance, shown once, never stored (not even hashed), expires after first use or 1 hour. Because cloud-init user data stays readable in provider metadata, the token must be useless after the claim, and the panel must reject it after the first admin exists (already true for setup tokens).
  - No SSH keys or root passwords are created or kept by the site. If the user wants SSH, they add their own public key in the create form; it goes straight to the provider.
- Optional, opt-in only: a `*.opsbox.app` subdomain for the panel (DNS record only, TLS issued on the server), and a health ping so the site can show "install finished". No remote access, no relay with credentials.
- Sizing advice comes from the capacity guard tiers (prompt 15b): the form suggests a size from what the user plans to run.

### Threat model (recorded 2026-11-18, not started)

Goal: a compromise of the site must not reach servers that are already claimed. At worst it affects servers created during the compromise.

- Claimed servers never trust the site: no inbound channel, no remote command, no stored access.
- Tampered cloud-init (backdoored binary or extra SSH key): the release signing key stays offline, never on the site; the cloud-init template is public in the repo, short and fixed, and shown to the user before create; at claim time the panel runs a self-check (binary signature, unexpected SSH keys, users, services) and shows the result.
- Claim hijack: short-lived one-time token; until claimed the panel accepts connections only from the IP that pressed Create; a second claim is refused and the site reports "already claimed".
- Provider tokens: used once and never stored. Guide the user per provider: DigitalOcean supports token expiry and custom scopes (shortest expiry, droplet-create scope only; OAuth access tokens expire after 30 days); Hetzner tokens are scoped to one project with no expiry option found, so use a dedicated project and delete the token after create. Check each new provider the same way.
- If tokens are ever stored for convenience (not recommended): encrypted OAuth refresh tokens only, revoked at the provider and deleted after N days unused, with "last used" shown.
- Site sessions: idle logout after 1 to 2 days plus a maximum session age; passkeys or 2FA required.
- `*.opsbox.app` subdomain takeover: remove DNS when health pings stop; separate panel domain on the Public Suffix List; DNS API token scoped to that zone only.
- Payments through Stripe Checkout only (no card data on the site). Phishing: the panel shows its install source and claim links only point at the server itself.
- Before opening to the public: independent security review of the site and the cloud-init and claim flow, SECURITY.md, a reporting channel; bug bounty later.

What opsbox itself needs for this (also useful for self-hosters): a fully unattended install (`install.sh --claim-token-file`, no prompts), installs that succeed on busy hosts (prompt 03c), a machine-readable install result, and the panel claim flow from prompt 04.

## Consequences

- `web/js/api.js` stays the single seam: `http` mode for the web panel, a future `tauri` mode for Desktop.
- The agent must not assume Desktop exists. Anything Desktop needs later (device auth, WireGuard) is added behind a feature flag without changing the single-instance experience.
- The setup token in the boot log is the first-run path. Desktop, when it exists, will add pairing as a second path.
- Safe mode: watchdog on the instance, reachable on its own port, plus emergency SSH.
