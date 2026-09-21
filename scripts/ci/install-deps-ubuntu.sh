#!/usr/bin/env bash
set -euo pipefail

# Build dependencies for instantWM's CI (and for local use).
#
#   bash scripts/ci/install-deps-ubuntu.sh                      # host dev libs
#   bash scripts/ci/install-deps-ubuntu.sh --cross arm64 <dir>  # cross toolchain + sysroot
#   eval "$(scripts/ci/install-deps-ubuntu.sh --env arm64 <dir>)"
#
# Native mode installs the X11/Wayland dev libraries the project links
# against into the host.
#
# Cross mode installs the cross toolchain plus a self-contained target sysroot
# and prints the environment (--env) that points cargo/pkg-config/cc at it:
#
#   sysroot="$(pwd)/sysroot/arm64"
#   scripts/ci/install-deps-ubuntu.sh --cross arm64 "$sysroot"
#   eval "$(scripts/ci/install-deps-ubuntu.sh --env arm64 "$sysroot")"
#   cargo build --release --target aarch64-unknown-linux-gnu
#
# Cross mode deliberately does NOT use Debian multiarch. Co-installing amd64
# and arm64 packages into one dpkg database forces every Multi-Arch: same
# package to have the SAME version on both architectures, and archive.ubuntu.com
# and ports.ubuntu.com publish SRUs at slightly different times - so a package
# can be updated on one mirror but not the other, which makes the foreign-arch
# candidate "not installable" and breaks the whole apt transaction (CI hit this
# repeatedly: libexpat1 on noble, libexpat1-dev on resolute/armhf).
#
# With a sysroot there is nothing to keep in sync: the target libraries are
# resolved against the ports mirror with a private apt root and unpacked with
# dpkg-deb -x, exactly like `debootstrap --foreign`'s first stage. Nothing in
# the sysroot is ever executed, so no qemu/binfmt handlers are required either.
# A package being newer on one mirror simply means that architecture is a
# little ahead - harmless, because no cross-arch version constraint exists.

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

# Map a dpkg architecture to its Rust target triple and GNU triplet.
arch_target() {
  case "$1" in
    arm64) echo "aarch64-unknown-linux-gnu" ;;
    armhf) echo "armv7-unknown-linux-gnueabihf" ;;
  esac
}

arch_triple() {
  case "$1" in
    arm64) echo "aarch64-linux-gnu" ;;
    armhf) echo "arm-linux-gnueabihf" ;;
  esac
}

# Name of the dynamic loader as referenced by the linker (e.g. the interpreter
# entry in produced executables).
arch_loader() {
  case "$1" in
    arm64) echo "ld-linux-aarch64.so.1" ;;
    armhf) echo "ld-linux-armhf.so.3" ;;
  esac
}

usage() {
  cat >&2 <<'EOF'
Usage:
  install-deps-ubuntu.sh                  Install host build dependencies
  install-deps-ubuntu.sh --cross <arch> [dir]   Build cross toolchain + sysroot
  install-deps-ubuntu.sh --env <arch> [dir]     Print the cross build environment

  <arch> is arm64 or armhf. [dir] defaults to $PWD/sysroot/<arch>.
EOF
  exit 1
}

native_install() {
  # fonts-dejavu-core: cosmic-text (bar text rasterizer) panics with "no
  # default font found" when shaping text on a system without any fonts, so
  # tests need a real font package, not just fontconfig/freetype dev headers.
  apt-get update
  apt-get install -y --no-install-recommends \
    build-essential \
    pkg-config \
    wayland-protocols \
    fonts-dejavu-core \
    curl \
    ca-certificates \
    "${DEV_LIBS[@]}"
}

cross_sysroot_dir() {
  echo "${1:-$PWD/sysroot/$2}"
}

cross_install() {
  local arch="$1" sysroot="$2" codename triple
  codename="$(. /etc/os-release && printf '%s' "$VERSION_CODENAME")"
  triple="$(arch_triple "$arch")"

  echo "==> Installing ${triple} cross toolchain"
  apt-get update
  apt-get install -y --no-install-recommends \
    "binutils-${triple}" \
    "gcc-${triple}" \
    "g++-${triple}" \
    pkg-config \
    ca-certificates

  echo "==> Resolving ${arch} packages for the sysroot"
  local aptroot
  aptroot="$(mktemp -d)"
  trap 'rm -rf "$aptroot"' RETURN

  mkdir -p \
    "$aptroot/etc/apt/preferences.d" \
    "$aptroot/var/lib/dpkg/info" \
    "$aptroot/var/lib/dpkg/updates" \
    "$aptroot/var/cache/apt/archives/partial"
  : >"$aptroot/var/lib/dpkg/status"

  # arch= and signed-by= are explicit so the private apt root needs neither the
  # host's sources nor its trusted keyring configuration.
  cat >"$aptroot/etc/apt/sources.list" <<EOF
deb [arch=${arch} signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] http://ports.ubuntu.com/ubuntu-ports ${codename} main restricted universe multiverse
deb [arch=${arch} signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] http://ports.ubuntu.com/ubuntu-ports ${codename}-updates main restricted universe multiverse
deb [arch=${arch} signed-by=/usr/share/keyrings/ubuntu-archive-keyring.gpg] http://ports.ubuntu.com/ubuntu-ports ${codename}-security main restricted universe multiverse
EOF

  local -a apt_opts=(
    -o "Dir=${aptroot}"
    -o "Dir::Etc::sourcelist=${aptroot}/etc/apt/sources.list"
    -o "Dir::Etc::sourceparts=-"
    -o "Dir::State::status=${aptroot}/var/lib/dpkg/status"
    -o "APT::Architecture=${arch}"
    -o "APT::Architectures::=${arch}"
    -o "APT::Get::List-Cleanup=0"
  )

  # An empty dpkg status means apt resolves the full dependency closure of the
  # requested packages; --download-only stops before any configuration step.
  apt-get "${apt_opts[@]}" update
  apt-get "${apt_opts[@]}" install --download-only --no-install-recommends -y \
    libc6-dev \
    wayland-protocols \
    "${DEV_LIBS[@]}"

  echo "==> Unpacking into ${sysroot}"
  rm -rf "$sysroot"
  mkdir -p "$sysroot"
  find "$aptroot/var/cache/apt/archives" -maxdepth 1 -name '*.deb' \
    -exec dpkg-deb -x '{}' "$sysroot" \;

  # The merged-/usr symlinks (including the dynamic loader the linker records as
  # the interpreter of the produced binaries) are created by libc6's postinst,
  # which we deliberately never run. Recreate the one the linker needs; without
  # it linking fails with "cannot find /lib/<loader> inside <sysroot>".
  local loader
  loader="$(arch_loader "$arch")"
  ln -sfn usr/lib "$sysroot/lib"
  ln -sfn "${triple}/${loader}" "$sysroot/lib/${loader}"

  # Prepending this directory to PATH makes the C compilers rustc/cc-rs invoke
  # for the target add --sysroot automatically, so the target headers and
  # libraries are found without patching every crate's build script.
  mkdir -p "$sysroot/bin"
  local name
  for prefix in "$triple" "$(arch_target "$arch")"; do
    for cc in gcc g++; do
      name="$sysroot/bin/${prefix}-${cc}"
      cat >"$name" <<EOF
#!/bin/sh
exec /usr/bin/${triple}-${cc} --sysroot=${sysroot} "\$@"
EOF
      chmod +x "$name"
    done
  done
}

cross_env() {
  local arch="$1" sysroot="$2" triple target target_env
  triple="$(arch_triple "$arch")"
  target="$(arch_target "$arch")"
  target_env="$(printf '%s' "$target" | tr 'a-z-' 'A-Z_')"

  cat <<EOF
export PATH='${sysroot}/bin':"\$PATH"
export CARGO_TARGET_${target_env}_LINKER='${sysroot}/bin/${triple}-gcc'
export PKG_CONFIG_ALLOW_CROSS=1
export PKG_CONFIG_SYSROOT_DIR='${sysroot}'
export PKG_CONFIG_LIBDIR='${sysroot}/usr/lib/${triple}/pkgconfig:${sysroot}/usr/share/pkgconfig'
EOF
}

case "${1:-}" in
  "" | --native)
    native_install
    ;;
  --cross)
    [[ $# -ge 2 ]] || usage
    arch="$2"
    case "$arch" in arm64 | armhf) ;; *) usage ;; esac
    cross_install "$arch" "$(cross_sysroot_dir "${3:-}" "$arch")"
    ;;
  --env)
    [[ $# -ge 2 ]] || usage
    arch="$2"
    case "$arch" in arm64 | armhf) ;; *) usage ;; esac
    cross_env "$arch" "$(cross_sysroot_dir "${3:-}" "$arch")"
    ;;
  *)
    usage
    ;;
esac
