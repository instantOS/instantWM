use clap::{Parser, Subcommand};
use instantwm::ipc_types::{IpcCommand, IpcResponse};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

#[derive(Debug, Parser)]
#[command(name = "instantwmctl", version, disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
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
    /// List all managed windows.
    List,
    /// Get window geometry.
    Geom {
        /// Window ID (defaults to currently selected window)
        window_id: Option<u32>,
    },
    /// Spawn a command.
    Spawn {
        /// Command to execute
        command: Vec<String>,
    },
    /// Close a window.
    Close {
        /// Window ID (defaults to currently selected window)
        window_id: Option<u32>,
    },
    /// Warp cursor to the currently focused window.
    WarpFocus,
    /// Switch to a tag (workspace).
    Tag {
        /// Tag number (1-8, defaults to 2; 0 is treated as 2)
        number: Option<u32>,
    },
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
    /// Switch focus to another monitor.
    ///
    /// Direction:
    ///   positive (e.g., 1): next monitor (right/down)
    ///   negative (e.g., -1): previous monitor (left/up)
    FocusMon {
        /// Direction (1 for next, -1 for previous)
        direction: Option<i32>,
    },
    /// Switch focus to a specific monitor by index.
    FocusNMon {
        /// Monitor index (0-based, defaults to 0)
        index: Option<i32>,
    },
    /// Rename the current tag.
    ///
    /// Names longer than 16 bytes are ignored. Empty string resets to default.
    NameTag {
        /// New tag name (max 16 bytes)
        name: String,
    },
    /// Reset all tag names to defaults ("1" through "9").
    ResetNameTag,
    /// Create a scratchpad from the selected window.
    ///
    /// The selected window is assigned the given name, moved to scratchpad tag,
    /// and set to floating. Requires a non-empty name argument.
    ScratchpadMake {
        /// Scratchpad name (required, must be non-empty)
        name: Option<String>,
    },
    /// Remove scratchpad status from the selected window.
    ///
    /// Restores the window's previous tag visibility.
    ScratchpadUnmake,
    /// Toggle scratchpad visibility.
    ///
    /// Shows the scratchpad if hidden, hides if shown. Requires a name argument.
    ScratchpadToggle {
        /// Scratchpad name (required)
        name: Option<String>,
    },
    /// Show scratchpad window (make visible on current tag).
    ScratchpadShow {
        /// Scratchpad name (empty string = no-op)
        name: Option<String>,
    },
    /// Hide scratchpad window (remove from current tag).
    ScratchpadHide {
        /// Scratchpad name (empty string = no-op)
        name: Option<String>,
    },
    /// Get scratchpad visibility status.
    ///
    /// Returns "ipc:scratchpad:<name>:0" (hidden) or "ipc:scratchpad:<name>:1" (visible),
    /// or "ipc:scratchpads:<name1>=<status>,..." for all scratchpads.
    ScratchpadStatus {
        /// Scratchpad name ("all" for all, empty for all)
        name: Option<String>,
    },
    /// Set keyboard layout by index (0-based).
    KeyboardLayout {
        /// Layout index (0-based)
        index: u32,
    },
    /// Set keyboard layout by name (e.g. "us", "de").
    KeyboardLayoutName {
        /// Layout name (e.g. "us", "de", "fr")
        name: String,
    },
    /// Cycle to the next keyboard layout.
    NextKeyboardLayout,
    /// Cycle to the previous keyboard layout.
    PrevKeyboardLayout,
    /// Show the current keyboard layout.
    GetKeyboardLayout,
    /// List all configured keyboard layouts.
    ListKeyboardLayouts,
    /// Replace configured keyboard layouts at runtime.
    /// Layouts are positional args, variants follow `--variant`.
    SetKeyboardLayouts {
        /// Layout names, e.g. "us" "de" "fr"
        layouts: Vec<String>,
        /// Per-layout variants (optional, e.g. "" "nodeadkeys")
        #[arg(long, num_args = 1..)]
        variant: Vec<String>,
    },
    /// Update status text on the bar. If text is "-", read from stdin continuously.
    UpdateStatus { text: String },
}

fn main() {
    let cli = Cli::parse();
    let request = match cli.command {
        CommandKind::Action { name, list } => {
            if list {
                use instantwm::config::keybind_config::{get_named_actions, get_structured_actions};
                // Simple actions
                for action in get_named_actions() {
                    if action.doc.is_empty() {
                        println!("{}", action.name);
                    } else {
                        println!("{} # {}", action.name, action.doc);
                    }
                }
                // Structured actions (take args)
                for action in get_structured_actions() {
                    println!("{} # {} (takes args)", action.name, action.doc);
                }
                return;
            }
            let name = name.expect("action name required (use --list to see available actions)");
            IpcCommand::RunAction(name)
        }
        CommandKind::List => IpcCommand::List,
        CommandKind::Geom { window_id } => IpcCommand::Geom(window_id),
        CommandKind::Spawn { command } => {
            if command.is_empty() {
                eprintln!("instantwmctl: spawn requires a command");
                std::process::exit(2);
            }
            IpcCommand::Spawn(command.join(" "))
        }
        CommandKind::Close { window_id } => IpcCommand::Close(window_id),
        CommandKind::WarpFocus => IpcCommand::WarpFocus,
        CommandKind::Tag { number } => IpcCommand::Tag(number.unwrap_or(2)),
        CommandKind::Animated { action } => IpcCommand::Animated(action),
        CommandKind::FocusFollowsMouse { action } => IpcCommand::FocusFollowsMouse(action),
        CommandKind::FocusFollowsFloatMouse { action } => {
            IpcCommand::FocusFollowsFloatMouse(action)
        }
        CommandKind::AltTab { action } => IpcCommand::AltTab(action),
        CommandKind::AltTag { action } => IpcCommand::AltTag(action),
        CommandKind::HideTags { action } => IpcCommand::HideTags(action),
        CommandKind::Layout { number } => IpcCommand::Layout(number.unwrap_or(0)),
        CommandKind::Prefix { value } => IpcCommand::Prefix(value),
        CommandKind::Border { width } => IpcCommand::Border(width),
        CommandKind::SpecialNext { value } => IpcCommand::SpecialNext(value),
        CommandKind::TagMon { direction } => IpcCommand::TagMon(direction.unwrap_or(1)),
        CommandKind::FollowMon { direction } => IpcCommand::FollowMon(direction.unwrap_or(1)),
        CommandKind::FocusMon { direction } => IpcCommand::FocusMon(direction.unwrap_or(1)),
        CommandKind::FocusNMon { index } => IpcCommand::FocusNMon(index.unwrap_or(0)),
        CommandKind::NameTag { name } => IpcCommand::NameTag(name),
        CommandKind::ResetNameTag => IpcCommand::ResetNameTag,
        CommandKind::ScratchpadMake { name } => IpcCommand::ScratchpadMake(name),
        CommandKind::ScratchpadUnmake => IpcCommand::ScratchpadUnmake,
        CommandKind::ScratchpadToggle { name } => IpcCommand::ScratchpadToggle(name),
        CommandKind::ScratchpadShow { name } => IpcCommand::ScratchpadShow(name),
        CommandKind::ScratchpadHide { name } => IpcCommand::ScratchpadHide(name),
        CommandKind::ScratchpadStatus { name } => IpcCommand::ScratchpadStatus(name),
        CommandKind::KeyboardLayout { index } => IpcCommand::KeyboardLayout(index),
        CommandKind::KeyboardLayoutName { name } => IpcCommand::KeyboardLayoutName(name),
        CommandKind::NextKeyboardLayout => IpcCommand::CycleKeyboardLayout(true),
        CommandKind::PrevKeyboardLayout => IpcCommand::CycleKeyboardLayout(false),
        CommandKind::GetKeyboardLayout => IpcCommand::GetKeyboardLayout,
        CommandKind::ListKeyboardLayouts => IpcCommand::ListKeyboardLayouts,
        CommandKind::SetKeyboardLayouts { layouts, variant } => {
            IpcCommand::SetKeyboardLayouts(layouts, variant)
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

                    let socket = std::env::var("INSTANTWM_SOCKET").unwrap_or_else(|_| {
                        format!("/tmp/instantwm-{}.sock", unsafe { libc::geteuid() })
                    });

                    if let Ok(mut stream) = UnixStream::connect(&socket) {
                        let request = IpcCommand::UpdateStatus(trim_line.to_string());
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
            eprintln!("instantwmctl: connect failed ({}): {}", socket, err);
            std::process::exit(1);
        }
    };

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
