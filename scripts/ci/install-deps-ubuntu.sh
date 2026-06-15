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
  libxft-dev
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

  # Some :arm64 / :armhf packages (e.g. python3.12-minimal) run their own
  # foreign-arch interpreter from their postinst script. Without a qemu
  # user-mode binfmt handler registered, the kernel returns ENOEXEC and the
  # whole apt transaction aborts. Install qemu-user-static + binfmt-support
  # in a separate pass first so the handlers are registered before we pull
  # in any :arm64 / :armhf packages.
  #
  # NOTE: this requires /proc/sys/fs/binfmt_misc to be available inside the
  # container (true on standard Docker setups). If running in a container
  # where it isn't mounted, register handlers once on the host with:
  #   docker run --rm --privileged multiarch/qemu-user-static --reset -p yes
  apt-get update
  apt-get install -y --no-install-recommends qemu-user-static binfmt-support

  PKGS+=(
    gcc-aarch64-linux-gnu
    g++-aarch64-linux-gnu
    gcc-arm-linux-gnueabihf
    g++-arm-linux-gnueabihf
    # NOTE: the per-triple `pkg-config-<triple>` packages used to ship in
    # Ubuntu bionic but no longer exist in noble (24.04). In noble the cross
    # wrappers live inside `pkgconf:<arch>`, which conflicts with the native
    # `pkgconf:amd64` on `/usr/bin/pkg-config`, so we cannot co-install them.
    # We synthesise the wrappers ourselves below.
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
if $CROSS_COMPILE; then
  # Ubuntu 24.04 multi-arch packages like libgudev-1.0-dev ship .gir files in
  # /usr/share/gir-1.0/ that differ across architectures but share the same
  # path. dpkg treats this as a fatal conflict unless we force the overwrite.
  # This is a known packaging issue on Noble; the files are GObject
  # Introspection metadata and are not required for C/Rust compilation.
  apt-get install -y --no-install-recommends \
    -o Dpkg::Options::="--force-overwrite" \
    "${PKGS[@]}"

  # Some foreign-arch postinst scripts (e.g. libglib2.0-0t64) try to run
  # arch-specific helper binaries that may fail under qemu-user-static if
  # binfmt_misc isn't perfectly set up in the container. Those failures leave
  # packages half-configured, which is harmless for linking but can break
  # later apt operations. Attempt to finish configuration and ignore any errors.
  dpkg --configure -a || true
else
  apt-get install -y --no-install-recommends "${PKGS[@]}"
fi

if $CROSS_COMPILE; then
  # Create per-triple pkg-config wrappers so the `pkg-config` Rust crate (used
  # by libudev-sys, libinput-sys, etc.) picks up arm64/armhf .pc files.
  # cargo's pkg-config helper auto-detects `<triple>-pkg-config` on PATH when
  # cross-compiling; pointing PKG_CONFIG_LIBDIR at the arch-specific pkgconfig
  # directory plus the arch-independent /usr/share/pkgconfig is what wayland-
  # protocols and friends rely on.
  install_cross_pkgconfig() {
    local triple="$1" libdir="$2"
    cat > "/usr/local/bin/${triple}-pkg-config" <<EOF
#!/bin/sh
exec env \\
  PKG_CONFIG_LIBDIR="/usr/lib/${libdir}/pkgconfig:/usr/share/pkgconfig" \\
  PKG_CONFIG_SYSROOT_DIR="\${PKG_CONFIG_SYSROOT_DIR:-/}" \\
  PKG_CONFIG_ALLOW_CROSS=1 \\
  pkg-config "\$@"
EOF
    chmod +x "/usr/local/bin/${triple}-pkg-config"
  }
  install_cross_pkgconfig aarch64-linux-gnu     aarch64-linux-gnu
  install_cross_pkgconfig arm-linux-gnueabihf   arm-linux-gnueabihf
fi
