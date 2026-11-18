#!/bin/sh
# Fixture preflight exits before any installer host mutations or downloads.
set -eu
cd "$(dirname "$0")/.."
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT HUP INT TERM
printf 'MemTotal: 262144 kB\nMemAvailable: 240000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n' >"$scratch/meminfo"
if OPSBOX_TEST_PREFLIGHT=1 OPSBOX_TEST_MEMINFO="$scratch/meminfo" sh install.sh --dev --dry-run >"$scratch/refused" 2>&1; then
  echo 'installer accepted 256 MB without --force' >&2; exit 1
fi
rg 'needs at least 512 MB' "$scratch/refused" >/dev/null
OPSBOX_TEST_PREFLIGHT=1 OPSBOX_TEST_MEMINFO="$scratch/meminfo" sh install.sh --dev --dry-run --force >"$scratch/forced" 2>&1
rg 'forcing an install below minimum' "$scratch/forced" >/dev/null
printf 'MemTotal: 1048576 kB\nMemAvailable: 800000 kB\nSwapTotal: 0 kB\nSwapFree: 0 kB\n' >"$scratch/meminfo"
OPSBOX_TEST_PREFLIGHT=1 OPSBOX_TEST_MEMINFO="$scratch/meminfo" sh install.sh --dev --dry-run >"$scratch/normal" 2>&1
rg '1024 MB RAM, no swap' "$scratch/normal" >/dev/null
printf '%s\n' 'Capacity preflight: refusal, --force, and small tier passed.'
