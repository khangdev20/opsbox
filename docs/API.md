# opsbox HTTP API — Phase 1

The agent serves JSON at `/api/v1` and embeds the UI at `/`. Use `?mode=http` for the real agent; `?mode=mock` keeps the demonstration. Times are Unix seconds unless a field is explicitly formatted for display; journal timestamps are milliseconds. IDs are ULIDs except system jobs and firewall rules.

The inventory below is generated from the actual axum router. Run `python3 tools/generate-api.py` after a route change; CI checks it. Links lead to each handler's request structures and response shaping. Shared system/privilege data types live in [opsbox-proto](../crates/opsbox-proto/src/lib.rs). A full OpenAPI schema is deferred: many UI response types are still `serde_json::Value`, so adding `utoipa` now would describe incomplete schemas.

## Authentication and roles

- `GET /session` is public and returns `authenticated`, `enrollRequired`, `setupRequired`, and the signed-in user's name/role when present.
- Login: `POST /auth/login {username,password,code}`. Password + six-digit TOTP; an account with reset 2FA gets an enrolment-only session. `/auth/totp/enroll` returns `{secret,otpauthUri,account}`; `/auth/totp/confirm {code}` upgrades it.
- First run: `/setup/claim {token}`, `/setup/admin {username,password}`, `/setup/totp/confirm {code}`. Each step uses the setup cookie; all return 404 permanently after the first admin finishes enrolment.
- The session cookie is `HttpOnly; SameSite=Strict; Path=/`, `Secure` according to configuration/HTTPS forwarding. Mutations using cookies require same-host `Origin` and `X-Opsbox-CSRF` equal to the script-readable `opsbox_csrf` cookie. Every browser WebSocket requires a same-host `Origin`.
- Scripts may send `Authorization: Bearer ops_…` without cookies/CSRF. Scopes: `read` permits host, metrics, attention; `deploy` adds journal reads and has no mutation capability in Phase 1; `full` permits administrator operations except account, token, SSH-key and terminal management. Cookie authentication takes precedence. A token cannot be used as a cookie. Tokens are hashed at rest, returned once, explicitly revoked and have no expiry yet. Per-service deployment scopes wait for Phase 3.
- System/metrics reads require viewer; system mutations require admin. Files require deployer, confined to the apps directory in `priv`; viewers have no file access. Cleanup, Access, Settings, token management and Terminal require admin. Security checks are readable by signed-in accounts; fixes, invitations and account removal require admin.

## Payloads and streams

- Updates: `GET /system/updates` returns `list` entries `{pkg,cur,next,sec,kernel}` plus totals, reboot/auto-update/kernel/index metadata. Install `{which:"security"|"all"}` or `{which:"selected",packages:["curl"]}`; only currently pending, validated packages can be selected.
- System mutations: hostname `{name}`, timezone `{zone}`, swap `{sizeMb}` (0 disables), automatic updates `{on}`, reboot `{delayMin}` (1–60, default 1). Unit action is one of `start`, `stop`, `restart`, `enable`, `disable`; the refusal list applies in `priv` and systemd calls use D-Bus via zbus.
- Journal query: `unit`, `prio` (0–7), `q`, `limit`. Stream `/stream/journal` accepts the same filters and sends `{type:"journal",entries:[{t,proc,prio,msg,cursor}]}`. Entries are displayed oldest first; the helper supports cursor-based tailing.
- Jobs: create `{name,cron,command}` (five-field cron, one-line shell command); job IDs are eight hex digits. Run/remove operate on those IDs.
- Firewall rule: `{port,proto:"tcp"|"udp",src:"anywhere"|IP|CIDR,note,confirm?}`. Risky changes return `confirmation_required`; repeat with confirmation, then `POST /firewall/confirm` within 60 seconds or the helper reverts. Emergency SSH accepts optional `{ip}` when an SSH tunnel hides the caller; expires after one hour.
- Files: list/read/download use `?path=`. Write `{path,content,mtime?}`, chmod `{path,mode}`, mkdir `{path}`, delete `{paths,permanent?}`. Upload is the raw file body at `/files/upload?dir=&name=&overwrite=`. See [files.rs](../crates/opsbox-api/src/files.rs) for exact query types and optimistic-concurrency fields. File operations are revalidated in `priv` after canonicalization; deny list/jail checks apply to all operations.
- Terminal: `/stream/terminal?user=&cols=&rows=&confirm=root` (confirmation required only for root). Binary frames carry bytes; text `{type:"resize",cols,rows}` controls size. Server messages include `ready`, `exit`, `error`. Nothing typed is audited; session start/end is.
- Cleanup: scan returns `{items,disk,total,system,auto,scanned}`; run `{ids:[...]}` is a task, auto `{on}` controls the weekly safe set. Docker scanners are absent when Docker is unavailable.
- Access keys: add `{user,key}` (public key only); removal `{id}` where ID is `user:fingerprint`; lock `{lock:true|false}`. Invitations `{name,role}` return a one-time link. Claim `{token,password}` starts TOTP enrolment. Removing an account also revokes its sessions.
- Security: `GET /security` combines real checks, panel accounts and recent audit entries. Fix actions are identified by check ID. Allow-list update `{value,confirm?}` accepts IP/CIDR text and cannot exclude the caller.
- Settings: `GET /settings` returns `{hostname,tz,timezones,swapMb,tokens}`. General settings save through the specific hostname/timezone endpoints; no generic PUT silently accepts future-phase fields. Tokens: create `{name,scope:"read"|"deploy"|"full"}` returns `{token,secret}`, listing exposes metadata only, DELETE revokes immediately.
- Metrics: `GET /metrics?window=48s` accepts 1 second through 1 hour (`s`, `m`, `h`), returns percentages, memory/disk GB and network MB/s. `/stream/metrics` sends `{type:"metrics",ts,data}` once per second, with the same data shape.
- Long operations return `202 {task}`; `GET /tasks/{id}` gives `running|ok|failed`, log tail, result and error. The UI polls it. Tasks are in memory, kept up to 24 hours/256 entries and are lost on restart. Task/event WebSocket streams are not part of this implementation.

## Errors and limits

Errors are `{error:{code,message,field?}}`; `message` is safe to display. Codes include `validation` (400), `unauthorized` (401), `forbidden` (403), `not_found` (404), `method_not_allowed` (405), `timeout` (408), `conflict`/`confirmation_required` (409), `too_large` (413), `rate_limited` (429), `internal` (500), `unsupported` (501), `unavailable` (503). A 429 includes `Retry-After` seconds. Every response has a fresh `X-Request-Id`.

API responses have `Cache-Control: no-store`. Ordinary requests are limited to 1 MiB and 30 seconds; editor requests allow 12 MiB JSON (5 MiB UTF-8 content) and 60 seconds. Transfers have a configurable 2 GiB default body limit and a two-hour handler deadline. WebSocket frames/messages are capped at 64 KiB, queued writes at 256 KiB, with at most 64 streams and eight terminals.

Token buckets: each IP has burst 120, refill 2/sec; login/setup/invitation/enrolment share burst 20, refill 1 per 3 sec; each authenticated session/token has burst 120, refill 4/sec. Each bucket table holds at most 4096 keys; new keys are refused when full until idle entries expire after 15 minutes. Health probes and static assets do not consume these budgets. Account/IP lockout remains a separate persisted policy.

Streams revalidate their session/token, role and allowed address every five seconds and end on revocation/expiry. Merely receiving output does not extend session idle expiry. Authenticated mutations first persist a `request.accepted` audit entry; inability to write it prevents dispatch. Named handler entries record detailed outcomes. Secrets, request bodies and query strings are excluded from the acceptance record.

## Implemented routes

<!-- GENERATED_ROUTES_START -->
| Method | Path | Handler / request types |
|---|---|---|
| `GET` | `/api/v1/access` | [access::get](../crates/opsbox-api/src/access.rs) |
| `POST` | `/api/v1/access/keys` | [access::add_key](../crates/opsbox-api/src/access.rs) |
| `POST` | `/api/v1/access/keys/remove` | [access::remove_key](../crates/opsbox-api/src/access.rs) |
| `POST` | `/api/v1/access/users/{name}/lock` | [access::lock_user](../crates/opsbox-api/src/access.rs) |
| `GET` | `/api/v1/attention` | [metrics::get_attention](../crates/opsbox-api/src/metrics.rs) |
| `POST` | `/api/v1/auth/login` | [auth::login](../crates/opsbox-api/src/auth.rs) |
| `POST` | `/api/v1/auth/logout` | [auth::logout](../crates/opsbox-api/src/auth.rs) |
| `POST` | `/api/v1/auth/sessions/revoke-all` | [auth::revoke_all](../crates/opsbox-api/src/auth.rs) |
| `POST` | `/api/v1/auth/totp/confirm` | [auth::totp_confirm](../crates/opsbox-api/src/auth.rs) |
| `POST` | `/api/v1/auth/totp/enroll` | [auth::totp_enroll](../crates/opsbox-api/src/auth.rs) |
| `PUT` | `/api/v1/cleanup/auto` | [cleanup::set_auto](../crates/opsbox-api/src/cleanup.rs) |
| `POST` | `/api/v1/cleanup/run` | [cleanup::run](../crates/opsbox-api/src/cleanup.rs) |
| `GET` | `/api/v1/cleanup/scan` | [cleanup::scan](../crates/opsbox-api/src/cleanup.rs) |
| `GET` | `/api/v1/files` | [files::list](../crates/opsbox-api/src/files.rs) |
| `POST` | `/api/v1/files/chmod` | [files::chmod](../crates/opsbox-api/src/files.rs) |
| `GET` | `/api/v1/files/content` | [files::read](../crates/opsbox-api/src/files.rs) |
| `PUT` | `/api/v1/files/content` | [files::write](../crates/opsbox-api/src/files.rs) |
| `POST` | `/api/v1/files/delete` | [files::delete](../crates/opsbox-api/src/files.rs) |
| `GET` | `/api/v1/files/download` | [files::download](../crates/opsbox-api/src/files.rs) |
| `POST` | `/api/v1/files/mkdir` | [files::mkdir](../crates/opsbox-api/src/files.rs) |
| `POST` | `/api/v1/files/upload` | [files::upload](../crates/opsbox-api/src/files.rs) |
| `GET` | `/api/v1/firewall` | [system::get_firewall](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/firewall/confirm` | [system::confirm_firewall](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/firewall/rules` | [system::add_rule](../crates/opsbox-api/src/system.rs) |
| `DELETE` | `/api/v1/firewall/rules/{id}` | [system::remove_rule](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/host` | [host::get_host](../crates/opsbox-api/src/host.rs) |
| `POST` | `/api/v1/invites/claim` | [users::claim](../crates/opsbox-api/src/users.rs) |
| `GET` | `/api/v1/invites/{token}` | [users::info](../crates/opsbox-api/src/users.rs) |
| `GET` | `/api/v1/jobs` | [system::get_jobs](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/jobs` | [system::add_job](../crates/opsbox-api/src/system.rs) |
| `DELETE` | `/api/v1/jobs/{id}` | [system::remove_job](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/jobs/{id}/run` | [system::run_job](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/metrics` | [metrics::get_metrics](../crates/opsbox-api/src/metrics.rs) |
| `GET` | `/api/v1/security` | [security::get](../crates/opsbox-api/src/security.rs) |
| `PUT` | `/api/v1/security/allow-ips` | [security::set_allow_ips](../crates/opsbox-api/src/security.rs) |
| `POST` | `/api/v1/security/checks/{id}/fix` | [security::fix](../crates/opsbox-api/src/security.rs) |
| `GET` | `/api/v1/session` | [auth::session](../crates/opsbox-api/src/auth.rs) |
| `GET` | `/api/v1/settings` | [tokens::get_settings](../crates/opsbox-api/src/tokens.rs) |
| `POST` | `/api/v1/setup/admin` | [setup::admin](../crates/opsbox-api/src/setup.rs) |
| `POST` | `/api/v1/setup/claim` | [setup::claim](../crates/opsbox-api/src/setup.rs) |
| `POST` | `/api/v1/setup/totp/confirm` | [setup::confirm](../crates/opsbox-api/src/setup.rs) |
| `POST` | `/api/v1/ssh/emergency` | [system::ssh_emergency](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/stream/journal` | [system::stream_journal](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/stream/metrics` | [metrics::stream](../crates/opsbox-api/src/metrics.rs) |
| `GET` | `/api/v1/stream/terminal` | [terminal::stream](../crates/opsbox-api/src/terminal.rs) |
| `PUT` | `/api/v1/system/hostname` | [system::set_hostname](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/system/journal` | [system::get_journal](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/system/reboot` | [system::reboot](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/system/reboot/cancel` | [system::reboot_cancel](../crates/opsbox-api/src/system.rs) |
| `PUT` | `/api/v1/system/swap` | [system::set_swap](../crates/opsbox-api/src/system.rs) |
| `PUT` | `/api/v1/system/timezone` | [system::set_timezone](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/system/units` | [system::get_units](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/system/units/{name}/{action}` | [system::unit_action](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/system/updates` | [system::get_updates](../crates/opsbox-api/src/system.rs) |
| `PUT` | `/api/v1/system/updates/auto` | [system::set_auto_updates](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/system/updates/install` | [system::install_updates](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/tasks/{id}` | [system::get_task](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/terminal/users` | [terminal::users](../crates/opsbox-api/src/terminal.rs) |
| `POST` | `/api/v1/tokens` | [tokens::create](../crates/opsbox-api/src/tokens.rs) |
| `DELETE` | `/api/v1/tokens/{id}` | [tokens::revoke](../crates/opsbox-api/src/tokens.rs) |
| `POST` | `/api/v1/updates/apply` | [update::apply](../crates/opsbox-api/src/update.rs) |
| `PUT` | `/api/v1/updates/auto` | [update::set_auto](../crates/opsbox-api/src/update.rs) |
| `POST` | `/api/v1/updates/check` | [update::check](../crates/opsbox-api/src/update.rs) |
| `POST` | `/api/v1/updates/rollback` | [update::rollback](../crates/opsbox-api/src/update.rs) |
| `GET` | `/api/v1/updates/self` | [update::get_status](../crates/opsbox-api/src/update.rs) |
| `POST` | `/api/v1/users/invites` | [users::invite](../crates/opsbox-api/src/users.rs) |
| `DELETE` | `/api/v1/users/{name}` | [users::remove](../crates/opsbox-api/src/users.rs) |
| `GET` | `/healthz` | [health::healthz](../crates/opsbox-api/src/health.rs) |
| `GET` | `/readyz` | [health::readyz](../crates/opsbox-api/src/health.rs) |
<!-- GENERATED_ROUTES_END -->

## Future phases (sample UI)

Recovery, watchdog and self-update: Phase 2. Docker management, deployments, GitHub, templates and Compose: Phase 3. Proxy/panel domains and certificates: Phase 4. Backups/alerts: Phase 5. WireGuard, desktop device authentication and other-server terminals: Phase 6. These APIs are not implemented; their UI retains sample-data labels.
