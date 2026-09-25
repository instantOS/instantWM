pub use crate::backend::WindowProtocol;
pub use crate::config::config_toml::{
    AccelProfile, ColorTheme, MirrorFit, MonitorConfig, ToggleSetting, Transform, VrrMode,
};
pub use crate::floating::scratchpad::DEFAULT_SCRATCHPAD_NAME;
pub use crate::types::{EdgeDirection, KeyboardLayout, MonitorSelector, RuleGeometry, TagMask};
use bincode::{Decode, Encode};
use clap::{ArgAction, Subcommand};

pub const IPC_PROTOCOL_VERSION: &str = env!("IPC_PROTOCOL_VERSION");

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct IpcRequest {
    pub version: String,
    pub ignore_version: bool,
    pub command: IpcCommand,
}

impl IpcRequest {
    pub fn new(command: IpcCommand, ignore_version: bool) -> Self {
        Self {
            version: IPC_PROTOCOL_VERSION.to_string(),
            ignore_version,
            command,
        }
    }

    pub fn validate_version(&self) -> Result<(), String> {
        if self.ignore_version || self.version == IPC_PROTOCOL_VERSION {
            return Ok(());
        }
        Err(format!(
            "version mismatch: client is {}, server is {}. Please ensure instantwmctl and instantWM are the same version.",
            self.version, IPC_PROTOCOL_VERSION
        ))
    }
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize, Subcommand)]
pub enum MonitorCommand {
    /// List connected monitors and their current configuration.
    List,
    /// Switch focus to a monitor by output name, layout position,
    /// "focused" or "primary".
    Switch {
        #[arg(value_name = "MONITOR")]
        monitor: MonitorSelector,
    },
    /// Focus the next monitor.
    Next {
        #[arg(default_value_t = 1)]
        count: u32,
    },
    /// Focus the previous monitor.
    Prev {
        #[arg(default_value_t = 1)]
        count: u32,
    },
    /// Configure a monitor's mode, position, scale, transform, VRR, mirroring,
    /// or power state. Omitted options keep their current value.
    Set {
        #[arg(default_value = "focused")]
        identifier: String,
        #[command(flatten)]
        config: MonitorConfig,
    },
    /// List the available modes for a monitor.
    Modes {
        #[arg(default_value = "focused")]
        identifier: String,
    },
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize, Subcommand)]
pub enum ConfigCommand {
    /// Get a runtime config value by key (e.g. layout.inner_gap).
    Get { key: String },
    /// Set a runtime config value by key (e.g. layout.inner_gap 12).
    Set { key: String, value: String },
    /// Flip a boolean runtime config value by key (e.g. window.decor_hints).
    /// Returns the new value.
    Toggle { key: String },
    /// List runtime config keys and their current values, optionally only
    /// those under a section or key prefix (e.g. `fonts`, `fonts.icon_size`).
    List { prefix: Option<String> },
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize, Subcommand)]
pub enum ScratchpadCommand {
    /// Show the status of all scratchpads, or only of NAME.
    #[command(visible_alias = "list")]
    Status { name: Option<String> },
    /// Show a scratchpad.
    Show {
        #[arg(default_value = DEFAULT_SCRATCHPAD_NAME)]
        name: String,
        /// Show every scratchpad.
        #[arg(short, long, conflicts_with = "name")]
        all: bool,
    },
    /// Hide a scratchpad.
    Hide {
        #[arg(default_value = DEFAULT_SCRATCHPAD_NAME)]
        name: String,
        /// Hide every scratchpad.
        #[arg(short, long, conflicts_with = "name")]
        all: bool,
    },
    /// Toggle a scratchpad's visibility.
    Toggle {
        #[arg(default_value = DEFAULT_SCRATCHPAD_NAME)]
        name: String,
    },
    /// Resize a scratchpad as a percentage of its monitor.
    Resize {
        #[arg(default_value = DEFAULT_SCRATCHPAD_NAME)]
        name: String,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
        width: u32,
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..=100))]
        height: u32,
    },
    /// Create or mark a window as a scratchpad.
    #[command(alias = "make")]
    Create {
        #[arg(default_value = DEFAULT_SCRATCHPAD_NAME)]
        name: String,
        #[arg(long, short = 'w')]
        window_id: Option<u32>,
        #[arg(long, default_value = "hidden")]
        status: ScratchpadInitialStatus,
        #[arg(long)]
        direction: Option<EdgeDirection>,
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

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Encode,
    Decode,
    serde::Serialize,
    serde::Deserialize,
    clap::ValueEnum,
)]
pub enum ScratchpadInitialStatus {
    Hidden,
    Shown,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize, Subcommand)]
pub enum KeyboardCommand {
    /// List configured keyboard layouts.
    List {
        /// List every layout known to XKB instead.
        #[arg(long)]
        all: bool,
    },
    /// Show the active keyboard layout.
    Status,
    /// Select the next keyboard layout.
    Next,
    /// Select the previous keyboard layout.
    Prev,
    /// Set the keyboard layouts, e.g. `us de(nodeadkeys)`.
    Set {
        #[arg(required = true, num_args = 1..)]
        layouts: Vec<KeyboardLayout>,
    },
    /// Add a keyboard layout.
    Add { layout: KeyboardLayout },
    /// Remove a keyboard layout.
    Remove { layout: String },
    /// Enable or disable swapping Escape and Caps Lock.
    SwapEscape {
        #[arg(action = ArgAction::Set)]
        enabled: bool,
    },
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize, Subcommand)]
pub enum TagCommand {
    /// List the tags of the selected monitor with their name, icon, the
    /// label currently shown, and occupancy/selection state.
    List,
    /// Set the selected tag's name for the session (cleared on reload).
    Name { name: String },
    /// Reset all tag names to their configured values.
    Reset,
}

/// Runtime commands for [`PendingTmpRule`]s (the `pending-tmp-rule` IPC).
#[derive(Debug, Clone, Encode, Decode, serde::Serialize, serde::Deserialize, Subcommand)]
pub enum PendingTmpRuleCmd {
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
        /// Force the matched window floating (true) or tiled (false).
        #[arg(long = "floating", value_name = "BOOL")]
        is_floating: Option<bool>,
        /// 1-indexed tag number to assign to the matched window.
        #[arg(long)]
        tag: Option<u32>,
        /// Monitor to place the matched window on: output name, layout
        /// position, "focused" or "primary".
        #[arg(long, value_name = "MONITOR")]
        on_monitor: Option<MonitorSelector>,
        /// Exact floating placement relative to the target monitor's work
        /// area. Implies floating placement.
        #[arg(long, value_name = "X,Y,W,H")]
        geometry: Option<RuleGeometry>,
        /// Manage the matched window without a WM border.
        #[arg(long)]
        borderless: bool,
        /// Time-to-live in milliseconds. Must be > 0.
        #[arg(long, default_value_t = 30_000)]
        timeout_ms: u64,
    },
    /// List all currently-pending one-shot rules.
    List,
    /// Cancel a pending rule by id.
    Cancel {
        /// Rule id (from `list` or the value returned by `add`).
        id: u64,
    },
}

/// Response payload for `PendingTmpRuleCmd::List`.
#[derive(Debug, Clone, Encode, Decode, serde::Serialize, serde::Deserialize)]
pub struct PendingTmpRuleInfo {
    pub id: u64,
    pub class: Option<String>,
    pub instance: Option<String>,
    pub title: Option<String>,
    pub is_floating: Option<bool>,
    pub tag: Option<u32>,
    /// Monitor the rule targets, in selector syntax (`"DP-1"`, `"focused"`,
    /// `"primary"` or a layout position). `None` targets any monitor.
    pub on_monitor: Option<String>,
    /// Exact placement, if the rule pins one.
    pub geometry: Option<String>,
    pub borderless: bool,
    /// Milliseconds remaining until the rule expires.
    pub ms_remaining: u64,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize, Subcommand)]
pub enum WindowCommand {
    /// List managed windows.
    List { window_id: Option<u32> },
    /// Show information about a managed window.
    Info { window_id: Option<u32> },
    /// Activate a managed window: switch to its monitor and tags, then focus
    /// and raise it. Hidden (minimized) windows are restored first.
    Focus { window_id: Option<u32> },
    /// Resize and optionally move a managed window.
    Resize {
        window_id: Option<u32>,
        /// Monitor whose top-left corner the coordinates are relative to:
        /// output name, layout position, "focused" or "primary".
        #[arg(long, value_name = "MONITOR")]
        monitor: Option<MonitorSelector>,
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

/// Unstable commands intended for profiling and automated compositor tests.
///
/// The server rejects these unless `INSTANTWM_TEST=1` was present when
/// instantWM started. Compatibility is deliberately not guaranteed.
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize, Subcommand)]
pub enum TestCommand {
    /// Inject one absolute pointer-motion transaction.
    PointerMove {
        #[arg(allow_negative_numbers = true)]
        x: f64,
        #[arg(allow_negative_numbers = true)]
        y: f64,
        /// Treat coordinates as 0..1 fractions of the focused monitor.
        #[arg(long)]
        normalized: bool,
    },
    /// Focus a window by its stable IPC id.
    FocusWindow { window_id: u32 },
    /// Assign a window to exactly one tag.
    TagWindow { window_id: u32, tag: u32 },
    /// Set tiling/floating state without relying on the current focus.
    SetWindowFloating {
        window_id: u32,
        #[arg(action = ArgAction::Set)]
        floating: bool,
    },
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize, Subcommand)]
pub enum InputCommand {
    /// List input configuration, optionally for one device identifier.
    List { identifier: Option<String> },
    /// List connected input devices.
    Devices,
    /// Set pointer acceleration speed (-1.0 to 1.0).
    PointerAccel {
        #[arg(allow_negative_numbers = true)]
        value: f64,
        #[arg(short, long)]
        identifier: Option<String>,
    },
    /// Set the pointer acceleration profile.
    AccelProfile {
        profile: AccelProfile,
        #[arg(short, long)]
        identifier: Option<String>,
    },
    /// Enable or disable tap-to-click.
    Tap {
        state: ToggleSetting,
        #[arg(short, long)]
        identifier: Option<String>,
    },
    /// Enable or disable natural scrolling.
    NaturalScroll {
        state: ToggleSetting,
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
        state: ToggleSetting,
        #[arg(short, long)]
        identifier: Option<String>,
    },
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum IpcCommand {
    Status,
    Reload,
    RunAction {
        name: String,
        #[serde(default)]
        args: Vec<String>,
    },
    /// Add, list, or cancel runtime-added one-shot window rules.
    PendingTmpRule(PendingTmpRuleCmd),
    UpdateStatus(String),
    Monitor(MonitorCommand),
    Scratchpad(ScratchpadCommand),
    Keyboard(KeyboardCommand),
    Tag(TagCommand),
    Window(WindowCommand),
    Wallpaper(String),
    Input(InputCommand),
    /// List every window layout, marking the selected monitor's active one.
    LayoutList,
    /// Show the selected monitor's full layout state.
    LayoutStatus,
    ListModes,
    Config(ConfigCommand),
    Test(TestCommand),
    GetTheme,
    SetTheme(ColorTheme),
    ListThemes,
    /// List every active keybinding (global, desktop, and per-mode).
    ///
    /// Each entry is a single key + modifier combo, not a multi-key chord.
    ListKeybinds,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct GeometryInfo {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl From<crate::types::Rect> for GeometryInfo {
    fn from(rect: crate::types::Rect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.w,
            height: rect.h,
        }
    }
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct WindowState {
    pub mode: crate::types::ClientMode,
    pub sticky: bool,
    pub hidden: bool,
    pub urgent: bool,
    pub locked: bool,
    pub fixed_size: bool,
    pub never_focus: bool,
}

impl From<&crate::types::client::Client> for WindowState {
    fn from(c: &crate::types::client::Client) -> Self {
        Self {
            mode: c.mode(),
            sticky: c.is_sticky,
            hidden: c.is_hidden,
            urgent: c.is_urgent,
            locked: c.is_locked,
            fixed_size: c.is_fixed_size,
            never_focus: c.never_focus,
        }
    }
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct SizeHintsInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_height: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_height: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_height: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width_increment: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height_increment: Option<i32>,
}

impl TryFrom<&crate::types::client::Client> for SizeHintsInfo {
    type Error = ();

    fn try_from(c: &crate::types::client::Client) -> Result<Self, Self::Error> {
        if !c.size_hints_valid {
            return Err(());
        }
        let h = &c.size_hints;
        Ok(Self {
            min_width: (h.min_width > 0).then_some(h.min_width),
            min_height: (h.min_height > 0).then_some(h.min_height),
            max_width: (h.max_width > 0).then_some(h.max_width),
            max_height: (h.max_height > 0).then_some(h.max_height),
            base_width: (h.base_width > 0).then_some(h.base_width),
            base_height: (h.base_height > 0).then_some(h.base_height),
            width_increment: (h.width_inc > 0).then_some(h.width_inc),
            height_increment: (h.height_inc > 0).then_some(h.height_inc),
        })
    }
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub protocol: WindowProtocol,
    /// Spatial monitor position used by monitor-switching commands.
    pub monitor_position: usize,
    pub tags: TagMask,
    pub geometry: GeometryInfo,
    pub border_width: i32,
    pub state: WindowState,
    /// True for the selected window of the selected monitor.
    pub is_focused: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scratchpad: Option<ScratchpadInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_hints: Option<SizeHintsInfo>,
}

impl WindowInfo {
    pub fn from_client(
        c: &crate::types::client::Client,
        valid_tag_mask: TagMask,
        protocol: WindowProtocol,
        monitor_position: usize,
        is_focused: bool,
    ) -> Self {
        Self {
            id: c.win.0,
            title: c.name.clone(),
            protocol,
            monitor_position,
            tags: c.tags & valid_tag_mask,
            geometry: c.geo.into(),
            border_width: c.border_width,
            state: c.into(),
            is_focused,
            scratchpad: ScratchpadInfo::from_client(c, monitor_position),
            size_hints: SizeHintsInfo::try_from(c).ok(),
        }
    }
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct MonitorInfo {
    /// Stable identifier, distinct from display ordering.
    pub id: u64,
    /// Current spatial position in the monitor list.
    pub position: usize,
    /// Backend-assigned monitor number retained for diagnostics.
    pub backend_index: i32,
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    pub is_selected: bool,
    pub vrr_support: crate::backend::BackendVrrSupport,
    pub vrr_mode: Option<VrrMode>,
    pub vrr_enabled: bool,
    /// Other physical outputs presenting this logical monitor: realized
    /// mirrors and clones set up by other tools.
    pub mirrors: Vec<String>,
    /// Mirror declarations targeting this logical monitor, including heads
    /// that are disconnected or whose output configuration has not applied.
    pub requested_mirrors: Vec<String>,
}

pub use crate::floating::scratchpad::ScratchpadInfo;

/// A single display mode (resolution + refresh rate).
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct MonitorMode {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,
}

impl std::str::FromStr for MonitorMode {
    type Err = String;

    /// Parse a mode string like "1920x1080@60.000" into a MonitorMode.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || format!("invalid monitor mode: {s}");
        let (res, rate_str) = s.split_once('@').ok_or_else(invalid)?;
        let (w, h) = res.split_once('x').ok_or_else(invalid)?;
        let width: u32 = w.parse().map_err(|_| invalid())?;
        let height: u32 = h.parse().map_err(|_| invalid())?;
        let rate_hz: f64 = rate_str.parse().map_err(|_| invalid())?;
        let refresh_mhz = (rate_hz * 1000.0) as u32;
        Ok(Self {
            width,
            height,
            refresh_mhz,
        })
    }
}

/// Modes for a specific display.
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct DisplayModes {
    pub name: String,
    pub modes: Vec<MonitorMode>,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct ModeInfo {
    pub name: String,
    pub description: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct WmStatusInfo {
    pub version: String,
    pub protocol_version: String,
    pub build_commit: String,
    pub backend: String,
    pub running: bool,
    pub monitors: usize,
    pub windows: usize,
    pub tags: usize,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct KeyboardLayoutInfo {
    pub name: String,
    pub variant: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct TagInfo {
    /// 0-based tag index.
    pub index: u32,
    /// Configured (or session-renamed) name; `None` when unset.
    pub name: Option<String>,
    /// Configured icon; `None` when the tag has none.
    pub icon: Option<String>,
    /// The label the bar shows right now: the icon while icon mode is on
    /// and an icon is set, otherwise the name.
    pub label: String,
    /// Whether at least one window occupies this tag (scratchpad excluded).
    pub occupied: bool,
    /// Whether this tag is in the selected monitor's current tagset.
    pub selected: bool,
}

/// One keybinding as reported by `instantwmctl keybinds`.
///
/// Bindings are rendered as human-friendly strings (`Super + Shift + S`)
/// rather than raw masks so the consumer (a help menu, a settings UI) can
/// display them directly. `origin` distinguishes compiled defaults from the
/// user's config.toml.
///
/// `modifiers`/`key` are sent raw; the composed `Modifiers + Key` string is
/// presentation logic owned by each renderer (instantWM's `binding_text` for
/// the text table, instantCLI's `KeybindRow::binding` for the fzf UI). Keeping
/// them separate is deliberate: the wire contract stays minimal and each side
/// composes exactly once, at its own call site.
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct KeybindInfo {
    pub modifiers: String,
    pub key: String,
    pub action: String,
    pub mode: Option<String>,
    pub origin: crate::types::KeybindOrigin,
}

/// One window-layout entry as reported by layout queries.
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct LayoutInfo {
    pub name: String,
    pub label: String,
    pub symbol: String,
    pub is_active: bool,
}

/// The selected monitor's full layout state.
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct LayoutStatusInfo {
    pub monitor_id: u64,
    /// "tiled", "floating", or "maximized".
    pub presentation: String,
    /// The active cycle entry: the lens while lensed, the slot otherwise.
    pub layout: LayoutInfo,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum Response {
    Ok,
    Err(String),
    WindowList(Vec<WindowInfo>),
    WindowInfo(WindowInfo),
    MonitorList(Vec<MonitorInfo>),
    MonitorModes(Vec<DisplayModes>),
    ScratchpadList(Vec<ScratchpadInfo>),
    ModeList(Vec<ModeInfo>),
    LayoutList(Vec<LayoutInfo>),
    LayoutStatus(LayoutStatusInfo),
    Status(WmStatusInfo),
    KeyboardLayoutList(Vec<KeyboardLayoutInfo>),
    TagList(Vec<TagInfo>),
    KeybindList(Vec<KeybindInfo>),
    ConfigValue(String),
    ConfigList(Vec<(String, String)>),
    Message(String),
    /// Active colour theme name (kebab-case), e.g. `nord`.
    Theme(String),
    /// Available colour theme names.
    ThemeList(Vec<String>),
    /// Listing of currently-pending one-shot rules.
    PendingTmpRuleList(Vec<PendingTmpRuleInfo>),
    /// Ack of a successful Add command. The id is needed for cancel/inspect.
    PendingTmpRuleAdded {
        id: u64,
        timeout_ms: u64,
    },
}

impl Response {
    pub fn ok() -> Self {
        Response::Ok
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Response::Err(msg.into())
    }
}
