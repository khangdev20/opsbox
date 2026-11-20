# Fix 21. Safe Mode (root) should not be open to everyone by default

Follow `docs/prompts/fix/README.md`. Re-check file refs with `rg` (they may have drifted).

## Problem
Safe Mode binds `0.0.0.0:8444` as root and its TLS key is chowned to `opsbox` (`crates/opsbox-watchdog/src/lib.rs` ~73, `server.rs` ~124). Do NOT force loopback: it exists for remote recovery.

## Steps
1. Test first: `[watchdog] allow_ips` refuses other addresses at accept time; empty list keeps today's behaviour; `--local-only` still binds loopback.
2. Give the watchdog its own TLS key (root 0600, not chowned to `opsbox`).
3. Print the allowlist state and certificate fingerprint in `opsbox status` and the installer summary.

## Done when
Allowlist works, the key is root-only, fingerprint is shown.

Commit `fix 21: Safe Mode (root) should not be open to everyone by default`, push `dev`, archive this file.
