# 04. Public first run (Phase 1c)

Spec: 6.3, 7.18, 11. Read `docs/ARCHITECTURE.md` "Reaching the panel without SSH".

Goal: a small user installs opsbox and opens the panel in a browser without SSH.

Do:
- Install generates a self-signed certificate (rcgen) for the server's public IPs and hostname and starts `serve` on `0.0.0.0:8443` (TLS) by default.
- Before an admin exists, only `/setup/*`, `/healthz` and static assets answer; everything else returns 403. The setup token is single-use, expires in 1 h, and is printed to `/dev/console` (through priv) and journald together with the setup URL `https://<public-ip>:8443/#/setup`.
- `--local-only` at install, and `opsbox web local-only on|off`, keep every listener on 127.0.0.1 (today's behaviour).
- Detect public IP from cloud metadata (AWS IMDSv2, GCP, Azure, DigitalOcean, Hetzner) with a short timeout, falling back to interface addresses.
- Session cookie gets `Secure` now that TLS is always on. Security checks: warn while the panel uses the self-signed certificate or has no domain.
- `install.sh` prints the next step clearly, including which inbound port to open in the cloud firewall (8443/tcp).

Verify in the VM: browse `https://<vm-ip>:8443` from the Mac, accept the certificate warning, claim with the token, enable 2FA. Refusal check: any API route before claim returns 403.
