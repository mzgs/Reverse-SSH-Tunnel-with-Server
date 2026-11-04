#!/bin/sh
set -eu
cd "$(dirname "$0")"
app_name="reverse-ssh-tunnel-with-server"
display_name="SSH Tunnel Plus"

if [ "$(uname -s)" != Darwin ]; then
    cargo build --release --locked
    printf 'Built target/release/%s\n' "$app_name"
    exit 0
fi

host=$(rustup run stable rustc -vV | sed -n 's/^host: //p')
rustup run stable cargo build --release --locked --target "$host" --target-dir target

bundle="target/release/$display_name.app"
identifier="local.$(printf '%s' "$app_name" | tr '_' '-')"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp resources/app-icon.icns "$bundle/Contents/Resources/AppIcon.icns"
cp "target/$host/release/$app_name" "$bundle/Contents/MacOS/$app_name.new"
mv "$bundle/Contents/MacOS/$app_name.new" "$bundle/Contents/MacOS/$app_name"
cat > "$bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>$display_name</string>
<key>CFBundleDisplayName</key><string>$display_name</string>
<key>CFBundleIdentifier</key><string>$identifier</string>
<key>CFBundleExecutable</key><string>$app_name</string>
<key>CFBundleIconFile</key><string>AppIcon.icns</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>0.1.0</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
</dict></plist>
PLIST
plutil -lint "$bundle/Contents/Info.plist"
codesign --force --sign - --timestamp=none "$bundle"
codesign --verify --deep --strict "$bundle"
printf 'Built %s\nOpen it with: open "%s"\n' "$bundle" "$bundle"
