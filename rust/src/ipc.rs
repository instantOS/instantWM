use crate::backend::Backend;
use crate::backend::BackendKind;
use crate::backend::BackendOps;
use crate::ipc_types::{IpcCommand, IpcResponse};
use crate::types::WindowId;
use crate::wm::Wm;
use std::fs;
use std::io::{BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

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

    pub fn process_pending(&mut self, wm: &mut Wm) {
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    self.handle_client(stream, wm);
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }

    fn handle_client(&self, mut stream: UnixStream, wm: &mut Wm) {
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

        let cmd: IpcCommand = match bincode::decode_from_slice(&buffer, bincode::config::standard())
        {
            Ok((cmd, _)) => cmd,
            Err(e) => {
                let _ = send_response(
                    &mut stream,
                    &IpcResponse::err(format!("deserialize error: {}", e)),
                );
                return;
            }
        };

        let response = handle_command(wm, cmd);
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
        IpcCommand::List => list_windows(wm),
        IpcCommand::Geom(window_id) => window_geometry(wm, window_id.map(WindowId::from)),
        IpcCommand::Spawn(command) => spawn_command(wm, command),
        IpcCommand::Close(window_id) => close_window(wm, window_id.map(WindowId::from)),
        IpcCommand::Quit => {
            wm.quit();
            IpcResponse::ok("")
        }
    }
}

fn list_windows(wm: &Wm) -> IpcResponse {
    let mut wins: Vec<_> = wm.g.clients.values().collect();
    wins.sort_by_key(|c| c.win.0);
    let mut out = String::new();
    for c in wins {
        let name = c.name.replace('\n', " ").replace('\t', " ");
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            c.win.0,
            c.monitor_id.unwrap_or(0),
            c.isfloating as u8,
            c.is_fullscreen as u8,
            name
        ));
    }
    IpcResponse::ok(out)
}

fn close_window(wm: &mut Wm, parsed_id: Option<WindowId>) -> IpcResponse {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let Some(win) = target else {
        return IpcResponse::err("no target window");
    };
    match wm.backend.kind() {
        BackendKind::X11 => {
            let mut ctx = wm.ctx();
            crate::client::close_win(&mut ctx, win);
            IpcResponse::ok("")
        }
        BackendKind::Wayland => match &wm.backend {
            Backend::Wayland(backend) if backend.close_window(win) => IpcResponse::ok(""),
            Backend::Wayland(_) => IpcResponse::err("window not found"),
            Backend::X11(_) => IpcResponse::err("backend mismatch"),
        },
    }
}

fn window_geometry(wm: &Wm, parsed_id: Option<WindowId>) -> IpcResponse {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let Some(win) = target else {
        return IpcResponse::err("no target window");
    };
    let Some(c) = wm.g.clients.get(&win) else {
        return IpcResponse::err("window not found");
    };
    IpcResponse::ok(format!(
        "{}\t{}\t{}\t{}\t{}",
        c.win.0, c.geo.x, c.geo.y, c.geo.w, c.geo.h
    ))
}

fn spawn_command(wm: &mut Wm, command: String) -> IpcResponse {
    if command.trim().is_empty() {
        return IpcResponse::err("spawn requires a command");
    }
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(&command);
    if wm.backend.kind() == BackendKind::Wayland {
        match &wm.backend {
            Backend::Wayland(backend) => {
                if let Some(display) = backend.xdisplay() {
                    cmd.env("DISPLAY", format!(":{display}"));
                } else {
                    return IpcResponse::err("XWayland not ready (DISPLAY unavailable)");
                }
            }
            Backend::X11(_) => {}
        }
    }
    match cmd.spawn() {
        Ok(child) => IpcResponse::ok(format!("pid={}", child.id())),
        Err(err) => IpcResponse::err(format!("spawn failed: {}", err)),
    }
}
