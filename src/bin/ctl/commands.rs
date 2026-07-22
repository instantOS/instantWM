use clap::{ArgAction, Parser, Subcommand};
use instantwm::ipc_types::{
    ConfigCommand, FocusFollowsMouseMode, InputCommand, IpcCommand, KeyboardCommand,
    KeyboardLayout, LayoutCommand, ModeCommand, MonitorCommand, MonitorDirection,
    ScratchpadCommand, ScratchpadInitialStatus, SpecialNext, TagCommand, TestCommand, ToggleAction,
    ToggleCommand, Transform, VrrMode, WindowCommand,
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

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum TestWindowMode {
    Tiled,
    Floating,
}

#[derive(Debug, Clone, Subcommand)]
pub enum TestWindowAction {
    /// Focus a window by its stable IPC id.
    Focus { window_id: u32 },
    /// Assign a window to exactly one tag.
    Tag { window_id: u32, tag: u32 },
    /// Set tiling/floating state without relying on the current focus.
    Mode {
        window_id: u32,
        mode: TestWindowMode,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum TestPointerAction {
    /// Inject one absolute pointer-motion transaction.
    Move {
        x: f64,
        y: f64,
        /// Treat coordinates as 0..1 fractions of the focused monitor.
        #[arg(long)]
        normalized: bool,
    },
    /// Interpolate a pointer path. Points use the form X,Y.
    Path {
        #[arg(required = true, num_args = 2..)]
        points: Vec<String>,
        #[arg(long, default_value = "1000")]
        duration_ms: u64,
        #[arg(long, default_value = "30")]
        hz: u32,
        /// Treat coordinates as 0..1 fractions of the focused monitor.
        #[arg(long)]
        normalized: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum TestWaitAction {
    /// Wait until at least COUNT windows are mapped.
    Windows {
        count: usize,
        #[arg(long, default_value = "5000")]
        timeout_ms: u64,
        #[arg(long, default_value = "25")]
        poll_ms: u64,
        /// Require exactly COUNT rather than at least COUNT.
        #[arg(long)]
        exact: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum TestAction {
    Pointer {
        #[command(subcommand)]
        action: TestPointerAction,
    },
    Window {
        #[command(subcommand)]
        action: TestWindowAction,
    },
    Wait {
        #[command(subcommand)]
        action: TestWaitAction,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ToggleCliAction {
    /// Toggle window animations
    Animated {
        /// What to do (default: toggle)
        action: Option<ToggleAction>,
    },
    /// Set focus-follows-mouse behavior
    FocusFollowsMouse {
        /// off: disabled; normal: pointer motion only; force: include scene changes
        mode: FocusFollowsMouseMode,
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
    /// List runtime config keys and their current values.
    ///
    /// With no argument, lists every key. Pass a section (e.g. `layout`) or a
    /// full key (e.g. `layout.inner_gap`) to narrow the output to matches.
    List {
        /// Section or key prefix to filter by (e.g. `fonts`, `fonts.fonts`).
        prefix: Option<String>,
    },
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
    /// Unstable profiling/test API. Requires INSTANTWM_TEST=1 on the compositor.
    Test {
        #[command(subcommand)]
        action: TestAction,
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

fn parse_dpms_state(state: &str) -> bool {
    match state.to_ascii_lowercase().as_str() {
        "on" | "enable" | "enabled" => true,
        "off" | "disable" | "disabled" => false,
        _ => {
            eprintln!("instantwmctl: invalid dpms state (expected on/off)");
            process::exit(1);
        }
    }
}

fn dpms_command(identifier: String, state: &str) -> MonitorCommand {
    MonitorCommand::Set {
        identifier,
        resolution: None,
        refresh_rate: None,
        position: None,
        scale: None,
        transform: None,
        enable: Some(parse_dpms_state(state)),
        vrr: None,
    }
}

impl From<MonitorAction> for MonitorCommand {
    fn from(action: MonitorAction) -> Self {
        match action {
            MonitorAction::List { .. } => Self::List,
            MonitorAction::Switch { index } => Self::Switch { index },
            MonitorAction::Next { count } => Self::Next { count },
            MonitorAction::Prev { count } => Self::Prev { count },
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
            } => Self::Set {
                identifier,
                resolution: res,
                refresh_rate: rate,
                position: pos,
                scale,
                transform,
                enable: if enable {
                    Some(true)
                } else if disable {
                    Some(false)
                } else {
                    None
                },
                vrr,
            },
            MonitorAction::Modes { identifier } => Self::Modes {
                identifier: Some(identifier),
            },
            MonitorAction::Dpms { state } => dpms_command("focused".to_string(), &state),
        }
    }
}

impl From<WindowAction> for WindowCommand {
    fn from(action: WindowAction) -> Self {
        match action {
            WindowAction::List { window_id } => Self::List(window_id),
            WindowAction::Info { window_id } => Self::Info(window_id),
            WindowAction::Resize {
                window_id,
                monitor,
                x,
                y,
                width,
                height,
            } => Self::Resize {
                window_id,
                monitor,
                x,
                y,
                width,
                height,
            },
            WindowAction::Close { window_id } => Self::Close(window_id),
        }
    }
}

impl From<TagAction> for TagCommand {
    fn from(action: TagAction) -> Self {
        match action {
            TagAction::View { number } => Self::View(number.unwrap_or(2)),
            TagAction::Name { name } => Self::Name(name),
            TagAction::Reset => Self::ResetNames,
        }
    }
}

impl From<ToggleCliAction> for ToggleCommand {
    fn from(action: ToggleCliAction) -> Self {
        match action {
            ToggleCliAction::Animated { action } => Self::Animated(action.unwrap_or_default()),
            ToggleCliAction::FocusFollowsMouse { mode } => Self::FocusFollowsMouse(mode),
            ToggleCliAction::FocusFollowsFloatMouse { action } => {
                Self::FocusFollowsFloatMouse(action.unwrap_or_default())
            }
            ToggleCliAction::AltTag { action } => Self::AltTag(action.unwrap_or_default()),
            ToggleCliAction::HideTags { action } => Self::HideTags(action.unwrap_or_default()),
        }
    }
}

impl From<KeyboardAction> for KeyboardCommand {
    fn from(action: KeyboardAction) -> Self {
        match action {
            KeyboardAction::List { all } => {
                if all {
                    Self::ListAll
                } else {
                    Self::List
                }
            }
            KeyboardAction::Status => Self::Status,
            KeyboardAction::Next => Self::Next,
            KeyboardAction::Prev => Self::Prev,
            KeyboardAction::Set { layouts } => Self::Set(
                layouts
                    .into_iter()
                    .map(KeyboardLayoutArg::from)
                    .map(KeyboardLayout::from)
                    .collect(),
            ),
            KeyboardAction::Add { name } => Self::Add(KeyboardLayoutArg::from(name).into()),
            KeyboardAction::Remove { layout } => Self::Remove(layout),
            KeyboardAction::SwapEscape { enabled } => Self::SwapEscape(enabled),
        }
    }
}

impl From<ScratchpadAction> for ScratchpadCommand {
    fn from(action: ScratchpadAction) -> Self {
        match action {
            ScratchpadAction::List { .. } => Self::List,
            ScratchpadAction::Status { name } => Self::Status(name),
            ScratchpadAction::Show { name, all } => {
                if all {
                    Self::ShowAll
                } else {
                    Self::Show(Some(
                        name.unwrap_or_else(|| DEFAULT_SCRATCHPAD_NAME.to_string()),
                    ))
                }
            }
            ScratchpadAction::Hide { name, all } => {
                if all {
                    Self::HideAll
                } else {
                    Self::Hide(Some(
                        name.unwrap_or_else(|| DEFAULT_SCRATCHPAD_NAME.to_string()),
                    ))
                }
            }
            ScratchpadAction::Toggle { name } => Self::Toggle(name),
            ScratchpadAction::Create {
                name,
                window_id,
                status,
                direction,
            } => Self::Create {
                name,
                window_id,
                status,
                direction,
            },
            ScratchpadAction::Delete { window_id } => Self::Delete { window_id },
        }
    }
}

impl From<InputAction> for InputCommand {
    fn from(action: InputAction) -> Self {
        match action {
            InputAction::List { identifier } => Self::List(identifier),
            InputAction::Devices => Self::Devices,
            InputAction::Speed { identifier, value } => Self::PointerAccel { identifier, value },
            InputAction::AccelProfile {
                identifier,
                profile,
            } => Self::AccelProfile {
                identifier,
                profile,
            },
            InputAction::Tap { identifier, state } => Self::Tap {
                identifier,
                enabled: state == "enabled" || state == "on",
            },
            InputAction::NaturalScroll { identifier, state } => Self::NaturalScroll {
                identifier,
                enabled: state == "enabled" || state == "on",
            },
            InputAction::ScrollFactor { identifier, value } => {
                Self::ScrollFactor { identifier, value }
            }
            InputAction::LeftHanded { identifier, state } => Self::LeftHanded {
                identifier,
                enabled: state == "enabled" || state == "on",
            },
        }
    }
}

impl From<ModeAction> for ModeCommand {
    fn from(action: ModeAction) -> Self {
        match action {
            ModeAction::List => Self::List,
            ModeAction::Set { name } => Self::Set(name),
            ModeAction::Toggle { name } => Self::Toggle(name),
        }
    }
}

fn test_command(action: TestAction) -> TestCommand {
    match action {
        TestAction::Pointer {
            action: TestPointerAction::Move { x, y, normalized },
        } => TestCommand::PointerMove { x, y, normalized },
        TestAction::Pointer {
            action: TestPointerAction::Path { .. },
        } => unreachable!("pointer paths are executed by instantwmctl"),
        TestAction::Window { action } => match action {
            TestWindowAction::Focus { window_id } => TestCommand::FocusWindow(window_id),
            TestWindowAction::Tag { window_id, tag } => TestCommand::TagWindow { window_id, tag },
            TestWindowAction::Mode { window_id, mode } => TestCommand::SetWindowFloating {
                window_id,
                floating: matches!(mode, TestWindowMode::Floating),
            },
        },
        TestAction::Wait { .. } => {
            unreachable!("wait conditions are executed by instantwmctl")
        }
    }
}

impl From<ConfigAction> for ConfigCommand {
    fn from(action: ConfigAction) -> Self {
        match action {
            ConfigAction::Default => unreachable!("config default is handled locally"),
            ConfigAction::Get { key } => Self::Get { key },
            ConfigAction::Set { key, value } => Self::Set { key, value },
            ConfigAction::List { .. } => Self::List,
        }
    }
}

impl From<CommandKind> for IpcCommand {
    fn from(command: CommandKind) -> Self {
        match command {
            CommandKind::Action { name, args, .. } => Self::RunAction {
                name: name.expect("action name required (use --list to see available actions)"),
                args,
            },
            CommandKind::Status => Self::Status,
            CommandKind::Reload => Self::Reload,
            CommandKind::Monitor { action } => Self::Monitor(action.into()),
            CommandKind::Window { action } => Self::Window(action.into()),
            CommandKind::Tag { action } => Self::Tag(action.into()),
            CommandKind::Toggle { action } => Self::Toggle(action.into()),
            CommandKind::Spawn { command } => Self::Spawn(command.join(" ")),
            CommandKind::WarpFocus => Self::WarpFocus,
            CommandKind::TagMon { direction } => Self::TagMon(direction),
            CommandKind::FollowMon { direction } => Self::FollowMon(direction),
            CommandKind::Layout { name } => Self::Layout(
                LayoutCommand::from_str(
                    &name.expect("layout name required (use 'layout list' to see layouts)"),
                )
                .expect("invalid layout name (use 'layout list' to see layouts)"),
            ),
            CommandKind::Theme { name, list } => {
                if list {
                    Self::ListThemes
                } else if let Some(name) = name {
                    match name.parse() {
                        Ok(theme) => Self::SetTheme(theme),
                        Err(_) => {
                            eprintln!(
                                "invalid theme name '{name}' \
                                 (use 'instantwmctl theme --list' to see themes)"
                            );
                            process::exit(2);
                        }
                    }
                } else {
                    Self::GetTheme
                }
            }
            CommandKind::Border { width } => Self::Border(width),
            CommandKind::SpecialNext { mode } => Self::SpecialNext(mode),
            CommandKind::Keyboard { action } => Self::Keyboard(action.into()),
            CommandKind::Scratchpad { action } => Self::Scratchpad(action.into()),
            CommandKind::Mouse { action } => Self::Input(action.into()),
            CommandKind::Mode { action } => Self::Mode(action.into()),
            CommandKind::Wallpaper { path } => Self::Wallpaper(path),
            CommandKind::Dpms { identifier, state } => {
                Self::Monitor(dpms_command(identifier, &state))
            }
            CommandKind::UpdateStatus { text } => Self::UpdateStatus(text),
            CommandKind::Config { action } => Self::Config(action.into()),
            CommandKind::Test { action } => Self::Test(test_command(action)),
            CommandKind::Quit => Self::Quit,
        }
    }
}
