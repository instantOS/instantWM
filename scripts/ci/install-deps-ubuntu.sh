#!/usr/bin/env bash
set -euo pipefail

# Install build dependencies for instantWM on Ubuntu 24.04 (Noble).
# Used by CI (release.yml cross-compile and deb jobs) and can be run locally.
#
# Usage:
#   bash scripts/ci/install-deps-ubuntu.sh            # base X11/Wayland dev libs
#   bash scripts/ci/install-deps-ubuntu.sh --cross    # also install cross-compile
#                                                     # toolchains plus arm64/armhf
#                                                     # dev libs for cross builds

CROSS_COMPILE=false
if [[ "${1:-}" == "--cross" ]]; then
  CROSS_COMPILE=true
fi

# Native dev libraries needed to build instantwm against the host glibc.
# When cross-compiling we also need an :arm64 / :armhf copy of each of these
# (multi-arch). Architecture-independent helpers like wayland-protocols only
# need to be installed once.
DEV_LIBS=(
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
  libinput-dev
  libseat-dev
  libegl-dev
  libgl-dev
  libevdev-dev
  libwacom-dev
  libdbus-1-dev
  libliftoff-dev
  libdisplay-info-dev
  libudev-dev
)

PKGS=(
  build-essential
  pkg-config
  wayland-protocols
  "${DEV_LIBS[@]}"
)

if $CROSS_COMPILE; then
  # archive.ubuntu.com only carries amd64/i386 binaries, while arm64/armhf
  # live on ports.ubuntu.com. Constrain the default sources to amd64 and add
  # a separate ports source for the cross architectures so apt doesn't try to
  # fetch arm64 packages from a mirror that doesn't have them.
  if [[ -f /etc/apt/sources.list.d/ubuntu.sources ]]; then
    # Ubuntu 24.04 ships sources in deb822 format. Add an `Architectures:`
    # field to every stanza so we keep pulling amd64 from archive.ubuntu.com.
    if ! grep -q '^Architectures:' /etc/apt/sources.list.d/ubuntu.sources; then
      sed -i '/^Types:/a Architectures: amd64' /etc/apt/sources.list.d/ubuntu.sources
    fi
  fi

  if [[ ! -f /etc/apt/sources.list.d/ubuntu-ports.sources ]]; then
    cat > /etc/apt/sources.list.d/ubuntu-ports.sources <<'EOF'
Types: deb
URIs: http://ports.ubuntu.com/ubuntu-ports
Suites: noble noble-updates noble-backports noble-security
Components: main restricted universe multiverse
Architectures: arm64 armhf
Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg
EOF
  fi

  dpkg --add-architecture arm64
  dpkg --add-architecture armhf

  PKGS+=(
    gcc-aarch64-linux-gnu
    g++-aarch64-linux-gnu
    gcc-arm-linux-gnueabihf
    g++-arm-linux-gnueabihf
    # Cross pkg-config wrappers. cargo's `pkg-config` build helper looks for
    # `<triple>-pkg-config` automatically when cross-compiling, so installing
    # these is what unblocks crates like `libudev-sys`.
    pkg-config-aarch64-linux-gnu
    pkg-config-arm-linux-gnueabihf
    unzip
    curl
    ca-certificates
  )

  # Mirror DEV_LIBS for arm64/armhf so smithay/wayland/X11 -sys crates can
  # find their native deps via the cross pkg-config wrappers.
  for lib in "${DEV_LIBS[@]}"; do
    PKGS+=("${lib}:arm64" "${lib}:armhf")
  done
fi

apt-get update
apt-get install -y --no-install-recommends "${PKGS[@]}"
