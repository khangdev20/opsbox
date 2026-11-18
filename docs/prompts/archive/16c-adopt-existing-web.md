# 16c. Move an existing website behind opsbox without taking it offline

Files: `crates/opsbox-proxy` (listener, routing, a new TLS passthrough path), `crates/opsbox-priv/src/ops/network.rs` (port conflict ops from 03c), `crates/opsbox-api/src/network.rs`, `crates/opsbox-api/src/domains.rs`, `web/js/pages/settings.js` (Network card) and the Overview banner, `docs/API.md`. Spec: 7.18, plus a new subsection "Adopting an existing web server" in `docs/BACKEND_SPEC.md`.

Problem: prompt 03c handles a busy port 80/443 with two choices. "Let opsbox take ports 80/443" stops the container or unit that holds them, so the user's existing sites go offline. "Keep it" leaves opsbox on fallback ports, so opsbox apps cannot get domains or certificates. A real user (Ubuntu 26.04, a Docker container publishing 80 and 443) has live sites and wants both: the old sites keep working and opsbox owns 80/443 for new apps. Goal: a third, recommended choice, "Move behind opsbox", that puts opsbox in front of the old web server with no downtime and no certificate gap, with automatic rollback if anything fails.

## 1. Discover what the old server serves
Read only, in priv, shown in the panel before any change:
- Holder from 03c: container (id, name, image, compose project and file from labels `com.docker.compose.project`, `.working_dir`, `.config_files`) or systemd unit (nginx, apache2, httpd, caddy, traefik, other).
- Domains it serves, best effort:
  - nginx: `nginx -T`, parse `server_name` and `listen` per server block.
  - apache: `apachectl -S`.
  - caddy: Caddyfile site addresses or the admin API on localhost if enabled.
  - traefik: Docker labels `traefik.http.routers.*.rule` `Host(...)`.
  - Unknown: no domain list; the user adds domains by hand, or uses "everything else" (section 3).
- Certificates on disk for those domains (`/etc/letsencrypt/live/*`, caddy and traefik storage): names and expiry only, never contents in the UI.
- Show the result as a list: domain, served by, has a certificate (expiry), reachable now (local request with the Host header).

## 2. Rebind the old server to a private port
Pick a free internal port pair (default `127.0.0.1:18080` for HTTP and `127.0.0.1:18443` for HTTPS). Then the holder must listen there instead of on 80/443:
- **Compose project**: write `docker-compose.opsbox.override.yml` next to the project's compose file, overriding only the published ports of the service that holds 80/443 (`127.0.0.1:18080:80`, `127.0.0.1:18443:443`), then `docker compose -f <files> -f override up -d <service>`. Keep the original compose file untouched. Record the receipt (files, service, previous ports) for undo.
- **Plain container (no compose)**: recreating a container from `docker inspect` is risky, so do not do it automatically. Offer the generated `docker run` diff and a "Recreate with new ports" action only after a typed confirmation, keeping the old container stopped (renamed `<name>-pre-opsbox`) until the user removes it.
- **nginx or apache unit**: generate a patch that changes only `listen` directives (80 → 127.0.0.1:18080, 443 → 127.0.0.1:18443), show the diff, back up the files, validate with `nginx -t` or `apachectl configtest`, then reload. Never touch other directives.
- **caddy or traefik unit**: set the HTTP and HTTPS ports through their config (caddy global `http_port`/`https_port`, traefik entrypoints), same diff, backup and validate steps.
- Anything else: explain that it cannot be moved automatically, and keep the two 03c choices.

## 3. Route the old traffic through opsbox
Two modes per domain, both supported by the proxy:
- **TLS passthrough (default for every adopted domain)**: the opsbox proxy reads the SNI from the ClientHello and, when the name is an adopted domain (or matches "everything else"), forwards the raw TCP stream to `127.0.0.1:18443` without terminating TLS. The old server keeps its own certificates and renewals, so there is no certificate gap. Port 80 requests for adopted domains are forwarded to `127.0.0.1:18080` unchanged (Host preserved), which keeps the old server's own ACME HTTP-01 renewals and redirects working.
- **opsbox terminates** (later, per domain, from the domain page): opsbox issues its own certificate (import the existing one from disk first if it is valid, so there is no gap), then proxies HTTP to `127.0.0.1:18080` with `X-Forwarded-For/Proto/Host`. After this the domain is a normal opsbox domain.
- "Everything else" route: SNI names and Host headers not known to opsbox go to the old server (passthrough on 443, forward on 80). Off by default when the domain list is known, on by default when it is not.
- opsbox's own domains (panel domain, app domains) always win over adopted routes.
- Passthrough needs: SNI peek without consuming bytes, a per-connection timeout, connection limits like other routes, and access logs (domain, bytes, duration; no payload).

## 4. Cutover with automatic rollback
One `PrivOp` sequence, run as a task with a step list in the UI:
1. Pre-check: internal ports free, config validates, the old server answers on its current ports for each domain (record status codes as the baseline).
2. Load the adopted routes into the proxy (not yet listening on 80/443).
3. Rebind the old server (section 2).
4. The proxy binds 80/443 immediately (no 30 s retry wait from 03c) and leaves fallback ports.
5. Verify: for each adopted domain, a request through the proxy on 127.0.0.1:80 and :443 (with Host or SNI) returns the same status class as the baseline within 20 s.
6. On any failure: release 80/443, undo the rebind from the receipt, reload the old server, verify it answers again, and report which step failed. Total downtime target: under 2 s on success, under 10 s on rollback.
- "Undo move" stays available afterwards in Settings > Network and runs the same rollback.
- Everything is audited (receipt id, files changed, ports) and uses typed ops, no shell strings built from names.

## 5. Installer and panel
- Installer and `opsbox status`: when 80/443 are busy, the warning line now also says `run "Move behind opsbox" in the panel to keep your sites online` (03c message otherwise unchanged).
- Panel banner from 03c gets three choices, with "Move behind opsbox" first and recommended: a dialog shows the discovered domains, the plan (files or compose override, internal ports, passthrough), the diff where there is one, and runs the cutover task with live steps.
- The domain list page shows adopted domains with a "passthrough · served by nginx-proxy" tag and an action "Let opsbox handle TLS".

## Tests
- SNI parsing: ClientHello fixtures (with and without SNI, fragmented records, TLS 1.2 and 1.3); passthrough forwards bytes unchanged in both directions.
- Routing order: opsbox domains, then adopted domains, then everything-else, then 404.
- nginx and apache listen rewriting on fixture configs: only `listen` lines change; IPv6 `[::]:80` and `443 ssl http2` variants; default_server kept.
- Compose override generation from fixture `docker inspect` labels.
- Cutover state machine with fakes: success path, failure at each step rolls back to the original state, undo after success restores everything.

## Verify on `opsbox-vm`
1. Run `docker run -d --name web -p 80:80 -p 443:443` with an nginx image and a self-signed certificate for `old.test` (add `old.test` and `new.test` to `/etc/hosts` on the VM). Install or update opsbox: expect the 03c conflict state and the new choice.
2. Run "Move behind opsbox": `curl -k https://old.test` keeps answering during and after the cutover (run it in a loop and count failures; target 0 to 2 failed requests).
3. Deploy an opsbox app on `new.test`: it is served by opsbox on 443 while `old.test` still comes from the container.
4. Undo: the container publishes 80/443 again and `old.test` still works.
5. Repeat with nginx installed as a system service. Record the results in `docs/STATUS.md`.

Out of scope: migrating the old sites into opsbox apps, editing any part of the old server's config other than ports, Windows or BSD hosts.
