use bincode::{Decode, Encode};

#[derive(Debug, Clone, Decode, Encode)]
pub enum IpcCommand {
    List,
    Geom(Option<u32>),
    Spawn(String),
    Close(Option<u32>),
    Quit,
    Overlay,
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
