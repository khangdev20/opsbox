# Status

Last completed prompt: `docs/prompts/archive/09-deploy-pipeline.md` (Deploy pipeline)

## What works
- `crates/opsbox-store` (`0006_deploy_pipeline.sql`), `crates/opsbox-proto` (`deploy.rs`), `crates/opsbox-deploy` (`pipeline.rs`), `crates/opsbox-docker`, `crates/opsbox-priv` (`ops/deploy.rs`): full single-service deploy pipeline (`clone -> detect -> BuildKit build with --secret mounts & secret scrubbing -> container start on opsbox-<proj> bridge with 127.0.0.1:0:<port> -> HTTP/TCP/worker health check -> atomic switch & drain -> retain 5 images & rollback without rebuild`).
- Privilege split & isolation: `serve` never touches `/var/run/docker.sock`, `git`, `docker`, or `/var/lib/opsbox/apps`; `Role::Deployer` file access is strictly jailed per owned service directory, and `deploy:<service>` API tokens (`TokenScope::DeployService`) are enforced strictly on deploy, rollback, status, and build-log endpoints.
- `crates/opsbox-proxy` (`proxy.rs`) & `crates/opsbox-api` (`deploy.rs`, `panel.rs`, `github.rs`, `metrics.rs`): `GET /projects/{id}`, `POST /services`, `GET/PATCH/DELETE /services/{id}`, `POST /services/{id}/deploys`, `POST /services/{id}/deploys/{deploy_id}/rollback`, `PUT/DELETE /services/{id}/vars/{key}`, `GET /deploys/{id}/log`, `WS /stream/deploys/{id}`, webhook `push` auto-deploys, failed-deploy `Attention` items, and proxy HTTP route switching.
- `web/js` (`api.js`, `newservice.js`, `configure.js`, `apps.js`, `settings.js`): Configure and New service Docker image tab fully wired (`REAL`), Apps graph/deploys/live WS build log/rollback/vars/delete service wired live (`Domains & SSL` tab remains mock preview until prompt 10), and Settings supports `deploy:<service>` token creation.

## Partial / Found, not fixed
- Linux-only runtime/CI checks (`tools/check-idle-memory.py`, `musl` static build, `cargo-deny`, real Docker Engine daemon build/healthcheck/proxy traffic on Ubuntu) cannot run natively on macOS and remain for Linux CI / `opsbox-vm`.
- Nixpacks builds require `nixpacks` installed on the host PATH; without it, opsbox builds `Dockerfile`s and static sites directly and surfaces installation guidance for non-Dockerfile apps.

## Next
- Verify full GitHub/URL/Docker-image deploys, live WS build logs, health check failure recovery, zero-downtime switching, rollback without rebuild, and `deploy:<svc>` CI token in `opsbox-vm`.
- Next prompt: `docs/prompts/10-app-domains.md`.
