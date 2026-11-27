#!/bin/sh
# Usage: sh scripts/dist.sh [--smoke-test] [--output <directory>]
# Default: build arm64, x86_64 and universal bundles, archives and SHA256SUMS.
# --smoke-test builds/packages only the host target, using temporary output
# unless --output is supplied. Nothing here publishes a release.
set -eu
. "$(dirname "$0")/macos-common.sh"
smoke=false
out=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --smoke-test) smoke=true; shift ;;
        --output) [ "$#" -ge 2 ] && [ -n "$2" ] || fail '--output needs a directory'; out="$2"; shift 2 ;;
        --help|-h) sed -n '2,5p' "$0"; exit 0 ;;
        *) fail "unknown option: $1" ;;
    esac
done
work="$(mktemp -d "${TMPDIR:-/tmp}/yamusiq-dist.XXXXXX")"
trap 'rm -rf "$work"' EXIT
trap 'exit 1' HUP INT TERM
if [ -z "$out" ]; then
    if "$smoke"; then out="$work/output"; else out="$root/dist"; fi
fi
# Resolve relative output paths before changing into the repository.
mkdir -p "$out"
out="$(CDPATH= cd -- "$out" && pwd)"
[ ! -e "$out/SHA256SUMS" ] && [ ! -L "$out/SHA256SUMS" ] || fail 'output already contains SHA256SUMS; use a fresh --output directory'
cd "$root"
target_dir="${CARGO_TARGET_DIR:-$root/target}"
host="$(rustc -vV | awk '$1 == "host:" {print $2}')"
targets='aarch64-apple-darwin x86_64-apple-darwin'
if "$smoke"; then targets="$host"; fi
arches=
for target in $targets; do
    case "$target" in
        aarch64-apple-darwin) arch=arm64 ;;
        x86_64-apple-darwin) arch=x86_64 ;;
        *) fail "unsupported target: $target" ;;
    esac
    [ ! -e "$out/yamusiq-$version-$arch.tar.gz" ] && [ ! -L "$out/yamusiq-$version-$arch.tar.gz" ] || fail "archive already exists for $arch"
    arches="$arches $arch"
done
if ! "$smoke"; then
    [ ! -e "$out/yamusiq-$version-universal.tar.gz" ] && [ ! -L "$out/yamusiq-$version-universal.tar.gz" ] || fail 'universal archive already exists'
fi
for target in $targets; do
    case "$target" in aarch64-apple-darwin) arch=arm64 ;; x86_64-apple-darwin) arch=x86_64 ;; esac
    cargo build --release --locked --target "$target"
    sh "$root/scripts/pack.sh" "$target_dir/$target/release/yamusiq" "$arch" "$out/yamusiq-$version-$arch.tar.gz"
done
if ! "$smoke"; then
    /usr/bin/lipo -create -output "$work/yamusiq-universal" \
        "$target_dir/aarch64-apple-darwin/release/yamusiq" \
        "$target_dir/x86_64-apple-darwin/release/yamusiq"
    sh "$root/scripts/pack.sh" "$work/yamusiq-universal" universal "$out/yamusiq-$version-universal.tar.gz"
    arches="$arches universal"
fi
# Relative names make `shasum -a 256 -c SHA256SUMS` work after downloading.
for arch in $arches; do
    (cd "$out" && shasum -a 256 "yamusiq-$version-$arch.tar.gz") >> "$out/SHA256SUMS"
done
(cd "$out" && shasum -a 256 -c SHA256SUMS)
echo "verified distribution: $out"
