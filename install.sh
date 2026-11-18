#!/bin/sh
# opsbox installer (docs/BACKEND_SPEC.md section 10). POSIX sh, no bashisms.
#
#   curl -fsSL https://github.com/khangdev20/opsbox/releases/latest/download/install.sh | sh
#   sudo sh install.sh --dev --binary ./target/release/opsbox     # test a local build
#
# Phase 0 note: `opsbox install` is still a stub, so this script creates the user,
# directories, config and systemd units itself. That logic moves into the binary
# in Phase 1 and this script shrinks to download + verify + exec.

set -eu

REPO="khangdev20/opsbox"
# Public key used to verify release signatures (minisign). A normal install refuses
# to continue if verification fails. Only --dev skips it.
MINISIGN_PUBKEY="RWRCUyMVksFGTIOeCqnTqQkIyfCLb5O1NPxAYB0tNyxfPl0whpWvfWJn"

PREFIX_BIN="/usr/local/bin/opsbox"
RESTIC_BIN="/usr/local/bin/restic"
RESTIC_VERSION="0.19.1"
RESTIC_SHA256_AMD64="f415415624dcc452f2a02b8c33641791a8c6d6d3b65bbb3543fcf9a25151585c"
RESTIC_SHA256_ARM64="a5f64aaab53d51e311fa3829124c5b703f2d14cf187d8640b6be3b2b49376465"
STATE_DIR="/var/lib/opsbox"
SLOT_DIR="$STATE_DIR/bin"
BACKUP_DIR="/var/backups/opsbox"
BACKUP_CACHE_DIR="/var/cache/opsbox/restic"
CONF_DIR="/etc/opsbox"
UNIT_DIR="/etc/systemd/system"
SVC_USER="opsbox"

VERSION=""
CHANNEL=""
LOCAL_BINARY=""
UNITS_DIR=""
DEV=0
DRY_RUN=0
LOCAL_ONLY=0
PAIR=""
WEB=0

# ------------------------------------------------------------------ output

say() {
  printf '%s\n' "$*"
  # Under cloud-init stdout is not a terminal; mirror to the console so the
  # provider's system log shows progress.
  if [ ! -t 1 ] && [ -w /dev/console ]; then
    printf '%s\n' "$*" >/dev/console 2>/dev/null || true
  fi
}

warn() { say "warning: $*"; }

die() {
  say "error: $*"
  exit 1
}

# Run a command, or only print it with --dry-run.
run() {
  if [ "$DRY_RUN" -eq 1 ]; then
    say "+ $*"
  else
    "$@"
  fi
}

usage() {
  cat <<'EOF'
Usage: install.sh [options]

  --version VERSION   install this release (default: the latest)
  --channel NAME      stable or beta, passed on to opsbox
  --pair SECRET       pairing secret from the desktop app, passed on to opsbox
  --web               turn the web panel on, passed on to opsbox
  --local-only        listen on 127.0.0.1 only (requires an SSH tunnel to reach the UI)

Development:
  --binary PATH       install this local binary instead of downloading a release
  --units-dir DIR     directory with opsbox.service, opsbox-priv.service, opsbox-proxy.service and opsbox-watchdog.service
                      (default: packaging/systemd next to this script)
  --dev               skip signature verification. Required with --binary. Unsafe:
                      never use it for a server that matters.
  --dry-run           print what would be done and change nothing
  -h, --help          show this help
EOF
}

# ------------------------------------------------------------------ arguments

while [ $# -gt 0 ]; do
  case "$1" in
    --version) [ $# -ge 2 ] || die "--version needs a value"; VERSION="$2"; shift 2 ;;
    --channel) [ $# -ge 2 ] || die "--channel needs a value"; CHANNEL="$2"; shift 2 ;;
    --pair) [ $# -ge 2 ] || die "--pair needs a value"; PAIR="$2"; shift 2 ;;
    --web) WEB=1; shift ;;
    --local-only) LOCAL_ONLY=1; shift ;;
    --binary) [ $# -ge 2 ] || die "--binary needs a path"; LOCAL_BINARY="$2"; shift 2 ;;
    --units-dir) [ $# -ge 2 ] || die "--units-dir needs a path"; UNITS_DIR="$2"; shift 2 ;;
    --dev) DEV=1; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) usage; die "unknown option: $1" ;;
  esac
done

if [ -n "$LOCAL_BINARY" ] && [ "$DEV" -ne 1 ]; then
  die "--binary installs something that was never signed; add --dev to confirm that is what you want"
fi

# ------------------------------------------------------------------ root

if [ "$(id -u)" -ne 0 ] && [ "$DRY_RUN" -ne 1 ]; then
  # Re-exec through sudo only when $0 really is this script. When piped from curl,
  # $0 is the shell itself (a file, but not something to re-run).
  if command -v sudo >/dev/null 2>&1 && [ -f "$0" ] && grep -q '^# opsbox installer' "$0" 2>/dev/null; then
    exec sudo -E sh "$0" "$@"
  fi
  die "this installer must run as root. Try: curl -fsSL <url> | sudo sh"
fi

# ------------------------------------------------------------------ platform

# shellcheck source=/dev/null
[ -r /etc/os-release ] && OS_NAME="$(. /etc/os-release && printf '%s' "${PRETTY_NAME:-unknown}")" || OS_NAME="unknown"
KERNEL="$(uname -s)"
ARCH="$(uname -m)"

if [ "$KERNEL" != "Linux" ]; then
  [ "$DRY_RUN" -eq 1 ] || die "opsbox installs on Linux only (this is $KERNEL)"
  warn "this is $KERNEL, not Linux: continuing only because of --dry-run"
fi

case "$ARCH" in
  x86_64|amd64)
    TARGET="x86_64-unknown-linux-musl"
    RESTIC_ARCH="amd64"
    RESTIC_SHA256="$RESTIC_SHA256_AMD64"
    ;;
  aarch64|arm64)
    TARGET="aarch64-unknown-linux-musl"
    RESTIC_ARCH="arm64"
    RESTIC_SHA256="$RESTIC_SHA256_ARM64"
    ;;
  *) die "unsupported CPU architecture: $ARCH (supported: x86_64, aarch64)" ;;
esac

if [ "$DRY_RUN" -ne 1 ]; then
  [ -d /run/systemd/system ] || die "systemd is not running here; opsbox needs systemd"
  command -v useradd >/dev/null 2>&1 || die "useradd not found (install the shadow-utils or passwd package)"
fi

say "opsbox installer: $OS_NAME, $ARCH ($TARGET)"
[ "$DRY_RUN" -eq 1 ] && say "(dry run: nothing will be changed)"

# ------------------------------------------------------------------ helpers

fetch() { # fetch URL DEST
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --retry 3 -o "$2" "$1" || die "download failed: $1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$2" "$1" || die "download failed: $1"
  else
    die "neither curl nor wget is installed"
  fi
}

latest_version() {
  tmp_json="$WORK/latest.json"
  if [ "$CHANNEL" = "beta" ]; then
    fetch "https://api.github.com/repos/$REPO/releases?per_page=20" "$tmp_json" ||
      die "could not look up releases; pass --version"
    awk '
      /"tag_name"[[:space:]]*:/ {
        gsub(/.*"tag_name"[[:space:]]*:[[:space:]]*"v?/, "");
        gsub(/".*/, "");
        tag = $0;
      }
      /"draft"[[:space:]]*:[[:space:]]*false/ {
        if (tag != "") { print tag; exit }
      }
    ' "$tmp_json"
  else
    fetch "https://api.github.com/repos/$REPO/releases/latest" "$tmp_json" ||
      die "could not look up the latest release; pass --version"
    # tag_name looks like "v0.1.0"
    sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' "$tmp_json" | head -n 1
  fi
}

ensure_minisign() {
  command -v minisign >/dev/null 2>&1 && return 0
  if command -v apt-get >/dev/null 2>&1; then
    say "installing minisign to verify release signature"
    DEBIAN_FRONTEND=noninteractive apt-get update -qq >/dev/null 2>&1 || true
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq minisign >/dev/null 2>&1 || true
  elif command -v dnf >/dev/null 2>&1; then
    say "installing minisign to verify release signature"
    dnf install -y -q minisign >/dev/null 2>&1 || true
  fi
  command -v minisign >/dev/null 2>&1 ||
    die "minisign is required to verify the download (apt install minisign, or dnf install minisign)"
}

verify_signature() { # verify_signature ARCHIVE
  if [ "$DEV" -eq 1 ]; then
    warn "--dev: NOT verifying the signature of the download"
    return 0
  fi
  [ -n "$MINISIGN_PUBKEY" ] ||
    die "this installer has no signing key embedded, so it cannot verify a download. Use an official install.sh from a release, or --dev for a local test."
  ensure_minisign
  minisign -Vm "$1" -x "$1.minisig" -P "$MINISIGN_PUBKEY" >/dev/null ||
    die "signature verification FAILED for $1; refusing to install"
  say "signature verified"
}

verify_checksum() { # verify_checksum ARCHIVE
  command -v sha256sum >/dev/null 2>&1 || die "sha256sum not found"
  expected="$(cut -d ' ' -f 1 "$1.sha256")"
  actual="$(sha256sum "$1" | cut -d ' ' -f 1)"
  [ -n "$expected" ] && [ "$expected" = "$actual" ] ||
    die "checksum mismatch for $1 (expected $expected, got $actual); refusing to install"
  say "checksum verified"
}

verify_sha256_literal() { # verify_sha256_literal FILE EXPECTED_HEX
  command -v sha256sum >/dev/null 2>&1 || die "sha256sum not found"
  actual="$(sha256sum "$1" | cut -d ' ' -f 1)"
  [ "$1" != "" ] && [ "$2" = "$actual" ] ||
    die "checksum mismatch for $1 (expected $2, got $actual); refusing to install"
}

ensure_bunzip2() {
  if command -v bunzip2 >/dev/null 2>&1 || command -v bzip2 >/dev/null 2>&1 || command -v python3 >/dev/null 2>&1; then
    return 0
  fi
  if command -v apt-get >/dev/null 2>&1; then
    DEBIAN_FRONTEND=noninteractive apt-get update -qq >/dev/null 2>&1 || true
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq bzip2 >/dev/null 2>&1 || true
  elif command -v dnf >/dev/null 2>&1; then
    dnf install -y -q bzip2 >/dev/null 2>&1 || true
  fi
}

install_restic() {
  if [ -x "$RESTIC_BIN" ] && "$RESTIC_BIN" version 2>/dev/null | grep -q "^restic $RESTIC_VERSION "; then
    say "restic $RESTIC_VERSION already installed"
    return 0
  fi
  restic_base="${OPSBOX_RESTIC_BASE:-https://github.com/restic/restic/releases/download/v$RESTIC_VERSION}"
  restic_file="restic_${RESTIC_VERSION}_linux_${RESTIC_ARCH}.bz2"
  if [ "$DRY_RUN" -eq 1 ]; then
    say "+ download & verify sha256=$RESTIC_SHA256 for $restic_base/$restic_file -> $RESTIC_BIN"
    return 0
  fi
  say "installing restic $RESTIC_VERSION ($RESTIC_ARCH)"
  fetch "$restic_base/$restic_file" "$WORK/$restic_file"
  verify_sha256_literal "$WORK/$restic_file" "$RESTIC_SHA256"
  say "restic checksum verified"
  ensure_bunzip2
  if command -v bunzip2 >/dev/null 2>&1; then
    bunzip2 -c "$WORK/$restic_file" >"$WORK/restic"
  elif command -v bzip2 >/dev/null 2>&1; then
    bzip2 -dc "$WORK/$restic_file" >"$WORK/restic"
  elif command -v python3 >/dev/null 2>&1; then
    python3 -c 'import bz2,sys;sys.stdout.buffer.write(bz2.decompress(sys.stdin.buffer.read()))' <"$WORK/$restic_file" >"$WORK/restic"
  else
    die "bzip2/bunzip2 is required to unpack restic"
  fi
  install -m 0755 -o root -g root "$WORK/restic" "$RESTIC_BIN"
  "$RESTIC_BIN" version >/dev/null || die "installed restic binary failed verification"
}

# ------------------------------------------------------------------ get the binary

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT INT TERM

if [ -n "$LOCAL_BINARY" ]; then
  [ -f "$LOCAL_BINARY" ] || die "--binary: $LOCAL_BINARY does not exist"
  warn "--dev: installing an unsigned local binary"
  NEW_BIN="$LOCAL_BINARY"
  if [ -z "$UNITS_DIR" ]; then
    SCRIPT_DIR="$(cd "$(dirname "$0")" 2>/dev/null && pwd)" || SCRIPT_DIR="."
    UNITS_DIR="$SCRIPT_DIR/packaging/systemd"
  fi
else
  [ -n "$VERSION" ] || VERSION="$(latest_version)"
  [ -n "$VERSION" ] || die "could not determine which version to install"
  VERSION="${VERSION#v}"
  name="opsbox-$VERSION-$TARGET"
  # OPSBOX_RELEASE_BASE points at a directory holding the three release files, for
  # mirrors, air-gapped installs and testing.
  base="${OPSBOX_RELEASE_BASE:-https://github.com/$REPO/releases/download/v$VERSION}"
  say "downloading opsbox $VERSION"
  fetch "$base/$name.tar.gz" "$WORK/$name.tar.gz"
  fetch "$base/$name.tar.gz.sha256" "$WORK/$name.tar.gz.sha256"
  if [ "$DEV" -ne 1 ]; then
    fetch "$base/$name.tar.gz.minisig" "$WORK/$name.tar.gz.minisig"
  fi
  verify_checksum "$WORK/$name.tar.gz"
  verify_signature "$WORK/$name.tar.gz"
  tar -xzf "$WORK/$name.tar.gz" -C "$WORK"
  NEW_BIN="$WORK/$name/opsbox"
  [ -z "$UNITS_DIR" ] && UNITS_DIR="$WORK/$name/systemd"
  [ -f "$NEW_BIN" ] || die "the release archive does not contain opsbox"
fi

[ -f "$UNITS_DIR/opsbox.service" ] && [ -f "$UNITS_DIR/opsbox-priv.service" ] && [ -f "$UNITS_DIR/opsbox-proxy.service" ] && [ -f "$UNITS_DIR/opsbox-watchdog.service" ] ||
  die "systemd unit files not found in $UNITS_DIR"

# Check the binary runs on this machine before touching anything.
if [ "$DRY_RUN" -eq 1 ]; then
  say "+ $NEW_BIN selftest"
else
  chmod +x "$NEW_BIN"
  "$NEW_BIN" selftest >/dev/null || die "the new binary failed its self-test on this machine"
  say "self-test passed"
fi

VER="$("$NEW_BIN" version 2>/dev/null | awk '{print $2}')" || VER=""
[ -n "$VER" ] || VER="${VERSION:-unknown}"
SLOT="$SLOT_DIR/opsbox-$VER"

# ------------------------------------------------------------------ system user, directories

if [ "$DRY_RUN" -eq 1 ] || ! getent passwd "$SVC_USER" >/dev/null 2>&1; then
  NOLOGIN="/usr/sbin/nologin"
  [ -x "$NOLOGIN" ] || NOLOGIN="/sbin/nologin"
  [ -x "$NOLOGIN" ] || NOLOGIN="/bin/false"
  getent group "$SVC_USER" >/dev/null 2>&1 || run groupadd --system "$SVC_USER"
  run useradd --system --gid "$SVC_USER" --no-create-home --home-dir /nonexistent --shell "$NOLOGIN" "$SVC_USER"
fi

run install -d -m 0755 -o root -g root "$CONF_DIR"

# /var/lib/opsbox is writable by the unprivileged `opsbox` user (the database lives
# there) but root executes the binaries under bin/. Without the sticky bit that user
# could rename bin/ away and plant its own binary, which root would then run. Mode
# 1770 means it can create files here but cannot rename or delete entries it does
# not own, such as bin/.
run install -d -m 1770 -o root -g "$SVC_USER" "$STATE_DIR"
run install -d -m 0750 -o "$SVC_USER" -g "$SVC_USER" "$STATE_DIR/proxy"
run install -d -m 0755 -o root -g root "$SLOT_DIR"
run install -d -m 0700 -o root -g root "$BACKUP_DIR"
run install -d -m 0700 -o root -g root "$BACKUP_CACHE_DIR"

install_restic

# ------------------------------------------------------------------ install

run install -m 0755 -o root -g root "$NEW_BIN" "$SLOT"
run ln -sfn "$SLOT" "$PREFIX_BIN"

if [ ! -e "$CONF_DIR/config.toml" ]; then
  if [ "$DRY_RUN" -eq 1 ]; then
    say "+ write $CONF_DIR/config.toml (defaults, commented)"
  else
    cat >"$CONF_DIR/config.toml" <<'EOF'
# opsbox configuration. Everything here is optional; the values shown are the
# defaults. See docs/CONFIG.md.

# [api]
# listen = "0.0.0.0:8443"

# [priv]
# socket = "/run/opsbox/priv.sock"
# allowed_users = ["opsbox", "root"]

# [proxy]
# listen_http = "0.0.0.0:80"
# listen_https = "0.0.0.0:443"
# socket = "/run/opsbox-proxy/proxy.sock"
# state_dir = "/var/lib/opsbox/proxy"

# [watchdog]
# listen = "0.0.0.0:8444"
# runtime_dir = "/run/opsbox-watchdog"

# [store]
# db_path = "/var/lib/opsbox/opsbox.db"

# [backup]
# dir = "/var/backups/opsbox"
# cache_dir = "/var/cache/opsbox/restic"

# [log]
# level = "info"
# format = "text"
EOF
    chmod 0644 "$CONF_DIR/config.toml"
  fi
fi

if [ "$LOCAL_ONLY" -eq 1 ]; then
  run "$PREFIX_BIN" web local-only on
fi

run install -m 0644 -o root -g root "$UNITS_DIR/opsbox-watchdog.service" "$UNIT_DIR/opsbox-watchdog.service"
run install -m 0644 -o root -g root "$UNITS_DIR/opsbox-priv.service" "$UNIT_DIR/opsbox-priv.service"
run install -m 0644 -o root -g root "$UNITS_DIR/opsbox-proxy.service" "$UNIT_DIR/opsbox-proxy.service"
run install -m 0644 -o root -g root "$UNITS_DIR/opsbox.service" "$UNIT_DIR/opsbox.service"
run systemctl daemon-reload
run systemctl enable opsbox-watchdog.service opsbox-priv.service opsbox-proxy.service opsbox.service
# restart (not start) so that re-running the installer upgrades a running install.
run systemctl restart opsbox-watchdog.service
run systemctl restart opsbox-priv.service
run systemctl restart opsbox-proxy.service
run systemctl restart opsbox.service

# Flags that `opsbox install` will act on in a later phase. Accepted so cloud-init
# scripts written today keep working; nothing consumes them yet.
[ -n "$PAIR" ] && warn "--pair is not used yet (pairing arrives with the desktop app)"
[ "$WEB" -eq 1 ] && warn "--web is not used yet (the panel is always on, on loopback only)"

# ------------------------------------------------------------------ check and report

if [ "$DRY_RUN" -ne 1 ]; then
  say "waiting for opsbox to come up"
  tries=0
  until "$PREFIX_BIN" status >/dev/null 2>&1; do
    tries=$((tries + 1))
    if [ "$tries" -ge 20 ]; then
      "$PREFIX_BIN" status || true
      die "opsbox did not become healthy. See: journalctl -u opsbox -u opsbox-priv -n 50"
    fi
    sleep 1
  done
  "$PREFIX_BIN" status
fi

say ""
if [ "$DRY_RUN" -eq 1 ]; then
  say "Dry run finished: nothing was changed. A real run would install opsbox $VER."
  exit 0
fi
say "opsbox $VER is installed and running."
# First run: a one-time token proves you can read this machine's console. It is
# printed here, on /dev/console and in the journal. Once an admin exists this prints
# nothing and setup is closed.
if setup_out="$("$PREFIX_BIN" setup-token 2>/dev/null)"; then
  say ""
  say "$setup_out"
else
  say "An admin account already exists; first-run setup is closed."
fi
say ""
if [ "$LOCAL_ONLY" -eq 1 ]; then
  say "It listens on 127.0.0.1:8443 only. To open the UI from your computer:"
  say "    ssh -L 8443:127.0.0.1:8443 <user>@<this-server>"
  say "then browse to https://127.0.0.1:8443/#/setup (accept the self-signed certificate warning)."
else
  say "Open port 8443/tcp in your cloud firewall, then open the setup URL above in your browser"
  say "and accept the self-signed certificate warning."
  say "To restrict to localhost only, run: sudo opsbox web local-only on"
fi
say "Logs: journalctl -u opsbox-watchdog -u opsbox -u opsbox-priv -u opsbox-proxy"
