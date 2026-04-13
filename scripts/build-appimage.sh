#!/usr/bin/env bash
set -euo pipefail

# Build Pax AppImage
# Requirements: cargo, libgtk-4-dev, libadwaita-1-dev, libvte-2.91-gtk4-dev, libgtksourceview-5-dev

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BUILD_TOOLS="$ROOT_DIR/target/build-tools"
APPDIR="$ROOT_DIR/target/AppDir"
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

# GtkSourceView5 styles + language specs (not bundled by linuxdeploy-plugin-gtk)
if [ -d "/usr/share/gtksourceview-5" ]; then
    echo "    Bundling GtkSourceView5 data..."
    cp -r /usr/share/gtksourceview-5 "$APPDIR/usr/share/gtksourceview-5"
fi

# Custom SourceView styles (Pax themes)
if [ -d "$ROOT_DIR/resources/sourceview-styles" ]; then
    mkdir -p "$APPDIR/usr/share/gtksourceview-5/styles"
    cp "$ROOT_DIR"/resources/sourceview-styles/*.xml "$APPDIR/usr/share/gtksourceview-5/styles/" 2>/dev/null || true
fi

# Adwaita icon theme (fallback if not bundled by plugin)
if [ -d "/usr/share/icons/Adwaita" ] && [ ! -d "$APPDIR/usr/share/icons/Adwaita" ]; then
    echo "    Bundling Adwaita icon theme..."
    cp -r /usr/share/icons/Adwaita "$APPDIR/usr/share/icons/Adwaita"
fi

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

echo "==> Preparing AppImage (bundling libraries + GTK plugin)..."
cd "$ROOT_DIR"

export DEPLOY_GTK_VERSION=4
export APPIMAGE_EXTRACT_AND_RUN=1
export OUTPUT="Pax-$ARCH.AppImage"

# Step 1: linuxdeploy bundles libs and GTK plugin (no AppImage output yet)
"$LINUXDEPLOY" \
    --appdir "$APPDIR" \
    --desktop-file "$APPDIR/usr/share/applications/pax.desktop" \
    --icon-file "$APPDIR/usr/share/icons/hicolor/scalable/apps/pax.svg" \
    --plugin gtk

# Step 2: patch the GTK plugin hook — linuxdeploy-plugin-gtk sets env vars
# that break libadwaita theming
HOOK_FILE="$APPDIR/apprun-hooks/linuxdeploy-plugin-gtk.sh"
if [ -f "$HOOK_FILE" ]; then
    echo "    Patching GTK plugin hook..."
    # GTK_THEME override breaks libadwaita (it manages its own theme via AdwStyleManager)
    sed -i 's/^export GTK_THEME=/#export GTK_THEME=/' "$HOOK_FILE"
    # GDK_BACKEND=x11 forced by plugin — remove to allow Wayland
    sed -i 's/^export GDK_BACKEND=x11/#export GDK_BACKEND=x11/' "$HOOK_FILE"
fi

# Step 3: create the AppImage
echo "==> Creating AppImage..."
"$LINUXDEPLOY" \
    --appdir "$APPDIR" \
    --output appimage

echo ""
echo "==> Done! AppImage created: $OUTPUT"
echo "    Run with: chmod +x $OUTPUT && ./$OUTPUT"
