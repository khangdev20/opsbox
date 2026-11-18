# Status

Last completed prompt: `docs/prompts/archive/15-alerts.md` (Alerts, Spec 7.13)

## What works
- `opsbox-alerts`: persistent per-rule/target ok → pending → firing → resolved state, duration thresholds, warning-to-critical escalation, dedupe, four-hour repeats, per-channel retries/receipts, per-incident timed silences and UTC quiet hours (critical bypasses quiet hours). History/cleared-target retention is bounded; unknown sources do not falsely resolve incidents.
- Thirteen enabled defaults cover disk, CPU, memory, failed units, stopped/restarting containers, failed deploys, failed/checksum/restore-test backups, missed schedules, expiring/failed certificates, waiting security updates, repeated failed panel sign-ins, emergency SSH and unhealthy metrics/proxy. Evaluates every 15 seconds using existing priv-backed sources and SQLite events.
- SMTP (`lettre`, verified rustls TLS/STARTTLS), Slack, Telegram and HMAC-signed HTTPS webhooks. All channel configuration is sealed/opened via `PrivOp::SecretSeal/SecretOpen` under `alerts:channel:<id>`; no alert code reads host files or `master.key`. Redirects refused, timeouts bounded, provider errors redacted; channel tests/disconnects require an interactive admin.
- Real Alerts UI forms, delivery tests/status, rule switches, incident silences and quiet hours; Overview links to the matching incident with its fix link. All Alerts API functions are in `REAL`, removing sample-data badges. API payloads and webhook verification are documented in `docs/API.md`.
- Required fmt, API generator check, locked workspace clippy/tests and release build pass. Additional UI contract, credential/RBAC/CSRF/context-isolation and real TLS/HMAC receiver tests pass; licenses pass with cargo-deny 0.21.0 (including new 0BSD allowance), and native-tls/OpenSSL are explicitly banned.
- Ubuntu arm64 native release and full static `aarch64-unknown-linux-musl` release builds pass. Disposable 64 MiB ext4 disk reached 89%, triggered firing, then resolved after clearing; anonymous alerts access refused. Existing service user cannot read `/etc/opsbox/master.key`. TLS-aware isolated idle probe: 13.21 MiB RSS (<40 MiB).

## Partial / Found, not fixed
- Live SMTP/Slack/Telegram/provider receipt needs administrator channel credentials; local trusted-TLS webhook receipt/signature, HTTP failure and redirect refusal are automated. x86_64 musl remains for CI.
- Existing CI tooling: `tools/check-idle-memory.py` probes HTTP although serve always uses HTTPS; an isolated temporary TLS-aware copy measured memory. Pinned cargo-deny 0.18.2 cannot parse current CVSS 4 advisories, and bans rejects existing unversioned internal path dependencies. Kept outside this prompt's scope; current-tool license checks pass.
- Earlier real R2/MinIO/SFTP and Docker DB restore verification remains pending from prompt 14.

## Next
- Reinstall in `opsbox-vm`, configure one channel, send a test, then confirm real firing/recovery delivery. `tools/check-alerts-vm.py --binary <binary>` safely tests a disposable disk without filling `/`.
- Next implementation prompt: `docs/prompts/16-databases-manage.md`; separately staged prompt 03c remains independent.
