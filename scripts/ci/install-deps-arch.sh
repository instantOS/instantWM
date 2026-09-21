#!/usr/bin/env bash
set -euo pipefail

# Install all build/test dependencies for instantWM on Arch Linux.
# Used by CI (.github/actions/setup-arch, which handles pacman keyring init
# and -Syu first) and can be run locally on an already-initialised system.

# cosmic-text (bar text rasterizer) panics with "no default font found"
# when shaping text on a system without any fonts, so tests need a real
# font package (ttf-dejavu below), not just the fontconfig/freetype libs.
pacman -S --noconfirm --needed \
  base-devel \
  rust \
  pkgconf \
  git \
  cmake \
  jq \
  python \
  pacman-contrib \
  fzf \
  sudo \
  xorg-server \
  xorg-server-xvfb \
  xorg-xev \
  xorg-xmessage \
  xorg-xprop \
  xorg-xwininfo \
  xdotool \
  libx11 \
  libxext \
  libxrandr \
  libxinerama \
  libxcb \
  libxkbcommon \
  libxcursor \
  libxdamage \
  libxfixes \
  libxi \
  libxres \
  libxtst \
  libxss \
  libxvmc \
  libxxf86vm \
  libxcomposite \
  libxrender \
  libxt \
  libxmu \
  libxpm \
  libxaw \
  fontconfig \
  freetype2 \
  ttf-dejavu \
  libxft \
  libdrm \
  wayland \
  wayland-protocols \
  libinput \
  seatd \
  egl-wayland \
  mesa \
  libglvnd \
  libevdev \
  libwacom \
  systemd \
  dbus \
  libliftoff \
  libdisplay-info \
  scdoc
