#!/bin/sh
# Usage: sh scripts/test-bundle.sh <app> <arm64|x86_64|universal> [--run]
# --run invokes only --version, and only when the bundle has a native slice.
# Never opens Terminal, authenticates, starts audio, or needs Rosetta.
set -eu
. "$(dirname "$0")/macos-common.sh"
[ "$#" -ge 2 ] && [ "$#" -le 3 ] || fail 'expected <app> <architecture> [--run]'
app="$1"
expected_arch="$2"
case "$expected_arch" in arm64|x86_64|universal) ;; *) fail "invalid architecture: $expected_arch" ;; esac
[ "$#" -eq 2 ] || [ "$3" = --run ] || fail "unknown option: $3"
plist="$app/Contents/Info.plist"
binary="$app/Contents/MacOS/yamusiq"
[ -f "$plist" ] || fail 'Info.plist is missing'
/usr/bin/plutil -lint "$plist" >/dev/null
assert_plist() {
    actual="$(/usr/bin/plutil -extract "$1" raw -o - "$plist")" || fail "missing plist field $1"
    [ "$actual" = "$2" ] || fail "$1: expected '$2', found '$actual'"
}
assert_plist CFBundleIdentifier io.github.razil3.yamusiq
assert_plist CFBundleExecutable yamusiq
assert_plist CFBundleName yamusiq
assert_plist CFBundleDisplayName yamusiq
assert_plist CFBundlePackageType APPL
assert_plist CFBundleVersion "$version"
assert_plist CFBundleShortVersionString "$version"
assert_plist LSMinimumSystemVersion "$macos_min"
assert_plist CFBundleIconFile AppIcon.icns
assert_plist LSUIElement true
assert_plist NSHighResolutionCapable true
[ -f "$binary" ] && [ -x "$binary" ] || fail 'bundle executable is missing or not executable'
[ "$(binary_arch "$binary")" = "$expected_arch" ] || fail "incorrect architecture: expected $expected_arch"
# Validate every slice, not merely the slice of the current machine.
for arch in $(/usr/bin/lipo -archs "$binary"); do
    build="$(/usr/bin/xcrun vtool -arch "$arch" -show-build "$binary")" || fail "cannot inspect $arch Mach-O"
    platform="$(printf '%s\n' "$build" | awk '$1 == "platform" {print $2}')"
    [ "$platform" = MACOS ] || fail "$arch is not a macOS binary"
    minimum="$(printf '%s\n' "$build" | awk '$1 == "minos" {print $2}')"
    [ "$minimum" = "$macos_min" ] || fail "$arch Mach-O minimum '$minimum' differs from plist '$macos_min'"
done
for resource in AppIcon.icns Launch.command maximize-terminal.js; do
    file="$app/Contents/Resources/$resource"
    [ -s "$file" ] || fail "missing resource: $resource"
    cmp -s "$root/assets/macos/$resource" "$file" || fail "outdated resource: $resource"
done
[ -x "$app/Contents/Resources/Launch.command" ] || fail 'Launch.command is not executable'
/bin/sh -n "$app/Contents/Resources/Launch.command"
/usr/bin/codesign --verify --deep --strict --all-architectures "$app"
if [ "${3:-}" = --run ]; then
    host="$(uname -m)"
    if /usr/bin/lipo -verify_arch "$host" "$binary" >/dev/null 2>&1; then
        [ "$("$binary" --version)" = "yamusiq $version" ] || fail 'bundle --version failed'
        echo "bundle executable smoke passed ($host)"
    else
        echo "bundle executable smoke skipped: $expected_arch has no native $host slice (static checks passed)"
    fi
fi
echo "bundle verified: $expected_arch, macOS $macos_min, version $version"
