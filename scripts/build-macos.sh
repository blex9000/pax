#!/usr/bin/env bash
set -euo pipefail

# Build Pax macOS App Bundle
# Requirements: cargo, gtk4, libadwaita, gtksourceview5 (via Homebrew)

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BUNDLE_DIR="$ROOT_DIR/target/release/bundle"
APP_DIR="$BUNDLE_DIR/Pax.app"

echo "==> Building Pax release binary (macOS, no VTE)..."
cd "$ROOT_DIR"
cargo build --release --no-default-features --features sourceview

echo "==> Creating macOS App Bundle..."
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

# Binary
cp "$ROOT_DIR/target/release/pax" "$APP_DIR/Contents/MacOS/pax"

# Icon (convert SVG to icns if possible, otherwise use SVG)
if command -v rsvg-convert &>/dev/null && command -v iconutil &>/dev/null; then
    echo "    Converting SVG to icns..."
    ICONSET="$APP_DIR/Contents/Resources/pax.iconset"
    mkdir -p "$ICONSET"
    for size in 16 32 64 128 256 512; do
        rsvg-convert -w $size -h $size "$ROOT_DIR/resources/icons/pax.svg" -o "$ICONSET/icon_${size}x${size}.png"
    done
    for size in 32 64 256 512; do
        half=$((size / 2))
        cp "$ICONSET/icon_${size}x${size}.png" "$ICONSET/icon_${half}x${half}@2x.png"
    done
    iconutil -c icns "$ICONSET" -o "$APP_DIR/Contents/Resources/pax.icns"
    rm -rf "$ICONSET"
else
    echo "    Warning: rsvg-convert or iconutil not found, skipping icns conversion"
    cp "$ROOT_DIR/resources/icons/pax.svg" "$APP_DIR/Contents/Resources/pax.svg"
fi

# Resources
cp -r "$ROOT_DIR/resources/icons" "$APP_DIR/Contents/Resources/icons"
if [ -d "$ROOT_DIR/resources/sourceview-styles" ]; then
    cp -r "$ROOT_DIR/resources/sourceview-styles" "$APP_DIR/Contents/Resources/sourceview-styles"
fi

# Info.plist
VERSION=$(grep '^version' "$ROOT_DIR/Cargo.toml" | head -1 | sed 's/.*"\(.*\)"/\1/')
cat > "$APP_DIR/Contents/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>Pax</string>
    <key>CFBundleDisplayName</key>
    <string>Pax</string>
    <key>CFBundleIdentifier</key>
    <string>com.pax.terminal</string>
    <key>CFBundleVersion</key>
    <string>${VERSION}</string>
    <key>CFBundleShortVersionString</key>
    <string>${VERSION}</string>
    <key>CFBundleExecutable</key>
    <string>pax</string>
    <key>CFBundleIconFile</key>
    <string>pax</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

echo ""
echo "==> Done! App bundle created: $APP_DIR"
echo "    Run with: open $APP_DIR"
echo "    Or copy to /Applications"
