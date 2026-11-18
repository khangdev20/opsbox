# opsbox HTTP API — Phase 1

The agent serves JSON at `/api/v1` and embeds the UI at `/`. Use `?mode=http` for the real agent; `?mode=mock` keeps the demonstration. Times are Unix seconds unless a field is explicitly formatted for display; journal timestamps are milliseconds. IDs are ULIDs except system jobs and firewall rules.

The inventory below is generated from the actual axum router. Run `python3 tools/generate-api.py` after a route change; CI checks it. Links lead to each handler's request structures and response shaping. Shared system/privilege data types live in [opsbox-proto](../crates/opsbox-proto/src/lib.rs). A full OpenAPI schema is deferred: many UI response types are still `serde_json::Value`, so adding `utoipa` now would describe incomplete schemas.

## Authentication and roles

- `GET /session` is public and returns `authenticated`, `enrollRequired`, `setupRequired`, and when signed in `{user, displayName, role, totp: {enabled, method}}` (`method` is `"app"` when two-factor is on, otherwise `null`).
- Login: `POST /auth/login {username,password,code}`. Password + six-digit TOTP; an account with reset 2FA gets an enrolment-only session. `/auth/totp/enroll` returns `{secret,otpauthUri,account}`; `/auth/totp/confirm {code}` upgrades it. `POST /auth/reauth {password?,code?}` (interactive session) opens a 5-minute window for secret reveal endpoints; wrong credentials return `403`.
- Secrets in responses (prompt 16o): list/detail endpoints never return plaintext secrets — env vars show `••••••••`, destinations/channels/GitHub advertise `hasValue` / `hasCredentials` / `hasWebhookSecret`, API tokens are metadata only (secret shown once at creation and stored as a hash). Full values come only from dedicated reveal routes (`POST …/reveal`, database connection-string, backup recovery-key) after Owner/Admin (or the service deployer for vars), recent re-auth, with `Cache-Control: no-store`, a per-session reveal rate limit, and an audit row that names the secret and target (never the value). Deploy/task log writers scrub known secret values to `[REDACTED]`.
- First run: `/setup/claim {token}`, `/setup/admin {username,password,display_name?}`, `/setup/totp/confirm {code}`. Each step uses the setup cookie; all return 404 permanently after the first admin finishes enrolment. `display_name` is optional (trimmed, ≤ 64 characters, no control characters).
- The session cookie is `HttpOnly; SameSite=Strict; Path=/`. Over HTTPS (our own TLS listener, or the opsbox proxy, which proves itself with a shared secret) it is named `__Host-opsbox_session` and is `Secure`; on a plain loopback listener it is `opsbox_session`. `X-Forwarded-*` headers from anyone else are ignored and removed. Mutations using cookies require same-host `Origin` and `X-Opsbox-CSRF` equal to the script-readable `opsbox_csrf` cookie. Every browser WebSocket requires a same-host `Origin`.
- Scripts may send `Authorization: Bearer ops_…` without cookies/CSRF. Scopes: `read` permits host, metrics, attention; `deploy` adds journal, Docker container/log reads, GitHub repository reads, and deploy/rollback triggers; `deploy:<service>` restricts a token strictly to triggering deploys/rollbacks and reading status/build logs/tasks for that single named service only (never files, vars, or service deletion); `full` permits administrator operations except account, token, SSH-key and terminal management. Cookie authentication takes precedence. A token cannot be used as a cookie. Tokens are hashed at rest, returned once, explicitly revoked and have no expiry yet.
- System/metrics/Docker reads require viewer; system and Docker mutations (install, container start/stop/restart/delete) require admin. Files require deployer and are strictly jailed per service to `/var/lib/opsbox/apps/<their services>`; viewers have no file access. Service creation/management/variables/deletion require deployer (for owned services) or admin. GitHub repository listing, public-repo inspection, and deploy-key management require deployer; GitHub App manifest connect/sync/disconnect requires admin. Cleanup, Access, Settings, token management and Terminal require admin. Security checks are readable by signed-in accounts; fixes, invitations and account removal require admin.

## Payloads and streams

- Updates: `GET /system/updates` returns `list` entries `{pkg,cur,next,sec,kernel}` plus totals, reboot/auto-update/kernel/index metadata. Install `{which:"security"|"all"}` or `{which:"selected",packages:["curl"]}`; only currently pending, validated packages can be selected.
- opsbox self-update: `GET /updates/self` returns the snapshot `{current, channel, channelSource, devBuild, migration, migrationNote, auto, window, latest, updateAvailable, checkedAt, checkedChannel, reason, reasonDetail, retryAt, previousVersion, lastOutcome}`. When `updateAvailable` is false, `reason` says why: `no_releases_on_channel` (nothing published on the channel, e.g. stable with only alphas), `up_to_date` (the newest release is older than or equal to the running version; `latest` is it), `network_error` (GitHub unreachable or unusable; `reasonDetail` has the cause), `rate_limited` (`retryAt` is the Unix time to ask again) or `dev_build`. `reason` is null when an update is offered or before the first check. `checkedAt` is when GitHub was last asked (Unix seconds) and `checkedChannel` the channel that answer was for. A failed check never replaces an offer an earlier check found on the same channel.
- Release lookups send a `User-Agent`, reuse the GitHub answer for one hour (also across channel changes), and on a 403/429 with rate-limit headers wait until `X-RateLimit-Reset` (or `Retry-After`; at least a minute, at most an hour) before asking again. `POST /updates/check` is "Check now": it skips the cache but not a rate-limit wait. `PUT /updates/auto {channel?,auto?,window?}` returns the snapshot, re-evaluated for a new channel from the cache. `POST /updates/apply` without a version gives the same reason in its 400 message instead of "up to date".
- System mutations: hostname `{name}`, timezone `{zone}`, swap `{sizeMb}` (0 disables), automatic updates `{on}`, reboot `{delayMin}` (1–60, default 1). Unit action is one of `start`, `stop`, `restart`, `enable`, `disable`; the refusal list applies in `priv` and systemd calls use D-Bus via zbus.
- Journal query: `unit`, `prio` (0–7), `q`, `limit`. Stream `/stream/journal` accepts the same filters and sends `{type:"journal",entries:[{t,proc,prio,msg,cursor}]}`. Entries are displayed oldest first; the helper supports cursor-based tailing.
- Jobs: create `{name,cron,command}` (five-field cron, one-line shell command); job IDs are eight hex digits. Run/remove operate on those IDs.
- Firewall rule: `{port,proto:"tcp"|"udp",src:"anywhere"|IP|CIDR,note,confirm?}`. Risky changes return `confirmation_required`; repeat with confirmation, then `POST /firewall/confirm` within 60 seconds or the helper reverts. Emergency SSH accepts optional `{ip}` when an SSH tunnel hides the caller; expires after one hour.
- Files: list/read/download use `?path=`. Write `{path,content,mtime?}`, chmod `{path,mode}`, mkdir `{path}`, delete `{paths,permanent?}`. Upload is the raw file body at `/files/upload?dir=&name=&overwrite=`. See [files.rs](../crates/opsbox-api/src/files.rs) for exact query types and optimistic-concurrency fields. File operations are revalidated in `priv` after canonicalization; deny list/jail checks apply to all operations.
- Terminal: `/stream/terminal?user=&cols=&rows=&confirm=root` (confirmation required only for root). Binary frames carry bytes; text `{type:"resize",cols,rows}` controls size. Server messages include `ready`, `exit`, `error`. Nothing typed is audited; session start/end is.
- Cleanup: scan returns `{items,disk,total,system,auto,scanned}`; run `{ids:[...]}` is a task, auto `{on}` controls the weekly safe set. Docker scanners are absent when Docker is unavailable.
- Access keys: add `{user,key}` (public key only); removal `{id}` where ID is `user:fingerprint`; lock `{lock:true|false}`. Invitations `{name,role,display_name?}` return a one-time link. Claim `{token,password,display_name?}` starts TOTP enrolment (`display_name` on claim overwrites only when sent). `PATCH /users/{name} {display_name}` lets a person change their own display name; an owner may change anyone's. Removing an account also revokes its sessions. Audit actors are stored as `display name (username)` when a display name is set.
- Security: `GET /security` combines real checks, panel accounts and recent audit entries (each with `ip`). Fix actions are identified by check ID. Allow-list update `{value,confirm?}` accepts IP/CIDR text and cannot exclude the caller. `GET /audit` (admin) returns `{entries,nextCursor,limit}` with filters `actor`, `action` (prefix), `result` (`ok`|`denied`|`failed`), `from`/`to` (RFC 3339), and `cursor` (previous page's oldest `seq`); default page size 50. Each entry includes `t`, `user`, `action`, `ip`, `result`, `warn`, `seq`. `GET /audit/export.csv` (admin) streams the same filters as CSV (`Content-Type: text/csv`, `Cache-Control: no-store`, `Content-Disposition: attachment; filename="opsbox-audit-<host>-<YYYY-MM-DD>.csv"`), columns `seq,ts,actor,actor_kind,ip,action,target,result,detail,hash`, RFC 4180 escaping, leading `'` on cells that start with `= + - @`, and trailing `# chain:` comments with first/last hash or the first broken `seq`.
- Settings: `GET /settings` returns `{hostname,tz,timezones,swapMb,tokens,opsboxUpdate,panel,github,deployKeys}`. General settings save through the specific hostname/timezone endpoints; no generic PUT silently accepts future-phase fields. Tokens: create `{name,scope:"read"|"deploy"|"deploy:<service>"|"full"}` returns `{token,secret}`, listing exposes metadata only, DELETE revokes immediately.
- GitHub & repository sources: `GET /github` returns the connected GitHub App status (`{connected,installed,appSlug,owner,account,manageUrl,installUrl,webhookUrl,webhookConfigured,panelDomainSet,recentDeliveries}`). Connecting uses the GitHub App manifest flow so each server owns its own App: `POST /github/manifest {org?}` generates a single-use CSRF `state` and returns `{actionUrl,manifest,state,webhookConfigured,panelDomainRequired}`; `GET /github/callback` and `GET /github/setup` perform same-origin meta-refreshes to `/#/settings?...`; `POST /github/complete {code,state}` exchanges `code` via `POST /app-manifests/{code}/conversions` and seals the App's RSA private key and webhook secret in SQLite via `opsbox-priv` (`SecretSeal` under `deploy:*` contexts); `POST /github/installation` refreshes installation access; `POST /github/webhook/sync` updates the GitHub App's webhook URL once a panel domain is live; `DELETE /github` disconnects and erases credentials. `GET /github/repos?q=&refresh=` lists installed repositories with search and detected build methods (`Dockerfile`, `Docker Compose`, `Nixpacks · <runtime>`, `static site`, `nothing to build`). Fallbacks: `POST /github/public-repo {url}` validates a public HTTPS Git URL (blocking private/loopback/local hosts) and detects build method on GitHub; `GET /github/deploy-keys`, `POST /github/deploy-keys {repo}`, and `DELETE /github/deploy-keys/{id}` manage per-repo Ed25519 SSH deploy keys (private keys sealed via `priv`, public keys/fingerprints returned). `POST /github/webhook` is public (and exempt from the panel IP allow-list) and requires a valid `X-Hub-Signature-256` HMAC-SHA256 signature verified against the sealed webhook secret; valid `push` events automatically enqueue deploys for matching `autoDeploy` services.
- Deploy pipeline & services: `GET /projects/{id}` (`"default"` auto-initialized with `"production"` environment) returns services (including `publicPort`, `networkMode`, `credentialsRotatedAt`, and `connectedApps` for services referenced via variables), redacted vars (`isSecret` values replaced by `"••••••••"` while pure `${{ service.KEY }}` references always render as `[key, "${{ service.KEY }}", isSecret, true]` without ever exposing the resolved secret), `edges` merging Compose `depends_on` and `${{ ... }}` variable references, and deploy history. `POST /services` creates a service (GitHub, SSH deploy key, public Git URL, or prebuilt Docker image), ensures `/var/lib/opsbox/apps/<svc>`, validates any `${{ ... }}` references, seals initial variables in `priv`, and runs the deploy pipeline (`201 {id,service,deployId,task}`). `GET/PATCH/DELETE /services/{id}` inspect, edit config/domain/auto-deploy, or clean up containers/images/source dirs. `PUT/DELETE /services/{id}/vars/{key}` manage sealed service variables (validating same-project `${{ service.KEY }}` references and preventing cycles; non-secret values passed as `--build-arg`, secrets mounted strictly via BuildKit `--secret id=KEY,src=...`, reference values resolved exclusively inside `opsbox-priv` at build/start, and scrubbed from logs). `POST /services/{id}/vars/{key}/reveal {password?,code?}` returns the plaintext after recent re-auth (`no-store`, audited as `service.var.reveal` with the key name). `POST /services/{id}/databases {database,sourceKey?,asKey?,includeAll?}` connects an app/tool to a database in the same project by inserting `${{ db.KEY }}` reference variable(s), and `DELETE /services/{id}/databases/{db}` removes those reference variables. `POST /services/{id}/deploys` triggers a manual/token redeploy (`202 {id,task}`, returning `409` if already running), `POST /services/{id}/deploys/{deploy_id}/rollback` re-releases a retained image (`keep = 5`) with zero rebuild, `GET /deploys/{id}/log` reads persisted logs, and `GET /stream/deploys/{id}` streams `{type:"log",ts,line}` / `{type:"done",status,error}` frames over WebSocket. Failed latest deploys appear in `GET /attention` until resolved while keeping any previously live container serving behind `opsbox-proxy`.
- App domains & TLS: `GET /services/{id}/domains` (readable by service owner, admin, or `deploy`/`deploy:<service>` token) returns `{service,project,network,port,internalHost,internalUrl,proxyReachable,serverIps,domains}` where each domain includes `{id,name,domain,primary,redirect,redirectTo,forceHttps,hsts,dns,dnsCheckedAt,cert,renews,certExpiresAt,certIssuer,certStaging,certError,consecutiveFailures}`. `POST /services/{id}/domains {domain,www?,redirectTo?,forceHttps?,hsts?,primary?}` (`www` in `none|www_to_bare|bare_to_www|serve_both`) adds one or more domains and syncs `opsbox-proxy` (`201`, allowed on `app` services and non-internal `tool`/`compose` services with `port > 0`, refused on `db`). `POST /domains/{name}/check` runs `hickory-resolver` DNS verification and updates `dns` state. `POST /domains/{name}/certificate` starts an ACME HTTP-01 issuance task (`202 {task}`) and persists sealed keys via `opsbox-priv` (`proxy:*`). `PATCH /domains/{name} {forceHttps?,hsts?,redirectTo?,primary?}` updates routing/security flags. `POST /domains/{name}/primary` promotes a non-redirecting domain to primary. `DELETE /domains/{name}` removes the domain and its certificate. Certificate renewal failures raise `"tls"` items in `GET /attention` scoped by role and service ownership.
- Templates & Compose: `GET /templates` returns the embedded database and tool catalog (`{databases,tools,templates}`). `POST /templates/{id}/instances {name,project?,version?,disk?,backup?,internalOnly?,publicPort?,confirm?,databaseUrl?,vars?,memoryLimitMb?,cpuLimitMillis?}` creates a template instance with generated secrets sealed via `opsbox-priv` (`deploy:*`), a persistent named volume (`opsbox-<name>-data`), safe default network isolation (`internalOnly:true` binds zero host ports, `internalOnly:false` binds `127.0.0.1:0:<port>`, and `publicPort:true` binds `0.0.0.0:0:<port>` on the host only when `confirm:true` for databases — returning `409 confirmation_required` otherwise), a persisted `backup_jobs` schedule row, and returns a one-time `connectionUrl` (`201 {id,service,deployId,task,connectionVar,connectionUrl}` — never returned plaintext on later `GET`s). `POST /services/{id}/rotate` executes password rotation inside the running database container via `opsbox-priv` (`PrivOp::DatabaseRotate`), re-seals the updated credentials, marks dependent apps as stale (`needsRedeploy:true`), and returns `{ok,service,rotatedKeys,rotatedAt,affectedApps}`. `PUT /services/{id}/exposure {mode:"internal"|"loopback"|"public",confirm?}` updates a service's port exposure setting (enforcing `409 confirmation_required` for unconfirmed public DB exposure and returning `{ok,mode,redeployRequired,service}`). `POST /compose {yaml,stack?,project?,confirm?,vars?}` runs `docker compose config` inside `opsbox-priv`, enforces pre-config path containment (`include`, `env_file`, `extends`, `build`), refuses `privileged`, `network_mode: host`, `driver: host`, unconfirmed public database port bindings (`5432`/`3306`/`6379`/`27017`), or bind mounts outside `/var/lib/opsbox/apps/<stack>` with `409 confirmation_required` unless `confirm:true`, deploys with `docker compose -p opsbox-<stack> up -d`, connects containers to `opsbox-<project>`, and registers `compose` service nodes + `depends_on` edges in `GET /projects/{id}` (`201 {stack,projectName,names,warnings,edges,task}`). `DELETE /compose/{stack}` runs `docker compose down` and deletes stack metadata/services.
- Backups & Restore: `GET /backups` (viewer+) returns `{jobs,snapshots,destinations,defaultDestinationId,localDisk,sameDisk,remoteDestinationsReady,restoreReady,restoreTest}` including the pre-seeded `"opsbox-self"` job, restic presence/version, filesystem capacity, and `st_dev` same-disk detection. `GET /backups/providers` returns the supported offsite destination presets (`r2`, `s3`, `b2`, `wasabi`, `do`, `hetzner`, `gcs`, `minio`, `sftp`, `custom`) and server hostname from `opsbox-proto`. `POST /backups/destinations/test` validates and runs a connection/init/encrypted-probe/delete-permission check (`SftpReachability` TCP check before SFTP key install, full `restic` probe for S3). `POST /backups/destinations` seals S3 credentials via `opsbox-priv` (`SecretSeal` under `backup:dest:<id>`) or generates an Ed25519 SSH keypair inside `priv`, storing verified offsite S3 destinations and automatically migrating `"local"` jobs (SFTP becomes verified and default once its saved test passes). `POST /backups/destinations/{id}/test`, `POST /backups/destinations/{id}/default`, and `DELETE /backups/destinations/{id}` verify, promote, or remove destinations (protecting `"local"`). `POST /backups/recovery-key` (admin interactive session, `deny_token`, password re-auth returning `403` on wrong password, `Cache-Control: no-store`) exports the HMAC-SHA256 deterministic `restic` repository password derived from `/etc/opsbox/master.key`. `GET /backups/snapshots` refreshes snapshot metadata across configured destinations. `POST /backups/jobs`, `PATCH /backups/jobs/{id}`, and `DELETE /backups/jobs/{id}` manage `path`, `volume`, and `database` schedules on any verified destination. `POST /backups/jobs/{id}/run`, `POST /backups/run`, `POST /backups/prune`, and `POST /backups/check` run serialized `restic` tasks (`202 {task}`). `POST /backups/snapshots/{id}/restore {mode:"copy"|"replace",confirm?,copyTarget?}` restores to a safe copy by default (starting a new connectable database service container for `database`, `<path>.restored-<date>` for `path`, `<vol>-restored-<date>` for `volume`, `/var/lib/opsbox/opsbox-restored-<date>` for `opsbox`); `"replace"` refuses without exact `confirm:"replace"` (`409 confirmation_required`), refuses live `opsbox` replace (`403`), and takes a live safety snapshot (`opsbox-safety:true`) FIRST before overwriting live data. `POST /backups/snapshots/{id}/test` and `POST /backups/restore-test` (and the 30-day automatic monthly restore-test schedule) restore into an ephemeral container/directory, verify DB sanity queries or SQLite `PRAGMA integrity_check`, tear down temporary resources, and record `"restore_tested"` or raise a `"backup"` item in `GET /attention` on failure. Also `opsbox restore <snapshot> --to <path>` runs copy restore via `priv`.
- Panel domain: `GET /panel` returns `{domain,activeDomain,certStatus,certExpiresAt,certDaysRemaining,certIssuer,certStaging,certError,proxyReachable,rawPortOpen,listen}`. `POST /panel/dns-check {domain}` verifies public A/AAAA records via `hickory-resolver` and returns `{domain,status:"points_here"|"waiting",pointsHere,message,resolvedIps,serverIps,cname,records}`. `POST /panel/certificate {domain}` starts an ACME HTTP-01 issuance task (`202 {task}`). `DELETE /panel/domain {confirm?}` detaches the panel domain and certificate (requires confirmation if raw `:8443` is closed). `POST /panel/local-only {localOnly,confirm?}` moves `serve` between `127.0.0.1:8443` and `0.0.0.0:8443` via `priv` and schedules a panel service restart (requires confirmation when closing `:8443` without a live certificate).
- Recovery & watchdog: `GET /recovery` (admin only) reads the watchdog status published at `/run/opsbox-watchdog/status.json` via `priv` (`PrivOp::WatchdogStatus`) plus `opsbox-watchdog` journal entries and returns `{running,failed,degraded,updatedAt,currentVersion,previousVersion,canRollback,updateMessage,safeModeListen,sshOpenUntil,checks,events,log}`. Recovery mutations (`free`, `restart`, `rollback`, `ssh`, `reboot`) are served exclusively by `opsbox-watchdog`'s standalone HTTPS Safe Mode server on `:8444`.
- Metrics: `GET /metrics?window=48s` accepts 1 second through 1 hour (`s`, `m`, `h`), returns percentages, memory/disk GB, network MB/s, top processes (`procs`), and running container stats (`containers`). Each process has `{name,pid,cpu,mem,self,kind}` where `kind` is `container`, `system`, `user`, or `unknown` (from `/proc/<pid>/cgroup`); container processes also include `container:{id,name,service?}` and system processes include `unit` (e.g. `docker.service`). Cgroup is read only for the top-N list and cached by PID until exit. `/stream/metrics` sends `{type:"metrics",ts,data}` once per second, with the same data shape.
- Docker & Containers: `GET /docker/status` returns `{installed,running,version,apiVersion,canInstall,installMethod,error}`. `POST /docker/install` (`202 {task}`) installs Docker Engine from verified distro or signed official Docker apt/dnf repositories (never the convenience script). `GET /docker/containers` lists managed and unmanaged containers with live CPU/memory counters; `GET /docker/containers/{id}` returns detailed inspect state with secret-looking environment variables masked; `POST /docker/containers/{id}/{action}` accepts `start`, `stop`, `restart`, or `delete`; `GET /docker/containers/{id}/logs?tail=200` fetches a log snapshot; `GET /stream/containers/{id}/logs?tail=200` upgrades to WebSocket and streams `{type:"logs",entries:[{ts,stream,text}]}`, `{type:"end"}`, or `{type:"error",message}`; `GET /docker/images`, `/docker/volumes`, and `/docker/networks` list Engine resources. Every Docker Engine call runs exclusively inside `priv`.
- Long operations return `202 {task}`; `GET /tasks/{id}` gives `running|ok|failed`, log tail, result and error. The UI polls it. Tasks are in memory, kept up to 24 hours/256 entries and are lost on restart. Task/event WebSocket streams are not part of this implementation.

## Errors and limits

Errors are `{error:{code,message,field?}}`; `message` is safe to display (no stack traces, paths or secrets). Codes include `validation` (400), `unauthorized` (401), `forbidden` (403), `not_found` (404), `method_not_allowed` (405), `timeout` (408), `conflict`/`confirmation_required` (409), `too_large` (413), `rate_limited` (429), `internal` (500), `unsupported` (501), `unavailable` (503). A 429 includes `Retry-After` seconds. Every response has a fresh `X-Request-Id`.

### Default security headers

Every response from `serve` (API, UI, downloads, errors, 404s) carries: `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: same-origin`, `Permissions-Policy: camera=(), microphone=(), geolocation=()`, `Cross-Origin-Opener-Policy: same-origin`, and a `Content-Security-Policy` of `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'self'; object-src 'none'; form-action 'self'`. Data tools under `/tools/…` (pgweb, Adminer) get a looser CSP on that prefix only (`unsafe-inline` / `unsafe-eval` for their UIs). There is no `Server` version header.

`Cache-Control: no-store` applies to all `/api/*` responses and HTML; hashed static assets under `fonts/` and `vendor/` may use long-lived `immutable` caching. `Strict-Transport-Security: max-age=31536000` is sent only when the request arrived over HTTPS on a real DNS name (panel domain via the proxy); it is absent on self-signed IP mode and loopback HTTP so a certificate change cannot lock browsers out. No `includeSubDomains` or preload by default.

Ordinary requests are limited to 1 MiB and 30 seconds; editor requests allow 12 MiB JSON (5 MiB UTF-8 content) and 60 seconds. Transfers have a configurable 2 GiB default body limit and a two-hour handler deadline. WebSocket frames/messages are capped at 64 KiB, queued writes at 256 KiB, with at most 64 streams and eight terminals.

Token buckets: each IP has burst 120, refill 2/sec; login/setup/invitation/enrolment share burst 20, refill 1 per 3 sec; each authenticated session/token has burst 120, refill 4/sec; secret reveal endpoints share a per-session burst of 10, refill 1 per 30 sec. Each bucket table holds at most 4096 keys; new keys are refused when full until idle entries expire after 15 minutes. Health probes and static assets do not consume these budgets. Account/IP lockout remains a separate persisted policy.

Streams revalidate their session/token, role and allowed address every five seconds and end on revocation/expiry. Merely receiving output does not extend session idle expiry. Authenticated mutations first persist a `request.accepted` audit entry; inability to write it prevents dispatch. Named handler entries record detailed outcomes. Secrets, request bodies and query strings are excluded from the acceptance record.

## Notifications and attention

`GET /api/v1/attention` (viewer+) returns `{items,unread,critical}` scoped by role, service ownership and token scope (`read`/`deploy` tokens can read; browser mutations require Origin/CSRF). Each item has `{id,kind,severity,since,text,link,alertId?,read,snoozed,snoozedUntil?}` where `severity` is `critical`, `warning`, or `info`, `since` is Unix seconds, and `link` is `[label, route]`. Items sort critical-first (unsnoozed criticals above snoozed criticals), then newest `since` first.

- `POST /api/v1/attention/{id}/read`: marks an active visible item read for the current user (`read` becomes `false` again if `since` advances or `severity` increases).
- `POST /api/v1/attention/read-all`: marks all currently visible non-snoozed items (including snoozed critical items) read for the current user.
- `POST /api/v1/attention/{id}/snooze {minutes}`: snoozes an active visible item (`1..=10080`, max 7 days). Snoozed `warning`/`info` items leave `items` and `unread` until expiry; snoozed `critical` items remain in `items` (dimmed, below unsnoozed criticals) and stay counted in `unread` and `critical`.

When a built-in check and a fired host alert (admin only) map to the same canonical `id`, one item is returned keeping the higher `severity` and linking to the fired alert (`#/alerts?id=...` with `alertId`), otherwise linking to the feature page:

| Source / check | Alert rule & target | Canonical `id` | Default severity |
|---|---|---|---|
| Pending security updates | `security` / `host` | `security:host` | `warning` |
| Reboot required (`/run/reboot-required`) | — | `reboot:host` | `warning` |
| Disk usage > 75% (critical >= 95%) | `disk` / `<mount>` | `disk:<mount>` | `warning` / `critical` |
| Failed systemd service | `units` / `<unit>` | `units:<unit>` | `critical` |
| Restarting or down container | `containers` / `<name>` | `containers:<name>` | `critical` |
| CPU high (> 90%, critical >= 99%) | `cpu` / `host` | `cpu:host` | `warning` / `critical` |
| Memory high (> 90%, critical >= 95%) | `memory` / `host` | `memory:host` | `warning` / `critical` |
| opsbox release available | — | `update:<version>` | `info` |
| Failed latest service deploy | `deploy` / `<serviceId>` | `deploy:<serviceName>` | `critical` |
| Expiring / failing TLS certificate | `tls` / `<domain>` | `cert:<domain>` | `warning` / `critical` |
| Backup job failed or scheduled run overdue | `backup` or `missed` / `opsbox-self` or `<jobId>` | `backup:opsbox` or `backup:<jobId>` | `critical` / `warning` |
| Backup snapshot checksum failed | `backup` / `checksum-<snapshotId>` | `backup:checksum` | `critical` |
| Monthly backup restore test failed | `backup` / `restore-test` | `backup:restore-test` | `critical` |
| Scheduled backup waited for memory | `backup` / `capacity` | `backup:capacity` | `critical` |
| Emergency SSH access open | `ssh` / `<target>` | `ssh:<target>` | `critical` |
| Repeated failed sign-ins | `signins` / `host` | `signins:host` | `critical` |
| Unhealthy metrics / proxy / port conflict | `health` / `host \| proxy \| proxy-ports` | `health:<target>` | `critical` |

## Alerts (Spec 7.13)

Alert configuration and history require an administrator. Channel connect/disconnect and test also require an interactive session (full API tokens are refused). Other roles keep their existing scoped Overview links. Browser mutations use the normal Origin/CSRF and audit guards.

| Method / path | Body / response |
|---|---|
| `GET /alerts` | `{channels,rules,incidents,history,quiet}`. Channel summaries contain no credentials, ciphertexts, webhook URLs or bot tokens. `on` means configured; `verified` means the latest delivery succeeded. |
| `PUT /alerts/rules/{id}` | `{on:bool}`. IDs: `disk`, `cpu`, `memory`, `units`, `containers`, `deploy`, `backup`, `missed`, `tls`, `security`, `signins`, `ssh`, `health`. Disabling resolves an active incident on the next successful evaluation. |
| `PUT /alerts/channels/{id}` | Full replacement configuration below. Sealed via `PrivOp::SecretSeal`, context `alerts:channel:<id>`. Saving does not claim a successful test. |
| `DELETE /alerts/channels/{id}` | Removes configuration and stops subsequent deliveries. An already in-flight request may finish. |
| `POST /alerts/test` | `{channel:"c1"|"c2"|"c3"|"c4"|null}`; null tests all configured channels concurrently. Explicitly bypasses silences/quiet hours. Returns `{ok:true}` only after delivery; `400` for no selected channel, `503` on delivery/helper failure. |
| `POST /alerts/silence` | `{id:"disk:/",seconds:3600}`. Exact rule/target ID from `incidents`; 0 ends silence, maximum 604800 (7 days). State remains visible while notifications wait. |
| `PUT /alerts/quiet` | `{from:"23:00",to:"07:00"}` in UTC. Equal times disable quiet hours. Invalid HH:MM returns `400`. |

Channel bodies (secret fields are accepted on write only):

- `c1`: `{kind:"email",host:"smtp.example.com",port:587,username:"user",password:"…",from:"opsbox@example.com",to:"admin@example.com"}`. Port 465 uses implicit TLS; 587 requires STARTTLS. Certificate verification is mandatory.
- `c2`: `{kind:"slack",url:"https://hooks.slack.com/services/…"}` (also accepts `hooks.slack-gov.com`). The entire bearer URL is sealed.
- `c3`: `{kind:"telegram",token:"12345:…",chat:"-100123456789"}`. Uses the fixed Telegram Bot API endpoint.
- `c4`: `{kind:"webhook",url:"https://example.com/alerts",secret:"at-least-16-characters"}`. HTTPS only; embedded user information and fragments refused. All HTTP channels refuse redirects and time out after 10 seconds. SMTP uses the same 10-second transport timeout. Transport errors never return provider bodies or credential-bearing URLs.

Generic webhooks send UTF-8 JSON `{id,name,rule,target,state,level,message,link,alertsLink}` with `Content-Type: application/json`, `X-Opsbox-Timestamp: <unix-seconds>` and `X-Opsbox-Signature: sha256=<lowercase-hex>`. Verify HMAC-SHA256 over ASCII timestamp + `.` + the **exact request body bytes**, compare in constant time, and reject old timestamps. `id` is stable for retries of a rule/target revision; consumers should deduplicate it. When a panel domain is configured, links are absolute HTTPS URLs.

Evaluation runs every 15 seconds over cached metrics/facts and persisted deploy/backup/security events. Defaults: disk >75% for 60s (critical at 95%), CPU >90% for 300s (critical at 99%), memory >90% for 300s (critical at 95%), failed units/down containers for 30s, security updates pending for two days, missed backup after 15 minutes, certificates within 14 days, five failed sign-ins within 15 minutes, and open emergency SSH until its expiry. Deploy/backup/restore-test/checksum failures are critical; stale metrics or an unavailable configured proxy for 60s raise a health alert. Unknown source groups preserve firing incidents and restart pending durations rather than reporting a false recovery.

Incidents persist `ok → pending → firing → resolved` independently per rule and target. Notifications fire on activation, warning-to-critical worsening, recovery and four-hour repeats. Warning delivery waits through quiet hours; critical bypasses quiet hours. Explicit silences suppress either level. Per-channel receipts survive restarts, failed deliveries retry after 60 seconds, and successful channels do not repeat for another channel's failure. Recovery is sent only to channels that received firing. Delivery is at least once across a crash between remote receipt and local acknowledgement. History retains 200 transitions; cleared targets expire after seven days. Overview returns `alertId` and `#/alerts?id=<encoded-id>` for active host alerts to administrators.

## Databases, part B: management (prompt 16)

PostgreSQL services only (`postgres`, `postgres-pgvector`, `postgis`). `{id}` is the service name or ID. Reading needs access to the service; every change needs write access to it (admin or owner) and refuses API tokens. All SQL and container actions run in `opsbox-priv` (`PrivOp::DatabaseInspect/Manage/Pooler/Upgrade`); `serve` stores sealed values and metadata only.

| Request | Behaviour |
|---|---|
| `GET /databases/{id}?database=name` | `{id,templateId,flavor,version,state,running,live,liveError,apps,pooler,upgrades,dataVolume,credentialsRotatedAt}`. `live` (null with a sentence in `liveError` when the database is stopped) holds `serverVersion`, `databases[{name,owner,sizeBytes}]`, `users[{name,canLogin,superuser,connections,databases}]`, `grants[{role,database,connect,create,temp}]`, `extensions[{name,installed,installedVersion,defaultVersion,comment}]`, `tables[{schema,table,totalBytes,rowsEstimate}]` for `database`, `connections{current,max}`, `statStatements{loaded,enabled}` and `slowQueries[{query,calls,meanMs,totalMs,rows}]`. |
| `POST /services/{app}/databases` | Existing connect call. For PostgreSQL the default is `perAppUser:true`: `priv` creates database and login `app_<app>` (no superuser, no create-db/role, `CONNECT` revoked from `PUBLIC` everywhere) and the app gets `${{ db.OPSBOX_APP_<APP>_DATABASE_URL }}` (+ `PGHOST/PGPORT/PGUSER/PGPASSWORD/PGDATABASE`). `perAppUser:false` or `sourceKey/asKey` keeps the shared main login. `pooled:true` links the pooler variants. `409` when the database is not running. |
| `POST /databases/{id}/apps/{app}/rotate` | New password for one app login; returns `{ok,app,rotatedAt,affectedApps}` and marks the app `needsRedeploy`. |
| `POST /databases/{id}/apps/{app}/remove` | `{dropDatabase?:bool,confirm?:string}`. Removes the login; `dropDatabase` also deletes the data and needs `confirm` equal to the database name (`409 confirmation_required`). `409` while the app still references the login. |
| `PUT /databases/{id}/apps/{app}/pooled` | `{pooled:bool}`. Re-points the app's variables at the pooler or the database; `409` if the pooler is off. |
| `PUT /databases/{id}/extensions/{name}` | `{enabled:bool,database?:string}`. Allowlist: `vector, postgis, pg_stat_statements, pgcrypto, uuid-ossp, pg_trgm, citext, hstore`. `pg_stat_statements` needs the one-time restart on databases created before this release (new ones preload it). `409` with a sentence when the image lacks `vector`/`postgis`. |
| `PUT /databases/{id}/pooler` | `{enabled:bool,mode?:"transaction"\|"session"}` → `202 {task}`. Starts/removes PgBouncer (`edoburu/pgbouncer`, internal network only, name `{id}-pooler:6432`) with a connect-only lookup login (`opsbox_pooler`) in the `postgres` database. |
| `POST /databases/{id}/upgrade` | `{version,confirm:true}` → `202 {task}`. `409 confirmation_required` without `confirm`. In `priv`: backup job snapshot first (failure cancels), `pg_dumpall`, stop old container, start the new major on a **new volume** (`opsbox-{id}-data-v<major>`), restore, compare databases/roles/table counts, then switch. Any failure removes the new container and volume and restarts the old one. The old volume is kept. |
| `POST /databases/{id}/connection-string` | `{password?,code?,app?,pooled?}`. Interactive session only. Needs a recent re-auth (password or 2FA within 5 minutes, or credentials in the body). Returns `{connectionString,login,note}` with `Cache-Control: no-store`, rate-limited. Audit: `database.connection_string.reveal` / `.denied` (names the secret key / login, never the value). |
| `ANY /tools/{service}/...` | Data-tool gateway (`adminer`, `pgweb` templates; loopback-only containers). Cookie session only (no tokens), role deployer+, owner/admin of the tool. Unsafe methods need a same-host `Origin`. The panel's cookies are never forwarded; the tool's cookies are rewritten to `Path=/tools/<service>/; SameSite=Strict`. Audit: `database.tool.open`. |

Per-app variables (`OPSBOX_APP_*`) and the pooler login (`OPSBOX_POOLER_PASSWORD`) live on the database service, are hidden from its Variables list and are never put in the database container's environment. Audit actions: `database.app.rotate`, `database.app.remove`, `database.app.pooled`, `database.extension`, `database.pooler.enable/disable`, `database.upgrade`, `database.upgrade.failed`. `GET /templates` adds `postgres-pgvector`, `postgis`, `adminer`, `pgweb`.

## Capacity guard (Spec 7.19)

All paths below use the `/api/v1` prefix and require authentication. Memory and disk values ending in `_mb` are MiB; CPU uses millicores. PSI averages use hundredths of a percent (`2150` means 21.5%).

| Request | Response / behaviour |
|---|---|
| `GET /capacity` | `{profile,reserves,reservations,allocatable,pressure,features,swap_offer_mb}`. Profile includes RAM/available/swap/free/zram, online CPUs/effective cgroup quota, cgroup v2, Docker/root, filesystem totals/free, PSI memory/CPU and pressure duration, below-minimum marker. Features include minimum/recommended values and `fit`. `swap_offer_mb` is null, 1024 or 2048. |
| `GET /capacity/check?feature=database&memory_mb=512` | Dry run returns `{request,fit,free}` with HTTP 200 even when blocked. Features: `install`, `docker`, `app`, `build`, `database`, `template`, `backup`, `image_pull`. Optional `database=true` excludes swap for a non-Postgres template/pooler; `min_memory_mb`, `recommended_memory_mb`, `cpu_millis`, `disk_mb`, and `replacing=<reservation-name>` support manifest checks, image sizes and memory edits. This does not reserve resources or authorize an override. |
| `POST /capacity/swap {size_mb:1024}` | Interactive Owner only, normal CSRF/Origin checks, no tokens. Only the current offered size is accepted; root revalidates RAM/no-swap/disk and persists swap plus swappiness=10. Returns `{ok:true}` and audits `capacity.swap`. |

Workload create/deploy/update, template/Compose install, database pooler/extension changes and manual backup admission return `409` before queueing when blocked:

```json
{"error":{"code":"conflict","message":"Needs 512 MB, 310 MB allocatable ...","capacity":{"asked":{"feature":"database","memory_mb":512},"free":{"allocatable_mb":310,"database_allocatable_mb":310},"reserves":{"os_mb":192,"opsbox_mb":128,"docker_mb":150},"reservations":[{"name":"api","memory_mb":256,"cpu_millis":0,"database":false}],"fit":{"status":"blocked","reasons":["Needs 512 MB, 310 MB allocatable ..."],"suggestions":["Lower the memory limit, or upgrade the instance.","Stop or remove api to free 256 MB.","Add a swap file for apps and builds; swap never counts for databases."]}}}}
```

`fit` is `{status:"ok"}`, `{status:"tight",reasons:[...]}`, or `{status:"blocked",reasons:[...],suggestions:[...]}`. Real errors include all reserve/accounting numbers. A tight result is allowed. Capacity-dependent operations fail closed if host/Docker/image metadata cannot be read. Default limits are persisted and enforced, not merely estimated.

A signed-in Owner (`admin` role) may add `override:true` and `capacityConfirmation:"I understand this server may run out of memory"` to a workload request. Missing/wrong confirmation is `409 confirmation_required`; Deployer and every API token receive `403 forbidden`, even if the workload would fit. Successful blocked overrides produce `capacity.override` audit entries and `overCapacity:true` on service views until reservations/pressure fit again. Cgroup v2 enforcement cannot be bypassed. Scheduled backups retry for 30 minutes, then run at lowest priority and raise a backup-capacity alert; manual runs return 409 instead.

## Grow filesystem (prompt 16h, Spec 7.8.1)

| Request | Response / behaviour |
|---|---|
| `GET /system/disk` | Any signed-in role. `{volumes:[{mount,device,fs,layout,partition,disk,fs_bytes,partition_bytes,disk_bytes,growable_bytes,growable,steps,refusal}],growpart_installed,growpart_installable}`. `layout` is `partition`, `disk`, `lvm` or `other`; `steps` is an ordered list of `install_growpart`, `grow_partition`, `resize_pv`, `resize_fs`. Volumes: `/`, plus the data and Docker directories when they are on other filesystems. `refusal` says why a volume cannot be grown (unsupported filesystem, RAID, encryption, several disks, ...); `growable:false` with no `refusal` means the disk simply has no unused space. |
| `POST /system/disk/grow {mount,confirm:"grow",password,install_tools?}` | Interactive admin only (no tokens). Wrong password is `403` and audits `system.disk.grow` as denied; wrong `confirm` is `422` (field `confirm`); an unknown `mount` is `422`; a refused or nothing-to-grow volume is `409` with the reason; a missing `growpart` without `install_tools:true` is `422` (field `install_tools`). Otherwise `202 {task}` for kind `system.disk_grow`; the task log lists the steps as they run and `result` is `{mount,before,after}` with `fs_bytes`, `partition_bytes`, `disk_bytes`, `growable_bytes`. Audit `system.disk.grow` records the steps and the before and after sizes (failures record the failing step). |

## Configuration export and import (prompts 16i / 16q, Spec 7.8.2)

| Request | Response / behaviour |
|---|---|
| `POST /config/export {password, passphrase?}` | Interactive admin only (no tokens), account password asked again. Returns the document (`Content-Type: application/json`, `Cache-Control: no-store`, `Content-Disposition: attachment; filename="opsbox-config-<host>-<YYYY-MM-DD>.json"`). Wrong password is `403` and audits `config.export` as denied; a `passphrase` shorter than 8 characters is `400` (field `passphrase`); an empty `passphrase` means "no secrets". It is a `POST` on purpose: the password and passphrase never go into a URL or an access log (the original plan said `GET`). |
| `POST /config/import/preview {password, passphrase?, document}` | Interactive admin only. Parses `opsbox-config/1`, validates shape, opens sealed secrets when a passphrase is given. Returns `{willCreate, needsInput, skipped, secretsDecrypted}` without writing anything. Unknown `format` is `400` (field `format`); wrong passphrase is `403` with a clear message and no partial write; wrong password is `403` and audits `config.import.preview` as denied. |
| `POST /config/import/apply {password, passphrase?, document}` | Interactive admin only. Same checks as preview, then `202 {task}` for kind `config.import`. Creates missing services (not deployed when secrets are missing), templates, compose projects, databases, backup destinations, alert rules/channels and firewall rules. Existing objects are never overwritten (skipped). Task `result` is `{created, skipped, needsSecret}`. Audit `config.import`. |
| `GET /system/uninstall/plan?purge=` | Interactive admin. Preview: `{purge,removed,kept,domains,adopted?,containers,warnings}`. Domains are panel + app hosts that stop on 80/443; `adopted` names a 16c web server that will be restored first. |
| `POST /system/uninstall {password,confirm:"uninstall",purge?}` | Interactive admin only (no tokens). Wrong password is `403` and audits `system.uninstall` as denied; wrong `confirm` is `400` (field `confirm`). Schedules `opsbox uninstall --yes [--purge]` via a detached `systemd-run` unit and returns `{ok,message,plan}`. The panel shows a final “opsbox is being removed” screen; apps keep running (`unless-stopped`). |

The document is `"format": "opsbox-config/1"`, checked in as a JSON Schema at [`docs/schemas/config-export-v1.json`](schemas/config-export-v1.json). Top-level keys: `exportedAt`, `exportedBy`, `opsboxVersion`, `host`, `services` (source, build, port, health path, domains, limits, exposure, owner, variables), `templates`, `composeProjects`, `databases` (engine, version, volume, pooler, linked apps; no data), `backups` (`destinations` metadata, `jobs`), `alerts` (`rules`, `quiet`, `channels` by id), `network` (panel domain, local-only, allowed addresses), `updates`, `firewall` (`null` and listed under `unavailable` when `priv` cannot read it), `users` (name, role, whether two-factor is on), `notExported` (what is left out and why) and `secrets`.

- **Without a passphrase** `secrets` is `{included:false, needsReentry:[{kind,reason,...}]}`: every secret variable, backup destination key, alert channel setting, deploy key, user password and two-factor secret, API tokens and the GitHub connection is named there and nothing secret is in the file. Variables the owner did not mark secret are carried in the open.
- **With a passphrase** `secrets` is `{included:true, encryption:{cipher:"xchacha20poly1305", kdf:"argon2id", memoryKib, iterations, parallelism, salt, nonce}, ciphertext, needsReentry}`: the secret variables, backup destination credentials and alert channel settings are sealed in `ciphertext` (Argon2id from the passphrase, XChaCha20-Poly1305, authenticated against this format name). `needsReentry` then lists only what cannot be exported at all (users, tokens, GitHub, deploy keys). The passphrase is used for this one request: not stored, logged or audited. A wrong passphrase or a changed file fails to open (`opsbox_api::export::decrypt_secrets`), it never yields partial data.
- Audit `config.export` (both ways; denied on a wrong password): `{file, includeSecrets, encrypted, services, secretsExported}`; never the passphrase or any value.

## Implemented routes

## Proxy network conflicts (prompt 03c)

Paths below are relative to `/api/v1`. All require admin access; mutations also require an interactive web session (tokens refused), CSRF and matching Origin. Unknown body fields are refused. Missing/false `confirm` returns `409 confirmation_required` in the privileged helper before changing anything.

| Request | Body / result |
|---|---|
| `GET /network/proxy` | `{network,undo}`. `network` includes `running`, `port_conflict`, `primary_http`, `primary_https`, `listen_http`, `listen_https`, `conflict_ports`, `holders:[{port,pid,process,unit?,container?:{id,name,image}}]`. `undo` holds durable `{id,target,previous_policy,was_running}` receipts. |
| `POST /network/proxy/takeover` | `{port,target:{kind:"container"\|"unit",id},confirm:true}` → `{ok:true,receipt}`. Root rechecks that the exact target currently holds the conflicted primary port. Docker: save policy, set `restart=no`, stop. Systemd: save enablement, disable, stop. Unknown/ambiguous owners and protected units fail safely. |
| `POST /network/proxy/undo` | `{id:<receipt-id>,confirm:true}` → `{ok:true,receipt}`. Yield proxy ports, restore original policy/enablement and start the previous holder. Successful receipts are consumed; retrying a consumed ID fails safely. |
| `POST /network/proxy/keep` | `{confirm:true}` → `{ok:true}`. Atomically save actual listener addresses as configured ports and apply them live, ending retries. Automatic domain HTTPS still needs 80/443. |

Mutations audit `network.proxy_takeover`, `network.proxy_undo`, `network.proxy_keep_ports`, including failed attempts. Successful takeover/Undo entries include the original restart policy/enablement; no secrets are involved. A partial host-command failure leaves a root-owned Undo receipt visible on the next read. No operation removes containers or volumes. Commands are typed `PrivOp::{PortHolders,ProxyNetwork,ProxyTakeover,ProxyUndo,ProxyKeepPorts}` with fixed argv, never shell strings. Control-socket listener changes are root-only.

A conflict leaves `/readyz` healthy and watchdog update verification successful. The proxy control `Status` returns its existing certificate/route fields plus `network`; `Ping` stays live. The 30-second retry promotes each listener without terminating accepted connections. Host ownership is best effort: a holder lookup failure does not turn a running conflict into a crash. A `health/proxy-ports` warning feeds the persistent alerts engine and Overview links to Network settings.

<!-- GENERATED_ROUTES_START -->
| Method | Path | Handler / request types |
|---|---|---|
| `GET` | `/api/v1/access` | [access::get](../crates/opsbox-api/src/access.rs) |
| `POST` | `/api/v1/access/keys` | [access::add_key](../crates/opsbox-api/src/access.rs) |
| `POST` | `/api/v1/access/keys/remove` | [access::remove_key](../crates/opsbox-api/src/access.rs) |
| `POST` | `/api/v1/access/users/{name}/lock` | [access::lock_user](../crates/opsbox-api/src/access.rs) |
| `GET` | `/api/v1/alerts` | [alerts::get](../crates/opsbox-api/src/alerts.rs) |
| `DELETE` | `/api/v1/alerts/channels/{id}` | [alerts::disconnect](../crates/opsbox-api/src/alerts.rs) |
| `PUT` | `/api/v1/alerts/channels/{id}` | [alerts::connect](../crates/opsbox-api/src/alerts.rs) |
| `POST` | `/api/v1/alerts/channels/{id}/reveal` | [alerts::reveal_channel](../crates/opsbox-api/src/alerts.rs) |
| `PUT` | `/api/v1/alerts/quiet` | [alerts::quiet](../crates/opsbox-api/src/alerts.rs) |
| `PUT` | `/api/v1/alerts/rules/{id}` | [alerts::set_rule](../crates/opsbox-api/src/alerts.rs) |
| `POST` | `/api/v1/alerts/silence` | [alerts::silence](../crates/opsbox-api/src/alerts.rs) |
| `POST` | `/api/v1/alerts/test` | [alerts::test](../crates/opsbox-api/src/alerts.rs) |
| `GET` | `/api/v1/attention` | [metrics::get_attention](../crates/opsbox-api/src/metrics.rs) |
| `POST` | `/api/v1/attention/read-all` | [metrics::read_all_attention](../crates/opsbox-api/src/metrics.rs) |
| `POST` | `/api/v1/attention/{id}/read` | [metrics::read_attention](../crates/opsbox-api/src/metrics.rs) |
| `POST` | `/api/v1/attention/{id}/snooze` | [metrics::snooze_attention](../crates/opsbox-api/src/metrics.rs) |
| `GET` | `/api/v1/audit` | [security::list_audit](../crates/opsbox-api/src/security.rs) |
| `GET` | `/api/v1/audit/export.csv` | [security::export_audit_csv](../crates/opsbox-api/src/security.rs) |
| `POST` | `/api/v1/auth/login` | [auth::login](../crates/opsbox-api/src/auth.rs) |
| `POST` | `/api/v1/auth/logout` | [auth::logout](../crates/opsbox-api/src/auth.rs) |
| `POST` | `/api/v1/auth/reauth` | [secrets::reauth](../crates/opsbox-api/src/secrets.rs) |
| `POST` | `/api/v1/auth/sessions/revoke-all` | [auth::revoke_all](../crates/opsbox-api/src/auth.rs) |
| `POST` | `/api/v1/auth/totp/confirm` | [auth::totp_confirm](../crates/opsbox-api/src/auth.rs) |
| `POST` | `/api/v1/auth/totp/enroll` | [auth::totp_enroll](../crates/opsbox-api/src/auth.rs) |
| `GET` | `/api/v1/backups` | [backups::overview](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/check` | [backups::check](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/destinations` | [backups::create_destination](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/destinations/test` | [backups::test_destination](../crates/opsbox-api/src/backups.rs) |
| `DELETE` | `/api/v1/backups/destinations/{id}` | [backups::delete_destination](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/destinations/{id}/default` | [backups::set_default_destination](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/destinations/{id}/reveal` | [backups::reveal_destination](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/destinations/{id}/test` | [backups::test_saved_destination](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/jobs` | [backups::create_job](../crates/opsbox-api/src/backups.rs) |
| `DELETE` | `/api/v1/backups/jobs/{id}` | [backups::delete_job](../crates/opsbox-api/src/backups.rs) |
| `PATCH` | `/api/v1/backups/jobs/{id}` | [backups::update_job](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/jobs/{id}/run` | [backups::run_job](../crates/opsbox-api/src/backups.rs) |
| `GET` | `/api/v1/backups/providers` | [backups::list_providers](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/prune` | [backups::prune](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/recovery-key` | [backups::recovery_key](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/restore-test` | [backups::run_restore_test_all](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/run` | [backups::run_all](../crates/opsbox-api/src/backups.rs) |
| `GET` | `/api/v1/backups/snapshots` | [backups::list_snapshots](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/snapshots/{id}/restore` | [backups::restore_snapshot](../crates/opsbox-api/src/backups.rs) |
| `POST` | `/api/v1/backups/snapshots/{id}/test` | [backups::test_snapshot_restore](../crates/opsbox-api/src/backups.rs) |
| `GET` | `/api/v1/capacity` | [capacity::get](../crates/opsbox-api/src/capacity.rs) |
| `GET` | `/api/v1/capacity/check` | [capacity::check](../crates/opsbox-api/src/capacity.rs) |
| `POST` | `/api/v1/capacity/swap` | [capacity::swap](../crates/opsbox-api/src/capacity.rs) |
| `PUT` | `/api/v1/cleanup/auto` | [cleanup::set_auto](../crates/opsbox-api/src/cleanup.rs) |
| `POST` | `/api/v1/cleanup/run` | [cleanup::run](../crates/opsbox-api/src/cleanup.rs) |
| `GET` | `/api/v1/cleanup/scan` | [cleanup::scan](../crates/opsbox-api/src/cleanup.rs) |
| `GET` | `/api/v1/cleanup/volumes` | [cleanup::volume_sizes](../crates/opsbox-api/src/cleanup.rs) |
| `POST` | `/api/v1/compose` | [templates::deploy_compose](../crates/opsbox-api/src/templates.rs) |
| `DELETE` | `/api/v1/compose/{stack}` | [templates::delete_compose_stack](../crates/opsbox-api/src/templates.rs) |
| `POST` | `/api/v1/config/export` | [export::export](../crates/opsbox-api/src/export.rs) |
| `POST` | `/api/v1/config/import/apply` | [import::apply](../crates/opsbox-api/src/import.rs) |
| `POST` | `/api/v1/config/import/preview` | [import::preview](../crates/opsbox-api/src/import.rs) |
| `GET` | `/api/v1/databases/{id}` | [databases::overview](../crates/opsbox-api/src/databases.rs) |
| `PUT` | `/api/v1/databases/{id}/apps/{app}/pooled` | [databases::set_app_pooled](../crates/opsbox-api/src/databases.rs) |
| `POST` | `/api/v1/databases/{id}/apps/{app}/remove` | [databases::remove_app](../crates/opsbox-api/src/databases.rs) |
| `POST` | `/api/v1/databases/{id}/apps/{app}/rotate` | [databases::rotate_app](../crates/opsbox-api/src/databases.rs) |
| `POST` | `/api/v1/databases/{id}/connection-string` | [databases::reveal_connection_string](../crates/opsbox-api/src/databases.rs) |
| `PUT` | `/api/v1/databases/{id}/extensions/{name}` | [databases::set_extension](../crates/opsbox-api/src/databases.rs) |
| `PUT` | `/api/v1/databases/{id}/pooler` | [databases::set_pooler](../crates/opsbox-api/src/databases.rs) |
| `POST` | `/api/v1/databases/{id}/upgrade` | [databases::upgrade](../crates/opsbox-api/src/databases.rs) |
| `GET` | `/api/v1/deploys/{id}/log` | [deploy::deploy_log](../crates/opsbox-api/src/deploy.rs) |
| `GET` | `/api/v1/docker/containers` | [docker::list_containers](../crates/opsbox-api/src/docker.rs) |
| `GET` | `/api/v1/docker/containers/{id}` | [docker::inspect_container](../crates/opsbox-api/src/docker.rs) |
| `GET` | `/api/v1/docker/containers/{id}/logs` | [docker::container_logs](../crates/opsbox-api/src/docker.rs) |
| `POST` | `/api/v1/docker/containers/{id}/{action}` | [docker::container_action](../crates/opsbox-api/src/docker.rs) |
| `GET` | `/api/v1/docker/images` | [docker::list_images](../crates/opsbox-api/src/docker.rs) |
| `POST` | `/api/v1/docker/install` | [docker::install](../crates/opsbox-api/src/docker.rs) |
| `GET` | `/api/v1/docker/networks` | [docker::list_networks](../crates/opsbox-api/src/docker.rs) |
| `GET` | `/api/v1/docker/status` | [docker::status](../crates/opsbox-api/src/docker.rs) |
| `GET` | `/api/v1/docker/volumes` | [docker::list_volumes](../crates/opsbox-api/src/docker.rs) |
| `POST` | `/api/v1/domains/{domain}/adopted-tls` | [network::adopted_tls](../crates/opsbox-api/src/network.rs) |
| `DELETE` | `/api/v1/domains/{name}` | [domains::remove_domain](../crates/opsbox-api/src/domains.rs) |
| `PATCH` | `/api/v1/domains/{name}` | [domains::update_domain](../crates/opsbox-api/src/domains.rs) |
| `POST` | `/api/v1/domains/{name}/certificate` | [domains::issue_cert](../crates/opsbox-api/src/domains.rs) |
| `POST` | `/api/v1/domains/{name}/check` | [domains::check_dns](../crates/opsbox-api/src/domains.rs) |
| `POST` | `/api/v1/domains/{name}/primary` | [domains::make_primary](../crates/opsbox-api/src/domains.rs) |
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
| `DELETE` | `/api/v1/github` | [github::disconnect](../crates/opsbox-api/src/github.rs) |
| `GET` | `/api/v1/github` | [github::get_status](../crates/opsbox-api/src/github.rs) |
| `GET` | `/api/v1/github/callback` | [github::manifest_callback](../crates/opsbox-api/src/github.rs) |
| `POST` | `/api/v1/github/complete` | [github::complete_manifest](../crates/opsbox-api/src/github.rs) |
| `GET` | `/api/v1/github/deploy-keys` | [github::list_deploy_keys](../crates/opsbox-api/src/github.rs) |
| `POST` | `/api/v1/github/deploy-keys` | [github::create_deploy_key](../crates/opsbox-api/src/github.rs) |
| `DELETE` | `/api/v1/github/deploy-keys/{id}` | [github::delete_deploy_key](../crates/opsbox-api/src/github.rs) |
| `POST` | `/api/v1/github/installation` | [github::sync_installation](../crates/opsbox-api/src/github.rs) |
| `POST` | `/api/v1/github/manifest` | [github::start_manifest](../crates/opsbox-api/src/github.rs) |
| `POST` | `/api/v1/github/public-repo` | [github::inspect_public_repo](../crates/opsbox-api/src/github.rs) |
| `GET` | `/api/v1/github/repos` | [github::list_repos](../crates/opsbox-api/src/github.rs) |
| `GET` | `/api/v1/github/setup` | [github::setup_callback](../crates/opsbox-api/src/github.rs) |
| `POST` | `/api/v1/github/webhook` | [github::webhook](../crates/opsbox-api/src/github.rs) |
| `POST` | `/api/v1/github/webhook-secret/reveal` | [github::reveal_webhook_secret](../crates/opsbox-api/src/github.rs) |
| `POST` | `/api/v1/github/webhook/sync` | [github::sync_webhook](../crates/opsbox-api/src/github.rs) |
| `GET` | `/api/v1/host` | [host::get_host](../crates/opsbox-api/src/host.rs) |
| `POST` | `/api/v1/invites/claim` | [users::claim](../crates/opsbox-api/src/users.rs) |
| `GET` | `/api/v1/invites/{token}` | [users::info](../crates/opsbox-api/src/users.rs) |
| `GET` | `/api/v1/jobs` | [system::get_jobs](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/jobs` | [system::add_job](../crates/opsbox-api/src/system.rs) |
| `DELETE` | `/api/v1/jobs/{id}` | [system::remove_job](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/jobs/{id}/run` | [system::run_job](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/metrics` | [metrics::get_metrics](../crates/opsbox-api/src/metrics.rs) |
| `GET` | `/api/v1/network/proxy` | [network::get_network](../crates/opsbox-api/src/network.rs) |
| `POST` | `/api/v1/network/proxy/discover` | [network::discover](../crates/opsbox-api/src/network.rs) |
| `POST` | `/api/v1/network/proxy/keep` | [network::keep](../crates/opsbox-api/src/network.rs) |
| `POST` | `/api/v1/network/proxy/move` | [network::move_web](../crates/opsbox-api/src/network.rs) |
| `POST` | `/api/v1/network/proxy/takeover` | [network::takeover](../crates/opsbox-api/src/network.rs) |
| `POST` | `/api/v1/network/proxy/undo` | [network::undo](../crates/opsbox-api/src/network.rs) |
| `POST` | `/api/v1/network/proxy/undo-move` | [network::undo_move](../crates/opsbox-api/src/network.rs) |
| `GET` | `/api/v1/panel` | [panel::status](../crates/opsbox-api/src/panel.rs) |
| `POST` | `/api/v1/panel/certificate` | [panel::issue_cert](../crates/opsbox-api/src/panel.rs) |
| `POST` | `/api/v1/panel/dns-check` | [panel::check_dns](../crates/opsbox-api/src/panel.rs) |
| `DELETE` | `/api/v1/panel/domain` | [panel::remove_domain](../crates/opsbox-api/src/panel.rs) |
| `POST` | `/api/v1/panel/local-only` | [panel::set_local_only](../crates/opsbox-api/src/panel.rs) |
| `GET` | `/api/v1/projects/{id}` | [deploy::get_project](../crates/opsbox-api/src/deploy.rs) |
| `GET` | `/api/v1/recovery` | [recovery::get](../crates/opsbox-api/src/recovery.rs) |
| `GET` | `/api/v1/security` | [security::get](../crates/opsbox-api/src/security.rs) |
| `PUT` | `/api/v1/security/allow-ips` | [security::set_allow_ips](../crates/opsbox-api/src/security.rs) |
| `POST` | `/api/v1/security/checks/{id}/fix` | [security::fix](../crates/opsbox-api/src/security.rs) |
| `POST` | `/api/v1/services` | [deploy::create_service](../crates/opsbox-api/src/deploy.rs) |
| `DELETE` | `/api/v1/services/{id}` | [deploy::delete_service](../crates/opsbox-api/src/deploy.rs) |
| `GET` | `/api/v1/services/{id}` | [deploy::get_service](../crates/opsbox-api/src/deploy.rs) |
| `PATCH` | `/api/v1/services/{id}` | [deploy::update_service](../crates/opsbox-api/src/deploy.rs) |
| `POST` | `/api/v1/services/{id}/databases` | [deploy::connect_database](../crates/opsbox-api/src/deploy.rs) |
| `DELETE` | `/api/v1/services/{id}/databases/{db}` | [deploy::disconnect_database](../crates/opsbox-api/src/deploy.rs) |
| `POST` | `/api/v1/services/{id}/deploys` | [deploy::redeploy](../crates/opsbox-api/src/deploy.rs) |
| `POST` | `/api/v1/services/{id}/deploys/{deploy_id}/rollback` | [deploy::rollback](../crates/opsbox-api/src/deploy.rs) |
| `GET` | `/api/v1/services/{id}/domains` | [domains::get_domains](../crates/opsbox-api/src/domains.rs) |
| `POST` | `/api/v1/services/{id}/domains` | [domains::add_domain](../crates/opsbox-api/src/domains.rs) |
| `PUT` | `/api/v1/services/{id}/exposure` | [templates::set_service_exposure](../crates/opsbox-api/src/templates.rs) |
| `POST` | `/api/v1/services/{id}/rotate` | [templates::rotate_database](../crates/opsbox-api/src/templates.rs) |
| `DELETE` | `/api/v1/services/{id}/vars/{key}` | [deploy::delete_var](../crates/opsbox-api/src/deploy.rs) |
| `PUT` | `/api/v1/services/{id}/vars/{key}` | [deploy::put_var](../crates/opsbox-api/src/deploy.rs) |
| `POST` | `/api/v1/services/{id}/vars/{key}/reveal` | [deploy::reveal_var](../crates/opsbox-api/src/deploy.rs) |
| `GET` | `/api/v1/session` | [auth::session](../crates/opsbox-api/src/auth.rs) |
| `GET` | `/api/v1/settings` | [tokens::get_settings](../crates/opsbox-api/src/tokens.rs) |
| `POST` | `/api/v1/setup/admin` | [setup::admin](../crates/opsbox-api/src/setup.rs) |
| `POST` | `/api/v1/setup/claim` | [setup::claim](../crates/opsbox-api/src/setup.rs) |
| `GET` | `/api/v1/setup/status` | [setup::status](../crates/opsbox-api/src/setup.rs) |
| `POST` | `/api/v1/setup/totp/confirm` | [setup::confirm](../crates/opsbox-api/src/setup.rs) |
| `POST` | `/api/v1/ssh/emergency` | [system::ssh_emergency](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/stream/containers/{id}/logs` | [docker::stream_logs](../crates/opsbox-api/src/docker.rs) |
| `GET` | `/api/v1/stream/deploys/{id}` | [deploy::stream_deploy_log](../crates/opsbox-api/src/deploy.rs) |
| `GET` | `/api/v1/stream/journal` | [system::stream_journal](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/stream/metrics` | [metrics::stream](../crates/opsbox-api/src/metrics.rs) |
| `GET` | `/api/v1/stream/terminal` | [terminal::stream](../crates/opsbox-api/src/terminal.rs) |
| `GET` | `/api/v1/system/disk` | [system::get_disk](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/system/disk/grow` | [system::grow_disk](../crates/opsbox-api/src/system.rs) |
| `PUT` | `/api/v1/system/hostname` | [system::set_hostname](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/system/journal` | [system::get_journal](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/system/reboot` | [system::reboot](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/system/reboot/cancel` | [system::reboot_cancel](../crates/opsbox-api/src/system.rs) |
| `PUT` | `/api/v1/system/swap` | [system::set_swap](../crates/opsbox-api/src/system.rs) |
| `PUT` | `/api/v1/system/timezone` | [system::set_timezone](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/system/uninstall` | [uninstall::exec](../crates/opsbox-api/src/uninstall.rs) |
| `GET` | `/api/v1/system/uninstall/plan` | [uninstall::plan](../crates/opsbox-api/src/uninstall.rs) |
| `GET` | `/api/v1/system/units` | [system::get_units](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/system/units/{name}/{action}` | [system::unit_action](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/system/updates` | [system::get_updates](../crates/opsbox-api/src/system.rs) |
| `PUT` | `/api/v1/system/updates/auto` | [system::set_auto_updates](../crates/opsbox-api/src/system.rs) |
| `POST` | `/api/v1/system/updates/install` | [system::install_updates](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/tasks/{id}` | [system::get_task](../crates/opsbox-api/src/system.rs) |
| `GET` | `/api/v1/templates` | [templates::list_templates](../crates/opsbox-api/src/templates.rs) |
| `POST` | `/api/v1/templates/{id}/instances` | [templates::create_template_instance](../crates/opsbox-api/src/templates.rs) |
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
| `PATCH` | `/api/v1/users/{name}` | [users::update](../crates/opsbox-api/src/users.rs) |
| `GET` | `/healthz` | [health::healthz](../crates/opsbox-api/src/health.rs) |
| `GET` | `/readyz` | [health::readyz](../crates/opsbox-api/src/health.rs) |
| `GET` | `/tools/{service}` | [tools::root](../crates/opsbox-api/src/tools.rs) |
| `DELETE` | `/tools/{service}/` | [tools::index](../crates/opsbox-api/src/tools.rs) |
| `GET` | `/tools/{service}/` | [tools::index](../crates/opsbox-api/src/tools.rs) |
| `PATCH` | `/tools/{service}/` | [tools::index](../crates/opsbox-api/src/tools.rs) |
| `POST` | `/tools/{service}/` | [tools::index](../crates/opsbox-api/src/tools.rs) |
| `PUT` | `/tools/{service}/` | [tools::index](../crates/opsbox-api/src/tools.rs) |
| `DELETE` | `/tools/{service}/{*rest}` | [tools::gateway](../crates/opsbox-api/src/tools.rs) |
| `GET` | `/tools/{service}/{*rest}` | [tools::gateway](../crates/opsbox-api/src/tools.rs) |
| `PATCH` | `/tools/{service}/{*rest}` | [tools::gateway](../crates/opsbox-api/src/tools.rs) |
| `POST` | `/tools/{service}/{*rest}` | [tools::gateway](../crates/opsbox-api/src/tools.rs) |
| `PUT` | `/tools/{service}/{*rest}` | [tools::gateway](../crates/opsbox-api/src/tools.rs) |
<!-- GENERATED_ROUTES_END -->

## Future phases (sample UI)

Database management console: Prompt 16. WireGuard, desktop device authentication and other-server terminals: Phase 6. Unimplemented UI actions remain clearly marked in `web/js/api.js` and retain sample-data labels.

### Existing website adoption

All adoption endpoints require an interactive administrator session; mutations use the normal CSRF and same-Origin checks. Tokens cannot discover or mutate adoption. `GET /api/v1/network/proxy` includes `moves` (public durable receipts) and `adopted` alongside existing network/takeover state.

| Endpoint | Request and response |
|---|---|
| `POST /api/v1/network/proxy/discover` | `{target:{kind:"container"|"unit",id}}` → `{plan}`. Read-only inspection: holder/image/Compose project, working directory/config files, private ports, discovered domains (`domain`, `served_by`, `has_certificate`, Unix `certificate_expiry`, `reachable_now`), masked command/directive diffs, support explanation and required typed confirmation. |
| `POST /api/v1/network/proxy/move` | `{target,domains:["old.example.com"],everything_else:false,confirmation:"move"}` → `202 {task}`. Plain containers require `confirmation:"recreate <name>"`. Discovered names remain included. Ownership and configuration are re-read in priv. |
| `POST /api/v1/network/proxy/undo-move` | `{id,confirm:true}` → `202 {task}`. Restores from the root receipt and verifies original responses. Incomplete recovery retains Undo. |
| `POST /api/v1/domains/{domain}/adopted-tls` | → `202 {task}`. Import a valid existing certificate, otherwise issue one; change routing only when ready. Restore passthrough on response regression. |

Poll `GET /api/v1/tasks/{id}` for live `log` steps and final `status/error/result`. Move results contain `receipt:{id,holder,files,http_port,https_port,steps,active}`; failed moves report the failed step and rollback result and are audited with public receipt details. No secret configuration, certificate key or environment value is returned. Per-service domain responses include an administrator-only `adopted` list with `terminate_tls` so the panel can label passthrough domains and offer TLS handoff. The `REAL` API map includes `discoverWeb`, `moveWeb`, `undoWebMove` and `adoptedTls`.
