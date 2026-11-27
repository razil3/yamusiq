#!/bin/sh
# Shared by the bundle builder, verifier and release packer. Source, do not run.
root="$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/Cargo.toml" | head -1)"
macos_min="$(sed -n 's/^MACOSX_DEPLOYMENT_TARGET = "\([0-9.]*\)"$/\1/p' "$root/.cargo/config.toml")"
[ -n "$version" ] && [ -n "$macos_min" ] || { echo 'missing package version or macOS minimum' >&2; exit 1; }
# An inherited SDK target must not silently produce a different release policy.
export MACOSX_DEPLOYMENT_TARGET="$macos_min"

fail() { echo "yamusiq packaging: $*" >&2; exit 1; }

binary_arch() {
    slices="$(/usr/bin/lipo -archs "$1" | tr ' ' '\n' | LC_ALL=C sort | paste -sd ' ' -)"
    case "$slices" in
        arm64) echo arm64 ;;
        x86_64) echo x86_64 ;;
        'arm64 x86_64') echo universal ;;
        *) fail "unexpected Mach-O architectures: $slices" ;;
    esac
}
