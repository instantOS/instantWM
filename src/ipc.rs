use crate::commands::{command_prefix, set_special_next};
use crate::ipc_types::{IpcCommand, IpcResponse};
use crate::keyboard_layout;
use crate::layouts::command_layout;
use crate::monitor::{focus_monitor, focus_n_mon, move_to_monitor_and_follow};
use crate::scratchpad::{
    scratchpad_hide_name, scratchpad_list, scratchpad_make, scratchpad_show_name,
    scratchpad_status, scratchpad_toggle, scratchpad_unmake,
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
        IpcCommand::WarpFocus => {
            crate::mouse::warp::warp_to_focus(&mut ctx);
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
            alt_tab_free(&mut ctx, action);
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
            send_to_monitor(&mut ctx, direction);
            IpcResponse::ok("")
        }
        IpcCommand::FollowMon(dir) => {
            let direction = MonitorDirection::from(dir);
            move_to_monitor_and_follow(&mut ctx, direction);
            IpcResponse::ok("")
        }
        IpcCommand::FocusMon(dir) => {
            let direction = MonitorDirection::from(dir);
            focus_monitor(&mut ctx, direction);
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
        IpcCommand::ScratchpadList => {
            let list = scratchpad_list(ctx.g());
            IpcResponse::ok(list)
        }
        IpcCommand::ScratchpadToggle(name) => {
            scratchpad_toggle(&mut ctx, name.as_deref());
            IpcResponse::ok("")
        }
        IpcCommand::ScratchpadShow(name) => {
            scratchpad_show_name(&mut ctx, &name);
            IpcResponse::ok("")
        }
        IpcCommand::ScratchpadHide(name) => {
            scratchpad_hide_name(&mut ctx, &name);
            IpcResponse::ok("")
        }
        IpcCommand::ScratchpadStatus(name) => {
            let status = scratchpad_status(ctx.g(), name.as_deref().unwrap_or(""));
            IpcResponse::ok(status)
        }
        IpcCommand::ScratchpadCreate(name) => {
            scratchpad_make(&mut ctx, name.as_deref());
            IpcResponse::ok("")
        }
        IpcCommand::ScratchpadDelete => {
            scratchpad_unmake(&mut ctx);
            IpcResponse::ok("")
        }
        IpcCommand::KeyboardNext => {
            keyboard_layout::cycle_keyboard_layout(&mut ctx, true);
            IpcResponse::ok("")
        }
        IpcCommand::KeyboardPrev => {
            keyboard_layout::cycle_keyboard_layout(&mut ctx, false);
            IpcResponse::ok("")
        }
        IpcCommand::KeyboardStatus => {
            let status = keyboard_layout::keyboard_layout_status(&ctx);
            IpcResponse::ok(status)
        }
        IpcCommand::KeyboardList => {
            let list = keyboard_layout::keyboard_layout_list(&ctx);
            IpcResponse::ok(list)
        }
        IpcCommand::KeyboardListAll => {
            let layouts = keyboard_layout::get_all_keyboard_layouts();
            let list = layouts.join("\n");
            IpcResponse::ok(list)
        }
        IpcCommand::KeyboardSet(layouts) => {
            let globals_layouts: Vec<crate::globals::KeyboardLayout> = layouts
                .into_iter()
                .map(|l| crate::globals::KeyboardLayout {
                    name: l.name,
                    variant: l.variant,
                })
                .collect();
            keyboard_layout::set_keyboard_layouts(&mut ctx, globals_layouts);
            IpcResponse::ok("")
        }
        IpcCommand::KeyboardAdd(layout) => {
            let globals_layout = crate::globals::KeyboardLayout {
                name: layout.name,
                variant: layout.variant,
            };
            match keyboard_layout::add_keyboard_layout(&mut ctx, globals_layout) {
                Ok(()) => IpcResponse::ok(""),
                Err(e) => IpcResponse::err(e),
            }
        }
        IpcCommand::KeyboardRemove(layout) => {
            match keyboard_layout::remove_keyboard_layout(&mut ctx, &layout) {
                Ok(()) => IpcResponse::ok(""),
                Err(e) => IpcResponse::err(e),
            }
        }
        IpcCommand::UpdateStatus(text) => {
            wm.g.status_text = text;

            if let crate::backend::Backend::X11(_) = wm.backend {
                let mut ctx = wm.ctx();
                if let crate::contexts::WmCtx::X11(mut x11_ctx) = ctx {
                    crate::bar::draw_bars_x11(
                        &mut x11_ctx.core,
                        x11_ctx.x11_runtime,
                        x11_ctx.systray.as_deref(),
                    );
                }
            }
            wm.bar.mark_dirty();

            IpcResponse::ok("")
        }
        IpcCommand::RunAction(name) => {
            use crate::config::keybind_config::compile_named_action;
            if let Some(action) = compile_named_action(&name) {
                action(&mut ctx);
                IpcResponse::ok("")
            } else {
                IpcResponse::err(format!("unknown action '{name}'"))
            }
        }
    }
}

/// Information about a single window for JSON output.
#[derive(Debug, serde::Serialize)]
struct WindowInfo {
    id: u64,
    title: String,
    monitor: usize,
    tags: Vec<u32>,
    geometry: GeometryInfo,
    border_width: i32,
    state: WindowState,
    #[serde(skip_serializing_if = "Option::is_none")]
    scratchpad: Option<ScratchpadInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_hints: Option<SizeHintsInfo>,
}

#[derive(Debug, serde::Serialize)]
struct GeometryInfo {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Debug, serde::Serialize)]
struct WindowState {
    floating: bool,
    fullscreen: bool,
    #[serde(rename = "fake_fullscreen")]
    fake_fullscreen: bool,
    sticky: bool,
    hidden: bool,
    urgent: bool,
    locked: bool,
    fixed_size: bool,
    never_focus: bool,
}

#[derive(Debug, serde::Serialize)]
struct ScratchpadInfo {
    name: String,
    #[serde(rename = "restore_tags")]
    restore_tags: Vec<u32>,
}

#[derive(Debug, serde::Serialize)]
struct SizeHintsInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    min_width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_height: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_height: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_height: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width_increment: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height_increment: Option<i32>,
}

/// Root structure for window list JSON output.
#[derive(Debug, serde::Serialize)]
struct WindowList {
    windows: Vec<WindowInfo>,
}

/// Convert a tags bitmask to an array of 1-indexed tag numbers.
fn tags_from_mask(tags_mask: u32, valid_mask: u32) -> Vec<u32> {
    (1..=32)
        .filter(|&t| {
            let tag_bit = 1u32 << (t - 1);
            (tags_mask & tag_bit) != 0 && (valid_mask & tag_bit) != 0
        })
        .collect()
}

/// Build scratchpad info from a client if it's a scratchpad window.
fn build_scratchpad_info(c: &crate::types::client::Client) -> Option<ScratchpadInfo> {
    if !c.is_scratchpad() {
        return None;
    }
    Some(ScratchpadInfo {
        name: c.scratchpad_name.clone(),
        restore_tags: tags_from_mask(c.scratchpad_restore_tags, u32::MAX),
    })
}

/// Build size hints info from a client, only including non-default values.
fn build_size_hints(c: &crate::types::client::Client) -> Option<SizeHintsInfo> {
    if c.size_hints_valid <= 0 {
        return None;
    }
    let h = &c.size_hints;
    Some(SizeHintsInfo {
        min_width: (h.minw > 0).then_some(h.minw),
        min_height: (h.minh > 0).then_some(h.minh),
        max_width: (h.maxw > 0).then_some(h.maxw),
        max_height: (h.maxh > 0).then_some(h.maxh),
        base_width: (h.basew > 0).then_some(h.basew),
        base_height: (h.baseh > 0).then_some(h.baseh),
        width_increment: (h.incw > 0).then_some(h.incw),
        height_increment: (h.inch > 0).then_some(h.inch),
    })
}

/// Build window state info from a client.
fn build_window_state(c: &crate::types::client::Client) -> WindowState {
    WindowState {
        floating: c.is_floating,
        fullscreen: c.is_fullscreen,
        fake_fullscreen: c.isfakefullscreen,
        sticky: c.issticky,
        hidden: c.is_hidden,
        urgent: c.isurgent,
        locked: c.is_locked,
        fixed_size: c.is_fixed_size,
        never_focus: c.never_focus,
    }
}

/// Convert a single client to WindowInfo for JSON output.
fn client_to_window_info(
    c: &crate::types::client::Client,
    valid_tag_mask: u32,
) -> WindowInfo {
    WindowInfo {
        id: c.win.0 as u64,
        title: c.name.clone(),
        monitor: c.monitor_id,
        tags: tags_from_mask(c.tags, valid_tag_mask),
        geometry: GeometryInfo {
            x: c.geo.x,
            y: c.geo.y,
            width: c.geo.w,
            height: c.geo.h,
        },
        border_width: c.border_width,
        state: build_window_state(c),
        scratchpad: build_scratchpad_info(c),
        size_hints: build_size_hints(c),
    }
}

fn list_windows(wm: &Wm) -> IpcResponse {
    let mut wins: Vec<_> = wm.g.clients.values().collect();
    wins.sort_by_key(|c| c.win.0);

    let tag_mask = wm.g.tags.mask();
    let windows: Vec<WindowInfo> = wins
        .iter()
        .map(|c| client_to_window_info(c, tag_mask))
        .collect();

    match serde_json::to_string_pretty(&WindowList { windows }) {
        Ok(json) => IpcResponse::ok(json),
        Err(e) => IpcResponse::err(format!("JSON serialization failed: {}", e)),
    }
}

fn close_window(ctx: &mut crate::contexts::WmCtx, parsed_id: Option<WindowId>) -> IpcResponse {
    let target = parsed_id.or_else(|| ctx.g_mut().selected_win());
    let Some(win) = target else {
        return IpcResponse::err("no target window");
    };
    crate::client::close_win(ctx, win);
    IpcResponse::ok("")
}

/// Geometry information for a single window (JSON output).
#[derive(Debug, serde::Serialize)]
struct WindowGeometryInfo {
    id: u64,
    geometry: GeometryInfo,
}

fn window_geometry(wm: &Wm, parsed_id: Option<WindowId>) -> IpcResponse {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let Some(win) = target else {
        return IpcResponse::err("no target window");
    };
    let Some(c) = wm.g.clients.get(&win) else {
        return IpcResponse::err("window not found");
    };

    let info = WindowGeometryInfo {
        id: c.win.0 as u64,
        geometry: GeometryInfo {
            x: c.geo.x,
            y: c.geo.y,
            width: c.geo.w,
            height: c.geo.h,
        },
    };

    match serde_json::to_string_pretty(&info) {
        Ok(json) => IpcResponse::ok(json),
        Err(e) => IpcResponse::err(format!("JSON serialization failed: {}", e)),
    }
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
