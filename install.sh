#!/bin/sh
# Installs Platter for the current user from a GitHub release. On Linux the AppImage goes to
# ~/.local/bin/platter, with a desktop entry and icons in ~/.local/share. On a Mac with Apple Silicon the app
# goes to ~/Applications/Platter.app. Nothing is written outside your home folder and no administrator
# rights are asked for. The download is checked against the release's SHA256SUMS before it is run or
# installed.
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

# Whether the folder is a Platter app bundle (its Info.plist carries Platter's identifier). Anything else at
# the install path is not Platter's, so it is neither replaced nor removed.
is_platter_app() {
  [ -f "$1/Contents/Info.plist" ] && grep -q 'io\.github\.mspiechowicz\.platter' "$1/Contents/Info.plist"
}

check_version() {
  if [ -n "$version" ]; then
    printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$' ||
      fail "$version is not a version, such as 0.1.0."
    [ -z "${PLATTER_RELEASE_URL:-}" ] || fail "Give either --version or PLATTER_RELEASE_URL, not both."
  fi
}

# Defines `fetch URL FILE` with curl, or with wget where there is no curl.
choose_fetch() {
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
}

# Sets `base`, the folder the release's files are downloaded from.
choose_base() {
  if [ -n "${PLATTER_RELEASE_URL:-}" ]; then
    base=$PLATTER_RELEASE_URL
  elif [ -n "$version" ]; then
    base=https://github.com/$repo/releases/download/v$version
  else
    base=https://github.com/$repo/releases/latest/download
  fi
}

# The exit of the Mac install: removes the work folder and, if the previous app was moved aside and the new
# one is not in place, puts the previous one back (or says where it is).
cleanup_macos() {
  rm -rf "$work"
  [ -e "$previous" ] || return 0
  if [ ! -e "$app" ] && mv "$previous" "$app"; then
    printf 'install.sh: the previous Platter was put back in %s.\n' "$app" >&2
  else
    printf 'install.sh: the previous Platter is kept in %s.\n' "$previous" >&2
  fi
}

# The Mac build: an app bundle in ~/Applications. It is not notarized, but a download made by curl carries no
# quarantine mark, so Gatekeeper does not stop it.
install_macos() {
  app_dir=$HOME/Applications
  app=$app_dir/Platter.app

  [ "$(id -u)" -ne 0 ] || fail "Run it as yourself, not as root. It installs into your home folder."

  if [ -e "$app" ] && ! is_platter_app "$app"; then
    fail "$app is not Platter. Move it away first. Nothing was changed."
  fi

  if [ "$action" = uninstall ]; then
    rm -rf "$app"
    say "Platter was removed. Your scans and settings in ~/Library/Application Support/platter were kept."
    exit 0
  fi

  case $(uname -m) in
    arm64 | aarch64) ;;
    *) fail "There is no build for $(uname -m). Only Apple Silicon is built." ;;
  esac
  command -v shasum >/dev/null 2>&1 || fail "shasum is needed to check the download."
  command -v tar >/dev/null 2>&1 || fail "tar is needed to unpack the download."
  check_version
  choose_fetch
  choose_base

  mkdir -p "$app_dir"
  # The previous app waits here, outside `$work`, so that no cleanup deletes it. It is put back when the new
  # app is not in place on exit (a failed move, Ctrl-C), and removed only once the new app is.
  previous=$app_dir/.Platter.app.previous-$$
  # A leftover from an earlier run with the same process id would take the app inside it: refuse it.
  if [ -e "$previous" ] || [ -L "$previous" ]; then
    fail "$previous is left from an earlier install. Move it away and run the script again."
  fi
  # Inside ~/Applications, so the last step is a rename on one volume.
  work=$(mktemp -d "$app_dir/.platter-install.XXXXXX")
  trap 'cleanup_macos' EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM

  say "Downloading the checksums from $base"
  fetch "$base/SHA256SUMS" "$work/SHA256SUMS" || fail "SHA256SUMS could not be downloaded from $base."

  # The one app line: "<64 hex digits>  Platter_<version>_aarch64.app.tar.gz", and nothing else.
  line=$(grep -E '^[0-9a-f]{64}  Platter_[0-9A-Za-z.+~-]+_aarch64\.app\.tar\.gz$' "$work/SHA256SUMS" || true)
  [ -n "$line" ] || fail "SHA256SUMS names no Mac app."
  [ "$(printf '%s\n' "$line" | wc -l)" -eq 1 ] || fail "SHA256SUMS names more than one Mac app."
  file=${line#*  }
  sum=${line%%  *}

  say "Downloading $file"
  fetch "$base/$file" "$work/$file" || fail "$file could not be downloaded."
  [ "$(shasum -a 256 "$work/$file" | cut -d ' ' -f 1)" = "$sum" ] ||
    fail "$file does not match its checksum. Nothing was installed."
  say "The checksum matches."

  # Nothing outside the folder it is unpacked into: no absolute path and no `..`.
  if tar -tzf "$work/$file" | grep -Eq '^/|(^|/)\.\.(/|$)'; then
    fail "$file holds a path outside its folder. Nothing was installed."
  fi
  mkdir "$work/unpacked"
  tar -xzf "$work/$file" -C "$work/unpacked" || fail "$file could not be unpacked."
  is_platter_app "$work/unpacked/Platter.app" || fail "$file holds no Platter app. Nothing was installed."

  # The old app is moved aside first and put back (by `cleanup_macos`) if the new one cannot take its place,
  # so a failed install never leaves the folder without Platter. A running Platter keeps the files it has open.
  if [ -e "$app" ]; then
    mv "$app" "$previous" || fail "$app could not be replaced. Is Platter running from it?"
  fi
  mv "$work/unpacked/Platter.app" "$app" || fail "Platter could not be put in $app."
  rm -rf "$previous"

  say "Platter is installed: $app"
  say "Open it from Launchpad or Spotlight. To scan every folder, give it Full Disk Access in System Settings, Privacy & Security."
}

linux_main() {
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
  [ "$(uname -s)" = Linux ] || fail "This script installs the Linux and macOS builds only."
  case $(uname -m) in
    x86_64 | amd64) ;;
    *) fail "There is no build for $(uname -m) yet. Only x86_64 is built." ;;
  esac
  command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is needed to check the download."
  check_version

  choose_fetch
  choose_base

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

  case $(uname -s) in
    Darwin) install_macos ;;
    *) linux_main ;;
  esac
}

main "$@"
