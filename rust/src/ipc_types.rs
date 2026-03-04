use bincode::{Decode, Encode};

#[derive(Debug, Clone, Decode, Encode)]
pub enum IpcCommand {
    List,
    Geom(Option<u32>),
    Spawn(String),
    Close(Option<u32>),
    Quit,
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
