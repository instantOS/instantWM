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
    List,
    Geom { window_id: Option<u32> },
    Spawn { command: Vec<String> },
    Close { window_id: Option<u32> },
    Quit,
    Overlay,
    WarpFocus,
    Tag { number: Option<u32> },
    Animated { action: Option<String> },
    FocusFollowsMouse { action: Option<String> },
    FocusFollowsFloatMouse { action: Option<String> },
    AltTab { action: Option<String> },
    AltTag { action: Option<String> },
    HideTags { action: Option<String> },
    Layout { number: Option<u32> },
    Prefix { value: Option<u32> },
    Border { width: Option<u32> },
    SpecialNext { value: Option<u32> },
    TagMon { direction: Option<i32> },
    FollowMon { direction: Option<i32> },
    FocusMon { direction: Option<i32> },
    FocusNMon { index: Option<i32> },
    NameTag { name: String },
    ResetNameTag,
    ScratchpadMake { name: Option<String> },
    ScratchpadUnmake,
    ScratchpadToggle { name: Option<String> },
    ScratchpadShow { name: Option<String> },
    ScratchpadHide { name: Option<String> },
    ScratchpadStatus { name: Option<String> },
    /// Set keyboard layout by index (0-based).
    KeyboardLayout { index: u32 },
    /// Set keyboard layout by name (e.g. "us", "de").
    KeyboardLayoutName { name: String },
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
}

fn main() {
    let cli = Cli::parse();
    let request = match cli.command {
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
        CommandKind::Quit => IpcCommand::Quit,
        CommandKind::Overlay => IpcCommand::Overlay,
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
