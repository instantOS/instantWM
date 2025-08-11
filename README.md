# InstantWM - Rust/Wayland Implementation

A fast and lightweight Wayland compositor inspired by InstantWM, rewritten in Rust using the Smithay 0.7.0 framework.

## Project Status

⚠️ **This is currently a work-in-progress implementation**. The Rust version is being actively developed to match the functionality of the C/X11 version while providing better performance and modern Wayland support.

### What's Working
- ✅ Basic Smithay 0.7.0 integration
- ✅ TOML-based configuration system
- ✅ Comprehensive keybinding system matching C version
- ✅ Window rules for automatic floating/tagging
- ✅ Tag-based workspace management (design)
- ✅ Multiple layout support (tiling, floating, monocle)
- ✅ Top bar with tag and layout information
- ✅ CLI interface (`instantctl`)

### In Development
- 🔄 Full Smithay compositor implementation
- 🔄 Window rendering and management
- 🔄 Input event handling
- 🔄 Layout algorithms
- 🔄 System tray integration
- 🔄 Multi-monitor support

### Planned Features
- 📋 Window animations
- 📋 Scratchpad support
- 📋 Overlay mode
- 📋 Advanced window decorations
- 📋 Gesture support

## Features (Target)

### Core Window Management
- **Tag-based Workspaces**: Organize windows using tags instead of traditional workspaces, just like dwm/instantWM
- **Multiple Layouts**: 
  - Tiling (master-stack)
  - Floating
  - Monocle (fullscreen)
  - Grid (planned)
  - Deck (planned)
  - Bstack (planned)
- **Floating Window Support**: Mix floating and tiling windows seamlessly
- **Window Rules**: Automatic window placement based on class/title

### User Interface
- **Customizable Top Bar**: Tag indicators, layout info, window titles, system tray
- **Theme Support**: Comprehensive color scheme configuration
- **Status Information**: System stats, time, notifications

### Input & Control
- **Extensive Keybindings**: Vim-like navigation, tag switching, layout control
- **Mouse Interactions**: Drag to move, resize windows, click to focus
- **CLI Control**: Command-line interface similar to `swaymsg`

## Installation

### Dependencies

#### Arch Linux / instantOS
```bash
sudo pacman -S rust cargo wayland wayland-protocols libxkbcommon
```

#### Ubuntu/Debian
```bash
sudo apt install rustc cargo libwayland-dev wayland-protocols libxkbcommon-dev
```

#### From Source
```bash
# Clone the repository
git clone https://github.com/instantos/instantWM
cd instantWM

# Build
cargo build --release

# Install (when ready)
sudo install -m 755 target/release/instantwm /usr/bin/
sudo install -m 755 target/release/instantctl /usr/bin/
```

## Configuration

### Setup
```bash
mkdir -p ~/.config/instantwm
cp config.toml.example ~/.config/instantwm/config.toml
```

### Configuration Structure

The configuration uses TOML format with the following sections:

#### General Settings
```toml
[general]
mod_key = "Mod4"          # Super/Windows key
terminal = "alacritty"
browser = "firefox"
launcher = "rofi -show drun"
```

#### Tags Configuration
```toml
[tags]
names = ["1", "2", "3", "4", "5", "6", "7", "8", "9"]
layouts = ["tiling", "floating", "monocle"]
```

#### Appearance
```toml
[appearance]
border_width = 3
border_focus = "#536DFE"
border_normal = "#384252"
gap_size = 5
bar_height = 24
bar_background = "#121212"
bar_foreground = "#DFDFDF"
```

#### Keybindings
```toml
[keybindings]
"Mod4+Return" = "spawn terminal"
"Mod4+q" = "close_window"
"Mod4+f" = "toggle_floating"
"Mod4+1" = "switch_tag 1"
"Mod4+Shift+1" = "move_to_tag 1"
# ... many more
```

#### Window Rules
```toml
[[rules]]
class = "Pavucontrol"
floating = true

[[rules]]
class = "firefox"
tag = 1
```

### Default Keybindings

#### Application Management
- `Super+Return` - Spawn terminal
- `Super+d` - Launch application launcher
- `Super+Shift+Return` - Spawn browser

#### Window Management
- `Super+q` - Close focused window
- `Super+f` - Toggle floating mode
- `Super+Shift+f` - Toggle fullscreen
- `Super+space` - Cycle through layouts

#### Navigation
- `Super+h/j/k/l` - Focus window left/down/up/right
- `Super+Shift+h/j/k/l` - Move window left/down/up/right
- `Super+Tab` - Focus next window
- `Super+Shift+Tab` - Focus previous window

#### Tag Management
- `Super+1-9` - Switch to tag 1-9
- `Super+Shift+1-9` - Move focused window to tag 1-9

#### Layout Control
- `Super+comma/period` - Increase/decrease master window count
- `Super+Left/Right` - Decrease/increase master area size

#### System
- `Super+Shift+r` - Reload configuration
- `Super+Shift+e` - Exit InstantWM
- `Print` - Take screenshot
- `Shift+Print` - Take area screenshot

## CLI Control

The `instantctl` command provides control over the running compositor:

```bash
# Basic window management
instantctl close                    # Close focused window
instantctl toggle-floating         # Toggle floating mode

# Tag management
instantctl tag 2                   # Switch to tag 2
instantctl move-to-tag 3           # Move window to tag 3

# Application launching
instantctl spawn firefox           # Launch Firefox
instantctl spawn terminal          # Launch terminal

# Information
instantctl get windows             # List windows
instantctl get focused             # Get focused window
instantctl get version             # Show version
```

## Development

### Building
```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run with logging
RUST_LOG=debug cargo run

# Check for issues
cargo check
cargo clippy
```

### Testing
```bash
cargo test
```

### Architecture

The codebase is organized into several modules:

- `src/main.rs` - Entry point and event loop
- `src/compositor.rs` - Smithay compositor integration
- `src/window_manager.rs` - Core window management logic
- `src/config.rs` - Configuration loading and validation
- `src/input.rs` - Input event handling
- `src/top_bar.rs` - Top bar rendering and interaction
- `src/cli.rs` - Command-line interface
- `src/types.rs` - Common data structures

### Key Dependencies

- **Smithay 0.7.0** - Wayland compositor framework
- **tokio** - Async runtime
- **serde/toml** - Configuration parsing
- **tracing** - Logging framework
- **calloop** - Event loop
- **xkbcommon** - Keyboard handling

## Comparison with C Version

| Feature | C/X11 Version | Rust/Wayland Version |
|---------|---------------|---------------------|
| Window Management | ✅ Full | 🔄 In Progress |
| Tag System | ✅ Full | ✅ Design Complete |
| Layouts | ✅ 9 layouts | 🔄 3 layouts planned |
| Top Bar | ✅ Full | 🔄 Basic implementation |
| System Tray | ✅ Full | 📋 Planned |
| Configuration | ✅ C headers | ✅ TOML format |
| Performance | ✅ Fast | ⚡ Expected faster |
| Memory Safety | ⚠️ C concerns | ✅ Rust safety |
| Wayland Support | ❌ X11 only | ✅ Native Wayland |

## Contributing

This project welcomes contributions! Areas where help is needed:

1. **Smithay Integration** - Help complete the compositor implementation
2. **Layout Algorithms** - Implement additional window layouts
3. **Input Handling** - Improve keyboard and mouse event processing
4. **Testing** - Add more comprehensive tests
5. **Documentation** - Improve code documentation and user guides

### Development Setup
```bash
git clone https://github.com/instantos/instantWM
cd instantWM
cargo build
# Make your changes
cargo test
cargo clippy
```

### Coding Standards
- Follow Rust standard formatting (`cargo fmt`)
- Ensure all tests pass (`cargo test`)
- Run clippy for linting (`cargo clippy`)
- Add documentation for public APIs
- Follow the existing code structure

## License

GPL-3.0 License - see LICENSE file for details.

This maintains compatibility with the original InstantWM license while providing the modern benefits of Rust and Wayland.

## Links

- [Original InstantWM](https://github.com/instantOS/instantWM) - The C/X11 version
- [InstantOS](https://instantos.io/) - The Linux distribution using InstantWM
- [Smithay](https://github.com/Smithay/smithay) - The Wayland compositor framework
- [Wayland](https://wayland.freedesktop.org/) - The display protocol

## Roadmap

### Phase 1 (Current) - Foundation
- [x] Project structure setup
- [x] Basic Smithay integration  
- [x] Configuration system
- [ ] Core compositor functionality
- [ ] Basic window management

### Phase 2 - Core Features
- [ ] Complete layout algorithms
- [ ] Input event handling
- [ ] Window rules implementation
- [ ] Top bar functionality

### Phase 3 - Advanced Features
- [ ] System tray integration
- [ ] Multi-monitor support
- [ ] Window animations
- [ ] Gesture support

### Phase 4 - Polish
- [ ] Performance optimization
- [ ] Comprehensive testing
- [ ] Documentation completion
- [ ] Package distribution

The goal is to create a Wayland compositor that feels identical to the original InstantWM while providing better performance, memory safety, and modern display protocol support.