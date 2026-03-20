pub use crate::layouts::LayoutKind;
pub use crate::types::{MonitorDirection, SpecialNext};
use bincode::{Decode, Encode};

/// Protocol version generated at compile time from crate version and git hash.
pub const IPC_PROTOCOL_VERSION: &str = env!("IPC_PROTOCOL_VERSION");

/// A single keyboard layout with optional variant (used for IPC).
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct KeyboardLayout {
    pub name: String,
    pub variant: Option<String>,
}

impl KeyboardLayout {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            variant: None,
        }
    }

    pub fn with_variant(name: impl Into<String>, variant: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            variant: Some(variant.into()),
        }
    }
}

impl From<&str> for KeyboardLayout {
    fn from(s: &str) -> Self {
        // Parse "layout(variant)" syntax
        if let Some((name, variant)) = s.strip_suffix(')').and_then(|s| s.rsplit_once('(')) {
            Self::with_variant(name, variant)
        } else {
            Self::new(s)
        }
    }
}

impl From<crate::globals::KeyboardLayout> for KeyboardLayout {
    fn from(l: crate::globals::KeyboardLayout) -> Self {
        Self {
            name: l.name,
            variant: l.variant,
        }
    }
}

/// IPC request with protocol version validation.
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct IpcRequest {
    pub version: String,
    pub ignore_version: bool,
    pub command: IpcCommand,
    /// Request JSON-formatted output instead of human-readable text.
    #[serde(default)]
    pub json_output: bool,
}

impl IpcRequest {
    /// Create a new IPC request with the current protocol version.
    pub fn new(command: IpcCommand) -> Self {
        Self {
            version: IPC_PROTOCOL_VERSION.to_string(),
            ignore_version: false,
            command,
            json_output: false,
        }
    }

    /// Create a new IPC request with the current protocol version and version ignore flag.
    pub fn new_ignore_version(command: IpcCommand, ignore: bool) -> Self {
        Self {
            version: IPC_PROTOCOL_VERSION.to_string(),
            ignore_version: ignore,
            command,
            json_output: false,
        }
    }

    /// Create a new IPC request with all options.
    pub fn new_with_options(command: IpcCommand, ignore_version: bool, json_output: bool) -> Self {
        Self {
            version: IPC_PROTOCOL_VERSION.to_string(),
            ignore_version,
            command,
            json_output,
        }
    }

    /// Validate that the request's protocol version matches the expected version.
    pub fn validate_version(&self) -> Result<(), String> {
        if self.ignore_version {
            return Ok(());
        }
        if self.version == IPC_PROTOCOL_VERSION {
            Ok(())
        } else {
            Err(format!(
                "version mismatch: client is {}, server is {}. Please ensure instantwmctl and instantWM are the same version.",
                self.version, IPC_PROTOCOL_VERSION
            ))
        }
    }
}

/// Monitor-related commands.
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum MonitorCommand {
    /// List all monitors with their information.
    List,
    /// Switch focus to a specific monitor by index.
    Switch { index: u32 },
    /// Move focus to the next monitor(s).
    Next { count: u32 },
    /// Move focus to the previous monitor(s).
    Prev { count: u32 },
    /// Configure a monitor.
    Set {
        /// Monitor identifier (name or "*" for all).
        identifier: String,
        /// Resolution (e.g., "1920x1080").
        resolution: Option<String>,
        /// Refresh rate in Hz.
        refresh_rate: Option<f32>,
        /// Position (e.g., "1920,0").
        position: Option<String>,
        /// Scale factor.
        scale: Option<f32>,
        /// Whether to enable or disable the output.
        enable: Option<bool>,
    },
}

/// Mode-related commands.
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum ModeCommand {
    /// List all configured modes (shows name and description).
    List,
    /// Set the current mode (enter a mode).
    Set(String),
    /// Toggle the current mode (enter if not active, else return to default).
    Toggle(String),
}

/// Scratchpad-related commands.
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum ScratchpadCommand {
    /// List all scratchpads (show names and visibility status).
    List,
    /// Toggle scratchpad visibility.
    Toggle(Option<String>),
    /// Show a scratchpad (make visible on current tag).
    Show(String),
    /// Hide a scratchpad (remove from current tag).
    Hide(String),
    /// Get scratchpad visibility status.
    Status(Option<String>),
    /// Create a scratchpad from the selected window.
    Create(Option<String>),
    /// Remove scratchpad status from the selected window.
    Delete,
}

/// Keyboard layout-related commands.
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum KeyboardCommand {
    /// Switch to the next keyboard layout.
    Next,
    /// Switch to the previous keyboard layout.
    Prev,
    /// Get current keyboard layout status (e.g., "2/3: us (nodeadkeys)").
    Status,
    /// List configured keyboard layouts (with * marker for current).
    List,
    /// List all available XKB layouts from the system.
    ListAll,
    /// Set keyboard layouts (replaces all, switches to first).
    Set(Vec<KeyboardLayout>),
    /// Add a keyboard layout to the active list.
    Add(KeyboardLayout),
    /// Remove a keyboard layout from the active list (by name or #index).
    Remove(String),
}

/// Tag-related commands.
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum TagCommand {
    /// Switch to a tag (workspace).
    View(u32),
    /// Rename the current tag.
    Name(String),
    /// Reset all tag names to defaults ("1" through "9").
    ResetNames,
}

/// Window-related commands.
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum WindowCommand {
    /// Get window geometry.
    Geom(Option<u32>),
    /// Close a window.
    Close(Option<u32>),
    /// List all managed windows.
    List(Option<u32>),
}

/// Toggle-related commands.
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum ToggleCommand {
    /// Toggle or set animated windows mode.
    Animated(Option<String>),
    /// Toggle or set focus follows mouse.
    FocusFollowsMouse(Option<String>),
    /// Toggle or set focus follows mouse for floating windows only.
    FocusFollowsFloatMouse(Option<String>),
    /// Toggle or set alt-tag mode (shows alternative tag names in bar).
    AltTag(Option<String>),
    /// Toggle or set hide tags visibility (hides tag bar).
    HideTags(Option<String>),
}

/// Input device configuration commands (sway-compatible).
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum InputCommand {
    /// List all input settings for a device identifier (or all if None).
    List(Option<String>),
    /// List all available physical input devices.
    Devices,
    /// Set pointer acceleration speed [-1.0, 1.0].
    PointerAccel {
        identifier: Option<String>,
        value: f64,
    },
    /// Set acceleration profile (flat or adaptive).
    AccelProfile {
        identifier: Option<String>,
        profile: String,
    },
    /// Enable or disable tap-to-click.
    Tap {
        identifier: Option<String>,
        enabled: bool,
    },
    /// Enable or disable natural scrolling.
    NaturalScroll {
        identifier: Option<String>,
        enabled: bool,
    },
    /// Set scroll factor (multiplier).
    ScrollFactor {
        identifier: Option<String>,
        value: f64,
    },
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum IpcCommand {
    /// Get status information about the running instantWM instance.
    Status,
    /// Reload the runtime configuration from disk.
    Reload,
    /// Run a named WM action with optional arguments.
    RunAction {
        /// Action name (e.g., "zoom", "focus_next").
        name: String,
        /// Optional arguments for the action.
        #[serde(default)]
        args: Vec<String>,
    },
    /// Spawn a command.
    Spawn(String),
    /// Warp cursor to the currently focused window.
    WarpFocus,
    /// Move the selected window to another monitor.
    TagMon(MonitorDirection),
    /// Move the selected window to another monitor and follow it.
    FollowMon(MonitorDirection),
    /// Set the layout type.
    Layout(LayoutKind),
    /// Set border width for the selected window.
    Border(Option<u32>),
    /// Set special next mode for cycling through windows.
    SpecialNext(SpecialNext),
    /// Update status text via IPC.
    UpdateStatus(String),
    /// Monitor-related commands.
    Monitor(MonitorCommand),
    /// Scratchpad-related commands.
    Scratchpad(ScratchpadCommand),
    /// Keyboard layout-related commands.
    Keyboard(KeyboardCommand),
    /// Tag-related commands.
    Tag(TagCommand),
    /// Window-related commands.
    Window(WindowCommand),
    /// Toggle-related commands.
    Toggle(ToggleCommand),
    /// Set the wallpaper.
    Wallpaper(String),
    /// Input device configuration commands.
    Input(InputCommand),
    /// Mode-related commands.
    Mode(ModeCommand),
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum IpcResponse {
    Ok(String),
    Err(String),
}

impl IpcResponse {
    pub fn ok(msg: impl Into<String>) -> Self {
        IpcResponse::Ok(msg.into())
    }

    pub fn err(msg: impl Into<String>) -> Self {
        IpcResponse::Err(msg.into())
    }
}
