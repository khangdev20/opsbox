# Status

Last completed prompt: `docs/prompts/archive/11-templates-compose.md` (Templates and Compose)

## What works
- `crates/opsbox-deploy` (`templates/*.toml`, `templates.rs`, `compose.rs`): 10 compiled-in TOML templates (`postgres`, `mysql`, `mariadb`, `redis`, `mongo`, `minio`, `kuma`, `umami`, `n8n`, `grafana`), CSPRNG secret/connection-string generation, pure-Rust Compose YAML pre-validation (blocking `include` and escaping `env_file`/`extends`/`build`/`secrets`), elevated privilege/bind detection, and `depends_on` graph extraction.
- `crates/opsbox-store` (`0008_templates_compose.sql`, `deploy.rs`) & `crates/opsbox-docker`: `compose_stacks` and `backup_jobs` tables, `services.{compose_stack_id,template_id,internal_only}`, named Docker volume creation, `internal_only` vs `127.0.0.1:0:<port>` container launch, network attach (`connect_container_to_network`), and Compose runtime inspection.
- `crates/opsbox-priv` (`ops/templates_compose.rs`): `TemplateStart` (pull, volume create, secret open/scrubbing, `exec`/HTTP/TCP health checks) and `ComposeValidate`/`ComposeDeploy`/`ComposeDown`/`ComposeRemoveService` (`docker compose config` safety gates returning `ConfirmRequired`, `compose.yml` staging without plaintext `.env`, `up -d`, `opsbox-<project>` attachment).
- `crates/opsbox-api` (`templates.rs`, `deploy.rs`, `domains.rs`, `files.rs`): `GET /api/v1/templates`, `POST /api/v1/templates/{id}/instances` (one-time `connectionUrl`, masked subsequent `GET`s, backup schedule persistence), `POST /api/v1/compose` (`409 confirmation_required`, `edges` in `GET /projects/{id}`), `DELETE /api/v1/compose/{stack}`, Compose/template redeploy and delete support.
- `web/js` (`api.js`, `pages/templates.js`, `pages/apps.js`, `pages/newservice.js`, `main.js`): Templates/Compose wired to `REAL` (zero sample badge), real sample YAML (`nginx:1.27-alpine` + `redis:7-alpine`), one-time connection string dialog, Compose confirmation prompt, `proj.edges` topology rendering, and full `db`/`tool`/`compose` detail tabs.

## Partial / Found, not fixed
- Linux-only runtime/CI checks (`tools/check-idle-memory.py`, `musl` static build, `cargo-deny`, real Docker Engine template container startup and `docker compose` CLI on Ubuntu) cannot run natively on macOS and remain for Linux CI / `opsbox-vm`.
- Backup schedules (`backup_jobs`) are persisted on template creation; automated execution, retention pruning, and restore arrive in prompts 13–14.
- Linking database env references (`${{ db.DATABASE_URL }}`) into app services arrives in prompt 12.

## Next
- Verify deploying PostgreSQL (`postgres:16.4`), inspecting masked secrets, and deploying + deleting the two-service Compose file on `opsbox-vm`.
- Next prompt: `docs/prompts/12-databases-link.md`.
