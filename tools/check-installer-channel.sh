#!/bin/sh
# install.sh keeps `--channel beta` (and a pre-release --version) in config.toml and
# writes nothing for a stable install. Tests the real functions from install.sh.
set -eu
cd "$(dirname "$0")/.."
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT HUP INT TERM
sed -n '/^# BEGIN update-channel/,/^# END update-channel/p' install.sh >"$scratch/funcs.sh"
# shellcheck disable=SC1091
. "$scratch/funcs.sh"

fail() { echo "check-installer-channel: $*" >&2; exit 1; }

[ "$(channel_to_record beta "")" = beta ] || fail "--channel beta must be recorded"
[ "$(channel_to_record "" v0.1.0-alpha.21)" = beta ] || fail "a pre-release install must record beta"
[ -z "$(channel_to_record "" 0.1.0)" ] || fail "a stable install must not write a channel"
[ -z "$(channel_to_record stable 0.1.0-alpha.1)" ] || fail "--channel stable must not write a channel"
[ -z "$(channel_to_record "" "")" ] || fail "no version and no channel must not write a channel"

conf="$scratch/config.toml"
printf '# [log]\n# level = "info"\n' >"$conf"
write_update_channel "$conf" beta
grep -qx '\[update\]' "$conf" || fail "missing [update] section"
grep -qx 'channel = "beta"' "$conf" || fail "missing channel line"
grep -q '^# level' "$conf" || fail "existing content was lost"

write_update_channel "$conf" beta
[ "$(grep -c '^\[update\]' "$conf")" = 1 ] || fail "[update] was written twice"
[ "$(grep -c '^channel' "$conf")" = 1 ] || fail "channel was written twice"

printf '[update]\nchannel = "stable"\n\n[log]\nlevel = "debug"\n' >"$conf"
write_update_channel "$conf" beta
grep -qx 'channel = "beta"' "$conf" || fail "existing channel was not replaced"
grep -qx 'level = "debug"' "$conf" || fail "other sections were changed"

printf '[update]\n\n[log]\nlevel = "debug"\n' >"$conf"
write_update_channel "$conf" beta
grep -qx 'channel = "beta"' "$conf" || fail "channel not added to an empty [update]"
grep -qx 'level = "debug"' "$conf" || fail "other sections were changed"

printf '%s\n' 'Installer channel: beta recorded, stable writes nothing, edits are idempotent.'
