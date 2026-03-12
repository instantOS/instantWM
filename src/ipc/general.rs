use crate::commands::{command_prefix, set_special_next};
use crate::ipc_types::IpcResponse;
use crate::layouts::command_layout;
use crate::monitor::move_to_monitor_and_follow;
use crate::tags::send_to_monitor;
use crate::toggles::set_border_width;
use crate::types::MonitorDirection;
use crate::wm::Wm;

pub fn set_wallpaper(wm: &mut Wm, path: String) -> IpcResponse {
    if wm.ctx().is_wayland() {
        // Use swaybg on Wayland
        let _ = std::process::Command::new("killall")
            .arg("swaybg")
            .status();
        let status = std::process::Command::new("swaybg")
            .arg("-i")
            .arg(&path)
            .arg("-m")
            .arg("fill")
            .spawn();
        match status {
            Ok(_) => IpcResponse::ok(format!("Wallpaper set to {}", path)),
            Err(e) => IpcResponse::err(format!("Failed to spawn swaybg: {}", e)),
        }
    } else {
        // Use feh on X11
        let status = std::process::Command::new("feh")
            .arg("--bg-fill")
            .arg(&path)
            .spawn();
        match status {
            Ok(_) => IpcResponse::ok(format!("Wallpaper set to {}", path)),
            Err(e) => IpcResponse::err(format!("Failed to spawn feh: {}", e)),
        }
    }
}

pub fn run_action(wm: &mut Wm, name: String, args: Vec<String>) -> IpcResponse {
    use crate::config::keybind_config::compile_action_with_args;
    if let Some(action) = compile_action_with_args(&name, &args) {
        action(&mut wm.ctx());
        IpcResponse::ok("")
    } else {
        IpcResponse::err(format!("unknown or invalid action '{name}'"))
    }
}

pub fn spawn_command(wm: &mut Wm, command: String) -> IpcResponse {
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

pub fn warp_focus(wm: &mut Wm) -> IpcResponse {
    crate::mouse::warp::warp_to_focus(&mut wm.ctx());
    IpcResponse::ok("")
}

pub fn tag_mon(wm: &mut Wm, dir: i32) -> IpcResponse {
    let direction = MonitorDirection::from(dir);
    send_to_monitor(&mut wm.ctx(), direction);
    IpcResponse::ok("")
}

pub fn follow_mon(wm: &mut Wm, dir: i32) -> IpcResponse {
    let direction = MonitorDirection::from(dir);
    move_to_monitor_and_follow(&mut wm.ctx(), direction);
    IpcResponse::ok("")
}

pub fn set_layout(wm: &mut Wm, val: u32) -> IpcResponse {
    command_layout(&mut wm.ctx(), val);
    IpcResponse::ok("")
}

pub fn set_prefix(wm: &mut Wm, arg: Option<u32>) -> IpcResponse {
    let val = arg.unwrap_or(1);
    command_prefix(&mut wm.ctx(), val);
    IpcResponse::ok("")
}

pub fn set_border(wm: &mut Wm, arg: Option<u32>) -> IpcResponse {
    let val = arg.unwrap_or(crate::config::mod_consts::BORDERPX as u32);
    if let Some(win) = wm.ctx().selected_client() {
        set_border_width(wm.ctx().core_mut(), win, val as i32);
    }
    IpcResponse::ok("")
}

pub fn set_special_next_cmd(wm: &mut Wm, arg: Option<u32>) -> IpcResponse {
    let val = arg.unwrap_or(0);
    set_special_next(wm.ctx().core_mut(), val);
    IpcResponse::ok("")
}

pub fn update_status(wm: &mut Wm, text: String) -> IpcResponse {
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

/// Status information for the running instantWM instance.
#[derive(Debug, serde::Serialize)]
struct WmStatusInfo {
    version: String,
    protocol_version: String,
    build_commit: String,
    backend: String,
    running: bool,
    monitors: usize,
    windows: usize,
    tags: usize,
}

pub fn get_status(wm: &Wm) -> IpcResponse {
    let backend = match &wm.backend {
        crate::backend::Backend::X11(_) => "x11",
        crate::backend::Backend::Wayland(_) => "wayland",
    };

    let info = WmStatusInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: crate::ipc_types::IPC_PROTOCOL_VERSION.to_string(),
        build_commit: env!("INSTANTWM_BUILD_COMMIT").to_string(),
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
