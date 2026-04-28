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
mkdir -p "$APP_DIR/Contents/Resources/share/icons/hicolor/scalable/apps"
mkdir -p "$APP_DIR/Contents/Resources/share/icons/hicolor/symbolic/apps"

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
if [ -d "$ROOT_DIR/resources/share" ]; then
    cp -r "$ROOT_DIR/resources/share" "$APP_DIR/Contents/Resources/share"
fi
cp "$ROOT_DIR/resources/icons/pax.svg" \
   "$APP_DIR/Contents/Resources/share/icons/hicolor/scalable/apps/pax.svg"
cp "$ROOT_DIR/resources/icons/code-symbolic.svg" \
   "$APP_DIR/Contents/Resources/share/icons/hicolor/symbolic/apps/code-symbolic.svg"
if [ -d "$ROOT_DIR/resources/sourceview-styles" ]; then
    cp -r "$ROOT_DIR/resources/sourceview-styles" "$APP_DIR/Contents/Resources/sourceview-styles"
fi

ICON_THEME_ROOT=""
for prefix in "${HOMEBREW_PREFIX:-}" "$(command -v brew >/dev/null 2>&1 && brew --prefix 2>/dev/null || true)" /opt/homebrew /usr/local /opt/local /usr; do
    if [ -n "$prefix" ] && [ -d "$prefix/share/icons/Adwaita" ]; then
        ICON_THEME_ROOT="$prefix/share/icons"
        break
    fi
done

if [ -n "$ICON_THEME_ROOT" ]; then
    echo "    Bundling icon themes from $ICON_THEME_ROOT..."
    cp -R "$ICON_THEME_ROOT/Adwaita" "$APP_DIR/Contents/Resources/share/icons/"
    if [ -d "$ICON_THEME_ROOT/hicolor" ]; then
        cp -R "$ICON_THEME_ROOT/hicolor" "$APP_DIR/Contents/Resources/share/icons/"
    fi
else
    echo "    Warning: Adwaita icon theme not found; symbolic icons may be missing on macOS"
fi

# Regenerate the bundled Pax theme's icon cache so any new SVGs added to
# resources/share/icons/Pax are discovered by GTK on first run. Without
# this, GTK on macOS sometimes only resolves names that were present
# when the previous cache was generated.
if command -v gtk4-update-icon-cache &>/dev/null || command -v gtk-update-icon-cache &>/dev/null; then
    UPDATE_ICON_CACHE="$(command -v gtk4-update-icon-cache || command -v gtk-update-icon-cache)"
    PAX_THEME="$APP_DIR/Contents/Resources/share/icons/Pax"
    if [ -d "$PAX_THEME" ]; then
        echo "    Regenerating Pax icon cache..."
        "$UPDATE_ICON_CACHE" --force --ignore-theme-index "$PAX_THEME"
    fi
else
    echo "    Warning: gtk-update-icon-cache not found; bundled icon discovery may rely on directory scan only"
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
