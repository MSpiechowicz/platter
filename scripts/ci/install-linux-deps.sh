#!/usr/bin/env bash
# Installs the system packages needed to build the Tauri 2 desktop crate on Debian/Ubuntu.
# Mirrors the "Linux (Debian)" prerequisites of https://v2.tauri.app/start/prerequisites/ plus patchelf,
# which the AppImage bundler needs, and cmake, clang and libclang for the advisor model runtime
# (llama.cpp is built from source and its bindings are generated with bindgen), and the Vulkan headers,
# SPIR-V headers and glslc for its Vulkan backend (feature gpu-vulkan, on by default). Meant for CI runners;
# it uses sudo and apt-get.
#
# Ubuntu 22.04 (the runner, kept for the AppImage's glibc) has no glslc package, and its Vulkan headers are
# older than llama.cpp expects. There glslc and the headers come from LunarG's Vulkan SDK, pinned by version
# and checksum, and VULKAN_SDK points CMake at them. The link library stays the system's (libvulkan-dev).
set -euo pipefail

packages=(
  build-essential
  clang
  cmake
  curl
  file
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
  xz-utils
)

# A mirror that stops answering makes apt wait for ever (a release once sat 48 minutes in `apt-get update`):
# each connection gives up after 30 seconds and is tried again, and a whole command that still hangs is
# stopped after 10 minutes and run once more.
apt_options=(-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30)
apt_get() {
  timeout 10m sudo apt-get "${apt_options[@]}" "$@" || timeout 10m sudo apt-get "${apt_options[@]}" "$@"
}

apt_get update
apt_get install --yes --no-install-recommends "${packages[@]}"

# A newer Ubuntu has it as a package.
apt_get install --yes --no-install-recommends glslc 2>/dev/null || true

if ! command -v glslc >/dev/null 2>&1; then
  sdk_version=1.4.363.1
  sdk_sha256=b7867d8ca658563813af666f34b93db87f1b1c7d9d69d97e924c210c7ba33eb8
  sdk_file=vulkansdk-linux-x86_64-$sdk_version.tar.xz
  sdk_root=/opt/vulkan-sdk
  download=${RUNNER_TEMP:-/tmp}/$sdk_file

  curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
    --connect-timeout 30 --max-time 600 --retry 3 --retry-all-errors \
    --output "$download" "https://sdk.lunarg.com/sdk/download/$sdk_version/linux/$sdk_file"
  echo "$sdk_sha256  $download" | sha256sum --check --strict
  sudo mkdir -p "$sdk_root"
  sudo tar -xJf "$download" -C "$sdk_root" --strip-components=2 \
    "$sdk_version/x86_64/bin/glslc" "$sdk_version/x86_64/include"
  rm -f "$download"

  if [ -n "${GITHUB_ENV:-}" ]; then
    echo "VULKAN_SDK=$sdk_root" >>"$GITHUB_ENV"
    echo "$sdk_root/bin" >>"$GITHUB_PATH"
  else
    echo "Set VULKAN_SDK=$sdk_root and add $sdk_root/bin to PATH."
  fi
  "$sdk_root/bin/glslc" --version | head -n 1
fi
