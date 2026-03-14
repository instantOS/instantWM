use crate::ipc_types::{IpcCommand, IpcRequest, IpcResponse};
use crate::reload::reload_config;
use crate::wm::Wm;
use std::fs;
use std::io::{BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

pub mod general;
pub mod input;
pub mod keyboard;
pub mod mode;
pub mod monitor;
pub mod scratchpad;
pub mod tag;
pub mod toggle;
pub mod window;

pub struct IpcServer {
    listener: UnixListener,
    path: PathBuf,
}

impl IpcServer {
    pub fn bind() -> std::io::Result<Self> {
        let path = socket_path();
        if path.exists() {
            let _ = fs::remove_file(&path);
        }
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        std::env::set_var("INSTANTWM_SOCKET", &path);
        Ok(Self { listener, path })
    }

    /// Process all pending IPC connections.  Returns `true` when at least one
    /// command was handled (callers can use this to decide whether to re-render).
    pub fn process_pending(&mut self, wm: &mut Wm) -> bool {
        let mut handled = false;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    self.handle_client(stream, wm);
                    handled = true;
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        handled
    }

    fn handle_client(&self, mut stream: UnixStream, wm: &mut Wm) {
        let _ = stream.set_nonblocking(false);
        let mut buffer = Vec::new();
        let mut reader = BufReader::new(&stream);

        loop {
            let mut byte = [0u8; 1];
            match reader.read(&mut byte) {
                Ok(1) => buffer.push(byte[0]),
                Ok(0) => break,
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(_) => {
                    let _ = send_response(&mut stream, &IpcResponse::err("read error"));
                    return;
                }
                _ => break,
            }
        }

        if buffer.is_empty() {
            let _ = send_response(&mut stream, &IpcResponse::err("empty request"));
            return;
        }

        // Attempt to decode just the version string first
        if let Ok((version_str, _)) =
            bincode::decode_from_slice::<String, _>(&buffer, bincode::config::standard())
        {
            if version_str != crate::ipc_types::IPC_PROTOCOL_VERSION {
                let error_msg = format!(
                    "version mismatch: client is {}, server is {}. Please ensure instantwmctl and instantWM are the same version.",
                    version_str, crate::ipc_types::IPC_PROTOCOL_VERSION
                );
                let _ = send_response(&mut stream, &IpcResponse::err(error_msg));
                return;
            }
        }

        let request: IpcRequest =
            match bincode::decode_from_slice(&buffer, bincode::config::standard()) {
                Ok((req, _)) => req,
                Err(e) => {
                    let _ = send_response(
                        &mut stream,
                        &IpcResponse::err(format!("deserialize error: {}", e)),
                    );
                    return;
                }
            };

        // Validate protocol version (fallback just in case)
        if let Err(e) = request.validate_version() {
            let _ = send_response(&mut stream, &IpcResponse::err(e));
            return;
        }

        let response = handle_command(wm, request.command);
        let _ = send_response(&mut stream, &response);
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl std::os::unix::io::AsRawFd for IpcServer {
    fn as_raw_fd(&self) -> std::os::unix::io::RawFd {
        self.listener.as_raw_fd()
    }
}

fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("INSTANTWM_SOCKET") {
        return PathBuf::from(p);
    }
    let uid = unsafe { libc::geteuid() };
    PathBuf::from(format!("/tmp/instantwm-{}.sock", uid))
}

fn send_response(stream: &mut UnixStream, response: &IpcResponse) -> std::io::Result<()> {
    let data = bincode::encode_to_vec(response, bincode::config::standard()).unwrap_or_else(|_| {
        bincode::encode_to_vec(
            &IpcResponse::err("serialization error"),
            bincode::config::standard(),
        )
        .unwrap()
    });
    stream.write_all(&data)?;
    stream.flush()
}

fn handle_command(wm: &mut Wm, cmd: IpcCommand) -> IpcResponse {
    match cmd {
        IpcCommand::Status => general::get_status(wm),
        IpcCommand::Reload => match reload_config(wm) {
            Ok(()) => IpcResponse::ok(""),
            Err(err) => IpcResponse::err(err),
        },
        IpcCommand::RunAction { name, args } => general::run_action(wm, name, args),
        IpcCommand::Spawn(command) => general::spawn_command(wm, command),
        IpcCommand::WarpFocus => general::warp_focus(wm),
        IpcCommand::TagMon(dir) => general::tag_mon(wm, dir),
        IpcCommand::FollowMon(dir) => general::follow_mon(wm, dir),
        IpcCommand::Layout(val) => general::set_layout(wm, val),
        IpcCommand::Prefix(arg) => general::set_prefix(wm, arg),
        IpcCommand::Border(arg) => general::set_border(wm, arg),
        IpcCommand::SpecialNext(arg) => general::set_special_next_cmd(wm, arg),
        IpcCommand::UpdateStatus(text) => general::update_status(wm, text),
        IpcCommand::Monitor(cmd) => monitor::handle_monitor_command(wm, cmd),
        IpcCommand::Window(cmd) => window::handle_window_command(wm, cmd),
        IpcCommand::Tag(cmd) => tag::handle_tag_command(wm, cmd),
        IpcCommand::Scratchpad(cmd) => scratchpad::handle_scratchpad_command(wm, cmd),
        IpcCommand::Keyboard(cmd) => keyboard::handle_keyboard_command(wm, cmd),
        IpcCommand::Toggle(cmd) => toggle::handle_toggle_command(wm, cmd),
        IpcCommand::Input(cmd) => input::handle_input_command(wm, cmd),
        IpcCommand::Mode(cmd) => mode::handle_mode_command(wm, cmd),
        IpcCommand::Wallpaper(path) => general::set_wallpaper(wm, path),
    }
}
