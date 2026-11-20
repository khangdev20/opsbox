# Fix 23. File manager can rewrite opsbox's own config

Follow `docs/prompts/fix/README.md`. Re-check file refs with `rg` (they may have drifted).

## Problem
`files.rs` (~76-103) denies `master.key` but not `/etc/opsbox/config.toml` (master key path, allowed uids, socket mode, listen).

## Steps
1. Test first: write, delete and move on `/etc/opsbox/` and its contents are denied, including via symlink and `..`.
2. Add `/etc/opsbox/` and the opsbox unit files (`/etc/systemd/system/opsbox*`) to the no-modify list.

## Done when
All modify operations on opsbox config and units are refused with a clear message.

Commit `fix 23: File manager can rewrite opsbox's own config`, push `dev`, archive this file.
