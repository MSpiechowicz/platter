#!/bin/sh
# Tests the macOS branch of install.sh against a release folder on disk (no network): a matching app is
# installed, replaced by a newer one and removed again, and a changed one is refused. Run on a Mac.
#
#   scripts/ci/test-install-mac.sh
set -eu

here=$(cd "$(dirname "$0")/../.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# The release: a tarball with an app bundle in it, and the SHA256SUMS that names it.
make_release() {
  version=$1
  release=$tmp/release
  src=$tmp/src
  rm -rf "$release" "$src"
  mkdir -p "$release" "$src/Platter.app/Contents/MacOS"
  printf '%s\n' '<plist><dict><key>CFBundleIdentifier</key><string>io.github.mspiechowicz.platter</string></dict></plist>' \
    >"$src/Platter.app/Contents/Info.plist"
  printf '%s\n' "$version" >"$src/Platter.app/Contents/MacOS/platter"
  name=Platter_${version}_aarch64.app.tar.gz
  tar -czf "$release/$name" -C "$src" Platter.app
  hash=$(shasum -a 256 "$release/$name" | cut -d ' ' -f 1)
  printf '%s  %s\n' "$hash" "$name" >"$release/SHA256SUMS"
}

fail() {
  printf 'test-install-mac: %s\n' "$*" >&2
  exit 1
}

export HOME="$tmp/home"
mkdir -p "$HOME"
export PLATTER_RELEASE_URL="file://$tmp/release"
app=$HOME/Applications/Platter.app

make_release 9.9.9
sh "$here/install.sh" || fail "a matching app was refused."
[ "$(cat "$app/Contents/MacOS/platter")" = 9.9.9 ] || fail "the app was not installed."

make_release 9.9.10
sh "$here/install.sh" || fail "an update was refused."
[ "$(cat "$app/Contents/MacOS/platter")" = 9.9.10 ] || fail "the app was not replaced."
[ -z "$(find "$HOME/Applications" -maxdepth 1 -name '.platter-install.*')" ] || fail "a temporary folder was left."

# A tarball that no longer matches its checksum must leave the installed app as it was.
printf 'changed' >>"$release/Platter_9.9.10_aarch64.app.tar.gz"
if sh "$here/install.sh" >/dev/null 2>&1; then
  fail "a changed app was accepted."
fi
[ "$(cat "$app/Contents/MacOS/platter")" = 9.9.10 ] || fail "a refused download changed the installed app."

# A new app that cannot be moved into place leaves the installed one as it was, and no temporary folder.
# `mv` is replaced by a shim that fails for the new app (and, in the second run, for the restore as well).
shim=$tmp/shim
mkdir -p "$shim"
cat >"$shim/mv" <<'SHIM'
#!/bin/sh
case $1 in
  */unpacked/Platter.app) exit 1 ;;
  */.Platter.app.previous-*) [ "${MV_FAIL_RESTORE:-}" != 1 ] || exit 1 ;;
esac
exec /bin/mv "$@"
SHIM
chmod +x "$shim/mv"

make_release 9.9.11
if PATH="$shim:$PATH" sh "$here/install.sh" >/dev/null 2>&1; then
  fail "an install that could not move the app was reported as done."
fi
[ "$(cat "$app/Contents/MacOS/platter")" = 9.9.10 ] || fail "a failed move did not leave the installed app as it was."
[ -z "$(find "$HOME/Applications" -maxdepth 1 \( -name '.platter-install.*' -o -name '.Platter.app.previous-*' \))" ] ||
  fail "a failed move left a temporary folder."

# If even the restore fails, the previous app is kept outside the folder that is deleted, and its place is said.
output=$(MV_FAIL_RESTORE=1 PATH="$shim:$PATH" sh "$here/install.sh" 2>&1) && fail "a failed move was reported as done."
kept=$(find "$HOME/Applications" -maxdepth 1 -name '.Platter.app.previous-*')
[ -n "$kept" ] || fail "a failed restore lost the previous app."
[ "$(cat "$kept/Contents/MacOS/platter")" = 9.9.10 ] || fail "the kept app is not the previous one."
case $output in
  *"$kept"*) ;;
  *) fail "a failed restore did not say where the previous app is." ;;
esac
mv "$kept" "$app"

# Something that is not Platter at the install path is neither replaced nor removed.
other=$tmp/other-home
mkdir -p "$other/Applications/Platter.app"
if HOME=$other sh "$here/install.sh" --uninstall >/dev/null 2>&1; then
  fail "a folder that is not Platter was removed."
fi
[ -d "$other/Applications/Platter.app" ] || fail "a folder that is not Platter was touched."

sh "$here/install.sh" --uninstall || fail "uninstall failed."
[ ! -e "$app" ] || fail "uninstall left the app."

printf 'install.sh (macOS): checked.\n'
