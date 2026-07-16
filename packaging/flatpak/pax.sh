#!/usr/bin/env bash
set -euo pipefail

# Pax — self-contained Flatpak installer / launcher.
#
# This script ships INSIDE the distribution zip, next to the
# dev.blex.pax.flatpak bundle. It is NOT the repo build helper
# (that is scripts/flatpak.sh) — it only installs the adjacent bundle
# and runs it, with no repo or manifest needed.
#
#   ./pax.sh            install (if needed) then run
#   ./pax.sh install    (re)install the bundled .flatpak (--user)
#   ./pax.sh run        run the app
#   ./pax.sh uninstall  remove the app
#
# If the executable bit was lost while unzipping, run: bash pax.sh

APP_ID="dev.blex.pax"
# Stamped by CI at package time (workspace version + short commit). Stays
# the literal placeholder when the script is run straight from the repo.
PAX_VERSION="@PAX_VERSION@"
# If unstamped (run straight from the repo), the value still contains '@'.
case "$PAX_VERSION" in *@*) PAX_VERSION="dev" ;; esac

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUNDLE="$(ls "$DIR"/*.flatpak 2>/dev/null | head -1 || true)"

command -v flatpak >/dev/null 2>&1 || {
    echo "error: flatpak is not installed." >&2
    echo "       Install it (e.g. 'sudo apt install flatpak'), then log out/in." >&2
    echo "       See https://flatpak.org/setup/ for other distros." >&2
    exit 1
}

is_installed() {
    flatpak info --user "$APP_ID" >/dev/null 2>&1 || flatpak info "$APP_ID" >/dev/null 2>&1
}

do_install() {
    [ -n "$BUNDLE" ] || {
        echo "error: no .flatpak bundle found next to this script (in $DIR)." >&2
        exit 1
    }
    # The app depends on the GNOME runtime, pulled from Flathub — add it
    # as a --user remote if missing so the install can resolve it.
    flatpak remote-list --user 2>/dev/null | grep -q '^flathub' || \
        flatpak remote-add --user --if-not-exists flathub \
            https://dl.flathub.org/repo/flathub.flatpakrepo
    echo "==> Installing $(basename "$BUNDLE") (--user)…"
    flatpak install --user -y --bundle "$BUNDLE"
}

case "${1:-auto}" in
    auto)      echo "Pax ${PAX_VERSION}"; is_installed || do_install; exec flatpak run "$APP_ID" ;;
    install)   echo "Pax ${PAX_VERSION}"; do_install ;;
    run)       echo "Pax ${PAX_VERSION}"; exec flatpak run "$APP_ID" ;;
    uninstall) flatpak uninstall --user -y "$APP_ID" ;;
    version|--version|-V) echo "Pax ${PAX_VERSION}" ;;
    -h|--help|help) sed -n '3,17p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' ;;
    *) echo "usage: ./pax.sh [install|run|uninstall|version]" >&2; exit 1 ;;
esac
