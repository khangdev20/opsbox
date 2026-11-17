# Configuration

opsbox reads `/etc/opsbox/config.toml`. Every setting has a default, so the file is optional and `install.sh` writes it with everything commented out.

## Precedence

Highest wins:

1. command-line flag
2. environment variable
3. `config.toml`
4. built-in default

`--config FILE` (or `OPSBOX_CONFIG`) chooses another file. A file given this way must exist; the default path may be missing. Unknown keys and bad values are errors, so a typo never silently falls back to a default.

The allow-lists (`allowed_users`, `allowed_uids`) are the exception to "highest wins": flags and environment variables **add** to the file's lists instead of replacing them.

## Settings

| Section / key | Default | Flag | Environment | Used by |
|---|---|---|---|---|
| `[api] listen` | `127.0.0.1:8443` | `--listen ADDR` | `OPSBOX_LISTEN` | serve, status |
| `[priv] socket` | `/run/opsbox/priv.sock` | `--socket PATH` | `OPSBOX_PRIV_SOCKET` | serve, priv, status |
| `[priv] allowed_users` | `["opsbox", "root"]` | `--allow-user NAME` (repeatable, or comma list) | `OPSBOX_PRIV_ALLOW_USERS` | priv |
| `[priv] allowed_uids` | `[]` | `--allow-uid UID` (repeatable, or comma list) | `OPSBOX_PRIV_ALLOW_UIDS` | priv |
| `[priv] socket_mode` | `"0660"` | `--socket-mode MODE` | `OPSBOX_PRIV_SOCKET_MODE` | priv |
| `[priv] socket_group` | `"opsbox"` (`""` for none) | `--socket-group GROUP` | `OPSBOX_PRIV_SOCKET_GROUP` | priv |
| `[api] secure_cookies` | `"auto"` (`"always"`, `"never"`) | `--secure-cookies MODE` | `OPSBOX_SECURE_COOKIES` | serve |
| `[priv] master_key` | `/etc/opsbox/master.key` | `--master-key PATH` | `OPSBOX_PRIV_MASTER_KEY` | priv |
| `[auth] argon2_memory_kib` | `65536` | | | serve, CLI |
| `[auth] argon2_iterations` | `3` | | | serve, CLI |
| `[store] db_path` | `/var/lib/opsbox/opsbox.db` | `--db PATH` | `OPSBOX_DB` | serve |
| `[log] level` | `"info"` | `--log-level FILTER` | `OPSBOX_LOG_LEVEL` | all |
| `[log] format` | `"text"` (`"json"`) | `--log-format text\|json` | `OPSBOX_LOG_FORMAT` | all |
| (config file path) | `/etc/opsbox/config.toml` | `--config FILE` | `OPSBOX_CONFIG` | all |

`--log-level` takes a `tracing` filter: `info`, `debug`, or per-crate like `info,opsbox_priv=debug`. Logs go to stderr, which systemd sends to the journal (`journalctl -u opsbox -u opsbox-priv`).

### What the privileged-helper settings mean

`priv` accepts a connection only if the connecting process runs as one of the allowed users. The kernel reports who is on the other end (`SO_PEERCRED` on Linux); the check happens before anything the peer sent is read. Names in `allowed_users` are looked up when `priv` starts; names that do not exist are skipped with a warning. If nobody is allowed, `priv` refuses to start.

The default includes `root` so an administrator can run `sudo opsbox status`. Remove it if you do not want that.

`socket_mode` and `socket_group` are the first gate (who may open the socket at all); the user check is the second.

### Sign-in settings

- `secure_cookies`: `auto` marks the session cookies `Secure` only when a request arrives through an HTTPS proxy (`X-Forwarded-Proto: https`); `always` and `never` force it. Plain HTTP on loopback cannot use `Secure`, because the browser would then not send the cookie back.
- `master_key`: the 32-byte key that seals TOTP secrets (and later other secrets). `priv` creates it with mode 0600 the first time it is needed and refuses a key file that others can read. Back it up with the database: without it, sealed values cannot be opened, and every user would need `opsbox user reset-2fa`.
- `argon2_memory_kib` and `argon2_iterations`: password hashing cost (default 64 MB, 3 passes, one lane; the spec's values). A sign-in should take well under a third of a second. On a very small machine lower the memory (for example `16384`) rather than the passes. Existing hashes keep working after a change, because each hash records its own cost. Two sign-ins may hash at once, so peak use is about twice the memory setting.

### Example

```toml
[api]
listen = "127.0.0.1:9000"

[priv]
allowed_users = ["opsbox", "root", "deploy"]

[log]
level = "info,opsbox_api=debug"
format = "json"
```

## Listening address

`serve` has no authentication in Phase 0. Keep `listen` on a loopback address. If you set another one, `serve` logs a warning at startup. Reach a remote server through an SSH tunnel:

```sh
ssh -L 8443:127.0.0.1:8443 you@server
# then open http://127.0.0.1:8443/?mode=http
```

## Commands that read the configuration

| Command | Notes |
|---|---|
| `opsbox serve` | Opens the database, applies migrations, listens. |
| `opsbox priv` | Creates the socket (and `/run/opsbox` if missing). |
| `opsbox status` | Probes `serve` over HTTP and `priv` over its socket. Exit status 0 only if both are healthy, 1 otherwise. |
| `opsbox setup-token` | Prints a one-time token while no admin exists. Run as root, it switches to the `opsbox` user before opening the database. |
| `opsbox user reset-2fa\|unlock\|password NAME` | Account recovery over SSH. `password` asks twice on a terminal and reads one line from standard input when piped. |
| `opsbox audit verify` | Walks the hash chain of the audit log; exit status 1 and the first bad entry if it was altered. |
| `opsbox selftest` | Ignores the configuration; uses a temporary directory. Exit status 0 if every check passes. |

Commands that exist but are not built yet (`pair`, `web`, `doctor`, ...) print `` `<name>` is not implemented yet `` and exit with status 2.

## Running on a development machine

On a machine where opsbox is not installed there is no `opsbox` user and `/var/lib/opsbox` is not writable, so point the defaults somewhere you own.

Linux (two terminals):

```sh
cargo build --release

# 1. the helper, as root, allowing your own user. The socket group is your primary
#    group so mode 0660 lets you connect.
sudo ./target/release/opsbox priv --allow-uid "$(id -u)" --socket-group "$(id -gn)"

# 2. the server, as you, with a database in the current directory
./target/release/opsbox serve --db ./opsbox-dev.db

# 3. check, then open http://127.0.0.1:8443/?mode=http
./target/release/opsbox status
```

macOS or any machine without root (no `sudo`; the socket path must stay under about 100 characters):

```sh
./target/release/opsbox priv  --socket /tmp/opsbox.sock --allow-uid "$(id -u)"
./target/release/opsbox serve --socket /tmp/opsbox.sock --db ./opsbox-dev.db
./target/release/opsbox status --socket /tmp/opsbox.sock
```

`*.db` files are git-ignored.
