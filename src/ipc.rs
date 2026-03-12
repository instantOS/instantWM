use crate::commands::{command_prefix, set_special_next};
use crate::ipc_types::{
    InputCommand, IpcCommand, IpcRequest, IpcResponse, KeyboardCommand, ModeCommand,
    MonitorCommand, ScratchpadCommand, TagCommand, ToggleCommand, WindowCommand,
};
use crate::keyboard_layout;
use crate::layouts::command_layout;
use crate::monitor::{focus_monitor, focus_n_mon, move_to_monitor_and_follow};
use crate::reload::reload_config;
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

        // Validate protocol version
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
        IpcCommand::Status => get_status(wm),
        IpcCommand::Reload => match reload_config(wm) {
            Ok(()) => IpcResponse::ok(""),
            Err(err) => IpcResponse::err(err),
        },
        IpcCommand::RunAction { name, args } => run_action(wm, name, args),
        IpcCommand::Spawn(command) => spawn_command(wm, command),
        IpcCommand::WarpFocus => warp_focus(wm),
        IpcCommand::TagMon(dir) => tag_mon(wm, dir),
        IpcCommand::FollowMon(dir) => follow_mon(wm, dir),
        IpcCommand::Layout(val) => set_layout(wm, val),
        IpcCommand::Prefix(arg) => set_prefix(wm, arg),
        IpcCommand::Border(arg) => set_border(wm, arg),
        IpcCommand::SpecialNext(arg) => set_special_next_cmd(wm, arg),
        IpcCommand::UpdateStatus(text) => update_status(wm, text),
        IpcCommand::Monitor(cmd) => handle_monitor_command(wm, cmd),
        IpcCommand::Window(cmd) => handle_window_command(wm, cmd),
        IpcCommand::Tag(cmd) => handle_tag_command(wm, cmd),
        IpcCommand::Scratchpad(cmd) => handle_scratchpad_command(wm, cmd),
        IpcCommand::Keyboard(cmd) => handle_keyboard_command(wm, cmd),
        IpcCommand::Toggle(cmd) => handle_toggle_command(wm, cmd),
        IpcCommand::Input(cmd) => handle_input_command(wm, cmd),
        IpcCommand::Mode(cmd) => handle_mode_command(wm, cmd),
    }
}

// ============================================================================
// Monitor Commands
// ============================================================================

fn handle_monitor_command(wm: &mut Wm, cmd: MonitorCommand) -> IpcResponse {
    match cmd {
        MonitorCommand::List => list_monitors(wm),
        MonitorCommand::Switch { index } => switch_monitor(wm, index as i32),
        MonitorCommand::Next { count } => next_monitor(wm, count as i32),
        MonitorCommand::Prev { count } => prev_monitor(wm, count as i32),
        MonitorCommand::Set {
            identifier,
            resolution,
            refresh_rate,
            position,
            scale,
            enable,
        } => set_monitor_config(
            wm,
            identifier,
            resolution,
            refresh_rate,
            position,
            scale,
            enable,
        ),
    }
}

/// Information about a single monitor for JSON output.
#[derive(Debug, serde::Serialize)]
struct MonitorInfo {
    id: usize,
    index: i32,
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    is_primary: bool,
}

/// Root structure for monitor list JSON output.
#[derive(Debug, serde::Serialize)]
struct MonitorList {
    monitors: Vec<MonitorInfo>,
    selected: usize,
}

fn list_monitors(wm: &Wm) -> IpcResponse {
    let selected_id = wm.g.selected_monitor_id();

    let monitors: Vec<MonitorInfo> =
        wm.g.monitors_iter()
            .map(|(id, m)| MonitorInfo {
                id,
                index: m.num,
                width: m.monitor_rect.w,
                height: m.monitor_rect.h,
                x: m.monitor_rect.x,
                y: m.monitor_rect.y,
                is_primary: id == selected_id,
            })
            .collect();

    let list = MonitorList {
        monitors,
        selected: selected_id,
    };

    match serde_json::to_string_pretty(&list) {
        Ok(json) => IpcResponse::ok(json),
        Err(e) => IpcResponse::err(format!("JSON serialization failed: {}", e)),
    }
}

fn switch_monitor(wm: &mut Wm, index: i32) -> IpcResponse {
    focus_n_mon(&mut wm.ctx(), index);
    IpcResponse::ok("")
}

fn next_monitor(wm: &mut Wm, count: i32) -> IpcResponse {
    let direction = MonitorDirection::new(count.max(1));
    for _ in 0..count.max(1) {
        focus_monitor(&mut wm.ctx(), direction);
    }
    IpcResponse::ok("")
}

fn prev_monitor(wm: &mut Wm, count: i32) -> IpcResponse {
    let direction = MonitorDirection::new(-count.max(1));
    for _ in 0..count.max(1) {
        focus_monitor(&mut wm.ctx(), direction);
    }
    IpcResponse::ok("")
}

fn set_monitor_config(
    wm: &mut Wm,
    identifier: String,
    resolution: Option<String>,
    refresh_rate: Option<f32>,
    position: Option<String>,
    scale: Option<f32>,
    enable: Option<bool>,
) -> IpcResponse {
    let resolved_id = if identifier == "focused" {
        let name = wm.g.selected_monitor().name.clone();
        if name.is_empty() {
            "*".to_string()
        } else {
            name
        }
    } else {
        identifier
    };

    let config = crate::config::config_toml::MonitorConfig {
        resolution,
        refresh_rate,
        position,
        scale,
        enable,
    };

    wm.g.cfg.monitors.insert(resolved_id, config);
    wm.g.monitor_config_dirty = true;
    IpcResponse::ok("")
}

// ============================================================================
// Window Commands
// ============================================================================

fn handle_window_command(wm: &mut Wm, cmd: WindowCommand) -> IpcResponse {
    match cmd {
        WindowCommand::List(window_id) => list_windows(wm, window_id.map(WindowId::from)),
        WindowCommand::Geom(window_id) => window_geometry(wm, window_id.map(WindowId::from)),
        WindowCommand::Close(window_id) => close_window(wm, window_id.map(WindowId::from)),
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
fn client_to_window_info(c: &crate::types::client::Client, valid_tag_mask: u32) -> WindowInfo {
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

fn list_windows(wm: &Wm, parsed_id: Option<WindowId>) -> IpcResponse {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let mut wins: Vec<_> = if let Some(win) = target {
        wm.g.clients.get(&win).into_iter().collect()
    } else {
        wm.g.clients.values().collect()
    };
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

fn close_window(wm: &mut Wm, parsed_id: Option<WindowId>) -> IpcResponse {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let Some(win) = target else {
        return IpcResponse::err("no target window");
    };
    crate::client::close_win(&mut wm.ctx(), win);
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

// ============================================================================
// Tag Commands
// ============================================================================

fn handle_tag_command(wm: &mut Wm, cmd: TagCommand) -> IpcResponse {
    match cmd {
        TagCommand::View(tag_num) => view_tag(wm, tag_num),
        TagCommand::Name(name) => name_tag_cmd(wm, name),
        TagCommand::ResetNames => reset_tag_names(wm),
    }
}

fn view_tag(wm: &mut Wm, tag_num: u32) -> IpcResponse {
    let tag = if tag_num == 0 { 2 } else { tag_num };
    if let Some(mask) = TagMask::single(tag as usize) {
        crate::tags::view::view(&mut wm.ctx(), mask);
    }
    IpcResponse::ok("")
}

fn name_tag_cmd(wm: &mut Wm, name: String) -> IpcResponse {
    name_tag(&mut wm.ctx(), &name);
    IpcResponse::ok("")
}

fn reset_tag_names(wm: &mut Wm) -> IpcResponse {
    reset_name_tag(&mut wm.ctx());
    IpcResponse::ok("")
}

// ============================================================================
// Scratchpad Commands
// ============================================================================

fn handle_scratchpad_command(wm: &mut Wm, cmd: ScratchpadCommand) -> IpcResponse {
    match cmd {
        ScratchpadCommand::List => {
            let list = scratchpad_list(&wm.g);
            IpcResponse::ok(list)
        }
        ScratchpadCommand::Toggle(name) => {
            scratchpad_toggle(&mut wm.ctx(), name.as_deref());
            IpcResponse::ok("")
        }
        ScratchpadCommand::Show(name) => {
            scratchpad_show_name(&mut wm.ctx(), &name);
            IpcResponse::ok("")
        }
        ScratchpadCommand::Hide(name) => {
            scratchpad_hide_name(&mut wm.ctx(), &name);
            IpcResponse::ok("")
        }
        ScratchpadCommand::Status(name) => {
            let status = scratchpad_status(&wm.g, name.as_deref().unwrap_or(""));
            IpcResponse::ok(status)
        }
        ScratchpadCommand::Create(name) => {
            scratchpad_make(&mut wm.ctx(), name.as_deref());
            IpcResponse::ok("")
        }
        ScratchpadCommand::Delete => {
            scratchpad_unmake(&mut wm.ctx());
            IpcResponse::ok("")
        }
    }
}

// ============================================================================
// Keyboard Commands
// ============================================================================

fn handle_keyboard_command(wm: &mut Wm, cmd: KeyboardCommand) -> IpcResponse {
    let mut ctx = wm.ctx();
    match cmd {
        KeyboardCommand::Next => {
            keyboard_layout::cycle_keyboard_layout(&mut ctx, true);
            IpcResponse::ok("")
        }
        KeyboardCommand::Prev => {
            keyboard_layout::cycle_keyboard_layout(&mut ctx, false);
            IpcResponse::ok("")
        }
        KeyboardCommand::Status => {
            let status = keyboard_layout::keyboard_layout_status(&ctx);
            IpcResponse::ok(status)
        }
        KeyboardCommand::List => {
            let list = keyboard_layout::keyboard_layout_list(&ctx);
            IpcResponse::ok(list)
        }
        KeyboardCommand::ListAll => {
            let layouts = keyboard_layout::get_all_keyboard_layouts();
            let list = layouts.join("\n");
            IpcResponse::ok(list)
        }
        KeyboardCommand::Set(layouts) => {
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
        KeyboardCommand::Add(layout) => {
            let globals_layout = crate::globals::KeyboardLayout {
                name: layout.name,
                variant: layout.variant,
            };
            match keyboard_layout::add_keyboard_layout(&mut ctx, globals_layout) {
                Ok(()) => IpcResponse::ok(""),
                Err(e) => IpcResponse::err(e),
            }
        }
        KeyboardCommand::Remove(layout) => {
            match keyboard_layout::remove_keyboard_layout(&mut ctx, &layout) {
                Ok(()) => IpcResponse::ok(""),
                Err(e) => IpcResponse::err(e),
            }
        }
    }
}

// ============================================================================
// Toggle Commands
// ============================================================================

fn handle_toggle_command(wm: &mut Wm, cmd: ToggleCommand) -> IpcResponse {
    let mut ctx = wm.ctx();
    match cmd {
        ToggleCommand::Animated(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_animated(ctx.core_mut(), action);
        }
        ToggleCommand::FocusFollowsMouse(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_focus_follows_mouse(ctx.core_mut(), action);
        }
        ToggleCommand::FocusFollowsFloatMouse(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_focus_follows_float_mouse(ctx.core_mut(), action);
        }
        ToggleCommand::AltTab(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            alt_tab_free(&mut ctx, action);
        }
        ToggleCommand::AltTag(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_alt_tag(&mut ctx, action);
        }
        ToggleCommand::HideTags(arg) => {
            let action = ToggleAction::from_arg(arg.as_deref().unwrap_or(""));
            toggle_show_tags(&mut ctx, action);
        }
    }
    IpcResponse::ok("")
}

// ============================================================================
// Input Commands
// ============================================================================

fn handle_input_command(wm: &mut Wm, cmd: InputCommand) -> IpcResponse {
    use crate::config::config_toml::{AccelProfile, InputConfig, ToggleSetting};

    let inputs = &mut wm.g.cfg.input;
    match cmd {
        InputCommand::List(identifier) => {
            let entries: Vec<_> = match &identifier {
                Some(id) => inputs
                    .iter()
                    .filter(|(k, _)| k.as_str() == id.as_str())
                    .collect(),
                None => inputs.iter().collect(),
            };
            if entries.is_empty() {
                return IpcResponse::ok("no input configuration found");
            }
            let info: Vec<String> = entries
                .iter()
                .map(|(id, cfg)| {
                    format!(
                        "[{}]\ntap: {:?}\nnatural_scroll: {:?}\naccel_profile: {:?}\npointer_accel: {:?}\nscroll_factor: {:?}",
                        id, cfg.tap, cfg.natural_scroll, cfg.accel_profile, cfg.pointer_accel, cfg.scroll_factor,
                    )
                })
                .collect();
            return IpcResponse::ok(info.join("\n\n"));
        }
        InputCommand::PointerAccel { identifier, value } => {
            let cfg = inputs
                .entry(identifier)
                .or_insert_with(InputConfig::default);
            cfg.pointer_accel = Some(value.clamp(-1.0, 1.0));
        }
        InputCommand::AccelProfile {
            identifier,
            profile,
        } => {
            let p = match profile.to_lowercase().as_str() {
                "flat" => AccelProfile::Flat,
                "adaptive" => AccelProfile::Adaptive,
                _ => return IpcResponse::err(format!("unknown accel profile '{profile}'")),
            };
            let cfg = inputs
                .entry(identifier)
                .or_insert_with(InputConfig::default);
            cfg.accel_profile = Some(p);
        }
        InputCommand::Tap {
            identifier,
            enabled,
        } => {
            let cfg = inputs
                .entry(identifier)
                .or_insert_with(InputConfig::default);
            cfg.tap = Some(if enabled {
                ToggleSetting::Enabled
            } else {
                ToggleSetting::Disabled
            });
        }
        InputCommand::NaturalScroll {
            identifier,
            enabled,
        } => {
            let cfg = inputs
                .entry(identifier)
                .or_insert_with(InputConfig::default);
            cfg.natural_scroll = Some(if enabled {
                ToggleSetting::Enabled
            } else {
                ToggleSetting::Disabled
            });
        }
        InputCommand::ScrollFactor { identifier, value } => {
            let cfg = inputs
                .entry(identifier)
                .or_insert_with(InputConfig::default);
            cfg.scroll_factor = Some(value);
        }
    }
    wm.g.input_config_dirty = true;
    IpcResponse::ok("")
}

// ============================================================================
// Other Commands
// ============================================================================

fn run_action(wm: &mut Wm, name: String, args: Vec<String>) -> IpcResponse {
    use crate::config::keybind_config::compile_action_with_args;
    if let Some(action) = compile_action_with_args(&name, &args) {
        action(&mut wm.ctx());
        IpcResponse::ok("")
    } else {
        IpcResponse::err(format!("unknown or invalid action '{name}'"))
    }
}

fn spawn_command(wm: &mut Wm, command: String) -> IpcResponse {
    if command.trim().is_empty() {
        return IpcResponse::err("spawn requires a command");
    }
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(&command);
    if wm.ctx().is_wayland() {
        if let crate::backend::BackendRef::Wayland(wayland) = wm.ctx().backend() {
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

fn warp_focus(wm: &mut Wm) -> IpcResponse {
    crate::mouse::warp::warp_to_focus(&mut wm.ctx());
    IpcResponse::ok("")
}

fn tag_mon(wm: &mut Wm, dir: i32) -> IpcResponse {
    let direction = MonitorDirection::from(dir);
    send_to_monitor(&mut wm.ctx(), direction);
    IpcResponse::ok("")
}

fn follow_mon(wm: &mut Wm, dir: i32) -> IpcResponse {
    let direction = MonitorDirection::from(dir);
    move_to_monitor_and_follow(&mut wm.ctx(), direction);
    IpcResponse::ok("")
}

fn set_layout(wm: &mut Wm, val: u32) -> IpcResponse {
    command_layout(&mut wm.ctx(), val);
    IpcResponse::ok("")
}

fn set_prefix(wm: &mut Wm, arg: Option<u32>) -> IpcResponse {
    let val = arg.unwrap_or(1);
    command_prefix(&mut wm.ctx(), val);
    IpcResponse::ok("")
}

fn set_border(wm: &mut Wm, arg: Option<u32>) -> IpcResponse {
    let val = arg.unwrap_or(crate::config::mod_consts::BORDERPX as u32);
    if let Some(win) = wm.ctx().selected_client() {
        set_border_width(wm.ctx().core_mut(), win, val as i32);
    }
    IpcResponse::ok("")
}

fn set_special_next_cmd(wm: &mut Wm, arg: Option<u32>) -> IpcResponse {
    let val = arg.unwrap_or(0);
    set_special_next(wm.ctx().core_mut(), val);
    IpcResponse::ok("")
}

fn update_status(wm: &mut Wm, text: String) -> IpcResponse {
    wm.g.status_text = text;

    if let crate::backend::Backend::X11(_) = wm.backend {
        let ctx = wm.ctx();
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

// ============================================================================
// Status Command
// ============================================================================

/// Status information for the running instantWM instance.
#[derive(Debug, serde::Serialize)]
struct WmStatusInfo {
    version: String,
    protocol_version: String,
    backend: String,
    running: bool,
    monitors: usize,
    windows: usize,
    tags: usize,
}

fn get_status(wm: &Wm) -> IpcResponse {
    let backend = match &wm.backend {
        crate::backend::Backend::X11(_) => "x11",
        crate::backend::Backend::Wayland(_) => "wayland",
    };

    let info = WmStatusInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: crate::ipc_types::IPC_PROTOCOL_VERSION.to_string(),
        backend: backend.to_string(),
        running: wm.running,
        monitors: wm.g.monitors.len(),
        windows: wm.g.clients.len(),
        tags: wm.g.tags.num_tags,
    };

    match serde_json::to_string_pretty(&info) {
        Ok(json) => IpcResponse::ok(json),
        Err(e) => IpcResponse::err(format!("JSON serialization failed: {}", e)),
    }
}

// ============================================================================
// Mode Commands
// ============================================================================

fn handle_mode_command(wm: &mut Wm, cmd: ModeCommand) -> IpcResponse {
    match cmd {
        ModeCommand::List => {
            let modes = &wm.g.cfg.modes;
            let current_mode = &wm.g.current_mode;

            if modes.is_empty() {
                return IpcResponse::ok("No modes configured");
            }

            let mut output = String::new();
            for (name, mode) in modes {
                let marker = if name == current_mode { "*" } else { " " };
                let desc = mode.description.as_deref().unwrap_or("(no description)");
                output.push_str(&format!("{} {} - {}\n", marker, name, desc));
            }
            IpcResponse::ok(output)
        }
        ModeCommand::Set(name) => {
            // Check if mode exists
            if !wm.g.cfg.modes.contains_key(&name) && name != "default" {
                return IpcResponse::err(format!("Mode '{}' not found", name));
            }
            wm.g.current_mode = name.clone();
            // Request bar update to reflect mode change
            wm.bar.mark_dirty();
            IpcResponse::ok(format!("Switched to mode '{}'", name))
        }
    }
}
