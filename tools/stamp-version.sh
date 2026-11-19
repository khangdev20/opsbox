#!/bin/sh
# Stamp the release tag into the workspace version and Cargo.lock, so the crate version
# matches the tag. The binary's reported version comes from OPSBOX_BUILD_VERSION, which
# the caller sets to the same value (see crates/opsbox-version/build.rs).
#
# usage: tools/stamp-version.sh v0.1.0-alpha.21
set -eu
cd "$(dirname "$0")/.."
tag="${1:?usage: stamp-version.sh <tag, e.g. v0.1.0-alpha.21>}"
version="${tag#v}"
case "$version" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "stamp-version: '$tag' is not a version tag" >&2; exit 1 ;;
esac
case "$version" in
  *[!0-9A-Za-z.+-]*) echo "stamp-version: '$tag' has unexpected characters" >&2; exit 1 ;;
esac
# Only the `version` line under [workspace.package] is touched.
awk -v v="$version" '
  /^\[/ { in_pkg = ($0 == "[workspace.package]") }
  in_pkg && /^version[[:space:]]*=/ && !done { print "version = \"" v "\""; done = 1; next }
  { print }
  END { if (!done) exit 1 }
' Cargo.toml >Cargo.toml.new || { rm -f Cargo.toml.new; echo "stamp-version: no workspace version in Cargo.toml" >&2; exit 1; }
mv Cargo.toml.new Cargo.toml
# Refresh the workspace members' versions in Cargo.lock without touching dependencies.
cargo update --workspace >/dev/null
echo "stamped workspace version $version"
