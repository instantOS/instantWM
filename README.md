<div align="center">
    <h1>instantWM</h1>
    <p>Window manager for instantOS</p>
    <img width="300" height="300" src="https://raw.githubusercontent.com/instantOS/instantLOGO/main/png/wm.png">
</div>

instantWM: a really nice WM

Yes, X11 and Wayland. 

Yes, Mouse and Keyboard. 

Yes, very customizeable. 

Yes, looks nice and is fast. Have your WM and eat it too. 

![img](https://github.com/instantOS/instantLOGO/blob/main/screeenshots/screenshot1.png)

## Installation

Please keep in mind that instantWM is not a full desktop environment and relies
on external tools like instantMENU, instantCLI or i3status-rust for some functionality. 

### instantOS

InstantWM is preinstalled and preconfigured on instantOS.

### Arch

Either add the `https://packages.instantos.io` repo to your pacman.conf or
download the latest pkg file from the GitHub releases. 

### Ubuntu/Debian

Download the latest deb file from the GitHub releases. 

### From Source


```sh
git clone --depth=1 https://github.com/instantOS/instantWM.git
cd instantWM
just install
```

This requires the dependencies listed below or equivalents thereoff to be installed. 

#### Arch Dependencies

```bash
sudo pacman -Sy needed libx11 libxcb libxkbcommon libxcursor libxinerama libxrandr libxss libxtst libxfixes libxdamage libxcomposite libxrender libxft libxi libxres libxvmc libxxf86vm libxt libxmu libxpm libxaw fontconfig freetype2 libdrm mesa wayland libinput seatd libglvnd libevdev libwacom dbus systemd xdg-desktop-portal-wlr
```

#### Ubuntu/Debian Dependencies

Verified on Ubuntu 24.04 (Noble):

```bash
sudo apt-get install -y --no-install-recommends \
    build-essential pkg-config \
    libx11-dev libxinerama-dev libxft-dev libxrender-dev \
    libfontconfig-dev libfreetype-dev libxkbcommon-dev \
    libudev-dev libseat-dev libinput-dev libgbm-dev
```

On older Debian releases, `libfontconfig-dev` is called `libfontconfig1-dev`.

Ubuntu 24.04 ships rustc 1.75, which is too old for instantWM (edition 2024
requires rustc 1.85+). Install a current toolchain via rustup instead:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
```

This is the same package list CI installs, see
`scripts/ci/install-deps-ubuntu.sh`.

Running the test suite additionally requires at least one real system font
(e.g. `fonts-dejavu-core`), otherwise the text rasterizer tests fail.

### Wayland screen sharing and screenshots

For broad application compatibility on Wayland, instantWM follows the standard
wlroots-style portal stack:

- `xdg-desktop-portal`
- `xdg-desktop-portal-wlr`
- `xdg-desktop-portal-gtk` as the fallback portal backend

The repository ships [`resources/instantwm-portals.conf`](resources/instantwm-portals.conf),
which routes `ScreenCast` and `Screenshot` to the `wlr` portal backend for
`XDG_CURRENT_DESKTOP=instantwm`.

This is the recommended setup for:

- OBS Studio
- Firefox / Chromium / Electron screen sharing
- portal-based screenshots in sandboxed applications

On a systemd-based session this additionally requires the Wayland session
environment to be imported into D-Bus activation. instantWM does this
automatically when starting its Wayland socket.

## [Documentation](https://instantos.io/documentation)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for version bump and release rules.

## Features

- General
  * Wayland and X11 support (Yes, really)
  * hybrid-wm: tiling and floating mode are both first-class citizens
  * Keyboard and Mouse based workflows
  * Start-menu
  * desktop bindings
  * Full multi monitor support
  * Tag system
  * Overview mode
  * Edge-anchored scratchpads
- Mouse support
  * Drag windows by grabbing the title
  * Drag windows onto other tags
  * Rio-like drawing feature
- Graphical Features
  * Animations
  * Hover indicators
  * Status markup
  * Color indicators for sticky windows, tag status etc.

This is just a quick list of some features. For a full list and explanation,
please refer to the documentation.
