use clap::{Parser, Subcommand};
use instantwm::actions::NamedAction;
use instantwm::ipc_types::{
    ColorTheme, ConfigCommand, InputCommand, IpcCommand, KeyboardCommand, MonitorCommand,
    PendingTmpRuleCmd, ScratchpadCommand, TagCommand, TestCommand, WindowCommand,
};
use instantwm::layouts::LayoutCommand;
use instantwm::types::{FocusFollowsMouseMode, MonitorDirection, ToggleAction};

#[derive(Debug, Clone, Subcommand)]
pub enum LayoutAction {
    /// List every layout, marking the selected monitor's active one.
    List,
    /// Show the selected monitor's full layout state.
    Status,
    /// Activate a layout (repeat to reset it to stock geometry).
    Set {
        #[arg(value_parser = parse_layout)]
        layout: LayoutCommand,
    },
    /// Step to the next layout in the cycle.
    Next,
    /// Step to the previous layout in the cycle.
    Prev,
}

fn parse_layout(name: &str) -> Result<LayoutCommand, String> {
    LayoutCommand::from_name(name).ok_or_else(|| format!("invalid layout '{name}'"))
}

#[derive(Debug, Clone, Subcommand)]
pub enum TestAction {
    #[command(flatten)]
    Remote(TestCommand),
    /// Interpolate a pointer path. Points use the form X,Y.
    PointerPath {
        #[arg(required = true, num_args = 2..)]
        points: Vec<String>,
        #[arg(long, default_value_t = 1000)]
        duration_ms: u64,
        #[arg(long, default_value_t = 30)]
        hz: u32,
        /// Treat coordinates as 0..1 fractions of the focused monitor.
        #[arg(long)]
        normalized: bool,
    },
    /// Wait until at least COUNT windows are mapped.
    WaitWindows {
        count: usize,
        #[arg(long, default_value_t = 5000)]
        timeout_ms: u64,
        #[arg(long, default_value_t = 25)]
        poll_ms: u64,
        /// Require exactly COUNT rather than at least COUNT.
        #[arg(long)]
        exact: bool,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum ToggleCliAction {
    /// Toggle window animations
    Animated { action: Option<ToggleAction> },
    /// Set focus-follows-mouse behavior
    FocusFollowsMouse {
        /// off: disabled; normal: pointer motion only; force: include scene changes
        mode: FocusFollowsMouseMode,
    },
    /// Toggle focus-follows-mouse for floating windows
    FocusFollowsFloatMouse { action: Option<ToggleAction> },
    /// Toggle alt-tag mode
    AltTag { action: Option<ToggleAction> },
    /// Show/hide tag bar
    HideTags { action: Option<ToggleAction> },
    /// Toggle the bottom gesture strip
    BottomBar { action: Option<ToggleAction> },
}

#[derive(Debug, Clone, Subcommand)]
pub enum TagAction {
    /// View a tag by number.
    View { number: u32 },
    #[command(flatten)]
    Remote(TagCommand),
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
    #[command(flatten)]
    Remote(ConfigCommand),
}

#[derive(Debug, Clone, Subcommand)]
pub enum CommandKind {
    /// Run a named compositor action, or list available actions.
    Action {
        #[arg(required_unless_present = "list")]
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
        action: MonitorCommand,
    },
    /// Inspect, resize, or close windows.
    Window {
        #[command(subcommand)]
        action: WindowCommand,
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
    Spawn {
        #[arg(required = true)]
        command: Vec<String>,
    },
    /// Warp the pointer to the focused window.
    WarpFocus,
    /// Move the focused client to another monitor without following.
    SendMon {
        #[arg(default_value = "next")]
        direction: MonitorDirection,
    },
    /// Follow focus to a neighboring monitor.
    FollowMon {
        #[arg(default_value = "next")]
        direction: MonitorDirection,
    },
    /// Query or set the window layout.
    Layout {
        #[command(subcommand)]
        action: LayoutAction,
    },
    /// Get or set the colour theme. With no argument, prints the current theme.
    Theme {
        #[arg(conflicts_with = "list")]
        name: Option<ColorTheme>,
        /// List the built-in themes.
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
        action: PendingTmpRuleCmd,
    },
    /// Manage keyboard layouts and input settings.
    Keyboard {
        #[command(subcommand)]
        action: KeyboardCommand,
    },
    /// Manage scratchpad windows.
    Scratchpad {
        #[command(subcommand)]
        action: ScratchpadCommand,
    },
    /// Manage pointer and input-device settings.
    #[command(alias = "input")]
    Mouse {
        #[command(subcommand)]
        action: InputCommand,
    },
    /// Manage named window modes.
    Mode {
        #[command(subcommand)]
        action: ModeAction,
    },
    /// Set the status-bar text ("-" reads lines from stdin).
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
    /// List active keybindings (global, desktop, and per-mode).
    Keybinds,
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

fn run(action: NamedAction) -> IpcCommand {
    IpcCommand::RunAction {
        name: action.name().to_string(),
        args: action.args(),
    }
}

impl ToggleCliAction {
    fn into_action(self) -> NamedAction {
        match self {
            Self::Animated { action } => NamedAction::ToggleAnimated(action),
            Self::FocusFollowsMouse { mode } => NamedAction::SetFocusFollowsMouse(mode),
            Self::FocusFollowsFloatMouse { action } => {
                NamedAction::ToggleFocusFollowsFloatMouse(action)
            }
            Self::AltTag { action } => NamedAction::ToggleAltTag(action),
            Self::HideTags { action } => NamedAction::ToggleHideTags(action),
            Self::BottomBar { action } => NamedAction::ToggleBottomBar(action),
        }
    }
}

impl CommandKind {
    /// The IPC request for this command, or the command itself when the
    /// client handles it alone (`action --list`, `config default`, test paths
    /// and waits).
    pub fn into_ipc(self) -> Result<IpcCommand, Self> {
        Ok(match self {
            Self::Action { list: true, .. }
            | Self::Config {
                action: ConfigAction::Default,
            }
            | Self::Test {
                action: TestAction::PointerPath { .. } | TestAction::WaitWindows { .. },
            } => return Err(self),
            Self::Action { name, args, .. } => IpcCommand::RunAction {
                name: name.expect("clap requires a name without --list"),
                args,
            },
            Self::Status => IpcCommand::Status,
            Self::Reload => IpcCommand::Reload,
            Self::Monitor { action } => IpcCommand::Monitor(action),
            Self::Window { action } => IpcCommand::Window(action),
            Self::Tag {
                action: TagAction::View { number },
            } => run(NamedAction::ViewTag(number)),
            Self::Tag {
                action: TagAction::Remote(command),
            } => IpcCommand::Tag(command),
            Self::Toggle { action } => run(action.into_action()),
            Self::Spawn { command } => run(NamedAction::Spawn(command)),
            Self::WarpFocus => run(NamedAction::WarpFocus),
            Self::SendMon { direction } => run(NamedAction::SendMon(direction)),
            Self::FollowMon { direction } => run(NamedAction::FollowMon(direction)),
            Self::Layout { action } => match action {
                LayoutAction::List => IpcCommand::LayoutList,
                LayoutAction::Status => IpcCommand::LayoutStatus,
                LayoutAction::Set { layout } => run(NamedAction::SetLayout(layout)),
                LayoutAction::Next => run(NamedAction::CycleLayoutNext),
                LayoutAction::Prev => run(NamedAction::CycleLayoutPrev),
            },
            Self::Theme { list: true, .. } => IpcCommand::ListThemes,
            Self::Theme {
                name: Some(theme), ..
            } => IpcCommand::SetTheme(theme),
            Self::Theme { name: None, .. } => IpcCommand::GetTheme,
            Self::Border { width } => run(NamedAction::SetBorder(width)),
            Self::PendingTmpRule { action } => IpcCommand::PendingTmpRule(action),
            Self::Keyboard { action } => IpcCommand::Keyboard(action),
            Self::Scratchpad { action } => IpcCommand::Scratchpad(action),
            Self::Mouse { action } => IpcCommand::Input(action),
            Self::Mode { action } => match action {
                ModeAction::List => IpcCommand::ListModes,
                ModeAction::Set { name } => run(NamedAction::SetMode(name)),
                ModeAction::Toggle { name } => run(NamedAction::ModeToggle(name)),
            },
            Self::Wallpaper { path } => IpcCommand::Wallpaper(path),
            Self::UpdateStatus { text } => IpcCommand::UpdateStatus(text),
            Self::Config {
                action: ConfigAction::Remote(command),
            } => IpcCommand::Config(command),
            Self::Test {
                action: TestAction::Remote(command),
            } => IpcCommand::Test(command),
            Self::Keybinds => IpcCommand::ListKeybinds,
            Self::Quit => run(NamedAction::Quit),
        })
    }
}
