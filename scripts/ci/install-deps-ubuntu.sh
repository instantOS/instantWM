#!/usr/bin/env bash
set -euo pipefail

# Install build dependencies for instantWM on Ubuntu 24.04 (Noble).
# Used by CI (release.yml cross-compile and deb jobs) and can be run locally.
#
# Usage:
#   bash scripts/ci/install-deps-ubuntu.sh            # base X11/Wayland dev libs
#   bash scripts/ci/install-deps-ubuntu.sh --cross    # also install cross-compile toolchains

CROSS_COMPILE=false
if [[ "${1:-}" == "--cross" ]]; then
  CROSS_COMPILE=true
fi

apt-get update

PKGS=(
  build-essential
  pkg-config
  libx11-dev
  libxext-dev
  libxrandr-dev
  libxinerama-dev
  libxcb1-dev
  libxkbcommon-dev
  libxcursor-dev
  libxdamage-dev
  libxfixes-dev
  libxi-dev
  libxres-dev
  libxtst-dev
  libxss-dev
  libxvmc-dev
  libxxf86vm-dev
  libxcomposite-dev
  libxrender-dev
  libxt-dev
  libxmu-dev
  libxpm-dev
  libxaw7-dev
  libfontconfig1-dev
  libfreetype-dev
  libdrm-dev
  libgbm-dev
  libwayland-dev
  wayland-protocols
  libinput-dev
  libseat-dev
  libegl-dev
  libgl-dev
  libevdev-dev
  libwacom-dev
  libdbus-1-dev
  libliftoff-dev
  libdisplay-info-dev
)

if $CROSS_COMPILE; then
  PKGS+=(
    gcc-aarch64-linux-gnu
    g++-aarch64-linux-gnu
    gcc-arm-linux-gnueabihf
    g++-arm-linux-gnueabihf
    musl-tools
    unzip
    curl
    ca-certificates
  )
fi

apt-get install -y --no-install-recommends "${PKGS[@]}"
