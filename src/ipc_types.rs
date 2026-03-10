use bincode::{Decode, Encode};

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
    /// Set keyboard layout by index (0-based).
    KeyboardLayout(u32),
    /// Set keyboard layout by name.
    KeyboardLayoutName(String),
    /// Cycle keyboard layout forward (true) or backward (false).
    CycleKeyboardLayout(bool),
    /// Get current keyboard layout info.
    GetKeyboardLayout,
    /// List all configured keyboard layouts.
    ListKeyboardLayouts,
    /// Replace the configured keyboard layouts at runtime.
    /// (layouts, variants)
    SetKeyboardLayouts(Vec<String>, Vec<String>),
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
