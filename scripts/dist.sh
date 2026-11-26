#!/bin/sh
# Build the macOS binaries and pack them into a release.
#
#   scripts/dist.sh              -> dist/yamusiq-<version>-{arm64,x86_64,universal}.tar.gz + SHA256SUMS
#
# The per-architecture archives are what GitHub Releases should carry; the
# universal one is for handing around locally. Every archive holds a single file
# named "yamusiq" and the checksums cover all three.
set -eu
cd "$(dirname "$0")/.."

NAME="yamusiq"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
OUT="dist"
TARGETS="aarch64-apple-darwin x86_64-apple-darwin"

for target in $TARGETS; do
    case "$target" in
        aarch64-apple-darwin) arch=arm64 ;;
        x86_64-apple-darwin) arch=x86_64 ;;
        *) echo "unknown target $target" >&2; exit 1 ;;
    esac

    cargo build --release --target "$target"
    stage="$(mktemp -d)"
    cp "target/$target/release/$NAME" "$stage/$NAME"
    mkdir -p "$OUT"
    tar -czf "$OUT/$NAME-$VERSION-$arch.tar.gz" -C "$stage" "$NAME"
    rm -rf "$stage"
    echo "packed $OUT/$NAME-$VERSION-$arch.tar.gz"
done

# Both halves in one file: runs natively on either Mac, no Rosetta needed on
# Apple Silicon. Not a substitute for the per-architecture archives, but handy.
lipo -create -output "$OUT/$NAME-$VERSION-universal" \
    target/aarch64-apple-darwin/release/$NAME \
    target/x86_64-apple-darwin/release/$NAME
stage="$(mktemp -d)"
cp "$OUT/$NAME-$VERSION-universal" "$stage/$NAME"
tar -czf "$OUT/$NAME-$VERSION-universal.tar.gz" -C "$stage" "$NAME"
rm -rf "$stage"
rm -f "$OUT/$NAME-$VERSION-universal"
echo "packed $OUT/$NAME-$VERSION-universal.tar.gz"

# Only include this release, even when dist still holds older archives.
: > "$OUT/SHA256SUMS"
for arch in arm64 x86_64 universal; do
    (cd "$OUT" && shasum -a 256 "$NAME-$VERSION-$arch.tar.gz") >> "$OUT/SHA256SUMS"
done
echo "wrote $OUT/SHA256SUMS"
