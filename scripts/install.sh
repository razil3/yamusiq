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

# "latest" means the most recent tag. releases/latest only answers for GitHub
# releases, and ours are cut by hand, so ask the API for the newest tag.
if [ "$VERSION" = "latest" ]; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/tags?per_page=1" \
        | sed -n 's/.*"name": *"\([^"]*\)".*/\1/p' | head -1)"
    if [ -z "$VERSION" ]; then
        echo "yamusiq: could not work out the latest release; set YAMUSIQ_VERSION=v1.0.0" >&2
        exit 1
    fi
fi

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

file="yamusiq-${VERSION}-${arch}.tar.gz"
url="$BASE/download/$VERSION/$file"
echo "yamusiq: fetching $url"
if ! curl -fsSL "$url" -o "$tmp/$file"; then
    echo "yamusiq: could not download $file" >&2
    echo "yamusiq: check the release tag and that it ships a $arch build" >&2
    exit 1
fi

if curl -fsSL "$BASE/download/$VERSION/SHA256SUMS" -o "$tmp/SHA256SUMS"; then
    (cd "$tmp" && grep " $file\$" SHA256SUMS > sums.txt && shasum -a 256 -c sums.txt)
else
    echo "yamusiq: no SHA256SUMS in this release, skipping verification" >&2
fi

tar -xzf "$tmp/$file" -C "$tmp"

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
