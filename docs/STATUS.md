# Status

Last completed prompt: `docs/prompts/archive/13-backups-engine.md` (Backups engine)

## What works
- `crates/opsbox-backup` & `crates/opsbox-proto`: pure-Rust UTC cron parser/evaluator (`next_run_after`, `is_run_missed`), path/volume/retention/subset validators, logical dump builders (`pg_dump -Fc`, `mysqldump`/`mariadb-dump`, `redis-cli --rdb`, `mongodump --archive`), `restic --json` parsers, HMAC repo password derivation, and typed `PrivOp::Backup{LocalInfo,Run,Snapshots,Forget,Check}` messages (zero new external crates).
- `crates/opsbox-store` (`0010_backups.sql`, `backup.rs`, `lib.rs`): `backup_destinations` (seeded `'local'`), `backup_jobs` (nullable `service_id`, seeded `'opsbox-self'`), `backup_snapshots`, snapshot sync preserving verification states, and `ReadOnlyStore::online_backup` via SQLite's online backup API.
- `crates/opsbox-docker` & `crates/opsbox-priv` (`ops/backup.rs`): zero-buffer `DockerClient::exec_to_writer` streaming DB dumps into `restic --stdin` (with automatic empty-snapshot rollback), volume mountpoint backup with optional container pause/unpause, path validation blocking `master.key`/`..`/system dirs, `opsbox-self` staging excluding `master.key` + symlinks, `forget --prune`, `check --read-data-subset`, and `st_dev` / `statvfs` same-disk inspection.
- `crates/opsbox-api` (`backups.rs`, `metrics.rs`) & `crates/opsbox`: serialized `BackupRunner` cron scheduler (`OPSBOX_DISABLE_BG_BACKUPS` opt-out), `/api/v1/backups*` routes, Attention alerts for failed/missed runs and checksum failures, `[backup]` config, and `opsbox backup list` / `opsbox backup now` CLI commands.
- `install.sh` & `web/js` (`api.js`, `pages/backups.js`): pinned `restic 0.19.1` download with SHA-256 verification before unpacking, wired schedule CRUD/run/prune/checksum UI, and real same-disk warning callout (`restoreBackup` and offsite destinations scoped to prompt 14).

## Partial / Found, not fixed
- Linux-only runtime/CI checks (`tools/check-idle-memory.py`, `musl` static build, `cargo-deny`, real `restic 0.19.1` & Docker Engine dumps) cannot execute on macOS and remain for Linux CI / `opsbox-vm`.
- Remote S3/SFTP destinations, recovery key download, snapshot download, and copy/replace restore arrive in prompt 14 (`14-backup-destinations-restore.md`).

## Next
- Verify `install.sh` restic install, DB/path/`opsbox-self` backups, `opsbox backup list|now`, and Backups UI on `opsbox-vm`.
- Next prompt: `docs/prompts/14-backup-destinations-restore.md`.
