# 01. File access race (Phase 1b)

Spec: 7.2, 7.10. Files: `crates/opsbox-priv/src/ops/files.rs`, `crates/opsbox-api/src/files.rs`.

Problem: paths are canonicalized and checked, then reopened or renamed by pathname. A concurrent symlink or parent-directory swap can make root write outside the jail or onto a denied path.

Do:
- Open the jail root once, resolve every path relative to it with `openat2(RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)` on Linux 5.6+, else walk component by component with `O_NOFOLLOW | O_DIRECTORY` on each parent. Use `rustix`.
- Read, write, mkdir, chmod, delete, move-to-trash, upload and download all operate on directory file descriptors (`*at` calls). Writes: temp file with `O_EXCL` in the parent fd, `fsync`, `renameat` in the same fd.
- Check the deny list on the opened object (device and inode via `fstat`), not on the string.
- Tests: concurrent parent-directory swap during read, write and delete never escapes the jail; denied paths via symlink and `../` fail; staging cannot escape `/var/lib/opsbox/xfer`.

Out of scope: per-service deployer jails (prompt 09).
