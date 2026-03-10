use bincode::{Decode, Encode};

/// A single keyboard layout with optional variant (used for IPC).
#[derive(Debug, Clone, Decode, Encode)]
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
        if let Some((name, variant)) = s.strip_suffix(')').and_then(|s| s.rsplit_once('('))
        {
            Self::with_variant(name, variant)
        } else {
            Self::new(s)
        }
    }
}

impl From< crate::globals::KeyboardLayout> for KeyboardLayout {
    fn from(l: crate::globals::KeyboardLayout) -> Self {
        Self {
            name: l.name,
            variant: l.variant,
        }
    }
}

#[derive(Debug, Clone, Decode, Encode)]
pub enum IpcCommand {
    List,
    Geom(Option<u32>),
    RunAction(String),
    Spawn(String),
    Close(Option<u32>),
    WarpFocus,
    Tag(u32),
    Animated(Option<String>),
    FocusFollowsMouse(Option<String>),
    FocusFollowsFloatMouse(Option<String>),
    AltTab(Option<String>),
    AltTag(Option<String>),
    HideTags(Option<String>),
    Layout(u32),
    Prefix(Option<u32>),
    Border(Option<u32>),
    SpecialNext(Option<u32>),
    TagMon(i32),
    FollowMon(i32),
    FocusMon(i32),
    FocusNMon(i32),
    NameTag(String),
    ResetNameTag,
    ScratchpadMake(Option<String>),
    ScratchpadUnmake,
    ScratchpadToggle(Option<String>),
    ScratchpadShow(Option<String>),
    ScratchpadHide(Option<String>),
    ScratchpadStatus(Option<String>),
    /// Switch to the next keyboard layout.
    KeyboardNext,
    /// Switch to the previous keyboard layout.
    KeyboardPrev,
    /// Get current keyboard layout status (e.g., "2/3: us (nodeadkeys)").
    KeyboardStatus,
    /// List configured keyboard layouts (with * marker for current).
    KeyboardList,
    /// List all available XKB layouts from the system.
    KeyboardListAll,
    /// Set keyboard layouts (replaces all, switches to first).
    KeyboardSet(Vec<KeyboardLayout>),
    /// Add a keyboard layout to the active list.
    KeyboardAdd(KeyboardLayout),
    /// Remove a keyboard layout from the active list (by name or #index).
    KeyboardRemove(String),
    /// Update status text via IPC.
    UpdateStatus(String),
}

#[derive(Debug, Clone, Decode, Encode)]
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
