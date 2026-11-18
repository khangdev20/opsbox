# Security policy

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

opsbox is meant to be safe by default for people who do not specialize in security. In this alpha you can expect:

- **Privilege split** — the network-facing `serve` process never changes the system; every system action is a typed `PrivOp` re-validated in the root `priv` helper.
- **Claim before use** — until setup, only the claim flow answers, and it needs a one-time token from the install / boot log.
- **Mandatory 2FA** — the first admin enrols TOTP; lockout and an audit log apply after claim.
- **TLS by default off-loopback** — non-loopback listeners require TLS; session cookies use `Secure` / `__Host-` where applicable.
- **Signed releases** — `install.sh` verifies checksums and minisign signatures before installing a downloaded binary.
- **No telemetry** — opsbox does not phone home.
- **Secrets stay sealed** — list/detail APIs mask secrets; reveal requires recent re-authentication.

Details of threats and controls: [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md).

## Supported platforms (alpha)

Primary test targets: **Ubuntu 22.04**, **Ubuntu 24.04**, and **Debian 12** (x86_64 and arm64, systemd). Other Linux distributions may work but are not the release bar yet.

## Prefer a VPN or allowlist while alpha

The panel has root-equivalent power through `priv`. Prefer binding it behind a VPN, SSH tunnel (`--local-only`), or an IP allowlist until you are comfortable with the alpha surface.
