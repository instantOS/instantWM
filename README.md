<div align="center">
    <h1>instantWM</h1>
    <p>Window manager for instantOS</p>
    <img width="300" height="300" src="https://raw.githubusercontent.com/instantOS/instantLOGO/main/png/wm.png">
</div>

instantWM the window manager of instantOS.

![img](https://github.com/instantOS/instantLOGO/blob/main/screeenshots/screenshot1.png)

## Installation

InstantWM is preinstalled on instantOS.
You can manually install the git build at your own risk by cloning the repo and then running build.sh,
however you'll likely be missing a lot of other tools if you're not on instantOS.
It is not recommended to use instantWM with other distributions.

```sh
git clone --depth=1 https://github.com/instantOS/instantWM.git
cd instantWM
just install
```

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
- Graphical Features
  * Animations
  * Hover indicators
  * Status markup
  * Color indicators for sticky windows, tag status etc.
- Mouse support
  * Drag windows by grabbing the title
  * Drag windows onto other tags
  * Rio-like drawing feature

This is just a quick list of some features. For a full list and explanation,
please refer to the documentation.
