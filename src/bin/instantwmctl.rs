use clap::{Parser, Subcommand};
use instantwm::ipc_types::{
    InputCommand, IpcCommand, IpcRequest, IpcResponse, KeyboardCommand, KeyboardLayout, LayoutKind,
    ModeCommand, MonitorCommand, MonitorDirection, ScratchpadCommand, TagCommand, ToggleCommand,
    WindowCommand,
};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_reload_command() {
        let cli = Cli::parse_from(["instantwmctl", "reload"]);
        assert!(matches!(cli.command, CommandKind::Reload));
    }
}

#[derive(Debug, Parser)]
#[command(name = "instantwmctl", version, disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

/// A keyboard layout argument (supports "layout" or "layout(variant)" syntax).
#[derive(Debug, Clone)]
struct KeyboardLayoutArg {
    name: String,
    variant: Option<String>,
}

impl From<String> for KeyboardLayoutArg {
    fn from(s: String) -> Self {
        // Parse "layout(variant)" syntax
        if let Some((name, variant)) = s.strip_suffix(')').and_then(|s| s.rsplit_once('(')) {
            Self {
                name: name.to_string(),
                variant: Some(variant.to_string()),
            }
        } else {
            Self {
                name: s,
                variant: None,
            }
        }
    }
}

impl From<KeyboardLayoutArg> for KeyboardLayout {
    fn from(arg: KeyboardLayoutArg) -> Self {
        KeyboardLayout {
            name: arg.name,
            variant: arg.variant,
        }
    }
}

/// Monitor-related commands.
#[derive(Debug, Subcommand)]
enum MonitorAction {
    /// List all monitors with their information.
    List {
        /// Window ID (defaults to all windows)
        window_id: Option<u32>,
    },
    /// Switch focus to a specific monitor by index.
    Switch {
        /// Monitor index (0-based)
        index: u32,
    },
    /// Move focus to the next monitor(s).
    Next {
        /// Number of monitors to move (default: 1)
        #[arg(default_value = "1")]
        count: u32,
    },
    /// Move focus to the previous monitor(s).
    Prev {
        /// Number of monitors to move (default: 1)
        #[arg(default_value = "1")]
        count: u32,
    },
    /// Configure a monitor.
    Set {
        /// Monitor identifier (name, 'focused' for currently focused, or '*' for all)
        #[arg(default_value = "focused")]
        identifier: String,
        /// Resolution (e.g., "1920x1080")
        #[arg(long, short = 'r')]
        res: Option<String>,
        /// Refresh rate in Hz
        #[arg(long, short = 'f')]
        rate: Option<f32>,
        /// Position (e.g., "1920,0")
        #[arg(long, short = 'p')]
        pos: Option<String>,
        /// Scale factor
        #[arg(long, short = 's')]
        scale: Option<f32>,
        /// Enable the monitor
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        /// Disable the monitor
        #[arg(long, conflicts_with = "enable")]
        disable: bool,
    },
}

/// Keyboard layout actions.
#[derive(Debug, Subcommand)]
enum KeyboardAction {
    /// List configured keyboard layouts (use --all for all available)
    List {
        /// List all available XKB layouts
        #[arg(long)]
        all: bool,
    },
    /// Show current keyboard layout
    Status,
    /// Switch to the next keyboard layout
    Next,
    /// Switch to the previous keyboard layout
    Prev,
    /// Set layouts (multiple allowed, like old set-keyboard-layouts)
    ///
    /// Layouts can be specified as "layout" or "layout(variant)"
    /// (e.g., "us" "de(nodeadkeys)" "fr")
    Set {
        /// Layouts to set (e.g., "us" "de" "fr" or "us(nodeadkeys)")
        #[arg(num_args = 1..)]
        layouts: Vec<String>,
    },
    /// Add a layout to the active list
    Add {
        /// Layout name to add (e.g., "fr" or "de(nodeadkeys)")
        name: String,
    },
    /// Remove a layout from the active list
    Remove {
        /// Layout name or index to remove (e.g., "de" or "#1")
        layout: String,
    },
}

/// Scratchpad actions.
#[derive(Debug, Subcommand)]
enum ScratchpadAction {
    /// List all scratchpads (show names and visibility status).
    List {
        /// Window ID (defaults to all windows)
        window_id: Option<u32>,
    },
    /// Show scratchpad visibility status.
    ///
    /// With no argument, shows status for all scratchpads.
    /// With a name, shows status for that specific scratchpad.
    Status {
        /// Scratchpad name (empty or omitted for all)
        name: Option<String>,
    },
    /// Show a scratchpad (make visible on current tag).
    Show {
        /// Scratchpad name (required)
        name: String,
    },
    /// Hide a scratchpad (remove from current tag).
    Hide {
        /// Scratchpad name (required)
        name: String,
    },
    /// Toggle scratchpad visibility.
    ///
    /// Shows the scratchpad if hidden, hides if visible.
    /// With no argument, toggles the default scratchpad.
    Toggle {
        /// Scratchpad name (optional, defaults to first/only scratchpad)
        name: Option<String>,
    },
    /// Create a scratchpad from the selected window.
    ///
    /// The selected window is assigned the given name, moved to scratchpad tag,
    /// and set to floating. If no name is given, uses "default".
    Create {
        /// Scratchpad name (optional, defaults to "default")
        name: Option<String>,
    },
    /// Remove scratchpad status from the selected window.
    ///
    /// Restores the window's previous tag visibility.
    Delete,
}

/// Window-related commands.
#[derive(Debug, Subcommand)]
enum WindowAction {
    /// List all managed windows.
    List {
        /// Window ID (defaults to all windows)
        window_id: Option<u32>,
    },
    /// Get window geometry.
    Geom {
        /// Window ID (defaults to currently selected window)
        window_id: Option<u32>,
    },
    /// Close a window.
    Close {
        /// Window ID (defaults to currently selected window)
        window_id: Option<u32>,
    },
}

/// Toggle-related commands.
#[derive(Debug, Subcommand)]
enum ToggleAction {
    /// Toggle or set animated windows mode.
    ///
    /// Action argument:
    ///   (empty), 0, or 2: toggle
    ///   1: disable (set false)
    ///   other: enable (set true)
    Animated {
        /// Action: toggle, enable, or disable
        action: Option<String>,
    },
    /// Toggle or set focus follows mouse.
    ///
    /// Action argument:
    ///   (empty), 0, or 2: toggle
    ///   1: disable (set false)
    ///   other: enable (set true)
    FocusFollowsMouse {
        /// Action: toggle, enable, or disable
        action: Option<String>,
    },
    /// Toggle or set focus follows mouse for floating windows only.
    ///
    /// Action argument:
    ///   (empty), 0, or 2: toggle
    ///   1: disable (set false)
    ///   other: enable (set true)
    FocusFollowsFloatMouse {
        /// Action: toggle, enable, or disable
        action: Option<String>,
    },
    /// Toggle or set alt-tab free mode (enables prefix-based window switching).
    ///
    /// Action argument:
    ///   (empty), 0, or 2: toggle
    ///   1: disable (set false)
    ///   other: enable (set true)
    AltTab {
        /// Action: toggle, enable, or disable
        action: Option<String>,
    },
    /// Toggle or set alt-tag mode (shows alternative tag names in bar).
    ///
    /// Action argument:
    ///   (empty), 0, or 2: toggle
    ///   1: disable (set false)
    ///   other: enable (set true)
    AltTag {
        /// Action: toggle, enable, or disable
        action: Option<String>,
    },
    /// Toggle or set hide tags visibility (hides tag bar).
    ///
    /// Action argument:
    ///   (empty), 0, or 2: toggle
    ///   1: disable (set false)
    ///   other: enable (set true)
    HideTags {
        /// Action: toggle, enable, or disable
        action: Option<String>,
    },
}

/// Tag-related commands.
#[derive(Debug, Subcommand)]
enum TagAction {
    /// Switch to a tag (workspace).
    View {
        /// Tag number (1-8, defaults to 2; 0 is treated as 2)
        number: Option<u32>,
    },
    /// Rename the current tag.
    Name {
        /// New tag name (max 16 bytes)
        name: String,
    },
    /// Reset all tag names to defaults ("1" through "9").
    Reset,
}

/// Input device configuration actions (sway-compatible).
#[derive(Debug, Subcommand)]
enum InputAction {
    /// List current input configuration.
    ///
    /// Shows configuration for all device types, or for a specific
    /// identifier (e.g., "type:touchpad", "type:pointer", "*").
    List {
        /// Device identifier (e.g., "type:touchpad", "type:pointer", "*")
        identifier: Option<String>,
    },
    /// Set pointer acceleration speed.
    ///
    /// Value must be between -1.0 and 1.0.
    /// Negative values slow down, positive values speed up.
    /// Identifier examples: "type:touchpad", "type:pointer", "*"
    PointerAccel {
        /// Device identifier (e.g., "type:pointer", "type:touchpad", "*")
        identifier: String,
        /// Acceleration value (-1.0 to 1.0)
        value: f64,
    },
    /// Set acceleration profile.
    ///
    /// "flat" disables acceleration, "adaptive" applies dynamic acceleration.
    AccelProfile {
        /// Device identifier
        identifier: String,
        /// Profile: "flat" or "adaptive"
        profile: String,
    },
    /// Enable or disable tap-to-click.
    Tap {
        /// Device identifier
        identifier: String,
        /// "enabled" or "disabled"
        state: String,
    },
    /// Enable or disable natural (inverted) scrolling.
    NaturalScroll {
        /// Device identifier
        identifier: String,
        /// "enabled" or "disabled"
        state: String,
    },
    /// Set scroll factor (speed multiplier).
    ///
    /// Values greater than 1.0 increase scroll speed,
    /// values less than 1.0 decrease it.
    ScrollFactor {
        /// Device identifier
        identifier: String,
        /// Scroll speed multiplier (must be non-negative)
        value: f64,
    },
}

/// Mode-related commands.
#[derive(Debug, Subcommand)]
enum ModeAction {
    /// List all configured modes with their descriptions.
    ///
    /// Shows an asterisk (*) next to the currently active mode.
    List,
    /// Set the current mode (enter a mode).
    ///
    /// Use "default" to exit any mode and return to normal operation.
    Set {
        /// Mode name (use "default" to exit current mode)
        name: String,
    },
}

#[derive(Debug, Subcommand)]
enum CommandKind {
    /// Run a keybind action by name. Use --list to see available actions.
    Action {
        /// Action name (e.g., "zoom", "quit", "toggle_bar")
        name: Option<String>,
        /// Arguments for the action.
        args: Vec<String>,
        /// List all available actions and exit.
        #[arg(long, short = 'l')]
        list: bool,
        /// Output action list as JSON (works with --list).
        #[arg(long, short = 'j')]
        json: bool,
    },

    /// Get status information about the running instantWM instance.
    Status,
    /// Reload the running configuration from disk.
    Reload,
    /// Monitor management.
    Monitor {
        #[command(subcommand)]
        action: MonitorAction,
    },
    /// Window management.
    Window {
        #[command(subcommand)]
        action: WindowAction,
    },
    /// Tag/workspace management.
    Tag {
        #[command(subcommand)]
        action: TagAction,
    },
    /// Toggle settings.
    Toggle {
        #[command(subcommand)]
        action: ToggleAction,
    },
    /// Spawn a command.
    Spawn {
        /// Command to execute
        command: Vec<String>,
    },
    /// Warp cursor to the currently focused window.
    WarpFocus,
    /// Move the selected window to another monitor.
    ///
    /// Direction: "next"/"prev" or "1"/"-1"
    TagMon {
        /// Direction (e.g., "next", "prev", "1", "-1")
        direction: Option<MonitorDirection>,
    },
    /// Move the selected window to another monitor and follow it.
    ///
    /// Direction: "next"/"prev" or "1"/"-1"
    FollowMon {
        /// Direction (e.g., "next", "prev", "1", "-1")
        direction: Option<MonitorDirection>,
    },
    /// Set the layout type.
    ///
    /// Layout names: tile, grid, floating, monocle, vert, deck, overview, bstack, horiz
    Layout {
        /// Layout name (e.g., "tile", "grid", "monocle")
        name: Option<LayoutKind>,
    },
    /// Enable or disable prefix mode for special keybindings.
    ///
    /// Non-zero value enables prefix mode; zero disables it.
    Prefix {
        /// Value: non-zero to enable, zero to disable (default: 1)
        value: Option<u32>,
    },
    /// Set border width for the selected window.
    Border {
        /// Border width in pixels (defaults to configured BORDERPX)
        width: Option<u32>,
    },
    /// Set special next mode for cycling through windows.
    ///
    /// Value 0 disables special next; non-zero enables floating window cycling.
    SpecialNext {
        /// Mode: 0=none, non-zero=float (default: 0)
        value: Option<u32>,
    },
    /// Keyboard layout management.
    Keyboard {
        #[command(subcommand)]
        action: KeyboardAction,
    },
    /// Scratchpad management.
    Scratchpad {
        #[command(subcommand)]
        action: ScratchpadAction,
    },
    /// Input device configuration (mouse, touchpad, pointer).
    ///
    /// Configure input device settings like pointer acceleration,
    /// scroll behavior, and tap-to-click. Uses sway-compatible
    /// identifiers ("type:touchpad", "type:pointer", or "*").
    #[command(alias = "input")]
    Mouse {
        #[command(subcommand)]
        action: InputAction,
    },
    /// Mode management (list and set modes).
    Mode {
        #[command(subcommand)]
        action: ModeAction,
    },
    /// Update status text on the bar. If text is "-", read from stdin continuously.
    UpdateStatus { text: String },
    /// Set the wallpaper.
    Wallpaper {
        /// Path to the wallpaper image
        path: String,
    },
}

fn main() {
    let cli = Cli::parse();
    let request = match cli.command {
        CommandKind::Action {
            name,
            args,
            list,
            json,
        } => {
            // If --list flag is set, show the list
            if list {
                use instantwm::config::keybind_config::print_actions;
                print_actions(json);
                return;
            }
            // No name provided - show error
            let name = name.expect("action name required (use --list to see available actions)");
            IpcCommand::RunAction { name, args }
        }
        CommandKind::Status => IpcCommand::Status,
        CommandKind::Reload => IpcCommand::Reload,
        CommandKind::Monitor { action } => {
            let cmd = match action {
                MonitorAction::List { window_id: _ } => MonitorCommand::List,
                MonitorAction::Switch { index } => MonitorCommand::Switch { index },
                MonitorAction::Next { count } => MonitorCommand::Next { count },
                MonitorAction::Prev { count } => MonitorCommand::Prev { count },
                MonitorAction::Set {
                    identifier,
                    res,
                    rate,
                    pos,
                    scale,
                    enable,
                    disable,
                } => {
                    let enable_val = if enable {
                        Some(true)
                    } else if disable {
                        Some(false)
                    } else {
                        None
                    };
                    MonitorCommand::Set {
                        identifier,
                        resolution: res,
                        refresh_rate: rate,
                        position: pos,
                        scale,
                        enable: enable_val,
                    }
                }
            };
            IpcCommand::Monitor(cmd)
        }
        CommandKind::Window { action } => {
            let cmd = match action {
                WindowAction::List { window_id } => WindowCommand::List(window_id),
                WindowAction::Geom { window_id } => WindowCommand::Geom(window_id),
                WindowAction::Close { window_id } => WindowCommand::Close(window_id),
            };
            IpcCommand::Window(cmd)
        }
        CommandKind::Tag { action } => {
            let cmd = match action {
                TagAction::View { number } => TagCommand::View(number.unwrap_or(2)),
                TagAction::Name { name } => TagCommand::Name(name),
                TagAction::Reset => TagCommand::ResetNames,
            };
            IpcCommand::Tag(cmd)
        }
        CommandKind::Toggle { action } => {
            let cmd = match action {
                ToggleAction::Animated { action } => ToggleCommand::Animated(action),
                ToggleAction::FocusFollowsMouse { action } => {
                    ToggleCommand::FocusFollowsMouse(action)
                }
                ToggleAction::FocusFollowsFloatMouse { action } => {
                    ToggleCommand::FocusFollowsFloatMouse(action)
                }
                ToggleAction::AltTab { action } => ToggleCommand::AltTab(action),
                ToggleAction::AltTag { action } => ToggleCommand::AltTag(action),
                ToggleAction::HideTags { action } => ToggleCommand::HideTags(action),
            };
            IpcCommand::Toggle(cmd)
        }
        CommandKind::Spawn { command } => {
            if command.is_empty() {
                eprintln!("instantwmctl: spawn requires a command");
                std::process::exit(2);
            }
            IpcCommand::Spawn(command.join(" "))
        }
        CommandKind::WarpFocus => IpcCommand::WarpFocus,
        CommandKind::TagMon { direction } => {
            IpcCommand::TagMon(direction.unwrap_or(MonitorDirection::NEXT))
        }
        CommandKind::FollowMon { direction } => {
            IpcCommand::FollowMon(direction.unwrap_or(MonitorDirection::NEXT))
        }
        CommandKind::Layout { name } => IpcCommand::Layout(name.unwrap_or(LayoutKind::Tile)),
        CommandKind::Prefix { value } => IpcCommand::Prefix(value),
        CommandKind::Border { width } => IpcCommand::Border(width),
        CommandKind::SpecialNext { value } => IpcCommand::SpecialNext(value),
        CommandKind::Keyboard { action } => {
            let cmd = match action {
                KeyboardAction::List { all } => {
                    if all {
                        KeyboardCommand::ListAll
                    } else {
                        KeyboardCommand::List
                    }
                }
                KeyboardAction::Status => KeyboardCommand::Status,
                KeyboardAction::Next => KeyboardCommand::Next,
                KeyboardAction::Prev => KeyboardCommand::Prev,
                KeyboardAction::Set { layouts } => {
                    let keyboard_layouts: Vec<KeyboardLayout> = layouts
                        .into_iter()
                        .map(KeyboardLayoutArg::from)
                        .map(KeyboardLayout::from)
                        .collect();
                    KeyboardCommand::Set(keyboard_layouts)
                }
                KeyboardAction::Add { name } => {
                    let arg = KeyboardLayoutArg::from(name);
                    KeyboardCommand::Add(KeyboardLayout::from(arg))
                }
                KeyboardAction::Remove { layout } => KeyboardCommand::Remove(layout),
            };
            IpcCommand::Keyboard(cmd)
        }
        CommandKind::Scratchpad { action } => {
            let cmd = match action {
                ScratchpadAction::List { window_id: _ } => ScratchpadCommand::List,
                ScratchpadAction::Status { name } => ScratchpadCommand::Status(name),
                ScratchpadAction::Show { name } => ScratchpadCommand::Show(name),
                ScratchpadAction::Hide { name } => ScratchpadCommand::Hide(name),
                ScratchpadAction::Toggle { name } => ScratchpadCommand::Toggle(name),
                ScratchpadAction::Create { name } => ScratchpadCommand::Create(name),
                ScratchpadAction::Delete => ScratchpadCommand::Delete,
            };
            IpcCommand::Scratchpad(cmd)
        }
        CommandKind::Mouse { action } => {
            let cmd = match action {
                InputAction::List { identifier } => InputCommand::List(identifier),
                InputAction::PointerAccel { identifier, value } => {
                    InputCommand::PointerAccel { identifier, value }
                }
                InputAction::AccelProfile {
                    identifier,
                    profile,
                } => InputCommand::AccelProfile {
                    identifier,
                    profile,
                },
                InputAction::Tap { identifier, state } => InputCommand::Tap {
                    identifier,
                    enabled: state == "enabled",
                },
                InputAction::NaturalScroll { identifier, state } => InputCommand::NaturalScroll {
                    identifier,
                    enabled: state == "enabled",
                },
                InputAction::ScrollFactor { identifier, value } => {
                    InputCommand::ScrollFactor { identifier, value }
                }
            };
            IpcCommand::Input(cmd)
        }
        CommandKind::Mode { action } => {
            let cmd = match action {
                ModeAction::List => ModeCommand::List,
                ModeAction::Set { name } => ModeCommand::Set(name),
            };
            IpcCommand::Mode(cmd)
        }
        CommandKind::Wallpaper { path } => IpcCommand::Wallpaper(path),
        CommandKind::UpdateStatus { text } => {
            if text == "-" {
                use std::io::BufRead;
                let stdin = std::io::stdin();
                let mut reader = stdin.lock();
                let mut line = String::new();

                while reader.read_line(&mut line).unwrap_or(0) > 0 {
                    let trim_line = line.trim();
                    if trim_line == "["
                        || trim_line.starts_with("{\"version\"")
                        || trim_line.is_empty()
                    {
                        line.clear();
                        continue;
                    }

                    let socket = std::env::var("INSTANTWM_SOCKET").unwrap_or_else(|_| {
                        format!("/tmp/instantwm-{}.sock", unsafe { libc::geteuid() })
                    });

                    if let Ok(mut stream) = UnixStream::connect(&socket) {
                        let request =
                            IpcRequest::new(IpcCommand::UpdateStatus(trim_line.to_string()));
                        if let Ok(data) =
                            bincode::encode_to_vec(&request, bincode::config::standard())
                        {
                            let _ = stream.write_all(&data);
                        }
                    }
                    line.clear();
                }
                std::process::exit(0);
            }
            IpcCommand::UpdateStatus(text)
        }
    };

    let socket = std::env::var("INSTANTWM_SOCKET")
        .unwrap_or_else(|_| format!("/tmp/instantwm-{}.sock", unsafe { libc::geteuid() }));
    let mut stream = match UnixStream::connect(&socket) {
        Ok(s) => s,
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                eprintln!(
                    "instantwmctl: instantWM is not running (socket not found: {})",
                    socket
                );
                eprintln!("Make sure instantWM is started before using instantwmctl.");
            } else {
                eprintln!("instantwmctl: connect failed ({}): {}", socket, err);
            }
            std::process::exit(1);
        }
    };

    let request = IpcRequest::new(request);
    let data = match bincode::encode_to_vec(&request, bincode::config::standard()) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("instantwmctl: serialization failed: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(err) = stream.write_all(&data) {
        eprintln!("instantwmctl: write failed: {}", err);
        std::process::exit(1);
    }
    let _ = stream.shutdown(std::net::Shutdown::Write);

    let mut data = Vec::new();
    if let Err(err) = stream.read_to_end(&mut data) {
        eprintln!("instantwmctl: read failed: {}", err);
        std::process::exit(1);
    }

    let response: IpcResponse = match bincode::decode_from_slice(&data, bincode::config::standard())
    {
        Ok((r, _)) => r,
        Err(e) => {
            eprintln!(
                "instantwmctl: deserialization failed: {} (this might be caused by a version mismatch between instantwm and instantwmctl)",
                e
            );
            std::process::exit(1);
        }
    };

    match response {
        IpcResponse::Ok(msg) => {
            if !msg.is_empty() {
                print!("{}", msg);
            }
        }
        IpcResponse::Err(msg) => {
            eprintln!("ERR {}", msg);
            std::process::exit(1);
        }
    }
}
