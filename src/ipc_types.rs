pub use crate::backend::WindowProtocol;
pub use crate::config::config_toml::VrrMode;
pub use crate::layouts::LayoutKind;
pub use crate::types::{MonitorDirection, SpecialNext, TagMask};
use bincode::{Decode, Encode};

pub const IPC_PROTOCOL_VERSION: &str = env!("IPC_PROTOCOL_VERSION");

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

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct IpcRequest {
    pub version: String,
    pub ignore_version: bool,
    pub command: IpcCommand,
}

impl IpcRequest {
    pub fn new(command: IpcCommand) -> Self {
        Self {
            version: IPC_PROTOCOL_VERSION.to_string(),
            ignore_version: false,
            command,
        }
    }

    pub fn new_ignore_version(command: IpcCommand, ignore: bool) -> Self {
        Self {
            version: IPC_PROTOCOL_VERSION.to_string(),
            ignore_version: ignore,
            command,
        }
    }

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
pub enum Transform {
    Normal,
    #[serde(rename = "90")]
    #[value(name = "90")]
    _90,
    #[serde(rename = "180")]
    #[value(name = "180")]
    _180,
    #[serde(rename = "270")]
    #[value(name = "270")]
    _270,
    Flipped,
    #[serde(rename = "flipped-90")]
    #[value(name = "flipped-90")]
    Flipped90,
    #[serde(rename = "flipped-180")]
    #[value(name = "flipped-180")]
    Flipped180,
    #[serde(rename = "flipped-270")]
    #[value(name = "flipped-270")]
    Flipped270,
}

impl Transform {
    pub fn to_smithay(self) -> smithay::utils::Transform {
        match self {
            Transform::Normal => smithay::utils::Transform::Normal,
            Transform::_90 => smithay::utils::Transform::_90,
            Transform::_180 => smithay::utils::Transform::_180,
            Transform::_270 => smithay::utils::Transform::_270,
            Transform::Flipped => smithay::utils::Transform::Flipped,
            Transform::Flipped90 => smithay::utils::Transform::Flipped90,
            Transform::Flipped180 => smithay::utils::Transform::Flipped180,
            Transform::Flipped270 => smithay::utils::Transform::Flipped270,
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            Transform::Normal => "normal".to_string(),
            Transform::_90 => "90".to_string(),
            Transform::_180 => "180".to_string(),
            Transform::_270 => "270".to_string(),
            Transform::Flipped => "flipped".to_string(),
            Transform::Flipped90 => "flipped-90".to_string(),
            Transform::Flipped180 => "flipped-180".to_string(),
            Transform::Flipped270 => "flipped-270".to_string(),
        }
    }
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum MonitorCommand {
    List,
    Switch {
        index: u32,
    },
    Next {
        count: u32,
    },
    Prev {
        count: u32,
    },
    Set {
        identifier: String,
        resolution: Option<String>,
        refresh_rate: Option<f32>,
        position: Option<String>,
        scale: Option<f32>,
        transform: Option<Transform>,
        enable: Option<bool>,
        vrr: Option<VrrMode>,
    },
    Modes {
        identifier: Option<String>,
    },
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum ModeCommand {
    List,
    Set(String),
    Toggle(String),
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum ScratchpadCommand {
    List,
    Toggle(Option<String>),
    Show(Option<String>),
    ShowAll,
    Hide(Option<String>),
    HideAll,
    Status(Option<String>),
    Create {
        name: String,
        window_id: Option<u32>,
        status: ScratchpadInitialStatus,
        direction: Option<String>,
    },
    Delete {
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

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum KeyboardCommand {
    Next,
    Prev,
    Status,
    List,
    ListAll,
    Set(Vec<KeyboardLayout>),
    Add(KeyboardLayout),
    Remove(String),
    SwapEscape(bool),
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum TagCommand {
    View(u32),
    Name(String),
    ResetNames,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum WindowCommand {
    Info(Option<u32>),
    Resize {
        window_id: Option<u32>,
        monitor: Option<String>,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    Close(Option<u32>),
    List(Option<u32>),
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum ToggleCommand {
    Animated(Option<String>),
    FocusFollowsMouse(Option<String>),
    FocusFollowsFloatMouse(Option<String>),
    AltTag(Option<String>),
    HideTags(Option<String>),
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum InputCommand {
    List(Option<String>),
    Devices,
    PointerAccel {
        identifier: Option<String>,
        value: f64,
    },
    AccelProfile {
        identifier: Option<String>,
        profile: String,
    },
    Tap {
        identifier: Option<String>,
        enabled: bool,
    },
    NaturalScroll {
        identifier: Option<String>,
        enabled: bool,
    },
    ScrollFactor {
        identifier: Option<String>,
        value: f64,
    },
    LeftHanded {
        identifier: Option<String>,
        enabled: bool,
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
    Spawn(String),
    WarpFocus,
    TagMon(MonitorDirection),
    FollowMon(MonitorDirection),
    Layout(LayoutKind),
    Border(Option<u32>),
    SpecialNext(SpecialNext),
    UpdateStatus(String),
    Monitor(MonitorCommand),
    Scratchpad(ScratchpadCommand),
    Keyboard(KeyboardCommand),
    Tag(TagCommand),
    Window(WindowCommand),
    Toggle(ToggleCommand),
    Wallpaper(String),
    Input(InputCommand),
    Mode(ModeCommand),
    Quit,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct GeometryInfo {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl GeometryInfo {
    pub fn from_rect(rect: crate::types::Rect) -> Self {
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

impl WindowState {
    pub fn from_client(c: &crate::types::client::Client) -> Self {
        Self {
            mode: c.mode,
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

impl SizeHintsInfo {
    pub fn from_client(c: &crate::types::client::Client) -> Option<Self> {
        if !c.size_hints_dirty {
            return None;
        }
        let h = &c.size_hints;
        Some(Self {
            min_width: (h.minw > 0).then_some(h.minw),
            min_height: (h.minh > 0).then_some(h.minh),
            max_width: (h.maxw > 0).then_some(h.maxw),
            max_height: (h.maxh > 0).then_some(h.maxh),
            base_width: (h.basew > 0).then_some(h.basew),
            base_height: (h.baseh > 0).then_some(h.baseh),
            width_increment: (h.incw > 0).then_some(h.incw),
            height_increment: (h.inch > 0).then_some(h.inch),
        })
    }
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub protocol: WindowProtocol,
    pub monitor: usize,
    pub tags: TagMask,
    pub geometry: GeometryInfo,
    pub border_width: i32,
    pub state: WindowState,
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
    ) -> Self {
        Self {
            id: c.win.0 as u64,
            title: c.name.clone(),
            protocol,
            monitor: c.monitor_id.index(),
            tags: c.tags & valid_tag_mask,
            geometry: GeometryInfo::from_rect(c.geo),
            border_width: c.border_width,
            state: WindowState::from_client(c),
            scratchpad: ScratchpadInfo::from_client(c),
            size_hints: SizeHintsInfo::from_client(c),
        }
    }
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct MonitorInfo {
    pub id: usize,
    pub index: i32,
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    pub is_primary: bool,
    pub vrr_support: crate::backend::BackendVrrSupport,
    pub vrr_mode: Option<VrrMode>,
    pub vrr_enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MonitorListInfo {
    pub monitors: Vec<MonitorInfo>,
    pub selected: usize,
}

pub use crate::floating::scratchpad::ScratchpadInfo;

/// A single display mode (resolution + refresh rate).
#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct MonitorMode {
    pub width: u32,
    pub height: u32,
    pub refresh_mhz: u32,
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
    pub index: u32,
    pub name: Option<String>,
    pub mask: u32,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct ActionInfo {
    pub name: String,
    pub description: Option<String>,
    pub arg_example: Option<String>,
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
    Status(WmStatusInfo),
    KeyboardLayoutList(Vec<KeyboardLayoutInfo>),
    TagList(Vec<TagInfo>),
    ActionList(Vec<ActionInfo>),
    Message(String),
}

impl Response {
    pub fn ok() -> Self {
        Response::Ok
    }

    pub fn err(msg: impl Into<String>) -> Self {
        Response::Err(msg.into())
    }
}
