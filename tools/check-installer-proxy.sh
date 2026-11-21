#!/bin/sh
# Exercise real installer parsing and config edits without host mutations.
set -eu
cd "$(dirname "$0")/.."
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT HUP INT TERM
fail() { echo "check-installer-proxy: $*" >&2; exit 1; }
printf 'MemTotal: 1048576 kB\nMemAvailable: 800000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n' >"$scratch/meminfo"
for mode in builtin external off; do
  OPSBOX_TEST_PREFLIGHT=1 OPSBOX_TEST_MEMINFO="$scratch/meminfo" sh install.sh --dev --dry-run "--proxy=$mode" >"$scratch/output" 2>&1 || fail "valid mode $mode rejected"
done
for mode in invalid '' EXTERNAL; do
  if sh install.sh "--proxy=$mode" >"$scratch/output" 2>&1; then fail "invalid mode accepted"; fi
  grep -q -- '--proxy must be builtin, external or off' "$scratch/output" || fail "missing mode validation"
done
sed -n '/^# BEGIN proxy-mode/,/^# END proxy-mode/p' install.sh >"$scratch/funcs.sh"
. "$scratch/funcs.sh"
conf="$scratch/config.toml"
printf '# [proxy]\n# mode = "builtin"\n[log]\nmode = "keep"\n' >"$conf"
write_proxy_mode "$conf" ''
! grep -q '^\[proxy\]' "$conf" || fail "omitted flag changed config"
write_proxy_mode "$conf" external
write_proxy_mode "$conf" external
[ "$(grep -c '^\[proxy\]' "$conf")" = 1 ] || fail "duplicate section"
[ "$(grep -c '^mode = "external"' "$conf")" = 1 ] || fail "duplicate mode"
grep -q '^mode = "keep"' "$conf" || fail "changed another section"
cp "$conf" "$scratch/before"
write_proxy_mode "$conf" ''
cmp -s "$conf" "$scratch/before" || fail "omitted flag overwrote existing mode"
write_proxy_mode "$conf" off
grep -q '^mode = "off"' "$conf" || fail "explicit flag did not replace mode"
# Whitespace, trailing comments, single quotes and unrelated mode keys.
printf "[log]\nmode = 'keep'\n  [proxy] # proxy config\n  mode = 'external' # chosen\nlisten_http = '127.0.0.1:8080'\n[auth]\nargon2_iterations = 3\n" >"$conf"
write_proxy_mode "$conf" builtin
[ "$(grep -c 'mode = "builtin"' "$conf")" = 1 ] || fail "existing assignment not updated"
grep -q "mode = 'keep'" "$conf" || fail "changed another mode"
grep -q "listen_http = '127.0.0.1:8080'" "$conf" || fail "lost listener"
printf '[proxy]\n# mode = "builtin"\n[log]\nlevel = "debug"\n' >"$conf"
write_proxy_mode "$conf" external
grep -q '^mode = "external"' "$conf" || fail "missing mode in empty section"
python3 - "$conf" <<'PY'
import sys, tomllib
with open(sys.argv[1], 'rb') as f:
    config = tomllib.load(f)
assert config['proxy']['mode'] == 'external'
assert config['log']['level'] == 'debug'
PY
# Disabled modes must not inspect public ports or print builtin fallback advice.
sed -n '/^# BEGIN proxy-port-preflight/,/^# END proxy-port-preflight/p' install.sh >"$scratch/preflight.sh"
. "$scratch/preflight.sh"
CONF_DIR="$scratch"
say() { :; }
ss() { fail "disabled mode inspected ports"; }
for PROXY_MODE in external off ''; do check_proxy_ports; done
printf '%s\n' 'Installer proxy: parsing, defaults, preservation and idempotent section edits passed.'
