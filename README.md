# instantWM - Rust/Wayland Implementation

A fast and lightweight Wayland compositor inspired by instantWM, rewritten in Rust using the Smithay framework.

## Features

- **Tiling Window Management**: Automatic tiling with multiple layouts
- **Tag-based Workspaces**: Organize windows using tags instead of traditional workspaces
- **Floating Windows**: Support for floating windows alongside tiling
- **Mouse Interactions**: Drag, resize, and hover behaviors
- **Top Bar**: Customizable top bar with drag-and-drop functionality
- **CLI Control**: Command-line interface similar to `swaymsg`
- **TOML Configuration**: Easy-to-edit configuration files
- **Theme Support**: Customizable appearance and themes

## Installation

### From Source

```bash
# Clone the repository
git clone https://github.com/instantos/instantwm-rs
cd instantwm-rs

# Build
cargo build --release

# Install
sudo install -m 755 target/release/instantwm /usr/bin/
sudo install -m 755 target/release/instantctl /usr/bin/
sudo install -m 755 scripts/instantwm-session /usr/bin/
```

### Configuration

Copy the example configuration:

```bash
mkdir -p ~/.config/instantwm
cp config.toml.example ~/.config/instantwm/config.toml
```

## Usage

### Starting instantWM

From a display manager:
- Select "instantWM" from the session menu

From the command line:
```bash
instantwm
```

### CLI Commands

```bash
# Switch to tag 1
instantctl tag 1

# Move focused window to tag 2
instantctl move-to-tag 2

# Toggle floating mode
instantctl toggle-floating

# Close focused window
instantctl close

# Spawn applications
instantctl spawn firefox

# Get current state
instantctl get tag
instantctl get windows
instantctl get focused

# Set configuration
instantctl set layout monocle
instantctl set gap 10
instantctl set border 2
```

### Key Bindings

The default key bindings are defined in the configuration file. Common bindings include:

- `Super+1-9`: Switch to tag
- `Super+Shift+1-9`: Move window to tag
- `Super+Enter`: Spawn terminal
- `Super+q`: Close window
- `Super+f`: Toggle floating
- `Super+Space`: Cycle layouts
- `Super+Shift+c`: Reload configuration
- `Super+Shift+e`: Exit instantWM

## Configuration

Configuration is done through TOML files located at `~/.config/instantwm/config.toml`.

### Example Configuration

```toml
# General settings
general = { mod_key = "Super", terminal = "alacritty", menu = "rofi -show drun" }

# Layouts
[layouts]
tall = { name = "Tall", type = "Tall", master_factor = 0.6 }
monocle = { name = "Monocle", type = "Monocle" }
grid = { name = "Grid", type = "Grid" }

# Tags
tags = [
    { name = "1", key = "1" },
    { name = "2", key = "2" },
    { name = "3", key = "3" },
    { name = "4", key = "4" },
    { name = "5", key = "5" },
    { name = "6", key = "6" },
    { name = "7", key = "7" },
    { name = "8", key = "8" },
    { name = "9", key = "9" },
]

# Appearance
[appearance]
gaps = 5
border_width = 2
border_color = "#ff0000"
focus_color = "#00ff00"
background_color = "#1a1a1a"

# Top bar
[top_bar]
height = 25
background = "#2a2a2a"
foreground = "#ffffff"
font = "monospace 10"
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
```

### Testing

```bash
# Run tests
cargo test

# Run with specific features
cargo test --features=debug
```

## Architecture

The instantWM codebase is organized into several modules:

- `compositor`: Core Wayland compositor using Smithay
- `window_manager`: Window management logic (tiling, floating, tags)
- `input`: Input handling (keyboard, mouse)
- `top_bar`: Top bar implementation
- `config`: Configuration management
- `cli`: Command-line interface

## License

GPL-3.0 License - see LICENSE file for details.

## Contributing

Contributions are welcome! Please read CONTRIBUTING.md for guidelines.
