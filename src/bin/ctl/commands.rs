use clap::{ArgAction, Parser, Subcommand};
use instantwm::ipc_types::{
    ConfigCommand, FocusFollowsMouseMode, InputCommand, IpcCommand, KeyboardCommand,
    KeyboardLayout, LayoutCommand, ModeCommand, MonitorCommand, MonitorDirection,
    PendingTmpRuleCmd, ScratchpadCommand, ScratchpadInitialStatus, TagCommand, TestCommand,
    ToggleAction, ToggleCommand, Transform, VrrMode, WindowCommand,
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
    /// List connected monitors and their current configuration.
    List { window_id: Option<u32> },
    /// Switch to a monitor by index.
    Switch { index: u32 },
    /// Focus the next monitor.
    Next {
        #[arg(default_value = "1")]
        count: u32,
    },
    /// Focus the previous monitor.
    Prev {
        #[arg(default_value = "1")]
        count: u32,
    },
    /// Configure a monitor's mode, position, scale, transform, VRR, or power state.
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
    /// List the available modes for a monitor.
    Modes {
        #[arg(default_value = "focused")]
        identifier: String,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum KeyboardAction {
    /// List configured keyboard layouts.
    List {
        #[arg(long)]
        all: bool,
    },
    /// Show the active keyboard layout.
    Status,
    /// Select the next keyboard layout.
    Next,
    /// Select the previous keyboard layout.
    Prev,
    /// Set the keyboard layouts.
    Set {
        #[arg(num_args = 1..)]
        layouts: Vec<String>,
    },
    /// Add a keyboard layout.
    Add { name: String },
    /// Remove a keyboard layout.
    Remove { layout: String },
    /// Enable or disable swapping Escape and Caps Lock.
    SwapEscape {
        #[arg(long, action = ArgAction::Set)]
        enabled: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ScratchpadAction {
    /// List scratchpads.
    List { window_id: Option<u32> },
    /// Show the status of a scratchpad.
    Status { name: Option<String> },
    /// Show a scratchpad.
    Show {
        name: Option<String>,
        #[arg(short, long)]
        all: bool,
    },
    /// Hide a scratchpad.
    Hide {
        name: Option<String>,
        #[arg(short, long)]
        all: bool,
    },
    /// Toggle a scratchpad's visibility.
    Toggle { name: Option<String> },
    /// Resize a scratchpad as a percentage of its monitor.
    Resize {
        #[arg(default_value = "instantwm_scratchpad")]
        name: String,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
        width: u32,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
        height: u32,
    },
    /// Create or mark a window as a scratchpad.
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
    /// Restore a scratchpad to an ordinary window.
    #[command(alias = "unmake")]
    Restore {
        /// Scratchpad name; omit to restore the focused scratchpad.
        #[arg(conflicts_with = "window_id")]
        name: Option<String>,
        /// Stable window ID, useful when the scratchpad is hidden.
        #[arg(long, short = 'w')]
        window_id: Option<u32>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum WindowAction {
    /// List managed windows.
    List { window_id: Option<u32> },
    /// Show information about a managed window.
    Info { window_id: Option<u32> },
    /// Resize and optionally move a managed window.
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
    /// Close a managed window.
    Close { window_id: Option<u32> },
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
    /// Inject pointer movement for automated tests.
    Pointer {
        #[command(subcommand)]
        action: TestPointerAction,
    },
    /// Manipulate windows by stable IDs for automated tests.
    Window {
        #[command(subcommand)]
        action: TestWindowAction,
    },
    /// Wait for compositor state in automated tests.
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
    /// Toggle the bottom gesture strip
    BottomBar {
        /// What to do (default: toggle)
        action: Option<ToggleAction>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum TagAction {
    /// View a tag by number.
    View { number: Option<u32> },
    /// Set a tag's name.
    Name { name: String },
    /// Reset all tag names.
    Reset,
}

#[derive(Debug, Clone, Subcommand)]
pub enum InputAction {
    /// List input devices or settings for one device.
    List { identifier: Option<String> },
    /// List input devices.
    Devices,
    /// Set pointer acceleration speed.
    #[command(alias = "pointer-accel")]
    Speed {
        value: f64,
        #[arg(short, long)]
        identifier: Option<String>,
    },
    /// Set the pointer acceleration profile.
    AccelProfile {
        profile: String,
        #[arg(short, long)]
        identifier: Option<String>,
    },
    /// Enable or disable tap-to-click.
    Tap {
        state: String,
        #[arg(short, long)]
        identifier: Option<String>,
    },
    /// Enable or disable natural scrolling.
    NaturalScroll {
        state: String,
        #[arg(short, long)]
        identifier: Option<String>,
    },
    /// Set the scroll factor.
    ScrollFactor {
        value: f64,
        #[arg(short, long)]
        identifier: Option<String>,
    },
    /// Enable or disable left-handed pointer mode.
    LeftHanded {
        state: String,
        #[arg(short, long)]
        identifier: Option<String>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ModeAction {
    /// List available window modes.
    List,
    /// Set the current window mode.
    Set { name: String },
    /// Toggle a window mode.
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

/// Subcommands of `pending-tmp-rule` — runtime-added one-shot window rules.
#[derive(Debug, Clone, Subcommand)]
pub enum PendingTmpRuleAction {
    /// Add a new pending tmp rule and print its id.
    ///
    /// With no `--class`/`--instance`/`--title` flag, the rule matches the
    /// *next* window regardless of identity — useful for forcing the very
    /// next spawn (e.g. an app launched via `spawn`) into a particular
    /// placement. With any matcher set, only a window whose
    /// class/instance/title contains the supplied string consumes it.
    Add {
        /// Window class to match.
        #[arg(long)]
        class: Option<String>,
        /// Window instance to match.
        #[arg(long)]
        instance: Option<String>,
        /// Window title substring to match.
        #[arg(long)]
        title: Option<String>,
        /// Force the matched window to be floating.
        #[arg(long, conflicts_with = "tile")]
        float: bool,
        /// Force the matched window to be tiled.
        #[arg(long, conflicts_with = "float")]
        tile: bool,
        /// 1-indexed tag number to assign to the matched window.
        #[arg(long)]
        tag: Option<u32>,
        /// Backend monitor index to place the matched window on.
        #[arg(long, value_name = "INDEX")]
        on_monitor: Option<i32>,
        /// Time-to-live in milliseconds. Must be > 0; default 30_000.
        #[arg(long, default_value_t = 30_000)]
        timeout_ms: u64,
    },
    /// List all currently-pending one-shot rules.
    List,
    /// Cancel a pending rule by id.
    Cancel {
        /// Rule id (from `--list` or the value returned by `add`).
        id: u64,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum CommandKind {
    /// Run a named compositor action, or list available actions.
    Action {
        name: Option<String>,
        args: Vec<String>,
        #[arg(long, short = 'l')]
        list: bool,
    },
    /// Show compositor status.
    Status,
    /// Reload the compositor configuration.
    Reload,
    /// List, switch, and configure monitors.
    Monitor {
        #[command(subcommand)]
        action: MonitorAction,
    },
    /// Inspect, resize, or close windows.
    Window {
        #[command(subcommand)]
        action: WindowAction,
    },
    /// View and name tags.
    Tag {
        #[command(subcommand)]
        action: TagAction,
    },
    /// Toggle compositor features.
    Toggle {
        #[command(subcommand)]
        action: ToggleCliAction,
    },
    /// Launch a command through the compositor.
    Spawn { command: Vec<String> },
    /// Warp the pointer to the focused window.
    WarpFocus,
    /// Move the focused tag view to another monitor.
    TagMon {
        #[arg(default_value = "next")]
        direction: MonitorDirection,
    },
    /// Follow focus to a neighboring monitor.
    FollowMon {
        #[arg(default_value = "next")]
        direction: MonitorDirection,
    },
    /// Set or list the current layout.
    Layout { name: Option<String> },
    /// Get or set the colour theme. With no argument, prints the current theme.
    /// Pass a theme name (e.g. `nord`) to switch, or `--list`/`-l` to list them.
    Theme {
        name: Option<String>,
        #[arg(long, short = 'l')]
        list: bool,
    },
    /// Set the border width for windows.
    Border { width: Option<u32> },
    /// Add, list, or cancel runtime-added one-shot window rules.
    ///
    /// Pending tmp rules apply to the next matching window's initial rule
    /// application and are then consumed. Each entry has a TTL (default
    /// 30 seconds) and is dropped if the deadline passes first.
    PendingTmpRule {
        #[command(subcommand)]
        action: PendingTmpRuleAction,
    },
    /// Manage keyboard layouts and input settings.
    Keyboard {
        #[command(subcommand)]
        action: KeyboardAction,
    },
    /// Manage scratchpad windows.
    Scratchpad {
        #[command(subcommand)]
        action: ScratchpadAction,
    },
    /// Manage pointer and input-device settings.
    #[command(alias = "input")]
    Mouse {
        #[command(subcommand)]
        action: InputAction,
    },
    /// Manage named window modes.
    Mode {
        #[command(subcommand)]
        action: ModeAction,
    },
    /// Set the status-bar text.
    UpdateStatus { text: String },
    /// Set the wallpaper image path.
    Wallpaper { path: String },
    /// Read or update runtime configuration values.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Unstable profiling/test API. Requires INSTANTWM_TEST=1 on the compositor.
    Test {
        #[command(subcommand)]
        action: TestAction,
    },
    /// Exit the compositor.
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
            ToggleCliAction::BottomBar { action } => Self::BottomBar(action.unwrap_or_default()),
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
            ScratchpadAction::Resize {
                name,
                width,
                height,
            } => Self::Resize {
                name,
                width_percent: width,
                height_percent: height,
            },
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
            ScratchpadAction::Restore { name, window_id } => Self::Restore { name, window_id },
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

impl From<PendingTmpRuleAction> for PendingTmpRuleCmd {
    fn from(action: PendingTmpRuleAction) -> Self {
        match action {
            PendingTmpRuleAction::Add {
                class,
                instance,
                title,
                float,
                tile,
                tag,
                on_monitor,
                timeout_ms,
            } => Self::Add {
                class,
                instance,
                title,
                is_floating: if float {
                    Some(true)
                } else if tile {
                    Some(false)
                } else {
                    None
                },
                tag,
                on_monitor,
                timeout_ms,
            },
            PendingTmpRuleAction::List => Self::List,
            PendingTmpRuleAction::Cancel { id } => Self::Cancel(id),
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
            CommandKind::PendingTmpRule { action } => Self::PendingTmpRule(action.into()),
            CommandKind::Keyboard { action } => Self::Keyboard(action.into()),
            CommandKind::Scratchpad { action } => Self::Scratchpad(action.into()),
            CommandKind::Mouse { action } => Self::Input(action.into()),
            CommandKind::Mode { action } => Self::Mode(action.into()),
            CommandKind::Wallpaper { path } => Self::Wallpaper(path),
            CommandKind::UpdateStatus { text } => Self::UpdateStatus(text),
            CommandKind::Config { action } => Self::Config(action.into()),
            CommandKind::Test { action } => Self::Test(test_command(action)),
            CommandKind::Quit => Self::Quit,
        }
    }
}
