use crate::ipc_types::Response;
use crate::layouts::{LayoutKind, set_layout as layouts_set_layout};
use crate::monitor::move_to_monitor_and_follow;
use crate::tags::send_to_monitor;
use crate::toggles::{set_border_width, set_special_next};
use crate::types::{MonitorDirection, SpecialNext};
use crate::wm::Wm;

pub fn set_wallpaper(wm: &mut Wm, path: String) -> Response {
    if wm.ctx().is_wayland() {
        let _ = std::process::Command::new("killall").arg("swaybg").status();
        let status = std::process::Command::new("swaybg")
            .arg("-i")
            .arg(&path)
            .arg("-m")
            .arg("fill")
            .spawn();
        match status {
            Ok(_) => Response::Message(format!("Wallpaper set to {}", path)),
            Err(e) => Response::err(format!("Failed to spawn swaybg: {}", e)),
        }
    } else {
        let status = std::process::Command::new("feh")
            .arg("--bg-fill")
            .arg(&path)
            .spawn();
        match status {
            Ok(_) => Response::Message(format!("Wallpaper set to {}", path)),
            Err(e) => Response::err(format!("Failed to spawn feh: {}", e)),
        }
    }
}

pub fn run_action(wm: &mut Wm, name: String, args: Vec<String>) -> Response {
    use crate::actions::execute_key_action;
    use crate::config::keybind_config::compile_action_with_args;
    if let Some(action) = compile_action_with_args(&name, &args) {
        let mut ctx = wm.ctx();
        execute_key_action(&mut ctx, &action);
        Response::ok()
    } else {
        Response::err(format!("unknown or invalid action '{name}'"))
    }
}

pub fn spawn_command(wm: &mut Wm, command: String) -> Response {
    if command.trim().is_empty() {
        return Response::err("spawn requires a command");
    }
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(&command);
    if wm.ctx().is_wayland()
        && let crate::backend::BackendRef::Wayland(wayland) = wm.ctx().backend()
        && let Some(display) = wayland.xdisplay()
    {
        cmd.env("DISPLAY", format!(":{display}"));
    }
    match cmd.spawn() {
        Ok(child) => Response::Message(format!("pid={}", child.id())),
        Err(err) => Response::err(format!("spawn failed: {}", err)),
    }
}

pub fn warp_focus(wm: &mut Wm) -> Response {
    crate::mouse::warp::warp_to_focus(&mut wm.ctx());
    Response::ok()
}

pub fn tag_mon(wm: &mut Wm, direction: MonitorDirection) -> Response {
    send_to_monitor(&mut wm.ctx(), direction);
    Response::ok()
}

pub fn follow_mon(wm: &mut Wm, direction: MonitorDirection) -> Response {
    move_to_monitor_and_follow(&mut wm.ctx(), direction);
    Response::ok()
}

pub fn set_layout(wm: &mut Wm, layout: LayoutKind) -> Response {
    layouts_set_layout(&mut wm.ctx(), layout);
    Response::ok()
}

pub fn set_border(wm: &mut Wm, arg: Option<u32>) -> Response {
    let val = arg.unwrap_or(crate::config::mod_consts::BORDERPX as u32);
    if let Some(win) = wm.ctx().selected_client() {
        set_border_width(
            &mut wm.ctx().core_mut().globals_mut().clients,
            win,
            val as i32,
        );
    }
    Response::ok()
}

pub fn set_special_next_cmd(wm: &mut Wm, mode: SpecialNext) -> Response {
    set_special_next(&mut wm.ctx().core_mut().globals_mut().behavior, mode);
    Response::ok()
}

pub fn update_status(wm: &mut Wm, text: String) -> Response {
    if !text.starts_with("instantwm-") {
        crate::bar::status::CUSTOM_STATUS_RECEIVED
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    wm.g.bar_runtime.status_text = text;
    wm.bar
        .request_async_status_parse(&wm.g.bar_runtime.status_text);
    wm.bar.mark_dirty();

    Response::ok()
}

pub fn get_status(wm: &Wm) -> Response {
    let backend = match &wm.backend {
        crate::backend::Backend::X11(_) => "x11",
        crate::backend::Backend::Wayland(_) => "wayland",
    };

    let info = crate::ipc_types::WmStatusInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: crate::ipc_types::IPC_PROTOCOL_VERSION.to_string(),
        build_commit: env!("INSTANTWM_BUILD_COMMIT").to_string(),
        backend: backend.to_string(),
        running: wm.running,
        monitors: wm.g.monitors.len(),
        windows: wm.g.clients.len(),
        tags: wm.g.tags.num_tags,
    };

    Response::Status(info)
}
