#!/bin/sh
# Usage: sh scripts/test-packaging.sh <native-yamusiq-binary>
# Regression tests for the verifier, using disposable copies and no account.
set -eu
. "$(dirname "$0")/macos-common.sh"
[ "$#" -eq 1 ] || fail 'expected a native yamusiq binary'
binary="$1"
arch="$(binary_arch "$binary")"
/usr/bin/lipo -verify_arch "$(uname -m)" "$binary" || fail 'tests need a native slice'
work="$(mktemp -d "${TMPDIR:-/tmp}/yamusiq-packaging-test.XXXXXX")"
trap 'rm -rf "$work"' EXIT
trap 'exit 1' HUP INT TERM
base="$work/valid.app"
sh "$root/scripts/bundle.sh" "$binary" "$base"
sh "$root/scripts/test-bundle.sh" "$base" "$arch" --run

reject() {
    label="$1"
    expected="$2"
    expected_arch="${3:-$arch}"
    if sh "$root/scripts/test-bundle.sh" "$copy" "$expected_arch" > "$work/rejected.log" 2>&1; then
        fail "verifier accepted $label"
    fi
    grep -F "$expected" "$work/rejected.log" >/dev/null || { cat "$work/rejected.log" >&2; fail "unexpected failure for $label"; }
    echo "rejection passed: $label"
}

# Re-sign wrong metadata so these tests exercise semantic checks, not merely
# codesign's detection of a modified plist.
for key in CFBundleIdentifier CFBundleExecutable CFBundleVersion CFBundleShortVersionString LSMinimumSystemVersion CFBundleIconFile; do
    copy="$work/$key.app"
    cp -R "$base" "$copy"
    /usr/bin/plutil -replace "$key" -string invalid "$copy/Contents/Info.plist"
    /usr/bin/codesign --force --sign - "$copy" >/dev/null 2>&1
    reject "$key" "$key: expected"
done
copy="$work/missing-minimum.app"
cp -R "$base" "$copy"
/usr/bin/plutil -remove LSMinimumSystemVersion "$copy/Contents/Info.plist"
reject 'missing minimum' 'missing plist field LSMinimumSystemVersion'

for resource in AppIcon.icns Launch.command maximize-terminal.js; do
    copy="$work/missing-$resource.app"
    cp -R "$base" "$copy"
    rm "$copy/Contents/Resources/$resource"
    reject "missing $resource" "missing resource: $resource"
done
copy="$work/not-executable.app"
cp -R "$base" "$copy"
chmod 644 "$copy/Contents/MacOS/yamusiq"
reject 'non-executable binary' 'not executable'

copy="$base"
if [ "$arch" = arm64 ]; then other=x86_64; else other=arm64; fi
reject 'wrong architecture' 'incorrect architecture' "$other"

copy="$work/unsigned.app"
cp -R "$base" "$copy"
/usr/bin/codesign --remove-signature "$copy/Contents/MacOS/yamusiq"
reject 'missing signature' 'not signed'

# Change only the binary's deployment target; the plist remains valid.
copy="$work/wrong-macho-minimum.app"
cp -R "$base" "$copy"
wrong_min="$(awk -v value="$macos_min" 'BEGIN {split(value, v, "."); print v[1]+1 ".0"}')"
/usr/bin/xcrun clang -mmacosx-version-min="$wrong_min" -x c -o "$copy/Contents/MacOS/yamusiq" - <<'C'
int main(void) { return 0; }
C
/usr/bin/codesign --force --sign - "$copy" >/dev/null 2>&1
reject 'Mach-O minimum differs from plist' 'differs from plist' "$(binary_arch "$copy/Contents/MacOS/yamusiq")"

# Builders must refuse to overwrite existing bundles or release artifacts.
if sh "$root/scripts/bundle.sh" "$binary" "$base" > "$work/rejected.log" 2>&1; then fail 'bundle builder overwrote an app'; fi
grep -F 'already exists' "$work/rejected.log" >/dev/null
printf 'keep me\n' > "$work/existing.tar.gz"
if sh "$root/scripts/pack.sh" "$binary" "$arch" "$work/existing.tar.gz" > "$work/rejected.log" 2>&1; then fail 'packer overwrote an archive'; fi
[ "$(cat "$work/existing.tar.gz")" = 'keep me' ] || fail 'existing archive was modified'
echo 'packaging regression tests passed'
