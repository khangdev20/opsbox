# 13. Backups engine (Phase 5)

Spec: 7.12 (engine, what, schedule, retention). Crate: `opsbox-backup`.

Do:
- restic downloaded at install with a pinned version and checksum, run by priv.
- Local destination `/var/backups/opsbox` first (S3 providers come in prompt 14).
- Jobs: template databases via logical dump streamed to restic stdin (pg_dump -Fc, mysqldump --single-transaction, redis-cli --rdb, mongodump --archive); volumes; paths; opsbox itself (SQLite online backup, /etc/opsbox without master key).
- Schedules as tasks, retention with forget/prune, snapshot list with checksum state.
- Wire Backups page schedules and snapshot list; "same disk" warning real.

Verify: back up the Postgres from prompt 11 and a path; see snapshots; prune runs.
