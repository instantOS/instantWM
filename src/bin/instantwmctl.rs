use clap::{Parser, Subcommand};
use instantwm::ipc_types::{
    IpcCommand, IpcRequest, IpcResponse, KeyboardCommand, KeyboardLayout, MonitorCommand,
    ScratchpadCommand, TagCommand, ToggleCommand, WindowCommand,
};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

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
    List,
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
    List,
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
    List,
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

#[derive(Debug, Subcommand)]
enum CommandKind {
    /// Run a keybind action by name. Use --list to see available actions.
    Action {
        /// Action name (e.g., "zoom", "quit", "toggle_bar")
        name: Option<String>,
        /// List all available actions and exit.
        #[arg(long = "list", short = 'l')]
        list: bool,
    },
    /// Get status information about the running instantWM instance.
    Status,
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
    /// Direction:
    ///   positive (e.g., 1): next monitor (right/down)
    ///   negative (e.g., -1): previous monitor (left/up)
    TagMon {
        /// Direction (1 for next, -1 for previous)
        direction: Option<i32>,
    },
    /// Move the selected window to another monitor and follow it.
    ///
    /// Direction:
    ///   positive (e.g., 1): next monitor (right/down)
    ///   negative (e.g., -1): previous monitor (left/up)
    FollowMon {
        /// Direction (1 for next, -1 for previous)
        direction: Option<i32>,
    },
    /// Set the layout type.
    ///
    /// Layout indices: 0=Tile, 1=Grid, 2=Floating, 3=Monocle, 4=Vert, 5=Deck,
    /// 6=Overview, 7=Bstack, 8=Horiz. Invalid indices default to Tile (0).
    Layout {
        /// Layout index (0-based, invalid values default to Tile)
        number: Option<u32>,
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
    /// Update status text on the bar. If text is "-", read from stdin continuously.
    UpdateStatus { text: String },
}

fn main() {
    let cli = Cli::parse();
    let request = match cli.command {
        CommandKind::Action { name, list } => {
            // If --list flag is set, show the list
            if list {
                use instantwm::config::keybind_config::print_actions;
                print_actions();
                return;
            }
            // No name provided - show error
            let name = name.expect("action name required (use --list to see available actions)");
            IpcCommand::RunAction(name)
        }
        CommandKind::Status => IpcCommand::Status,
        CommandKind::Monitor { action } => {
            let cmd = match action {
                MonitorAction::List => MonitorCommand::List,
                MonitorAction::Switch { index } => MonitorCommand::Switch { index },
                MonitorAction::Next { count } => MonitorCommand::Next { count },
                MonitorAction::Prev { count } => MonitorCommand::Prev { count },
            };
            IpcCommand::Monitor(cmd)
        }
        CommandKind::Window { action } => {
            let cmd = match action {
                WindowAction::List => WindowCommand::List,
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
        CommandKind::TagMon { direction } => IpcCommand::TagMon(direction.unwrap_or(1)),
        CommandKind::FollowMon { direction } => IpcCommand::FollowMon(direction.unwrap_or(1)),
        CommandKind::Layout { number } => IpcCommand::Layout(number.unwrap_or(0)),
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
                ScratchpadAction::List => ScratchpadCommand::List,
                ScratchpadAction::Status { name } => ScratchpadCommand::Status(name),
                ScratchpadAction::Show { name } => ScratchpadCommand::Show(name),
                ScratchpadAction::Hide { name } => ScratchpadCommand::Hide(name),
                ScratchpadAction::Toggle { name } => ScratchpadCommand::Toggle(name),
                ScratchpadAction::Create { name } => ScratchpadCommand::Create(name),
                ScratchpadAction::Delete => ScratchpadCommand::Delete,
            };
            IpcCommand::Scratchpad(cmd)
        }
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

                    let socket = std::env::var("INSTANTWM_SOCKET").unwrap_or_else(|| {
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
        .unwrap_or_else(|| format!("/tmp/instantwm-{}.sock", unsafe { libc::geteuid() }));
    let mut stream = match UnixStream::connect(&socket) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("instantwmctl: connect failed ({}): {}", socket, err);
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
            eprintln!("instantwmctl: deserialization failed: {}", e);
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
