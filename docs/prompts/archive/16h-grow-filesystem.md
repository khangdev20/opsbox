# 16h. Settings: grow the filesystem after the disk was enlarged

Follow `docs/prompts/CONVENTIONS.md`: work on branch `dev`; when fully done, `git mv` this file to `docs/prompts/archive/`, mark it done in `docs/prompts/README.md`, update `docs/STATUS.md`, commit as `prompt NN: <title>` and `git push origin dev`.

Files: `crates/opsbox-priv/src/ops/` (new `disk.rs` or in `system`), `crates/opsbox-proto` (`PrivOp`), `crates/opsbox-api/src/system.rs` or `settings`, `web/js/api.js` (`growFs`, add to `REAL`), `web/js/pages/settings.js` ("Memory and disk"), `docs/API.md`. Spec: 7.x System (add a short subsection).

Today: the "Grow filesystem to fill disk" button is a mock (disabled with a "sample data" badge in real mode). Users who enlarge the volume at their cloud provider still need SSH to run `growpart` and `resize2fs`.

Do:
- `GET /api/v1/system/disk`: for `/` (and the opsbox data and Docker roots if on other filesystems): device, partition, filesystem type, size, partition size, disk size, and `growable_bytes` (disk or partition space not used by the filesystem). Read from `/proc/partitions`, `/sys/class/block`, `findmnt -J`, `lsblk -J -b`.
- `POST /api/v1/system/disk/grow` (Owner, re-auth, typed confirm, task): `growpart <disk> <part>` when the partition is smaller than the disk, then the online resize for the type: `resize2fs` (ext4), `xfs_growfs` (xfs), `btrfs filesystem resize max` (btrfs). LVM: `pvresize` then `lvextend -r -l +100%FREE`. Refuse anything else with a clear reason. Install `cloud-guest-utils` if `growpart` is missing (ask in the confirm dialog).
- Every step is a typed `PrivOp` with no shell strings built from input; device names come from the detection result only. Audited with before and after sizes.
- UI: show the current size, "X GB not used yet" when growable, and the button only then; otherwise a short how-to ("Enlarge the volume at your cloud provider first"). Task progress with steps; numbers update after.

Tests: parsing fixtures for `lsblk`/`findmnt` (ext4 on partition, xfs, LVM, whole-disk without partition, NVMe names `nvme0n1p1`); refuse unknown filesystems; the op sequence for each layout with a fake runner.

Verify on `opsbox-vm`: stop the VM, `multipass` resize the disk (or attach a larger disk image), start, the panel shows growable space, Grow works, `df -h /` shows the new size. Record in `docs/STATUS.md`.
