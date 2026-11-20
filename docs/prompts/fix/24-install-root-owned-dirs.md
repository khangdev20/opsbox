# Fix 24. apps/, xfer/, trash/ created at runtime inside a group-writable dir

Follow `docs/prompts/fix/README.md`. Re-check file refs with `rg` (they may have drifted).

## Problem
`/var/lib/opsbox` is `1770 root:opsbox`; `ensure_service_dir` (`ops/deploy.rs` ~303) and `ensure_xfer` (`api/files.rs` ~421) use path-based `create_dir_all`/`set_permissions`, so a planted symlink redirects root.

## Steps
1. Test first: replace `apps` with a symlink to a temp dir; `ensure_service_dir` must fail and the target stays untouched.
2. Create the three dirs in `install.sh` (and the update path) with fixed owner and mode; refuse if one is a symlink.
3. In `priv`, open the root dirs with `O_NOFOLLOW|O_DIRECTORY`, check the owner, then use `openat` for children.

## Done when
The symlink test fails safely; fresh install and upgrade still work.

Commit `fix 24: apps/, xfer/, trash/ created at runtime inside a group-writable dir`, push `dev`, archive this file.
