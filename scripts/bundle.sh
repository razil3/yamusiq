#!/bin/sh
# Usage: sh scripts/bundle.sh <binary> <new-output.app>
# Build a windowless macOS identity wrapper, not a GUI application.
set -eu
. "$(dirname "$0")/macos-common.sh"
[ "$#" -eq 2 ] || fail 'expected <binary> <new-output.app>'
binary="$1"
app="$2"
[ -f "$binary" ] || fail "binary not found: $binary"
arch="$(binary_arch "$binary")"
if [ -e "$app" ] || [ -L "$app" ]; then
    fail "bundle output already exists: $app"
fi
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$binary" "$app/Contents/MacOS/yamusiq"
chmod 755 "$app/Contents/MacOS/yamusiq"
cp "$root/assets/macos/AppIcon.icns" "$app/Contents/Resources/AppIcon.icns"
cp "$root/assets/macos/Launch.command" "$app/Contents/Resources/Launch.command"
cp "$root/assets/macos/maximize-terminal.js" "$app/Contents/Resources/maximize-terminal.js"
chmod 755 "$app/Contents/Resources/Launch.command"
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>io.github.razil3.yamusiq</string>
<key>CFBundleExecutable</key><string>yamusiq</string>
<key>CFBundleName</key><string>yamusiq</string>
<key>CFBundleDisplayName</key><string>yamusiq</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>$version</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>LSMinimumSystemVersion</key><string>$macos_min</string>
<key>CFBundleIconFile</key><string>AppIcon.icns</string>
<key>LSUIElement</key><true/>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
EOF
/usr/bin/plutil -lint "$app/Contents/Info.plist"
# Local ad-hoc signature, not Developer ID signing/notarization.
/usr/bin/codesign --force --sign - "$app"
sh "$root/scripts/test-bundle.sh" "$app" "$arch"
