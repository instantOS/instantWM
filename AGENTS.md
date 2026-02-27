This is a Rust rewrite of a C dwm fork. The Rust rewrite aims to reproduce the
functionality of the C version, but to make use of what Rust can do which C
cannot. 
As such, things like the generic Arg struct have been eliminated in favor of
more type safe and comprehensible systems. 

The Rust rewrite is located in ./rust
The c source files are in ./

Agents are not allowed to use git at all. No commits, no diffs, no checkouts. 
When git is needed, ask the user to do it. 

## Cursor Cloud specific instructions

### System dependencies

The following system packages are required (installed via apt):
`libxft-dev libxrender-dev libxinerama-dev libwayland-dev libudev-dev libinput-dev libseat-dev libxkbcommon-dev libxkbcommon-x11-dev libegl-dev libgl-dev libdrm-dev xvfb xwayland xterm`

### Rust toolchain

Requires Rust 1.85+ (the `getrandom` crate uses edition 2024). The default toolchain must be set to `stable` via `rustup default stable`. The pre-installed 1.83.0 toolchain is too old.

### Building

- **C version**: `make -j$(nproc)` from repo root.
- **Rust version**: `cargo build` from `./rust`.
- Both produce an `instantwm` binary. The Rust version also builds `instantwmctl` (IPC client).

### Running

This is a window manager, so it needs an X11 display. Use Xvfb for headless testing:

```
Xvfb :99 -screen 0 1280x800x24 -ac &
```

- **C version**: `DISPLAY=:99 ./instantwm`
- **Rust X11 backend**: `DISPLAY=:99 INSTANTWM_AUTOSTART=0 ./rust/target/debug/instantwm --backend x11`
- **Rust Wayland backend**: `XDG_RUNTIME_DIR=/run/user/$(id -u) DISPLAY=:99 INSTANTWM_AUTOSTART=0 INSTANTWM_WL_AUTOSPAWN=0 ./rust/target/debug/instantwm --backend wayland`

Set `INSTANTWM_AUTOSTART=0` to skip launching `instantautostart` (not available outside instantOS).

### Testing

- **Unit tests**: `cargo test` in `./rust` — 14 tests covering UTF-8 parsing and tag mask logic.
- **Lint**: `cargo clippy` in `./rust` — many dead-code warnings are expected (WIP rewrite).
- **E2e tests**: `tests/e2e.sh` uses the Wayland backend. Requires `XDG_RUNTIME_DIR` set and `Xvfb` running. Note: XWayland integration is not yet wired up, so spawned X11 apps (gtk3-demo, xterm, xmessage) cannot open a display. The test will fail until XWayland spawning is implemented in the Wayland backend.

### Key caveats

- The Wayland backend removes `DISPLAY` from the environment at startup (`std::env::remove_var("DISPLAY")` in `src/main.rs`), so child processes spawned via IPC need XWayland or native Wayland apps.
- libEGL warnings about DRI3 are expected in headless/VM environments without GPU acceleration.
- The `justfile` in the repo root has tasks for the C version only (`build`, `clean`, `fmt`, `install`).
