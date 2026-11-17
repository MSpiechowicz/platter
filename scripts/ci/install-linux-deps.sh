#!/usr/bin/env bash
# Installs the system packages needed to build the Tauri 2 desktop crate on Debian/Ubuntu.
# Mirrors the "Linux (Debian)" prerequisites of https://v2.tauri.app/start/prerequisites/ plus patchelf,
# which the AppImage bundler needs, and cmake, clang and libclang for the advisor model runtime
# (llama.cpp is built from source and its bindings are generated with bindgen), and the Vulkan headers,
# SPIR-V headers and glslc for its Vulkan backend (feature gpu-vulkan, on by default). Meant for CI runners;
# it uses sudo and apt-get.
set -euo pipefail

packages=(
  build-essential
  clang
  cmake
  curl
  file
  glslc
  libayatana-appindicator3-dev
  libclang-dev
  libgtk-3-dev
  librsvg2-dev
  libssl-dev
  libvulkan-dev
  libwebkit2gtk-4.1-dev
  libxdo-dev
  patchelf
  pkg-config
  spirv-headers
  wget
)

sudo apt-get update
sudo apt-get install --yes --no-install-recommends "${packages[@]}"
