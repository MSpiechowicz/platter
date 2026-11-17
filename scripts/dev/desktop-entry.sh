#!/usr/bin/env sh
# Installs (or removes) a desktop entry and icons for a locally built Platter,
# so the window and taskbar show the Platter icon when the binary is run from
# the build folder. Packaged installs (deb, rpm, AppImage) bring their own.
#
#   scripts/dev/desktop-entry.sh install   # default
#   scripts/dev/desktop-entry.sh remove
set -eu

repo=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}
entry=$data/applications/Platter.desktop
# The name this script used before the app id became "Platter".
old_entry=$data/applications/platter.desktop
icons=$repo/apps/desktop/src-tauri/icons
binary=$repo/target/release/platter

case ${1:-install} in
  install)
    [ -x "$binary" ] || { echo "Build the app first: pnpm -C apps/desktop tauri build --no-bundle" >&2; exit 1; }
    # The path goes into Exec= as one quoted argument (desktop entry spec): refuse what cannot be
    # written there safely.
    case $binary in
      *[!A-Za-z0-9/._\ -]*) echo "The build path has characters a desktop entry cannot hold: $binary" >&2; exit 1 ;;
    esac
    mkdir -p "$data/applications"
    for size in 32x32 128x128 256x256 512x512; do mkdir -p "$data/icons/hicolor/$size/apps"; done
    mkdir -p "$data/icons/hicolor/scalable/apps"
    cp "$icons/32x32.png" "$data/icons/hicolor/32x32/apps/platter.png"
    cp "$icons/128x128.png" "$data/icons/hicolor/128x128/apps/platter.png"
    cp "$icons/128x128@2x.png" "$data/icons/hicolor/256x256/apps/platter.png"
    cp "$icons/icon.png" "$data/icons/hicolor/512x512/apps/platter.png"
    cp "$icons/icon.svg" "$data/icons/hicolor/scalable/apps/platter.svg"
    # The file name and StartupWMClass must match the window's app id ("Platter", APP_ID in lib.rs).
    rm -f "$old_entry"
    cat > "$entry" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Platter
Comment=See what is using your disk
Exec="$binary"
Icon=platter
Terminal=false
Categories=Utility;System;
StartupWMClass=Platter
DESKTOP
    echo "Installed $entry"
    ;;
  remove)
    rm -f "$entry" "$old_entry" "$data/icons/hicolor/scalable/apps/platter.svg"
    for size in 32x32 128x128 256x256 512x512; do rm -f "$data/icons/hicolor/$size/apps/platter.png"; done
    echo "Removed the Platter desktop entry and icons"
    ;;
  *) echo "usage: $0 [install|remove]" >&2; exit 2 ;;
esac

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$data/applications" >/dev/null 2>&1 || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$data/icons/hicolor" >/dev/null 2>&1 || true
command -v kbuildsycoca6 >/dev/null 2>&1 && kbuildsycoca6 >/dev/null 2>&1 || true
