#!/usr/bin/env bash
# Installs the system packages needed to build the Tauri 2 desktop crate on Debian/Ubuntu.
# Mirrors the "Linux (Debian)" prerequisites of https://v2.tauri.app/start/prerequisites/ plus patchelf,
# which the AppImage bundler needs. Meant for CI runners; it uses sudo and apt-get.
set -euo pipefail

packages=(
  build-essential
  curl
  file
  libayatana-appindicator3-dev
  libgtk-3-dev
  librsvg2-dev
  libssl-dev
  libwebkit2gtk-4.1-dev
  libxdo-dev
  patchelf
  pkg-config
  wget
)

sudo apt-get update
sudo apt-get install --yes --no-install-recommends "${packages[@]}"
