# Security policy

## Alpha status

Treat a compromise of the panel process (`serve`, UID `opsbox`) as host-root compromise: its helper connection permits every typed system operation, including root jobs and terminals, without independent proof of a panel administrator. Keep the panel behind a VPN, SSH tunnel (`--local-only`), or IP allowlist.

Fixes 26/27 separate the proxy into UID `opsbox-proxy` with its own helper socket. Its helper allowlist permits only port-holder queries and sealing/opening `proxy:` secrets. Proxy compromise still exposes routed traffic, the forwarding secret and proxy certificates, but does not grant the panel's full helper access.

Use trusted deployers only. Fix 19 now requires Admin confirmation for elevated Compose and checks raw and resolved configuration; earlier versions let Deployers reach host root through Compose. This alpha does not provide a hostile-tenant sandbox. Full API tokens are sensitive: fix 20 defines Full as “everything except root-equivalent actions”, denies root-equivalent and sensitive account/secret routes to tokens, and checks token Compose redeploys as Deployer. Ordinary deployments remain available, so a leaked token still grants broad application access.

Restrict Safe Mode separately. This root recovery service defaults to the panel IP on port 8444; an empty/default `[watchdog] allow_ips = []` allows any source to reach recovery authentication. Fix 21 supports exact IPv4/IPv6 addresses (no CIDRs), rejects other peers before TLS, and gives the watchdog a separate root-only TLS key. Set the allowlist and restart `opsbox-watchdog`; include loopback explicitly for SSH tunnels. `--local-only` makes its default bind loopback unless `[watchdog] listen` overrides it. Check exposure and the certificate fingerprint with `opsbox status`.

These boundaries describe current source and packaged configuration; existing hosts must install the fixes and restart services. Live Linux checks still pending are recorded in [docs/QA.md](docs/QA.md).

## Supported versions

Security fixes target the **latest alpha release only**. Older alphas are not supported; upgrade to receive fixes. Changes in this repository may be newer than the latest published release.

## Reporting a vulnerability

Please **do not** open a public GitHub issue for security problems.

Report privately via GitHub Security Advisories:

https://github.com/khangdev20/opsbox/security/advisories/new

Include what you found, how to reproduce it, and the affected version or commit if you know it. We will acknowledge the report and keep the discussion private until a fix is ready.

## Response targets

These are goals for the alpha, not SLAs:

| Step | Target |
|---|---|
| Acknowledge a private report | within **3 business days** |
| Initial severity / scope assessment | within **7 days** of acknowledgement |
| Fix or mitigation for a confirmed high/critical issue | as soon as practical; we aim for a patched alpha within **30 days** |
| Public disclosure | coordinated with you after a release that includes the fix (or after we agree the risk is acceptable) |

Lower-severity issues may take longer. If we cannot meet a target, we will say so in the advisory thread.

## Scope

In scope: the opsbox agent, panel, privileged helper, installer, proxy, watchdog, and release signing / install path as shipped from this repository.

Out of scope (report only if they create a concrete opsbox compromise): third-party apps you deploy, your cloud provider, DNS or certificate authorities, and generic SSH / OS misconfiguration on the host.

## Security properties (alpha)

The alpha includes these mitigations; they do not contain panel compromise:

- **Typed helper operations** — `serve` runs unprivileged and delegates system actions to root `priv`, which validates requests. The panel UID retains all typed operations, as described above.
- **Claim before use** — until setup, only the claim flow answers, and it needs a one-time token from the install / boot log.
- **Mandatory 2FA** — the first admin enrols TOTP; lockout and an audit log apply after claim.
- **TLS by default off-loopback** — non-loopback listeners require TLS; session cookies use `Secure` / `__Host-` where applicable.
- **Signed releases** — `install.sh` verifies checksums and minisign signatures before installing a downloaded binary.
- **No telemetry** — opsbox does not phone home.
- **Secrets stay sealed** — list/detail APIs mask secrets; reveal requires recent re-authentication.

Details of threats and controls: [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md).

## Supported platforms (alpha)

Primary test targets: **Ubuntu 22.04**, **Ubuntu 24.04**, and **Debian 12** (x86_64 and arm64, systemd). Other Linux distributions may work but are not the release bar yet.
