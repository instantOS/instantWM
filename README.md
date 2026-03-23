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
  * Overlays
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

## Profiling with Tracy

instantWM supports profiling with [Tracy](https://github.com/nicknisi/tracy) for performance analysis.

### Building with Tracy Support

```sh
cargo build --features profile-with-tracy
```

### Running with Tracy

1. Download and run the Tracy profiler from https://github.com/nicknisi/tracy/releases
2. Launch instantWM with profiling enabled:

```sh
TRACY_ENABLE=1 ./target/release/instantwm
```

3. In the Tracy profiler UI, click **Connect** to attach to the running process

### What to Profile

Key spans to look for:
- `libinput callback` - Input event processing latency
- `dispatch_libinput_event` - Input event dispatch time
- `render_outputs` - Frame rendering duration
- `process_completed_crtcs` - VBlank handling

### Interpreting Results

- High latency in `libinput callback` indicates input processing is blocking
- Multiple `render_outputs` calls per frame indicate redundant rendering
- Frame drops often correlate with `mark_pointer_output_dirty` calls during mouse movement
