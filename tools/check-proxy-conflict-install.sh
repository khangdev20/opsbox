#!/bin/sh
# Destructive test-host integration check: run only on an isolated Linux/systemd VM.
# Usage: sudo sh tools/check-proxy-conflict-install.sh /path/to/release/opsbox
set -eu
binary=${1:?pass the release opsbox binary}
[ "$(id -u)" -eq 0 ] || { echo 'Run as root on a test VM.' >&2; exit 1; }
[ -d /run/systemd/system ] || { echo 'This test needs Linux/systemd.' >&2; exit 1; }
log=$(mktemp)
cleanup() {
  systemctl stop opsbox-03c-holder.service >/dev/null 2>&1 || true
  systemctl reset-failed opsbox-03c-holder.service >/dev/null 2>&1 || true
  rm -f "$log"
}
trap cleanup EXIT HUP INT TERM
# Refuse to disrupt a pre-existing listener; the test owns only this transient service.
if ss -ltnH 'sport = :80' | grep -q .; then
  echo 'Port 80 is already in use; refusing to replace its owner.' >&2
  exit 1
fi
systemd-run --unit=opsbox-03c-holder --collect /usr/bin/python3 -m http.server 80 --bind 0.0.0.0
tries=0
until ss -ltnH 'sport = :80' | grep -q .; do
  tries=$((tries + 1))
  [ "$tries" -lt 10 ] || exit 1
  sleep 1
done
sh install.sh --dev --binary "$binary" >"$log" 2>&1 || { cat "$log"; exit 1; }
grep 'port 80 is used by' "$log"
grep 'Settings > Network' "$log"
"$binary" status | grep 'proxy.*conflict'
# A port conflict stays healthy for longer than the watchdog's three-failure window.
sleep 35
"$binary" status | grep 'proxy.*conflict'
systemctl is-active --quiet opsbox-proxy.service
curl -skf https://127.0.0.1:8444/healthz | python3 -c 'import json,sys; d=json.load(sys.stdin); assert not d["failed"]'
proxy_pid=$(systemctl show --property=MainPID --value opsbox-proxy.service)
systemctl stop opsbox-03c-holder.service
tries=0
while "$binary" status | grep -q 'proxy.*conflict'; do
  tries=$((tries + 1))
  [ "$tries" -lt 35 ] || exit 1
  sleep 1
done
[ "$proxy_pid" = "$(systemctl show --property=MainPID --value opsbox-proxy.service)" ]
"$binary" status
printf '%s\n' 'Installer exited 0 with busy :80; watchdog stayed healthy; proxy promoted without restart.'
