# Status

Last completed prompt: `docs/prompts/archive/14-backup-destinations-restore.md` (Backup destinations and restore)

## What works
- Offsite destination presets & wizard (`crates/opsbox-proto`, `crates/opsbox-backup/src/destinations.rs`, `GET /api/v1/backups/providers`, `POST /api/v1/backups/destinations/test`, `POST /api/v1/backups/destinations`): supports all 10 providers (`r2`, `s3`, `b2`, `wasabi`, `do`, `hetzner`, `gcs`, `minio`, `sftp`, `custom`). S3 keys are sealed via `opsbox-priv` (`backup:dest:<id>`); SFTP generates an Ed25519 SSH keypair inside `priv`, exposes only the public key/fingerprint, uses `StrictHostKeyChecking=accept-new` pinned per destination, and switches `'local'` jobs once verified.
- Credentials & recovery key (`POST /api/v1/backups/recovery-key`): `serve` never unseals destination credentials or touches `master.key`; `priv` injects `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` solely via `cmd.env` and ephemeral `0600` SFTP keyfiles (never CLI argv). Recovery password download requires admin interactive session (`deny_token`) + password re-authentication (returning `403` on bad password, `Cache-Control: no-store`).
- Restore (`POST /api/v1/backups/snapshots/{id}/restore`, `opsbox restore <snapshot> --to <path>`): copy-by-default (starts a brand-new connectable managed DB service for `database`, `<path>.restored-<date>` for `path`, `<vol>-restored-<date>` for `volume`, `/var/lib/opsbox/opsbox-restored-<date>` for `opsbox`). In-place `replace` requires exact `"replace"` typed confirmation (`409 confirmation_required`), refuses live `opsbox`, and takes a live safety snapshot (`opsbox-safety:true`) FIRST before overwriting.
- Automated monthly & on-demand restore testing (`POST /api/v1/backups/snapshots/{id}/test`, `POST /api/v1/backups/restore-test`, 30-day scheduler): restores into an ephemeral Docker DB container or temporary directory, runs SQL/Redis/Mongo or SQLite `PRAGMA integrity_check` verification, cleans up temporary resources, marks snapshots `restore_tested`, and surfaces failures in `GET /api/v1/attention`.
- Backups & destination UI (`web/js/api.js`, `web/js/pages/backups.js`, `web/js/pages/backupdest.js`): all backup/destination/restore/recovery-key calls moved into `REAL` and all `"sample data"` badges removed.

## Partial / Found, not fixed
- Linux-only CI/VM checks (`tools/check-idle-memory.py`, `musl` cross-compile, `cargo-deny`, real MinIO/R2/SFTP & Docker DB restore verification) did not execute on macOS and remain for Linux CI / `opsbox-vm`.

## Next
- Verify R2/MinIO & SFTP destinations, DB copy restore + app reconnect, `replace` safety snapshot/refusal, recovery key download, and `opsbox restore` in `opsbox-vm`.
- Next prompt: `docs/prompts/15-alerts.md`.
