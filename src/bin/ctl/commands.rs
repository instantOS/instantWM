use clap::{ArgAction, Parser, Subcommand};
use instantwm::config::config_toml::ColorTheme;
use instantwm::ipc_types::{
    ConfigCommand, InputCommand, IpcCommand, KeyboardCommand, KeyboardLayout, LayoutKind,
    ModeCommand, MonitorCommand, MonitorDirection, ScratchpadCommand, ScratchpadInitialStatus,
    SpecialNext, TagCommand, ToggleAction, ToggleCommand, Transform, VrrMode, WindowCommand,
};
use std::process;
use std::str::FromStr;

const DEFAULT_SCRATCHPAD_NAME: &str = "instantwm_scratchpad";

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
        #[arg(long)]
        vrr: Option<VrrMode>,
        #[arg(long, conflicts_with = "disable")]
        enable: bool,
        #[arg(long, conflicts_with = "enable")]
        disable: bool,
    },
    Modes {
        #[arg(default_value = "focused")]
        identifier: String,
    },
    Dpms {
        state: String,
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
    SwapEscape {
        #[arg(long, action = ArgAction::Set)]
        enabled: bool,
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
        name: Option<String>,
        #[arg(short, long)]
        all: bool,
    },
    Toggle {
        name: Option<String>,
    },
    #[command(alias = "make")]
    Create {
        #[arg(default_value = "instantwm_scratchpad")]
        name: String,
        #[arg(long, short = 'w')]
        window_id: Option<u32>,
        #[arg(long, default_value = "hidden")]
        status: ScratchpadInitialStatus,
        #[arg(long)]
        direction: Option<String>,
    },
    Delete {
        #[arg(long, short = 'w')]
        window_id: Option<u32>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum WindowAction {
    List {
        window_id: Option<u32>,
    },
    Info {
        window_id: Option<u32>,
    },
    Resize {
        window_id: Option<u32>,
        #[arg(long)]
        monitor: Option<String>,
        #[arg(long)]
        x: i32,
        #[arg(long)]
        y: i32,
        #[arg(long)]
        width: i32,
        #[arg(long)]
        height: i32,
    },
    Close {
        window_id: Option<u32>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ToggleCliAction {
    /// Toggle window animations
    Animated {
        /// What to do (default: toggle)
        action: Option<ToggleAction>,
    },
    /// Toggle focus-follows-mouse
    FocusFollowsMouse {
        /// What to do (default: toggle)
        action: Option<ToggleAction>,
    },
    /// Toggle focus-follows-mouse for floating windows
    FocusFollowsFloatMouse {
        /// What to do (default: toggle)
        action: Option<ToggleAction>,
    },
    /// Toggle alt-tag mode
    AltTag {
        /// What to do (default: toggle)
        action: Option<ToggleAction>,
    },
    /// Show/hide tag bar
    HideTags {
        /// What to do (default: toggle)
        action: Option<ToggleAction>,
    },
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
    LeftHanded {
        state: String,
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
pub enum ConfigAction {
    /// Print a commented-out default config to stdout
    Default,
    /// Get a runtime config value by key (e.g. layout.inner_gap)
    Get { key: String },
    /// Set a runtime config value by key (e.g. layout.inner_gap 12)
    Set { key: String, value: String },
    /// List all runtime config keys and their current values
    List,
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
        action: ToggleCliAction,
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
        name: Option<String>,
    },
    /// Get or set the colour theme. With no argument, prints the current theme.
    /// Pass a theme name (e.g. `nord`) to switch, or `--list`/`-l` to list them.
    Theme {
        name: Option<String>,
        #[arg(long, short = 'l')]
        list: bool,
    },
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
    Dpms {
        #[arg(default_value = "focused")]
        identifier: String,
        state: String,
    },
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    Quit,
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

impl From<CommandKind> for IpcCommand {
    fn from(command: CommandKind) -> Self {
        match command {
            CommandKind::Action {
                name,
                args,
                list: _,
            } => {
                let name =
                    name.expect("action name required (use --list to see available actions)");
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
                        vrr,
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
                            vrr,
                        }
                    }
                    MonitorAction::Modes { identifier } => MonitorCommand::Modes {
                        identifier: Some(identifier),
                    },
                    MonitorAction::Dpms { state } => {
                        let enable = match state.to_lowercase().as_str() {
                            "on" | "enable" | "enabled" => true,
                            "off" | "disable" | "disabled" => false,
                            _ => {
                                eprintln!("instantwmctl: invalid dpms state (expected on/off)");
                                process::exit(1);
                            }
                        };
                        MonitorCommand::Set {
                            identifier: "focused".to_string(),
                            resolution: None,
                            refresh_rate: None,
                            position: None,
                            scale: None,
                            transform: None,
                            enable: Some(enable),
                            vrr: None,
                        }
                    }
                };
                IpcCommand::Monitor(cmd)
            }
            CommandKind::Window { action } => {
                let cmd = match action {
                    WindowAction::List { window_id } => WindowCommand::List(window_id),
                    WindowAction::Info { window_id } => WindowCommand::Info(window_id),
                    WindowAction::Resize {
                        window_id,
                        monitor,
                        x,
                        y,
                        width,
                        height,
                    } => WindowCommand::Resize {
                        window_id,
                        monitor,
                        x,
                        y,
                        width,
                        height,
                    },
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
                    ToggleCliAction::Animated { action } => {
                        ToggleCommand::Animated(action.unwrap_or_default())
                    }
                    ToggleCliAction::FocusFollowsMouse { action } => {
                        ToggleCommand::FocusFollowsMouse(action.unwrap_or_default())
                    }
                    ToggleCliAction::FocusFollowsFloatMouse { action } => {
                        ToggleCommand::FocusFollowsFloatMouse(action.unwrap_or_default())
                    }
                    ToggleCliAction::AltTag { action } => {
                        ToggleCommand::AltTag(action.unwrap_or_default())
                    }
                    ToggleCliAction::HideTags { action } => {
                        ToggleCommand::HideTags(action.unwrap_or_default())
                    }
                };
                IpcCommand::Toggle(cmd)
            }
            CommandKind::Spawn { command } => IpcCommand::Spawn(command.join(" ")),
            CommandKind::WarpFocus => IpcCommand::WarpFocus,
            CommandKind::TagMon { direction } => IpcCommand::TagMon(direction),
            CommandKind::FollowMon { direction } => IpcCommand::FollowMon(direction),
            CommandKind::Layout { name } => {
                let name = name.expect("layout name required (use 'layout list' to see layouts)");
                let layout = LayoutKind::from_str(&name)
                    .expect("invalid layout name (use 'layout list' to see layouts)");
                IpcCommand::Layout(layout)
            }
            CommandKind::Theme { name, list } => {
                if list {
                    IpcCommand::ListThemes
                } else if let Some(name) = name {
                    match ColorTheme::from_name(&name) {
                        Some(theme) => IpcCommand::SetTheme(theme),
                        None => {
                            eprintln!(
                                "invalid theme name '{name}' \
                                 (use 'instantwmctl theme --list' to see themes)"
                            );
                            process::exit(2);
                        }
                    }
                } else {
                    IpcCommand::GetTheme
                }
            }
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
                    KeyboardAction::SwapEscape { enabled } => KeyboardCommand::SwapEscape(enabled),
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
                            ScratchpadCommand::Show(Some(
                                name.unwrap_or_else(|| DEFAULT_SCRATCHPAD_NAME.to_string()),
                            ))
                        }
                    }
                    ScratchpadAction::Hide { name, all } => {
                        if all {
                            ScratchpadCommand::HideAll
                        } else {
                            ScratchpadCommand::Hide(Some(
                                name.unwrap_or_else(|| DEFAULT_SCRATCHPAD_NAME.to_string()),
                            ))
                        }
                    }
                    ScratchpadAction::Toggle { name } => ScratchpadCommand::Toggle(name),
                    ScratchpadAction::Create {
                        name,
                        window_id,
                        status,
                        direction,
                    } => ScratchpadCommand::Create {
                        name,
                        window_id,
                        status,
                        direction,
                    },
                    ScratchpadAction::Delete { window_id } => {
                        ScratchpadCommand::Delete { window_id }
                    }
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
                    InputAction::NaturalScroll { identifier, state } => {
                        InputCommand::NaturalScroll {
                            identifier,
                            enabled: state == "enabled" || state == "on",
                        }
                    }
                    InputAction::ScrollFactor { identifier, value } => {
                        InputCommand::ScrollFactor { identifier, value }
                    }
                    InputAction::LeftHanded { identifier, state } => InputCommand::LeftHanded {
                        identifier,
                        enabled: state == "enabled" || state == "on",
                    },
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
            CommandKind::Dpms { identifier, state } => {
                let enable = match state.to_lowercase().as_str() {
                    "on" | "enable" | "enabled" => true,
                    "off" | "disable" | "disabled" => false,
                    _ => {
                        eprintln!("instantwmctl: invalid dpms state (expected on/off)");
                        process::exit(1);
                    }
                };
                IpcCommand::Monitor(MonitorCommand::Set {
                    identifier,
                    resolution: None,
                    refresh_rate: None,
                    position: None,
                    scale: None,
                    transform: None,
                    enable: Some(enable),
                    vrr: None,
                })
            }
            CommandKind::UpdateStatus { text } => IpcCommand::UpdateStatus(text),
            CommandKind::Config { action } => IpcCommand::Config(match action {
                ConfigAction::Default => unreachable!("config default is handled locally"),
                ConfigAction::Get { key } => ConfigCommand::Get { key },
                ConfigAction::Set { key, value } => ConfigCommand::Set { key, value },
                ConfigAction::List => ConfigCommand::List,
            }),
            CommandKind::Quit => IpcCommand::Quit,
        }
    }
}
