# Status

Last completed prompt: `docs/prompts/archive/12-databases-link.md` (Databases part A: link to apps)

## What works
- `crates/opsbox-deploy` (`refs.rs`, `templates.rs`, `compose.rs`): `${{ service.KEY }}` parsing, canonicalization, cycle/self-ref checks, and depth-bounded resolution; DB credential re-computation for `postgres`, `mysql`, `mariadb`, `redis`, `mongo`; Compose guard flagging unconfirmed public DB host ports (`5432`/`3306`/`6379`/`27017`).
- `crates/opsbox-store` (`0009_databases_link.sql`, `deploy.rs`) & `crates/opsbox-docker`: `services.{public_port,credentials_rotated_at}`, `service_vars.ref_expr`, `service_var_refs` (`stale`, monotonic `ref_seq`), project reference graph edges, and `PortBindMode` (`InternalOnly`, `Loopback`, `Public`).
- `crates/opsbox-priv` (`ops/deploy.rs`, `ops/templates_compose.rs`): reference variable resolution + secret scrubbing inside `priv` across builds/starts/templates/Compose, `ConfirmRequired` enforcement on unconfirmed public DB ports, and `PrivOp::DatabaseRotate` running `docker exec` inside live DB containers to rotate passwords and re-seal vars without `serve` seeing plaintext.
- `crates/opsbox-api` (`deploy.rs`, `templates.rs`, `lib.rs`): `POST /services/{id}/databases`, `DELETE /services/{id}/databases/{db}`, `POST /services/{id}/rotate`, `PUT /services/{id}/exposure`, non-leaking `${{ db.KEY }}` rendering in `GET /projects/{id}` & `GET /services/{id}` (`connectedApps`, `edges`, `networkMode`), `409 confirmation_required` on unconfirmed public DB ports, and strict RBAC/API-token enforcement.
- `web/js` (`api.js`, `pages/apps.js`, `pages/templates.js`): `"Connect database"` flow on app Variables tab, link icon + clickable DB references without secret exposure, bidirectional graph edge routing, upgraded DB detail overview (connected apps, stale `"Needs redeploy"` status, `"Rotate credentials"`, redeploy actions, exposure selector + danger warning), and zero `"sample data"` badges.

## Partial / Found, not fixed
- Linux-only runtime/CI checks (`tools/check-idle-memory.py`, `musl` static build, `cargo-deny`, real Docker Engine live DB `exec` rotation) cannot execute natively on macOS and remain for Linux CI / `opsbox-vm`.
- Automated backup execution, retention pruning, destinations, and restore arrive in prompts 13–14; per-app DB users, extensions, and SQL query console arrive in prompt 16 (`16-databases-manage.md`).

## Next
- Verify PostgreSQL deployment, `"Connect database"`, app redeploy, live DB credential rotation, and `409` public-port refusal on `opsbox-vm`.
- Next prompt: `docs/prompts/13-backups-engine.md`.
