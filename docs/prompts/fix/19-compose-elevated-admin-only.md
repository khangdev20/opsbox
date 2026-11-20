# Fix 19. Deployer must not reach root through Compose

Follow `docs/prompts/fix/README.md`. Re-check file refs with `rg` (they may have drifted).

## Problem
A Deployer can deploy Compose with `privileged`, host mounts (`/`, docker.sock), `cap_add`, `pid/ipc: host`, `devices`, `security_opt`, `userns_mode`, `sysctls`, `volumes_from`, and send `confirm: true` themselves. Files: `crates/opsbox-api/src/templates.rs` (~497, ~572), `crates/opsbox-deploy/src/compose.rs` (~548-650).

## Steps
1. Test first: as Deployer each of those keys gets 403 'needs an administrator'; a plain Compose (image, ports, named volume, env) still deploys; as Admin the typed confirmation still works.
2. Extend the risk detector to all keys above (also long-form bind mounts); parse failure counts as risky.
3. In the handler: elevated risk means Admin only; ignore `confirm` from non-Admin. Apply the same check to redeploy, import and template install.

## Done when
Deployer cannot start an elevated Compose by any route.

Commit `fix 19: Deployer must not reach root through Compose`, push `dev`, archive this file.
