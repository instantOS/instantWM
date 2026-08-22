use crate::ipc_types::{IpcCommand, IpcRequest, Response};
use crate::reload::reload_config;
use crate::wm::Wm;
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

const MAX_PENDING_CLIENTS: usize = 128;
const PENDING_CLIENT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

pub mod config;
pub mod general;
pub mod input;
pub mod keybinds;
pub mod keyboard;
pub mod layout;
pub mod mode;
pub mod monitor;
pub mod pending_tmp_rule;
pub mod scratchpad;
pub mod tag;
pub mod test;
pub mod theme;
pub mod window;

pub struct IpcServer {
    listener: UnixListener,
    path: PathBuf,
    clients: Vec<PendingClient>,
}

struct PendingClient {
    stream: UnixStream,
    buffer: Vec<u8>,
    last_activity: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingRead {
    Pending,
    Complete,
    RequestTooLarge,
    Failed,
}

/// Drain every byte currently available from one nonblocking IPC client.
///
/// `instantwmctl` writes the complete request and closes its write half before
/// waiting for a response. Reading through EOF in one event-loop tick is
/// therefore part of the IPC scheduling contract: stopping after the first
/// successful `read` would require an unrelated display event to wake the loop
/// again, which is especially visible on an otherwise-idle X11 session.
fn drain_pending_client(client: &mut PendingClient) -> PendingRead {
    let mut chunk = [0u8; 1024];
    loop {
        match client.stream.read(&mut chunk) {
            Ok(0) => return PendingRead::Complete,
            Ok(n) => {
                client.last_activity = Instant::now();
                client.buffer.extend_from_slice(&chunk[..n]);
                if client.buffer.len() > 1024 * 1024 {
                    return PendingRead::RequestTooLarge;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                return PendingRead::Pending;
            }
            Err(_) => return PendingRead::Failed,
        }
    }
}

fn prune_idle_clients(clients: &mut Vec<PendingClient>, now: Instant) {
    clients
        .retain(|client| now.duration_since(client.last_activity) <= PENDING_CLIENT_IDLE_TIMEOUT);
}

impl IpcServer {
    pub fn bind() -> io::Result<Self> {
        let path = get_available_socket_path();
        // The bind override is a launch-time input, not session state that
        // should leak into clients spawned by this compositor.
        unsafe { env::remove_var("INSTANTWM_SOCKET_BIND") };
        let listener = UnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        unsafe { env::set_var("INSTANTWM_SOCKET", &path) };
        Ok(Self {
            listener,
            path,
            clients: Vec::new(),
        })
    }

    /// Process all pending IPC connections and data. Returns `true` when at least one
    /// command was handled (callers can use this to decide whether to re-render).
    pub fn process_pending(&mut self, wm: &mut Wm) -> bool {
        let now = Instant::now();
        prune_idle_clients(&mut self.clients, now);

        // 1. Accept new connections
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if self.clients.len() < MAX_PENDING_CLIENTS
                        && let Ok(()) = stream.set_nonblocking(true)
                    {
                        self.clients.push(PendingClient {
                            stream,
                            buffer: Vec::new(),
                            last_activity: now,
                        });
                    }
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }

        // 2. Read from existing clients
        let mut handled = false;
        let mut i = 0;
        while i < self.clients.len() {
            match drain_pending_client(&mut self.clients[i]) {
                PendingRead::Complete => {
                    // Client closed their write half (EOF). Process the request.
                    let client = self.clients.remove(i);
                    if self.process_client_request(client, wm) {
                        handled = true;
                    }
                    // Don't increment i, as we removed the current element.
                }
                PendingRead::Pending => i += 1,
                PendingRead::RequestTooLarge => {
                    let mut client = self.clients.remove(i);
                    let _ = send_response(&mut client.stream, &Response::err("request too large"));
                }
                PendingRead::Failed => {
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
    // Tests and nested sessions need an exact socket so their controller
    // cannot accidentally connect to another compositor while this process
    // silently falls back to a suffixed path.
    if let Some(path) = env::var_os("INSTANTWM_SOCKET_BIND").filter(|path| !path.is_empty()) {
        let path = PathBuf::from(path);
        if path.exists() && UnixStream::connect(&path).is_err() {
            let _ = fs::remove_file(&path);
        }
        return path;
    }

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

/// Upper bound on time spent waiting for a client to drain its socket while a
/// response is written. Streams are nonblocking so a stuck client can never
/// wedge the compositor event loop; this budget only rides out transient
/// backpressure before the response is dropped (and logged).
const RESPONSE_WRITE_MAX_STALL: Duration = Duration::from_millis(20);

fn send_response(stream: &mut UnixStream, response: &Response) -> io::Result<()> {
    let data = bincode::encode_to_vec(response, bincode::config::standard()).unwrap_or_else(|_| {
        bincode::encode_to_vec(
            Response::err("serialization error"),
            bincode::config::standard(),
        )
        .unwrap()
    });
    match write_all_bounded(stream, &data) {
        Ok(()) => stream.flush(),
        Err(err) => {
            log::warn!("dropping IPC response after write stall: {err}");
            Err(err)
        }
    }
}

/// `write_all` for a nonblocking stream that tolerates transient
/// `WouldBlock` backpressure up to [`RESPONSE_WRITE_MAX_STALL`] before giving
/// up, so oversized responses survive slow-but-live readers without ever
/// blocking the loop indefinitely.
fn write_all_bounded(stream: &mut UnixStream, mut data: &[u8]) -> io::Result<()> {
    let deadline = Instant::now() + RESPONSE_WRITE_MAX_STALL;
    while !data.is_empty() {
        match stream.write(data) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "client socket accepted no bytes",
                ));
            }
            Ok(n) => data = &data[n..],
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "client stopped reading its socket",
                    ));
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

fn handle_command(wm: &mut Wm, cmd: IpcCommand) -> Response {
    if let Some(exit_mode) = ipc_overview_exit(&cmd) {
        crate::overview::exit_overview(&mut wm.ctx(), exit_mode);
    }
    match cmd {
        IpcCommand::Status => general::get_status(wm),
        IpcCommand::Reload => match reload_config(wm) {
            Ok(()) => Response::ok(),
            Err(err) => Response::err(err),
        },
        IpcCommand::RunAction { name, args } => general::run_action(wm, name, args),
        IpcCommand::PendingTmpRule(cmd) => pending_tmp_rule::handle_pending_tmp_rule(wm, cmd),
        IpcCommand::UpdateStatus(text) => general::update_status(wm, text),
        IpcCommand::Monitor(cmd) => monitor::handle_monitor_command(wm, cmd),
        IpcCommand::Window(cmd) => window::handle_window_command(wm, cmd),
        IpcCommand::Tag(cmd) => tag::handle_tag_command(wm, cmd),
        IpcCommand::Scratchpad(cmd) => scratchpad::handle_scratchpad_command(wm, cmd),
        IpcCommand::Keyboard(cmd) => keyboard::handle_keyboard_command(wm, cmd),
        IpcCommand::Input(cmd) => input::handle_input_command(wm, cmd),
        IpcCommand::LayoutList => layout::list_layouts(wm),
        IpcCommand::LayoutStatus => layout::layout_status(wm),
        IpcCommand::ListKeybinds => keybinds::list_keybinds(wm),
        IpcCommand::ListModes => mode::list_modes(wm),
        IpcCommand::Wallpaper(path) => general::set_wallpaper(wm, path),
        IpcCommand::Config(cmd) => config::handle_config_command(wm, cmd),
        IpcCommand::Test(cmd) => test::handle_test_command(wm, cmd),
        IpcCommand::GetTheme => theme::get_theme(wm),
        IpcCommand::SetTheme(theme) => theme::set_theme(wm, theme),
        IpcCommand::ListThemes => theme::list_themes(),
    }
}

/// Overview-exit policy for IPC commands that bypass the key/button action
/// machinery. Everything routed through [`crate::actions`] instead follows the
/// Preserve/Cancel/Confirm policy in [`crate::overview::prepare_key_action`]
/// (and its button/named counterparts) — including `RunAction`, which returns
/// `None` here precisely so it delegates to that machinery.
///
/// Keep the two policies aligned: workspace navigation cancels a projection,
/// mutations confirm it, cosmetic queries preserve it. The match below must
/// stay exhaustive over `IpcCommand` so new commands force an explicit decision.
fn ipc_overview_exit(cmd: &IpcCommand) -> Option<crate::overview::ExitMode> {
    use crate::ipc_types::{MonitorCommand, ScratchpadCommand, WindowCommand};
    use crate::overview::ExitMode::{RestorePrevious, ToSelectedWindow};

    match cmd {
        IpcCommand::Monitor(
            MonitorCommand::Switch { .. }
            | MonitorCommand::Next { .. }
            | MonitorCommand::Prev { .. }
            | MonitorCommand::Set { .. },
        ) => Some(RestorePrevious),
        IpcCommand::Window(WindowCommand::Resize { .. } | WindowCommand::Close(None))
        | IpcCommand::Scratchpad(
            ScratchpadCommand::Create { .. }
            | ScratchpadCommand::Restore {
                name: None,
                window_id: None,
            },
        ) => Some(ToSelectedWindow),

        IpcCommand::Status
        | IpcCommand::Reload
        | IpcCommand::RunAction { .. }
        | IpcCommand::UpdateStatus(_)
        | IpcCommand::Monitor(MonitorCommand::List | MonitorCommand::Modes { .. })
        | IpcCommand::Scratchpad(
            ScratchpadCommand::List
            | ScratchpadCommand::Toggle(_)
            | ScratchpadCommand::Show(_)
            | ScratchpadCommand::ShowAll
            | ScratchpadCommand::Hide(_)
            | ScratchpadCommand::HideAll
            | ScratchpadCommand::Status(_)
            | ScratchpadCommand::Resize { .. }
            | ScratchpadCommand::Restore { name: Some(_), .. }
            | ScratchpadCommand::Restore {
                window_id: Some(_), ..
            },
        )
        | IpcCommand::Keyboard(_)
        | IpcCommand::Tag(_)
        | IpcCommand::Window(
            WindowCommand::Info(_) | WindowCommand::Close(Some(_)) | WindowCommand::List(_),
        )
        | IpcCommand::Wallpaper(_)
        | IpcCommand::Input(_)
        | IpcCommand::LayoutList
        | IpcCommand::LayoutStatus
        | IpcCommand::ListModes
        | IpcCommand::Config(_)
        | IpcCommand::Test(_)
        | IpcCommand::PendingTmpRule(_)
        | IpcCommand::GetTheme
        | IpcCommand::SetTheme(_)
        | IpcCommand::ListThemes
        | IpcCommand::ListKeybinds => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use std::io::Write;
    use std::net::Shutdown;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn idle_pending_clients_are_pruned_without_dropping_active_clients() {
        let now = Instant::now();
        let (stale_stream, _stale_peer) = UnixStream::pair().unwrap();
        let (active_stream, _active_peer) = UnixStream::pair().unwrap();
        let mut clients = vec![
            PendingClient {
                stream: stale_stream,
                buffer: Vec::new(),
                last_activity: now - PENDING_CLIENT_IDLE_TIMEOUT - Duration::from_millis(1),
            },
            PendingClient {
                stream: active_stream,
                buffer: Vec::new(),
                last_activity: now,
            },
        ];

        prune_idle_clients(&mut clients, now);

        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].last_activity, now);
    }

    #[test]
    fn complete_client_request_is_drained_through_eof_in_one_tick() {
        let (server_stream, mut client_stream) = UnixStream::pair().unwrap();
        server_stream.set_nonblocking(true).unwrap();
        let request = vec![0x5a; 4096];
        client_stream.write_all(&request).unwrap();
        client_stream.shutdown(Shutdown::Write).unwrap();
        let mut client = PendingClient {
            stream: server_stream,
            buffer: Vec::new(),
            last_activity: Instant::now(),
        };

        assert_eq!(drain_pending_client(&mut client), PendingRead::Complete);
        assert_eq!(client.buffer, request);
    }

    #[test]
    fn one_server_tick_accepts_and_handles_a_complete_request() {
        static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);
        let socket_path = std::env::temp_dir().join(format!(
            "instantwm-ipc-test-{}-{}.sock",
            std::process::id(),
            NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
        ));
        let listener = UnixListener::bind(&socket_path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut server = IpcServer {
            listener,
            path: socket_path,
            clients: Vec::new(),
        };
        let mut stream = UnixStream::connect(&server.path).unwrap();
        let request = IpcRequest::new(IpcCommand::GetTheme);
        let bytes = bincode::encode_to_vec(&request, bincode::config::standard()).unwrap();
        stream.write_all(&bytes).unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));

        assert!(server.process_pending(&mut wm));
        assert!(server.clients.is_empty());
    }
}
