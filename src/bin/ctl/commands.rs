use clap::{Parser, Subcommand};
use instantwm::ipc_types::{
    InputCommand, IpcCommand, KeyboardCommand, KeyboardLayout, LayoutKind, ModeCommand,
    MonitorCommand, MonitorDirection, ScratchpadCommand, SpecialNext, TagCommand, ToggleCommand,
    Transform, WindowCommand,
};

#[derive(Debug, Clone)]
pub struct KeyboardLayoutArg {
    name: String,
    variant: Option<String>,
}

impl From<String> for KeyboardLayoutArg {
    fn from(s: String) -> Self {
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

#[derive(Debug, Clone, Subcommand)]
pub enum MonitorAction {
    List {
        window_id: Option<u32>,
    },
    Switch {
        index: u32,
    },
    Next {
        #[arg(default_value = "1")]
        count: u32,
    },
    Prev {
        #[arg(default_value = "1")]
        count: u32,
    },
    Set {
        #[arg(default_value = "focused")]
        identifier: String,
        #[arg(long, short = 'r')]
        res: Option<String>,
        #[arg(long, short = 'f')]
        rate: Option<f32>,
        #[arg(long, short = 'p')]
        pos: Option<String>,
        #[arg(long, short = 's')]
        scale: Option<f32>,
        #[arg(long, short = 't')]
        transform: Option<Transform>,
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long, conflicts_with = "enable")]
        disable: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum KeyboardAction {
    List {
        #[arg(long)]
        all: bool,
    },
    Status,
    Next,
    Prev,
    Set {
        #[arg(num_args = 1..)]
        layouts: Vec<String>,
    },
    Add {
        name: String,
    },
    Remove {
        layout: String,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ScratchpadAction {
    List {
        window_id: Option<u32>,
    },
    Status {
        name: Option<String>,
    },
    Show {
        name: Option<String>,
        #[arg(short, long)]
        all: bool,
    },
    Hide {
        name: String,
    },
    Toggle {
        name: Option<String>,
    },
    Create {
        name: Option<String>,
    },
    Delete,
}

#[derive(Debug, Clone, Subcommand)]
pub enum WindowAction {
    List { window_id: Option<u32> },
    Geom { window_id: Option<u32> },
    Close { window_id: Option<u32> },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ToggleAction {
    Animated { action: Option<String> },
    FocusFollowsMouse { action: Option<String> },
    FocusFollowsFloatMouse { action: Option<String> },
    AltTag { action: Option<String> },
    HideTags { action: Option<String> },
}

#[derive(Debug, Clone, Subcommand)]
pub enum TagAction {
    View { number: Option<u32> },
    Name { name: String },
    Reset,
}

#[derive(Debug, Clone, Subcommand)]
pub enum InputAction {
    List {
        identifier: Option<String>,
    },
    Devices,
    #[command(alias = "pointer-accel")]
    Speed {
        value: f64,
        #[arg(short, long)]
        identifier: Option<String>,
    },
    AccelProfile {
        profile: String,
        #[arg(short, long)]
        identifier: Option<String>,
    },
    Tap {
        state: String,
        #[arg(short, long)]
        identifier: Option<String>,
    },
    NaturalScroll {
        state: String,
        #[arg(short, long)]
        identifier: Option<String>,
    },
    ScrollFactor {
        value: f64,
        #[arg(short, long)]
        identifier: Option<String>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ModeAction {
    List,
    Set { name: String },
    Toggle { name: String },
}

#[derive(Debug, Clone, Subcommand)]
pub enum CommandKind {
    Action {
        name: Option<String>,
        args: Vec<String>,
        #[arg(long, short = 'l')]
        list: bool,
    },
    Status,
    Reload,
    Monitor {
        #[command(subcommand)]
        action: MonitorAction,
    },
    Window {
        #[command(subcommand)]
        action: WindowAction,
    },
    Tag {
        #[command(subcommand)]
        action: TagAction,
    },
    Toggle {
        #[command(subcommand)]
        action: ToggleAction,
    },
    Spawn {
        command: Vec<String>,
    },
    WarpFocus,
    TagMon {
        #[arg(default_value = "next")]
        direction: MonitorDirection,
    },
    FollowMon {
        #[arg(default_value = "next")]
        direction: MonitorDirection,
    },
    Layout {
        name: LayoutKind,
    },
    Desktop,
    Border {
        width: Option<u32>,
    },
    SpecialNext {
        mode: SpecialNext,
    },
    Keyboard {
        #[command(subcommand)]
        action: KeyboardAction,
    },
    Scratchpad {
        #[command(subcommand)]
        action: ScratchpadAction,
    },
    #[command(alias = "input")]
    Mouse {
        #[command(subcommand)]
        action: InputAction,
    },
    Mode {
        #[command(subcommand)]
        action: ModeAction,
    },
    UpdateStatus {
        text: String,
    },
    Wallpaper {
        path: String,
    },
}

#[derive(Debug, Parser)]
pub struct Cli {
    #[arg(long)]
    pub ignore_version_mismatches: bool,
    #[arg(long, short = 'j')]
    pub json: bool,
    #[command(subcommand)]
    pub command: CommandKind,
}

pub fn command_to_ipc(command: CommandKind) -> IpcCommand {
    match command {
        CommandKind::Action {
            name,
            args,
            list: _,
        } => {
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
                    transform,
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
                        transform,
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
                ToggleAction::AltTag { action } => ToggleCommand::AltTag(action),
                ToggleAction::HideTags { action } => ToggleCommand::HideTags(action),
            };
            IpcCommand::Toggle(cmd)
        }
        CommandKind::Spawn { command } => IpcCommand::Spawn(command.join(" ")),
        CommandKind::WarpFocus => IpcCommand::WarpFocus,
        CommandKind::TagMon { direction } => IpcCommand::TagMon(direction),
        CommandKind::FollowMon { direction } => IpcCommand::FollowMon(direction),
        CommandKind::Layout { name } => IpcCommand::Layout(name),
        CommandKind::Desktop => IpcCommand::Mode(ModeCommand::Toggle("desktop".to_string())),
        CommandKind::Border { width } => IpcCommand::Border(width),
        CommandKind::SpecialNext { mode } => IpcCommand::SpecialNext(mode),
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
                ScratchpadAction::Show { name, all } => {
                    if all {
                        ScratchpadCommand::ShowAll
                    } else {
                        ScratchpadCommand::Show(name)
                    }
                }
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
                InputAction::Devices => InputCommand::Devices,
                InputAction::Speed { identifier, value } => {
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
                    enabled: state == "enabled" || state == "on",
                },
                InputAction::NaturalScroll { identifier, state } => InputCommand::NaturalScroll {
                    identifier,
                    enabled: state == "enabled" || state == "on",
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
                ModeAction::Toggle { name } => ModeCommand::Toggle(name),
            };
            IpcCommand::Mode(cmd)
        }
        CommandKind::Wallpaper { path } => IpcCommand::Wallpaper(path),
        CommandKind::UpdateStatus { text } => IpcCommand::UpdateStatus(text),
    }
}
