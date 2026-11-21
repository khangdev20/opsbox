# Fix 38. opsbox proxy takes ports 80/443 from an app's own Caddy

Follow `docs/prompts/fix/README.md`. Rust and one panel message. Find with `rg -n 'PortHolders|opsbox-proxy|bind' crates/opsbox-proxy crates/opsbox-priv`.

## Problem
`opsbox-proxy` binds 80 and 443 on the host. An app that ships its own Caddy (Greenglish) cannot start because the ports are taken, and nothing in the panel says who holds them or how to choose.

## Steps
1. Test first: a fake port table where another process holds 80 or 443 before the proxy starts: the proxy must not crash-loop; it reports `ports_busy` with the holder (name, pid) and retries slowly (30 s).
2. Add a setting `proxy.mode`: `builtin` (default, today's behaviour), `external` (opsbox does not bind 80/443; domains are only stored and the panel shows the upstream `127.0.0.1:<port>` to paste into your own Caddy/nginx), `off`. The installer accepts `--proxy=external|off`.
3. When the proxy is `builtin` and 80/443 are held by something else, the Domains page shows: "Ports 80/443 are used by <name> (pid N). Switch to external proxy, or stop that service." with the two buttons.
4. `opsbox status` prints the proxy mode and, if busy, the holder.

## Done when
The busy-port test passes with no crash loop, the panel names the holder and offers a choice, and external mode leaves 80/443 free for the app's Caddy.

Commit `fix 38: proxy port conflict`, push `dev`, archive this file.
