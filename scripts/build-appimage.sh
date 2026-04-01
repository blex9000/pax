#!/usr/bin/env bash
set -euo pipefail

# Build Pax AppImage
# Requirements: cargo, libgtk-4-dev, libadwaita-1-dev, libvte-2.91-gtk4-dev, libgtksourceview-5-dev

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BUILD_TOOLS="$ROOT_DIR/build-tools"
APPDIR="$ROOT_DIR/AppDir"
ARCH="$(uname -m)"

echo "==> Building Pax release binary..."
cd "$ROOT_DIR"
cargo build --release

echo "==> Preparing AppDir..."
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin"
mkdir -p "$APPDIR/usr/resources/icons"
mkdir -p "$APPDIR/usr/share/applications"
mkdir -p "$APPDIR/usr/share/icons/hicolor/scalable/apps"

# Binary
cp "$ROOT_DIR/target/release/pax" "$APPDIR/usr/bin/pax"

# Desktop file
cp "$ROOT_DIR/pax.desktop" "$APPDIR/usr/share/applications/pax.desktop"

# Icons — app icon search path (../resources/icons relative to binary)
cp "$ROOT_DIR/resources/icons/pax.svg" "$APPDIR/usr/resources/icons/pax.svg"
cp "$ROOT_DIR/resources/icons/code-symbolic.svg" "$APPDIR/usr/resources/icons/code-symbolic.svg"

# Icons — hicolor theme (for GTK icon theme + desktop integration)
cp "$ROOT_DIR/resources/icons/pax.svg" "$APPDIR/usr/share/icons/hicolor/scalable/apps/pax.svg"

echo "==> Downloading linuxdeploy tools..."
mkdir -p "$BUILD_TOOLS"

LINUXDEPLOY="$BUILD_TOOLS/linuxdeploy-$ARCH.AppImage"
LINUXDEPLOY_GTK="$BUILD_TOOLS/linuxdeploy-plugin-gtk.sh"

if [ ! -f "$LINUXDEPLOY" ]; then
    echo "    Downloading linuxdeploy..."
    curl -fSL -o "$LINUXDEPLOY" \
        "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-$ARCH.AppImage"
    chmod +x "$LINUXDEPLOY"
fi

if [ ! -f "$LINUXDEPLOY_GTK" ]; then
    echo "    Downloading linuxdeploy-plugin-gtk..."
    curl -fSL -o "$LINUXDEPLOY_GTK" \
        "https://raw.githubusercontent.com/linuxdeploy/linuxdeploy-plugin-gtk/master/linuxdeploy-plugin-gtk.sh"
    chmod +x "$LINUXDEPLOY_GTK"
fi

echo "==> Building AppImage..."
cd "$ROOT_DIR"

export DEPLOY_GTK_VERSION=4
export APPIMAGE_EXTRACT_AND_RUN=1
export OUTPUT="Pax-$ARCH.AppImage"

"$LINUXDEPLOY" \
    --appdir "$APPDIR" \
    --desktop-file "$APPDIR/usr/share/applications/pax.desktop" \
    --icon-file "$APPDIR/usr/share/icons/hicolor/scalable/apps/pax.svg" \
    --plugin gtk \
    --output appimage

echo ""
echo "==> Done! AppImage created: $OUTPUT"
echo "    Run with: chmod +x $OUTPUT && ./$OUTPUT"
