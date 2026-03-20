use instantwm::ipc_types::{IpcCommand, IpcRequest, Response};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;

pub struct IpcClient {
    stream: UnixStream,
}

impl IpcClient {
    pub fn connect(socket_path: &str) -> Result<Self, std::io::Error> {
        let stream = UnixStream::connect(socket_path)?;
        Ok(Self { stream })
    }

    pub fn send(
        &mut self,
        command: IpcCommand,
        ignore_version_mismatches: bool,
    ) -> Result<Response, IpcError> {
        let request = if ignore_version_mismatches {
            IpcRequest::new_ignore_version(command, true)
        } else {
            IpcRequest::new(command)
        };

        let data = bincode::encode_to_vec(&request, bincode::config::standard())
            .map_err(|e| IpcError::Serialize(e.to_string()))?;

        self.stream
            .write_all(&data)
            .map_err(|e| IpcError::Write(e.to_string()))?;
        let _ = self.stream.shutdown(std::net::Shutdown::Write);

        let mut data = Vec::new();
        self.stream
            .read_to_end(&mut data)
            .map_err(|e| IpcError::Read(e.to_string()))?;

        let (response, _) = bincode::decode_from_slice(&data, bincode::config::standard())
            .map_err(|e| IpcError::Deserialize(e.to_string()))?;
        Ok(response)
    }
}

#[derive(Debug)]
pub enum IpcError {
    Serialize(String),
    Write(String),
    Read(String),
    Deserialize(String),
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::Serialize(s) => write!(f, "serialization failed: {}", s),
            IpcError::Write(s) => write!(f, "write failed: {}", s),
            IpcError::Read(s) => write!(f, "read failed: {}", s),
            IpcError::Deserialize(s) => write!(f, "deserialization failed: {}", s),
        }
    }
}

impl std::error::Error for IpcError {}

pub fn get_default_socket() -> String {
    format!("/tmp/instantwm-{}.sock", unsafe { libc::geteuid() })
}
