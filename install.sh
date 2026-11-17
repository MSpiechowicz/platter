#!/bin/sh
# Installs Platter for the current user from a GitHub release: the AppImage goes to ~/.local/bin/platter,
# with a desktop entry and icons in ~/.local/share. Nothing is written outside your home folder and no
# administrator rights are asked for. The download is checked against the release's SHA256SUMS before it
# is run or installed.
#
#   curl -fsSL https://github.com/MSpiechowicz/platter/releases/latest/download/install.sh | sh
#   sh install.sh                 # the latest release
#   sh install.sh --version 0.1.0 # a given release
#   sh install.sh --uninstall     # removes what this script installed (your scans and settings stay)
#
# PLATTER_RELEASE_URL replaces the folder the files are downloaded from. A file:// URL works with curl only
# (for testing).
#
# Everything runs from `main` on the last line, so a download cut short runs nothing.
set -eu

repo=MSpiechowicz/platter
icon_sizes="32x32 128x128 256x256@2 512x512"

say() { printf '%s\n' "$*"; }
fail() {
  printf 'install.sh: %s\n' "$*" >&2
  exit 1
}

usage() {
  say "usage: install.sh [--version X.Y.Z] [--uninstall]"
}

refresh_caches() {
  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$data_dir/applications" >/dev/null 2>&1 || true
  fi
  if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t "$data_dir/icons/hicolor" >/dev/null 2>&1 || true
  fi
}

remove_icons() {
  for size in $icon_sizes; do
    rm -f "$data_dir/icons/hicolor/$size/apps/platter.png"
  done
}

# Whether the file is an AppImage of type 2 (bytes 8 to 10 are "AI" and 2), as every Platter build is.
# Anything else at the install path is not Platter's, so it is neither replaced nor removed.
is_appimage() {
  [ "$(od -An -tx1 -j8 -N3 "$1" 2>/dev/null | tr -d ' \n')" = 414902 ]
}

has_vulkan_loader() {
  for ldconfig in ldconfig /sbin/ldconfig /usr/sbin/ldconfig; do
    if command -v "$ldconfig" >/dev/null 2>&1; then
      "$ldconfig" -p 2>/dev/null | grep -q 'libvulkan\.so\.1' && return 0
      return 1
    fi
  done
  # No way to tell: no warning.
  return 0
}

main() {
  version=
  action=install
  while [ $# -gt 0 ]; do
    case $1 in
      --version)
        [ $# -ge 2 ] || fail "--version needs a version, such as 0.1.0."
        version=${2#v}
        shift 2
        ;;
      --uninstall) action=uninstall; shift ;;
      -h | --help) usage; exit 0 ;;
      *) usage >&2; exit 2 ;;
    esac
  done

  bin_dir=${XDG_BIN_HOME:-$HOME/.local/bin}
  data_dir=${XDG_DATA_HOME:-$HOME/.local/share}
  binary=$bin_dir/platter
  entry=$data_dir/applications/Platter.desktop

  # The path goes into Exec= of the desktop entry as it is: an absolute path of plain characters only.
  case $binary in
    /*) ;;
    *) fail "The install folder must be an absolute path: $bin_dir" ;;
  esac
  case $binary in
    *[!A-Za-z0-9/._-]*) fail "The install path has characters a desktop entry cannot hold: $binary" ;;
  esac
  if [ -e "$binary" ] && ! is_appimage "$binary"; then
    fail "$binary is not a Platter AppImage. Move it away first. Nothing was changed."
  fi

  if [ "$action" = uninstall ]; then
    rm -f "$binary" "$entry"
    remove_icons
    refresh_caches
    say "Platter was removed. Your scans and settings in $data_dir/platter were kept."
    exit 0
  fi

  [ "$(id -u)" -ne 0 ] || fail "Run it as yourself, not as root. It installs into your home folder."
  [ "$(uname -s)" = Linux ] || fail "This script installs the Linux build only."
  case $(uname -m) in
    x86_64 | amd64) ;;
    *) fail "There is no build for $(uname -m) yet. Only x86_64 is built." ;;
  esac
  command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is needed to check the download."
  if [ -n "$version" ]; then
    printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$' ||
      fail "$version is not a version, such as 0.1.0."
    [ -z "${PLATTER_RELEASE_URL:-}" ] || fail "Give either --version or PLATTER_RELEASE_URL, not both."
  fi

  if command -v curl >/dev/null 2>&1; then
    fetch() {
      curl --fail --silent --show-error --location --proto '=https,file' --proto-redir '=https' \
        --output "$2" "$1"
    }
  elif command -v wget >/dev/null 2>&1; then
    fetch() { wget --quiet --https-only --output-document="$2" "$1"; }
  else
    fail "curl or wget is needed to download Platter."
  fi

  if [ -n "${PLATTER_RELEASE_URL:-}" ]; then
    base=$PLATTER_RELEASE_URL
  elif [ -n "$version" ]; then
    base=https://github.com/$repo/releases/download/v$version
  else
    base=https://github.com/$repo/releases/latest/download
  fi

  mkdir -p "$bin_dir" "$data_dir/applications"
  work=$(mktemp -d)
  staged=$binary.new
  trap 'rm -rf "$work"; rm -f "$staged"' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM

  say "Downloading the checksums from $base"
  fetch "$base/SHA256SUMS" "$work/SHA256SUMS" || fail "SHA256SUMS could not be downloaded from $base."

  # The one AppImage line: "<64 hex digits>  Platter_<version>_amd64.AppImage", and nothing else.
  line=$(grep -E '^[0-9a-f]{64}  Platter_[0-9A-Za-z.+~-]+_amd64\.AppImage$' "$work/SHA256SUMS" || true)
  [ -n "$line" ] || fail "SHA256SUMS names no AppImage."
  [ "$(printf '%s\n' "$line" | wc -l)" -eq 1 ] || fail "SHA256SUMS names more than one AppImage."
  file=${line#*  }
  sum=${line%%  *}

  # Downloaded next to its final place and checked there, so the file that is checked is the file that is
  # run and installed (and a /tmp mounted noexec does not matter).
  say "Downloading $file"
  fetch "$base/$file" "$staged" || fail "$file could not be downloaded."
  printf '%s  %s\n' "$sum" "$staged" | sha256sum --check --status ||
    fail "$file does not match its checksum. Nothing was installed."
  say "The checksum matches."
  chmod 0755 "$staged"

  # The icons come out of the checked AppImage. It extracts itself without being mounted, so FUSE is not
  # needed. Without them the menu shows a generic icon, which is no reason to stop.
  if (cd "$work" && "$staged" --appimage-extract 'usr/share/icons/*' </dev/null >/dev/null 2>&1); then
    for size in $icon_sizes; do
      icon=$work/squashfs-root/usr/share/icons/hicolor/$size/apps/platter.png
      if [ -f "$icon" ]; then
        mkdir -p "$data_dir/icons/hicolor/$size/apps"
        cp "$icon" "$data_dir/icons/hicolor/$size/apps/platter.png"
      fi
    done
  else
    say "The icons could not be taken out of $file. The menu shows a generic icon."
  fi

  # Replaced in one step, so a running Platter keeps its file.
  mv -f "$staged" "$binary"

  # The file name and StartupWMClass match the window's app id ("Platter").
  cat >"$entry" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Platter
Comment=See what is using your disk
Exec=$binary
Icon=platter
Terminal=false
Categories=Utility;System;
StartupWMClass=Platter
DESKTOP
  refresh_caches

  say "Platter is installed: $binary"
  case :${PATH}: in
    *:"$bin_dir":*) ;;
    *) say "$bin_dir is not on your PATH. Start Platter from the app menu, or add the folder to PATH." ;;
  esac
  if ! has_vulkan_loader; then
    say "The Vulkan loader (libvulkan.so.1) was not found. Install it to run the advisor on the graphics card."
  fi
}

main "$@"
