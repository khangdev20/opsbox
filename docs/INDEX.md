# Project index

Start here to locate a crate, then open its symbol map or entry file.
Regenerate with `python3 scripts/gen-index.py`; verify with
`python3 scripts/gen-index.py --check`. Edit prose outside the generated markers.

<!-- gen:start -->
## Task maps

- [HTTP and WebSocket routes](index/routes.md): handlers, permissions, API call links and documentation drift.
- [Privileged operations](index/privops.md): request types, helper dispatch, client methods and timeouts.
- [Store](index/store.md): current columns, indexes, migrations, Store methods and settings keys.
- [UI](index/ui.md): pages, hashes, API functions, styles and shared helpers; mock backlog first.

## Layout

| Folder | Contents |
|---|---|
| [crates/](../crates/) | Rust workspace: binary, privileged helper and shared libraries. |
| [web/](../web/) | Embedded vanilla-JS panel, styles, assets and UI checks. |
| [site/](../site/) | Public landing site. |
| [design/](../design/) | Panel design references and preview generators. |
| [docs/](../docs/) | Specs, decisions, status, prompts and generated navigation maps. |
| [scripts/](../scripts/) | Repository maintenance and verification scripts. |
| [packaging/](../packaging/) | System service units and distribution packaging. |
| [tools/](../tools/) | Build, API inventory, screenshot and VM verification tools. |

## Crates

LOC counts nonblank Rust code lines, excluding comments/literals and test-only items.
Dependencies include normal/build/target tables; dev dependencies are omitted.

| Crate / symbols | Purpose | Internal dependencies | Non-test LOC | Entry |
|---|---|---|---:|---|
| [opsbox](index/symbols/opsbox.md) | The `opsbox` binary: one executable, many roles (docs/BACKEND_SPEC.md section 2). | opsbox-api, opsbox-auth, opsbox-backup, opsbox-capacity, opsbox-priv, opsbox-proto, opsbox-proxy, opsbox-store, opsbox-update, opsbox-version, opsbox-watchdog | 2,970 | [crates/opsbox/src/main.rs](../crates/opsbox/src/main.rs) |
| [opsbox-alerts](index/symbols/opsbox-alerts.md) | Persistent alert state machine and TLS-only outbound channels (spec 7.13). | opsbox-proto | 590 | [crates/opsbox-alerts/src/lib.rs](../crates/opsbox-alerts/src/lib.rs) |
| [opsbox-api](index/symbols/opsbox-api.md) | The HTTP server: router, request ids, the error format, health endpoints and the | opsbox-alerts, opsbox-auth, opsbox-backup, opsbox-capacity, opsbox-deploy, opsbox-metrics, opsbox-priv, opsbox-proto, opsbox-proxy, opsbox-store, opsbox-update, opsbox-version | 23,829 | [crates/opsbox-api/src/lib.rs](../crates/opsbox-api/src/lib.rs) |
| [opsbox-auth](index/symbols/opsbox-auth.md) | Panel accounts: passwords, two-factor sign-in, sessions, lockout, first-run setup. | opsbox-priv, opsbox-proto, opsbox-store | 1,376 | [crates/opsbox-auth/src/lib.rs](../crates/opsbox-auth/src/lib.rs) |
| [opsbox-backup](index/symbols/opsbox-backup.md) | restic orchestration, backup schedules, retention policies, and database dump specs | opsbox-proto | 1,826 | [crates/opsbox-backup/src/lib.rs](../crates/opsbox-backup/src/lib.rs) |
| [opsbox-capacity](index/symbols/opsbox-capacity.md) | Pure admission policy. Host IO belongs exclusively to opsbox-priv. | — | 335 | [crates/opsbox-capacity/src/lib.rs](../crates/opsbox-capacity/src/lib.rs) |
| [opsbox-deploy](index/symbols/opsbox-deploy.md) | GitHub integration, build detection, deploy key generation, and webhook verification (Phase 3). | opsbox-capacity, opsbox-proto | 2,837 | [crates/opsbox-deploy/src/lib.rs](../crates/opsbox-deploy/src/lib.rs) |
| [opsbox-docker](index/symbols/opsbox-docker.md) | Docker Engine client wrapper over `/var/run/docker.sock` via `bollard` | opsbox-capacity, opsbox-proto | 2,362 | [crates/opsbox-docker/src/lib.rs](../crates/opsbox-docker/src/lib.rs) |
| [opsbox-metrics](index/symbols/opsbox-metrics.md) | Machine metrics: counters to rates, the live ring, SQLite rollups and the | opsbox-priv, opsbox-proto, opsbox-store | 967 | [crates/opsbox-metrics/src/lib.rs](../crates/opsbox-metrics/src/lib.rs) |
| [opsbox-net](index/symbols/opsbox-net.md) | WireGuard interface, peers, network rules and pairing (Phase 6). | — | 0 | [crates/opsbox-net/src/lib.rs](../crates/opsbox-net/src/lib.rs) |
| [opsbox-priv](index/symbols/opsbox-priv.md) | Privileged helper: runs typed system operations as root for the unprivileged panel. | opsbox-backup, opsbox-capacity, opsbox-deploy, opsbox-docker, opsbox-proto, opsbox-store, opsbox-update, opsbox-version | 22,042 | [crates/opsbox-priv/src/lib.rs](../crates/opsbox-priv/src/lib.rs) |
| [opsbox-proto](index/symbols/opsbox-proto.md) | Shared types for opsbox. | opsbox-capacity | 2,926 | [crates/opsbox-proto/src/lib.rs](../crates/opsbox-proto/src/lib.rs) |
| [opsbox-proxy](index/symbols/opsbox-proxy.md) | Reverse proxy, routing table, ACME HTTP-01, DNS check, and certificates (Phase 4). | opsbox-priv, opsbox-proto, opsbox-version | 3,294 | [crates/opsbox-proxy/src/lib.rs](../crates/opsbox-proxy/src/lib.rs) |
| [opsbox-store](index/symbols/opsbox-store.md) | SQLite storage for opsbox. | opsbox-proto | 4,921 | [crates/opsbox-store/src/lib.rs](../crates/opsbox-store/src/lib.rs) |
| [opsbox-sys](index/symbols/opsbox-sys.md) | System facts and actions used by priv: packages, systemd, journal, timers, firewall, users, ssh, swap, disks (Phase 1). | — | 0 | [crates/opsbox-sys/src/lib.rs](../crates/opsbox-sys/src/lib.rs) |
| [opsbox-update](index/symbols/opsbox-update.md) | Self-update, signature verification and version slots (Phase 2). | opsbox-version | 802 | [crates/opsbox-update/src/lib.rs](../crates/opsbox-update/src/lib.rs) |
| [opsbox-version](index/symbols/opsbox-version.md) | The version this build reports: `opsbox --version`, `/health`, `/updates/self`, | — | 47 | [crates/opsbox-version/src/lib.rs](../crates/opsbox-version/src/lib.rs) |
| [opsbox-watchdog](index/symbols/opsbox-watchdog.md) | `opsbox-watchdog`: independent supervisor, health probe loop, auto-rollback | opsbox-proto, opsbox-store, opsbox-version | 3,538 | [crates/opsbox-watchdog/src/lib.rs](../crates/opsbox-watchdog/src/lib.rs) |

## Dependency graph

Each indented row is a direct dependency of the crate above it; read upward for callers.

```text
opsbox
  opsbox-api
  opsbox-auth
  opsbox-backup
  opsbox-capacity
  opsbox-priv
  opsbox-proto
  opsbox-proxy
  opsbox-store
  opsbox-update
  opsbox-version
  opsbox-watchdog
opsbox-alerts
  opsbox-proto
opsbox-api
  opsbox-alerts
  opsbox-auth
  opsbox-backup
  opsbox-capacity
  opsbox-deploy
  opsbox-metrics
  opsbox-priv
  opsbox-proto
  opsbox-proxy
  opsbox-store
  opsbox-update
  opsbox-version
opsbox-auth
  opsbox-priv
  opsbox-proto
  opsbox-store
opsbox-backup
  opsbox-proto
opsbox-capacity
opsbox-deploy
  opsbox-capacity
  opsbox-proto
opsbox-docker
  opsbox-capacity
  opsbox-proto
opsbox-metrics
  opsbox-priv
  opsbox-proto
  opsbox-store
opsbox-net
opsbox-priv
  opsbox-backup
  opsbox-capacity
  opsbox-deploy
  opsbox-docker
  opsbox-proto
  opsbox-store
  opsbox-update
  opsbox-version
opsbox-proto
  opsbox-capacity
opsbox-proxy
  opsbox-priv
  opsbox-proto
  opsbox-version
opsbox-store
  opsbox-proto
opsbox-sys
opsbox-update
  opsbox-version
opsbox-version
opsbox-watchdog
  opsbox-proto
  opsbox-store
  opsbox-version
```

## Runtime roles

All roles start in the `opsbox` crate (`crates/opsbox/src/main.rs`).
`serve` delegates system actions to typed operations in the root helper.

| Role | Startup | Runtime crate |
|---|---|---|
| `serve` | [crates/opsbox/src/cmd/serve.rs](../crates/opsbox/src/cmd/serve.rs) | opsbox-api |
| `priv (root helper)` | [crates/opsbox/src/cmd/privileged.rs](../crates/opsbox/src/cmd/privileged.rs) | opsbox-priv |
| `proxy` | [crates/opsbox/src/cmd/proxy.rs](../crates/opsbox/src/cmd/proxy.rs) | opsbox-proxy |
| `watchdog` | [crates/opsbox/src/cmd/watchdog.rs](../crates/opsbox/src/cmd/watchdog.rs) | opsbox-watchdog |
<!-- gen:end -->
