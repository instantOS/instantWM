use instantwm::ipc_types::{IpcCommand, IpcRequest, Response};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("connect failed ({socket}): {source}")]
    Connect {
        socket: String,
        source: std::io::Error,
    },
    #[error("instantWM is not running (socket not found: {0})")]
    NotRunning(String),
    #[error("serialization failed: {0}")]
    Encode(#[from] bincode::error::EncodeError),
    #[error("deserialization failed: {0}")]
    Decode(#[from] bincode::error::DecodeError),
    #[error("socket I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Send one request to the running compositor and wait for its response.
pub fn send(command: IpcCommand, ignore_version: bool) -> Result<Response, IpcError> {
    let socket = get_default_socket();
    let mut stream = UnixStream::connect(&socket).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            IpcError::NotRunning(socket.clone())
        } else {
            IpcError::Connect {
                socket: socket.clone(),
                source,
            }
        }
    })?;

    let request = IpcRequest::new(command, ignore_version);
    stream.write_all(&bincode::encode_to_vec(
        &request,
        bincode::config::standard(),
    )?)?;
    let _ = stream.shutdown(std::net::Shutdown::Write);

    let mut data = Vec::new();
    stream.read_to_end(&mut data)?;
    let (response, _) = bincode::decode_from_slice(&data, bincode::config::standard())?;
    Ok(response)
}

pub fn get_default_socket() -> String {
    if let Ok(val) = std::env::var("INSTANTWM_SOCKET") {
        return val;
    }
    format!("/tmp/instantwm-{}.sock", unsafe { libc::geteuid() })
}
