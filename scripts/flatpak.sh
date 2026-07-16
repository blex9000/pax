#!/usr/bin/env bash
set -euo pipefail

# Pax Flatpak helper — build, install and run Pax as a Flatpak.
#
# A Flatpak runs against the GNOME 48 runtime (its own GTK4 + glibc), so it
# works on distros whose system GTK is too old or absent (e.g. Ubuntu 20.04
# / Linux Mint 20) where the AppImage cannot run.
#
# Usage:
#   scripts/flatpak.sh build            Build from source and install (--user)
#   scripts/flatpak.sh bundle [OUT]     Build and export a single-file .flatpak
#   scripts/flatpak.sh install FILE     Install a downloaded .flatpak bundle (--user)
#   scripts/flatpak.sh run              Run the installed app
#   scripts/flatpak.sh help             Show this help

APP_ID="dev.blex.pax"
MANIFEST="dev.blex.pax.yml"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BUILD_DIR="$ROOT_DIR/target/flatpak-build"
STATE_DIR="$ROOT_DIR/target/.flatpak-builder"

die() { echo "error: $*" >&2; exit 1; }

need() {
    command -v "$1" >/dev/null 2>&1 || die "'$1' not found. Install it first (e.g. 'sudo apt install $2')."
}

ensure_flathub() {
    # Runtimes are pulled from Flathub; add it as a --user remote if missing.
    flatpak remote-list --user 2>/dev/null | grep -q '^flathub' || \
        flatpak remote-add --user --if-not-exists flathub \
            https://dl.flathub.org/repo/flathub.flatpakrepo
}

cmd_build() {
    need flatpak flatpak
    need flatpak-builder flatpak-builder
    ensure_flathub
    cd "$ROOT_DIR"
    echo "==> Building $APP_ID from $MANIFEST (installs runtime/SDK deps on first run)..."
    flatpak-builder --user --install --force-clean \
        --install-deps-from=flathub \
        --state-dir="$STATE_DIR" \
        "$BUILD_DIR" "$MANIFEST"
    echo "==> Done. Run with: scripts/flatpak.sh run"
}

cmd_bundle() {
    need flatpak flatpak
    need flatpak-builder flatpak-builder
    ensure_flathub
    local out="${1:-$ROOT_DIR/${APP_ID}.flatpak}"
    local repo="$ROOT_DIR/target/flatpak-repo"
    cd "$ROOT_DIR"
    echo "==> Building into an OSTree repo..."
    flatpak-builder --user --force-clean --repo="$repo" \
        --install-deps-from=flathub \
        --state-dir="$STATE_DIR" \
        "$BUILD_DIR" "$MANIFEST"
    echo "==> Exporting single-file bundle -> $out"
    flatpak build-bundle "$repo" "$out" "$APP_ID"
    echo "==> Done: $out"
    echo "    Install with: scripts/flatpak.sh install \"$out\""
}

cmd_install() {
    need flatpak flatpak
    local file="${1:?usage: scripts/flatpak.sh install FILE.flatpak}"
    [ -f "$file" ] || die "file not found: $file"
    echo "==> Installing $file (--user)..."
    # --bundle keeps this working on older flatpak too.
    flatpak install --user -y --bundle "$file"
    echo "==> Done. Run with: scripts/flatpak.sh run"
}

cmd_run() {
    need flatpak flatpak
    flatpak info "$APP_ID" >/dev/null 2>&1 || \
        die "$APP_ID is not installed. Use 'scripts/flatpak.sh install FILE' or 'build' first."
    exec flatpak run "$APP_ID"
}

usage() { sed -n '3,15p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; }

case "${1:-help}" in
    build)   cmd_build ;;
    bundle)  shift; cmd_bundle "$@" ;;
    install) shift; cmd_install "$@" ;;
    run)     cmd_run ;;
    help|-h|--help) usage ;;
    *) usage; exit 1 ;;
esac
