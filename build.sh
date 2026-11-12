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

if [ -n "${MACOS_SIGNING_IDENTITY:-}" ]; then
    pinned_identity=$(cat resources/signing-certificate.sha1)
    case "$pinned_identity" in
        ''|*[!A-F0-9]*) printf '%s\n' 'Invalid pinned signing certificate fingerprint.' >&2; exit 1 ;;
    esac
    if [ "${#pinned_identity}" -ne 40 ] || [ "$MACOS_SIGNING_IDENTITY" != "$pinned_identity" ]; then
        printf '%s\n' 'MACOS_SIGNING_IDENTITY must match resources/signing-certificate.sha1.' >&2
        exit 1
    fi
fi

host=$(rustup run stable rustc -vV | sed -n 's/^host: //p')
rustup run stable cargo build --release --locked --target "$host" --target-dir target
version=$(rustup run stable cargo pkgid --offline | sed 's/.*@//')

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
<key>CFBundleVersion</key><string>$version</string>
<key>CFBundleShortVersionString</key><string>$version</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSPrincipalClass</key><string>NSApplication</string>
</dict></plist>
PLIST
plutil -lint "$bundle/Contents/Info.plist"
if [ -n "${MACOS_SIGNING_IDENTITY:-}" ]; then
    requirement="identifier \"$identifier\" and certificate leaf = H\"$MACOS_SIGNING_IDENTITY\""
    codesign --force --sign "$MACOS_SIGNING_IDENTITY" --timestamp=none --requirements "=designated => $requirement" "$bundle"
    codesign --verify --strict --test-requirement "=$requirement" "$bundle"
else
    codesign --force --sign - --timestamp=none "$bundle"
fi
codesign --verify --deep --strict "$bundle"
printf 'Built %s\nOpen it with: open "%s"\n' "$bundle" "$bundle"
