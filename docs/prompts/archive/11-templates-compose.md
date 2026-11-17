# 11. Templates and Compose (Phase 3)

Spec: 7.6.

Do:
- Templates as TOML in the binary: PostgreSQL, Redis, MySQL, MariaDB, MongoDB, MinIO, Uptime Kuma, Umami, n8n, Grafana. Image, versions, ports, volumes, generated secrets, health check, default backup job (job runs once prompt 13 exists).
- Creating an instance: generated credentials, named volume, project network only, registered as a service of kind db or tool.
- Compose: validate with `docker compose config`; refuse privileged, host network and bind mounts outside the service dir unless confirmed; run with project name `opsbox-<svc>`; each compose service appears on the Apps graph.
- Wire Templates page and Compose box.

Verify: add PostgreSQL from Templates, see it on the Apps graph; deploy the sample Compose file.
