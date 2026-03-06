use crate::commands::{command_prefix, set_special_next};
use crate::ipc_types::{IpcCommand, IpcResponse};
use crate::layouts::command_layout;
use crate::monitor::{focus_mon, focus_n_mon, follow_mon};
use crate::mouse::warp::warp_to_focus_x11;
use crate::overlay::set_overlay;
use crate::scratchpad::{
    scratchpad_hide_name, scratchpad_make, scratchpad_show_name, scratchpad_status,
    scratchpad_toggle, scratchpad_unmake,
};
use crate::tags::send_to_monitor;
use crate::tags::{name_tag, reset_name_tag};
use crate::toggles::{
    alt_tab_free, set_border_width, toggle_alt_tag, toggle_animated,
    toggle_focus_follows_float_mouse, toggle_focus_follows_mouse, toggle_show_tags,
};
use crate::types::MonitorDirection;
use crate::types::TagMask;
use crate::types::ToggleAction;
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
    let mut ctx = wm.ctx();

    match cmd {
        IpcCommand::List => list_windows(wm),
        IpcCommand::Geom(window_id) => window_geometry(wm, window_id.map(WindowId::from)),
        IpcCommand::Spawn(command) => spawn_command(&mut ctx, command),
        IpcCommand::Close(window_id) => close_window(&mut ctx, window_id.map(WindowId::from)),
        IpcCommand::Quit => {
            wm.quit();
            IpcResponse::ok("")
        }
        IpcCommand::Overlay => {
            set_overlay(&mut ctx);
            IpcResponse::ok("")
        }
        IpcCommand::WarpFocus => {
            if let crate::contexts::WmCtx::X11(x11) = &mut ctx {
                warp_to_focus_x11(&x11.core, &x11.x11, x11.x11_runtime);
            }
            IpcResponse::ok("")
        }
        IpcCommand::Tag(tag_num) => {
            let tag = if tag_num == 0 { 2 } else { tag_num };
            if let Some(mask) = TagMask::single(tag as usize) {
                crate::tags::view::view(&mut ctx, mask);
            }
            IpcResponse::ok("")
        }
        IpcCommand::Animated(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_animated(ctx.core_mut(), action);
            IpcResponse::ok("")
        }
        IpcCommand::FocusFollowsMouse(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_focus_follows_mouse(ctx.core_mut(), action);
            IpcResponse::ok("")
        }
        IpcCommand::FocusFollowsFloatMouse(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_focus_follows_float_mouse(ctx.core_mut(), action);
            IpcResponse::ok("")
        }
        IpcCommand::AltTab(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            if let crate::contexts::WmCtx::X11(x11) = &mut ctx {
                alt_tab_free(&mut x11.core, &x11.x11, x11.x11_runtime, action);
            }
            IpcResponse::ok("")
        }
        IpcCommand::AltTag(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_alt_tag(&mut ctx, action);
            IpcResponse::ok("")
        }
        IpcCommand::HideTags(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_show_tags(&mut ctx, action);
            IpcResponse::ok("")
        }
        IpcCommand::Layout(val) => {
            command_layout(&mut ctx, val);
            IpcResponse::ok("")
        }
        IpcCommand::Prefix(arg) => {
            let val = arg.unwrap_or(1);
            command_prefix(&mut ctx, val);
            IpcResponse::ok("")
        }
        IpcCommand::Border(arg) => {
            let val = arg.unwrap_or(crate::config::mod_consts::BORDERPX as u32);
            if let Some(win) = ctx.selected_client() {
                set_border_width(ctx.core_mut(), win, val as i32);
            }
            IpcResponse::ok("")
        }
        IpcCommand::SpecialNext(arg) => {
            let val = arg.unwrap_or(0);
            set_special_next(ctx.core_mut(), val);
            IpcResponse::ok("")
        }
        IpcCommand::TagMon(dir) => {
            let direction = MonitorDirection::from(dir);
            if let crate::contexts::WmCtx::X11(ref mut ctx_x11) = ctx {
                send_to_monitor(ctx_x11, direction);
            }
            IpcResponse::ok("")
        }
        IpcCommand::FollowMon(dir) => {
            let direction = MonitorDirection::from(dir);
            follow_mon(&mut ctx, direction);
            IpcResponse::ok("")
        }
        IpcCommand::FocusMon(dir) => {
            let direction = MonitorDirection::from(dir);
            focus_mon(&mut ctx, direction);
            IpcResponse::ok("")
        }
        IpcCommand::FocusNMon(val) => {
            focus_n_mon(&mut ctx, val);
            IpcResponse::ok("")
        }
        IpcCommand::NameTag(name) => {
            name_tag(&mut ctx, &name);
            IpcResponse::ok("")
        }
        IpcCommand::ResetNameTag => {
            reset_name_tag(&mut ctx);
            IpcResponse::ok("")
        }
        IpcCommand::ScratchpadMake(name) => {
            scratchpad_make(&mut ctx, name.as_deref());
            IpcResponse::ok("")
        }
        IpcCommand::ScratchpadUnmake => {
            scratchpad_unmake(&mut ctx);
            IpcResponse::ok("")
        }
        IpcCommand::ScratchpadToggle(name) => {
            scratchpad_toggle(&mut ctx, name.as_deref());
            IpcResponse::ok("")
        }
        IpcCommand::ScratchpadShow(name) => {
            scratchpad_show_name(&mut ctx, name.as_deref().unwrap_or(""));
            IpcResponse::ok("")
        }
        IpcCommand::ScratchpadHide(name) => {
            scratchpad_hide_name(&mut ctx, name.as_deref().unwrap_or(""));
            IpcResponse::ok("")
        }
        IpcCommand::ScratchpadStatus(name) => {
            let status = scratchpad_status(&ctx, name.as_deref().unwrap_or(""));
            IpcResponse::ok(status)
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

fn close_window(ctx: &mut crate::contexts::WmCtx, parsed_id: Option<WindowId>) -> IpcResponse {
    let target = parsed_id.or_else(|| ctx.g_mut().selected_win());
    let Some(win) = target else {
        return IpcResponse::err("no target window");
    };
    crate::client::close_win(ctx, win);
    IpcResponse::ok("")
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

fn spawn_command(ctx: &mut crate::contexts::WmCtx, command: String) -> IpcResponse {
    if command.trim().is_empty() {
        return IpcResponse::err("spawn requires a command");
    }
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(&command);
    if ctx.is_wayland() {
        if let crate::backend::BackendRef::Wayland(wayland) = ctx.backend() {
            if let Some(display) = wayland.xdisplay() {
                cmd.env("DISPLAY", format!(":{display}"));
            }
        }
    }
    match cmd.spawn() {
        Ok(child) => IpcResponse::ok(format!("pid={}", child.id())),
        Err(err) => IpcResponse::err(format!("spawn failed: {}", err)),
    }
}
