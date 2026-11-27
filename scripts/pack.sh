#!/bin/sh
# Usage: sh scripts/pack.sh <binary> <arm64|x86_64|universal> <new-archive.tar.gz>
# This is the only release archive layout, shared by local builds and CI.
set -eu
. "$(dirname "$0")/macos-common.sh"
[ "$#" -eq 3 ] || fail 'expected <binary> <architecture> <new-archive.tar.gz>'
binary="$1"
arch="$2"
archive="$3"
[ ! -e "$archive" ] && [ ! -L "$archive" ] || fail "archive already exists: $archive"
[ "$(binary_arch "$binary")" = "$arch" ] || fail 'binary architecture does not match archive'
stage="$(mktemp -d "${TMPDIR:-/tmp}/yamusiq-pack.XXXXXX")"
trap 'rm -rf "$stage"' EXIT
trap 'exit 1' HUP INT TERM
sh "$root/scripts/bundle.sh" "$binary" "$stage/yamusiq.app"
cp "$root/assets/macos/cli-launcher" "$stage/yamusiq"
chmod 755 "$stage/yamusiq"
tar -czf "$stage/archive.tar.gz" -C "$stage" yamusiq yamusiq.app
# Verify the bytes users receive, including execute bits and resource seals.
mkdir "$stage/unpacked"
tar -xzf "$stage/archive.tar.gz" -C "$stage/unpacked"
cmp -s "$root/assets/macos/cli-launcher" "$stage/unpacked/yamusiq" || fail 'archive launcher differs'
[ -x "$stage/unpacked/yamusiq" ] || fail 'archive launcher is not executable'
sh "$root/scripts/test-bundle.sh" "$stage/unpacked/yamusiq.app" "$arch" --run
if /usr/bin/lipo -verify_arch "$(uname -m)" "$stage/unpacked/yamusiq.app/Contents/MacOS/yamusiq" >/dev/null 2>&1; then
    [ "$("$stage/unpacked/yamusiq" --version)" = "yamusiq $version" ] || fail 'archive console launcher failed'
fi
mkdir -p "$(dirname "$archive")"
mv "$stage/archive.tar.gz" "$archive"
echo "packed and verified $archive"
