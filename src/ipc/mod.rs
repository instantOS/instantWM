use crate::ipc_types::{IpcCommand, IpcRequest, Response};
use crate::reload::reload_config;
use crate::wm::Wm;
use std::fs;
use std::io::{Read, Write};
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
    clients: Vec<PendingClient>,
}

struct PendingClient {
    stream: UnixStream,
    buffer: Vec<u8>,
}

impl IpcServer {
    pub fn bind() -> std::io::Result<Self> {
        let path = get_available_socket_path();
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        unsafe { std::env::set_var("INSTANTWM_SOCKET", &path) };
        Ok(Self {
            listener,
            path,
            clients: Vec::new(),
        })
    }

    /// Process all pending IPC connections and data. Returns `true` when at least one
    /// command was handled (callers can use this to decide whether to re-render).
    pub fn process_pending(&mut self, wm: &mut Wm) -> bool {
        // 1. Accept new connections
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Ok(()) = stream.set_nonblocking(true) {
                        self.clients.push(PendingClient {
                            stream,
                            buffer: Vec::new(),
                        });
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }

        // 2. Read from existing clients
        let mut handled = false;
        let mut i = 0;
        while i < self.clients.len() {
            let client = &mut self.clients[i];
            let mut chunk = [0u8; 1024];
            match client.stream.read(&mut chunk) {
                Ok(0) => {
                    // Client closed their write half (EOF). Process the request.
                    let client = self.clients.remove(i);
                    if self.process_client_request(client, wm) {
                        handled = true;
                    }
                    // Don't increment i, as we removed the current element.
                }
                Ok(n) => {
                    client.buffer.extend_from_slice(&chunk[..n]);
                    // Limit buffer size to prevent memory exhaustion (e.g. 1MB)
                    if client.buffer.len() > 1024 * 1024 {
                        let mut client = self.clients.remove(i);
                        let _ =
                            send_response(&mut client.stream, &Response::err("request too large"));
                    } else {
                        i += 1;
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    i += 1;
                }
                Err(_) => {
                    self.clients.remove(i);
                }
            }
        }
        handled
    }

    fn process_client_request(&self, mut client: PendingClient, wm: &mut Wm) -> bool {
        if client.buffer.is_empty() {
            let _ = send_response(&mut client.stream, &Response::err("empty request"));
            return false;
        }

        let request: IpcRequest =
            match bincode::decode_from_slice(&client.buffer, bincode::config::standard()) {
                Ok((req, _)) => req,
                Err(_) => {
                    // Try JSON fallback for older/simpler clients if bincode fails
                    match serde_json::from_slice(&client.buffer) {
                        Ok(req) => req,
                        Err(e) => {
                            let _ = send_response(
                                &mut client.stream,
                                &Response::err(format!("deserialize error: {}", e)),
                            );
                            return false;
                        }
                    }
                }
            };

        // Validate protocol version (skip if ignore_version is set)
        if let Err(e) = request.validate_version() {
            let _ = send_response(&mut client.stream, &Response::err(e));
            return false;
        }

        let response = handle_command(wm, request.command);
        let _ = send_response(&mut client.stream, &response);
        true
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

fn get_available_socket_path() -> PathBuf {
    let uid = unsafe { libc::geteuid() };
    let mut i = 0;
    loop {
        let path = if i == 0 {
            PathBuf::from(format!("/tmp/instantwm-{}.sock", uid))
        } else {
            PathBuf::from(format!("/tmp/instantwm-{}-{}.sock", uid, i))
        };

        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                i += 1;
                continue;
            } else {
                let _ = fs::remove_file(&path);
            }
        }
        return path;
    }
}

fn send_response(stream: &mut UnixStream, response: &Response) -> std::io::Result<()> {
    let data = bincode::encode_to_vec(response, bincode::config::standard()).unwrap_or_else(|_| {
        bincode::encode_to_vec(
            Response::err("serialization error"),
            bincode::config::standard(),
        )
        .unwrap()
    });
    stream.write_all(&data)?;
    stream.flush()
}

fn handle_command(wm: &mut Wm, cmd: IpcCommand) -> Response {
    match cmd {
        IpcCommand::Status => general::get_status(wm),
        IpcCommand::Reload => match reload_config(wm) {
            Ok(()) => Response::ok(),
            Err(err) => Response::err(err),
        },
        IpcCommand::RunAction { name, args } => general::run_action(wm, name, args),
        IpcCommand::Spawn(command) => general::spawn_command(wm, command),
        IpcCommand::WarpFocus => general::warp_focus(wm),
        IpcCommand::TagMon(dir) => general::tag_mon(wm, dir),
        IpcCommand::FollowMon(dir) => general::follow_mon(wm, dir),
        IpcCommand::Layout(val) => general::set_layout(wm, val),
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
