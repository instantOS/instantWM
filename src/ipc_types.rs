pub use crate::layouts::LayoutKind;
pub use crate::types::{MonitorDirection, SpecialNext};
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
        enable: Option<bool>,
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
    Hide(String),
    Status(Option<String>),
    Create(Option<String>),
    Delete,
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
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum TagCommand {
    View(u32),
    Name(String),
    ResetNames,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum WindowCommand {
    Geom(Option<u32>),
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
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct GeometryInfo {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct WindowState {
    pub floating: bool,
    pub fullscreen: bool,
    #[serde(rename = "fake_fullscreen")]
    pub fake_fullscreen: bool,
    pub sticky: bool,
    pub hidden: bool,
    pub urgent: bool,
    pub locked: bool,
    pub fixed_size: bool,
    pub never_focus: bool,
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

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub monitor: usize,
    pub tags: Vec<u32>,
    pub geometry: GeometryInfo,
    pub border_width: i32,
    pub state: WindowState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scratchpad: Option<ScratchpadInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_hints: Option<SizeHintsInfo>,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct WindowGeometryInfo {
    pub id: u64,
    pub geometry: GeometryInfo,
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub struct MonitorInfo {
    pub id: usize,
    pub index: i32,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    pub is_primary: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MonitorListInfo {
    pub monitors: Vec<MonitorInfo>,
    pub selected: usize,
}

pub use crate::floating::scratchpad::ScratchpadInfo;

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
}

#[derive(Debug, Clone, Decode, Encode, serde::Serialize, serde::Deserialize)]
pub enum Response {
    Ok,
    Err(String),
    WindowList(Vec<WindowInfo>),
    WindowGeometry(WindowGeometryInfo),
    MonitorList(Vec<MonitorInfo>),
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
