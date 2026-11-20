# Fix 28. Root helper runs a staged binary without checking its signature

Follow `docs/prompts/fix/README.md`. Re-check file refs with `rg` (they may have drifted).

## Problem
`SelfUpdateInstall` (`ops/update.rs` ~27-213) checks only that the file is regular and in `xfer`; minisign and SHA-256 are verified in `serve`. The file is also checked then executed by path (TOCTOU).

## Steps
1. Test first: an unsigned fake binary (a script that writes a marker file) is refused and the marker does not exist.
2. Verify SHA-256 and minisign inside `priv` with the embedded public key, copy the verified bytes to a root-owned 0700 temp file, run `selftest` from that copy.
3. Update the proto and the `serve` caller in the same commit; keep rollback working.

## Done when
Unsigned or modified binaries are refused in `priv`; normal update still works (owner check on the VM).

Commit `fix 28: Root helper runs a staged binary without checking its signature`, push `dev`, archive this file.
