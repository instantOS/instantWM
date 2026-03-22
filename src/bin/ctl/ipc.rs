use instantwm::ipc_types::{IPC_PROTOCOL_VERSION, IpcCommand, IpcRequest, Response};
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

    /// Try to fetch the server's version via a Status command to produce a
    /// helpful version-mismatch message.  Returns `None` when the check itself
    /// fails (the server might be unreachable by now).
    pub fn check_version(socket_path: &str) -> Option<String> {
        let mut client = IpcClient::connect(socket_path).ok()?;
        // Status is the most stable command – send with ignore_version so the
        // server doesn't reject it for the same mismatch we're diagnosing.
        let response = client.send(IpcCommand::Status, true).ok()?;
        if let Response::Status(info) = response {
            let client_version = IPC_PROTOCOL_VERSION;
            if info.protocol_version != client_version {
                return Some(format!(
                    "version mismatch: client is {}, server is {}. Please ensure instantwmctl and instantWM are the same version.",
                    client_version, info.protocol_version
                ));
            }
        }
        None
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
    if let Ok(val) = std::env::var("INSTANTWM_SOCKET") {
        return val;
    }
    format!("/tmp/instantwm-{}.sock", unsafe { libc::geteuid() })
}
