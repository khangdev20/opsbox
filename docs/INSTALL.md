# Installing opsbox (Phase 0)

Phase 0 has no signed releases yet, so the supported way to try the installer is with a binary you built yourself.

## Requirements

Proposed starting sizes: **1 GB to run apps, 2 GB or more to build on the server**; these have not yet been measured on the disposable VM. See the [server size guide](#server-size-guide).

## Server size guide

**Proposed — all RAM and disk sizes below are starting estimates, pending VM measurements.**

| What you run | RAM (proposed) | Disk (proposed) |
|---|---|---|
| Try it: 1 to 2 small apps from prebuilt images | 1 GB | 20 GB |
| 2 to 3 apps and 1 database, building on the server | 2 GB | 25 GB |
| 4 to 8 containers, several databases, frequent builds | 4 GB | 50 GB |

Use a 1 GB server to run apps, with builds done in GitHub Actions and the resulting image pulled onto the server. These are guides, and the panel's Capacity view checks your server's available memory and free disk for each workload; a busy server may need more space or memory than the proposed sizes.

## What gets installed

| Path | What | Owner / mode |
|---|---|---|
| `/usr/local/bin/opsbox` | symlink to the active version | root |
| `/var/lib/opsbox/bin/opsbox-<version>` | the binary (one file per version) | root:root 0755 |
| `/var/lib/opsbox/` | database and later app data | root:opsbox **1770** (sticky) |
| `/etc/opsbox/config.toml` | optional settings, see [CONFIG.md](CONFIG.md) | root 0644 |
| `/etc/systemd/system/opsbox-priv.service` | privileged helper, runs as root | root |
| `/etc/systemd/system/opsbox.service` | server, runs as user `opsbox` | root |
| `/run/opsbox/priv.sock` | helper socket (created by the helper) | root:opsbox 0660 |

The system user and group `opsbox` have no shell and no home directory.

The sticky bit on `/var/lib/opsbox` matters: the unprivileged `opsbox` user must write the database there, but root runs the binaries under `bin/`. Without the sticky bit that user could rename `bin/` and plant its own binary. See DECISIONS.md.

## Install a locally built binary

On a Linux machine with systemd (a VM is the right place to try this):

```sh
cargo build --release
sudo sh install.sh --dev --binary ./target/release/opsbox
```

`--dev` is required with `--binary` because a local binary has no signature. Add `--dry-run` first to see every command without running it (works on any OS).

The installer runs the new binary's `selftest` before changing anything, starts both services, waits until `opsbox status` is healthy, and prints how to reach the UI (an SSH tunnel; the server listens on loopback only).

Running it again with a newer binary upgrades in place: a new version file, the symlink moves, both services restart.

## Install a release

```sh
curl -fsSL https://github.com/khangdev20/opsbox/releases/latest/download/install.sh | sh
```

The installer downloads `opsbox-<version>-<target>.tar.gz`, its `.sha256` and its `.minisig`, and checks the checksum and the minisign signature against the public key embedded in the script. It stops on any mismatch. Until a signing key is embedded it refuses to install a download at all. `--dev` skips only the signature check, never the checksum.

Options: `--version`, `--channel`, `--pair`, `--web` (the last three are accepted and ignored in Phase 0), `--units-dir`, `--dry-run`, `--help`. `OPSBOX_RELEASE_BASE` points at a mirror directory holding the three release files.

## Check and troubleshoot

```sh
opsbox status                              # exit 0 only when serve and priv are healthy
systemctl status opsbox opsbox-priv
journalctl -u opsbox -u opsbox-priv -n 50
curl -s http://127.0.0.1:8443/readyz       # which dependency is failing
```

## Remove (manual, until `opsbox uninstall` exists)

```sh
sudo systemctl disable --now opsbox opsbox-priv
sudo rm /etc/systemd/system/opsbox.service /etc/systemd/system/opsbox-priv.service /usr/local/bin/opsbox
sudo systemctl daemon-reload
# data, only if you want it gone: /var/lib/opsbox and /etc/opsbox
```
