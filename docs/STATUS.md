# Status

Last completed prompt: `docs/prompts/archive/16h-grow-filesystem.md` (grow the filesystem after the disk was enlarged). Not committed, not tagged (per session request). Prior: `16g` (Capacity tab), `16f` (tag `v0.1.0-alpha.20`), plus unreleased `03f` / `16e` / `03e` / `03d` / `16g` work still on `dev`.

## What works (16h)
- **`GET /api/v1/system/disk`** (any signed-in role): for `/` and the data and Docker directories when they are on other filesystems: device, partition, disk, filesystem, layout (`partition`, `disk`, `lvm`, `other`), `fs_bytes`, `partition_bytes`, `disk_bytes`, `growable_bytes`, the steps Grow would run, and a plain-language `refusal`. Read from `findmnt -J -T`, `lsblk -J -b` and `/sys/class/block/<part>/start` (`crates/opsbox-priv/src/ops/disk.rs`; types in `opsbox-proto/src/disk.rs`).
- **`POST /api/v1/system/disk/grow`** (interactive admin, password again, typed `grow`, optional `install_tools`): one task (`system.disk_grow`), one typed `PrivOp::DiskGrow {mount, step}` per step (`InstallGrowpart`, `GrowPartition`, `ResizePv`, `ResizeFs`); `priv` re-detects and takes every device name from that. ext2/3/4 `resize2fs`, xfs `xfs_growfs`, btrfs `btrfs filesystem resize max`, LVM `pvresize` then `lvextend -r -l +100%FREE`. Audit `system.disk.grow` with steps and before/after sizes (denied on wrong password, failed with the failing step).
- **Settings → Memory and disk**: per-filesystem sizes, "X GB not used yet" and the Grow button only when growable, a how-to otherwise, the refusal reason when refused. The button opens a password + typed-`grow` dialog (and an "install growpart" checkbox when needed); progress shows in the task dock; sizes refresh and a "99 GB → 119 GB" toast follows. `getDisk` and `growFs` are in `REAL`; the "sample data" badge on that row is gone.
- Tests: 26 fixture and fake-runner tests in `ops/disk.rs` (ext4 on a partition, xfs on NVMe `nvme0n1p1`, LVM, bare disk, btrfs, old `lsblk` string format, refusals, exact command sequences, apt install of growpart, NOCHANGE and lvextend no-op handling); HTTP tests in `crates/opsbox-api/tests/system_http.rs` (roles, wrong password, view shape). `cargo test -p opsbox-api -p opsbox-priv`, `cargo fmt --check` and clippy on the three touched crates pass. Decisions in `docs/DECISIONS.md` (16h).

## Partial / Found, not fixed
- **Not verified on `opsbox-vm`** and not on any Linux disk: nothing here has run `growpart`, `resize2fs`, etc. Not checked at 390 px either. Steps below.
- Not covered: the HTTP happy path (a real grow cannot run in tests), an API test for refusal/`409` replies (needs a fake priv), a design-board entry for the new block.
- Refused on purpose: other filesystems, RAID, encryption, volumes over several disks, btrfs inside LVM, a partition that is not last on its disk.
- `Owner` in the prompt maps to the `admin` role (no separate Owner role exists).
- Carried over from 16g: capacity bell wiring, PSI sparkline, `16g` screenshots, `OOMScoreAdjust` only after reinstall; 03f / 16e / 03e / 03d still pending commit/release, TLS VM checks, database dialogs close before the request, streaming priv call logging.

## Next
- Commit 16h (with the pending 16g and 03f stack if intended), push `dev`, wait for CI, tag when appropriate.
- Then `16i-export-config.md`.
- VM check for 16h: stop `opsbox-vm`, `multipass set local.opsbox-vm.disk=<bigger>` (or `multipass stop` then resize), start; `cd /home/ubuntu/opsbox`; `CARGO_TARGET_DIR=~/target cargo build --release --locked`; `sudo sh install.sh --dev --binary ~/target/release/opsbox`; UI via `ssh -L 8443:127.0.0.1:8443 ubuntu@<vm-ip>`; Settings → Memory and disk shows "N GB not used yet"; Grow with the password and `grow`; `df -h /` shows the new size and the audit log has `system.disk.grow`. Must fail safely: a wrong password returns 403 and nothing changes.
