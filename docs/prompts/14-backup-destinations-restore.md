# 14. Backup destinations and restore (Phase 5)

Spec: 7.12 (destinations, test, restore). Design: Backups > Add destination screen (`web/js/pages/backupdest.js`).

Do:
- Provider presets from `PROVIDERS` moved to `opsbox-proto`: R2, S3, B2, Wasabi, Spaces, Hetzner, GCS interop, MinIO, custom S3, SFTP.
- Test connection (create repo, write and read back, delete-permission check) returning the step list the UI shows.
- Encryption: repository password derived from the master key; "Download recovery key".
- Restore to a copy by default; replace requires typed confirmation and takes a safety snapshot first. Monthly automatic restore test marks snapshots "restore-tested".

Verify: add an R2 or MinIO destination, back up, restore the database into a copy, connect to the copy.
