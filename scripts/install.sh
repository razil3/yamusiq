#!/bin/sh
# Install the yamusiq binary for this Mac.
#
#   curl -fsSL https://raw.githubusercontent.com/razil3/yamusiq/master/scripts/install.sh | sh
#
# Downloads the release archive for this machine's architecture, verifies it
# against SHA256SUMS, and installs the binary:
#   /usr/local/bin if that is writable, else ~/.local/bin (created if needed).
# Set YAMUSIQ_VERSION=v1.0.0 to pin a release, YAMUSIQ_BIN_DIR to choose a
# directory (use "sudo -E" for /usr/local/bin on a fresh machine).
#
# Environment: REPO defaults to razil3/yamusiq.
set -eu

REPO="${REPO:-razil3/yamusiq}"
VERSION="${YAMUSIQ_VERSION:-latest}"
BASE="https://github.com/$REPO/releases"

# "latest" means the newest published release, matched against the tags so the
# tag need not be spelled with a "v".
if [ "$VERSION" = "latest" ]; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases?per_page=1" \
        | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -1)"
    if [ -z "$VERSION" ]; then
        echo "yamusiq: could not work out the latest release; set YAMUSIQ_VERSION=1.0.0" >&2
        exit 1
    fi
fi

# Archives are named "yamusiq-<version>-<arch>.tar.gz"; the download URL keeps
# whatever tag the release was published under, with or without the "v".
TAG="$(curl -fsSL "https://api.github.com/repos/$REPO/tags?per_page=100" \
    | grep -o '"name": *"[^"]*"' | sed 's/.*"\([^"]*\)"$/\1/' \
    | while read -r t; do
          [ "${t#v}" = "${VERSION#v}" ] && echo "$t" && break
      done)"

case "$(uname -s)" in
    Darwin) ;;
    *) echo "yamusiq: this script installs macOS builds; for other systems build from source" >&2; exit 1 ;;
esac

# Rosetta reports the translated architecture; the native one is what we want.
arch="$(uname -m)"
if [ "$arch" = "x86_64" ] && [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = "1" ]; then
    arch="arm64"
fi
case "$arch" in
    arm64) ;;
    x86_64) ;;
    *) echo "yamusiq: unsupported architecture $arch" >&2; exit 1 ;;
esac

if ! command -v ffmpeg >/dev/null 2>&1; then
    echo "yamusiq: note — ffmpeg is required for playback: brew install ffmpeg"
fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

file="yamusiq-${VERSION#v}-${arch}.tar.gz"
tag="${TAG:-$VERSION}"
url="$BASE/download/$tag/$file"
echo "yamusiq: fetching $url"
rm -f "$tmp/$file"
if ! curl -fsSL "$url" -o "$tmp/$file"; then
    echo "yamusiq: could not download $file" >&2
    echo "yamusiq: check the release tag and that it ships a $arch build" >&2
    exit 1
fi

if curl -fsSL "$BASE/download/$tag/SHA256SUMS" -o "$tmp/SHA256SUMS"; then
    if ! (cd "$tmp" && grep " $file\$" SHA256SUMS > sums.txt && shasum -a 256 -c sums.txt); then
        echo "yamusiq: checksum mismatch for $file, not installing" >&2
        exit 1
    fi
else
    echo "yamusiq: this release has no SHA256SUMS, installing without verification" >&2
fi

rm -rf "$tmp/x"
if ! tar -xzf "$tmp/$file" -C "$tmp"; then
    echo "yamusiq: $file is not a readable archive" >&2
    exit 1
fi

# /usr/local/bin when it is writable, otherwise a per-user directory.
bin_dir="${YAMUSIQ_BIN_DIR:-}"
if [ -z "$bin_dir" ]; then
    if [ -w /usr/local/bin ] 2>/dev/null; then
        bin_dir=/usr/local/bin
    else
        bin_dir="$HOME/.local/bin"
    fi
fi
mkdir -p "$bin_dir"
install -m 755 "$tmp/yamusiq" "$bin_dir/yamusiq"

echo "yamusiq: installed to $bin_dir/yamusiq"
case ":$PATH:" in
    *":$bin_dir:"*) ;;
    *) echo "yamusiq: add it to your PATH: export PATH=\"$bin_dir:\$PATH\"" ;;
esac
"$bin_dir/yamusiq" --version
