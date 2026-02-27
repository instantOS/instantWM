use crate::backend::Backend;
use crate::backend::BackendKind;
use crate::backend::BackendOps;
use crate::types::WindowId;
use crate::wm::Wm;
use std::fs;
use std::io::{BufRead, BufReader, Write};
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
        let mut line = String::new();
        let mut reader = BufReader::new(&stream);
        let read_ok = reader.read_line(&mut line).ok().is_some_and(|n| n > 0);
        if !read_ok {
            let _ = stream.write_all(b"ERR empty request\n");
            return;
        }
        let response = handle_command(wm, line.trim());
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("INSTANTWM_SOCKET") {
        return PathBuf::from(p);
    }
    let uid = unsafe { libc::geteuid() };
    PathBuf::from(format!("/tmp/instantwm-{}.sock", uid))
}

fn handle_command(wm: &mut Wm, cmd: &str) -> String {
    if cmd.eq_ignore_ascii_case("list") {
        return list_windows(wm);
    }
    if cmd.eq_ignore_ascii_case("geom") || cmd.starts_with("geom ") {
        let parsed_id = cmd
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u32>().ok())
            .map(WindowId::from);
        return window_geometry(wm, parsed_id);
    }
    if let Some(rest) = cmd.strip_prefix("spawn ") {
        if rest.trim().is_empty() {
            return "ERR spawn requires a command\n".to_string();
        }
        match std::process::Command::new("sh").arg("-c").arg(rest).spawn() {
            Ok(child) => return format!("OK pid={}\n", child.id()),
            Err(err) => return format!("ERR spawn failed: {}\n", err),
        }
    }
    if cmd.eq_ignore_ascii_case("close") || cmd.starts_with("close ") {
        let parsed_id = cmd
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse::<u32>().ok())
            .map(WindowId::from);
        return close_window(wm, parsed_id);
    }
    "ERR unknown command\n".to_string()
}

fn list_windows(wm: &Wm) -> String {
    let mut wins: Vec<_> = wm.g.clients.values().collect();
    wins.sort_by_key(|c| c.win.0);
    let mut out = String::from("OK\n");
    for c in wins {
        let name = c.name.replace('\n', " ").replace('\t', " ");
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\n",
            c.win.0,
            c.mon_id.unwrap_or(0),
            c.isfloating as u8,
            c.is_fullscreen as u8,
            name
        ));
    }
    out
}

fn close_window(wm: &mut Wm, parsed_id: Option<WindowId>) -> String {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let Some(win) = target else {
        return "ERR no target window\n".to_string();
    };
    match wm.backend.kind() {
        BackendKind::X11 => {
            let mut ctx = wm.ctx();
            crate::client::close_win(&mut ctx, win);
            "OK\n".to_string()
        }
        BackendKind::Wayland => match &wm.backend {
            Backend::Wayland(backend) if backend.close_window(win) => "OK\n".to_string(),
            Backend::Wayland(_) => "ERR window not found\n".to_string(),
            Backend::X11(_) => "ERR backend mismatch\n".to_string(),
        },
    }
}

fn window_geometry(wm: &Wm, parsed_id: Option<WindowId>) -> String {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let Some(win) = target else {
        return "ERR no target window\n".to_string();
    };
    let Some(c) = wm.g.clients.get(&win) else {
        return "ERR window not found\n".to_string();
    };
    format!(
        "OK\n{}\t{}\t{}\t{}\t{}\n",
        c.win.0, c.geo.x, c.geo.y, c.geo.w, c.geo.h
    )
}
