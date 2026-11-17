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
STATE_DIR="/var/lib/opsbox"
SLOT_DIR="$STATE_DIR/bin"
CONF_DIR="/etc/opsbox"
UNIT_DIR="/etc/systemd/system"
SVC_USER="opsbox"

VERSION=""
CHANNEL=""
LOCAL_BINARY=""
UNITS_DIR=""
DEV=0
DRY_RUN=0
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

Development:
  --binary PATH       install this local binary instead of downloading a release
  --units-dir DIR     directory with opsbox.service and opsbox-priv.service
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
  x86_64|amd64) TARGET="x86_64-unknown-linux-musl" ;;
  aarch64|arm64) TARGET="aarch64-unknown-linux-musl" ;;
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

[ -f "$UNITS_DIR/opsbox.service" ] && [ -f "$UNITS_DIR/opsbox-priv.service" ] ||
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
run install -d -m 0755 -o root -g root "$SLOT_DIR"

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
# listen = "127.0.0.1:8443"

# [priv]
# socket = "/run/opsbox/priv.sock"
# allowed_users = ["opsbox", "root"]

# [store]
# db_path = "/var/lib/opsbox/opsbox.db"

# [log]
# level = "info"
# format = "text"
EOF
    chmod 0644 "$CONF_DIR/config.toml"
  fi
fi

run install -m 0644 -o root -g root "$UNITS_DIR/opsbox-priv.service" "$UNIT_DIR/opsbox-priv.service"
run install -m 0644 -o root -g root "$UNITS_DIR/opsbox.service" "$UNIT_DIR/opsbox.service"
run systemctl daemon-reload
run systemctl enable opsbox-priv.service opsbox.service
# restart (not start) so that re-running the installer upgrades a running install.
run systemctl restart opsbox-priv.service
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
say "It listens on 127.0.0.1:8443 only. To open the UI from your computer:"
say "    ssh -L 8443:127.0.0.1:8443 <user>@<this-server>"
say "then browse to http://127.0.0.1:8443/?mode=http"
say "Logs: journalctl -u opsbox -u opsbox-priv"
