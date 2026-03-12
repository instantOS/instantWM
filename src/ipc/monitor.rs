use crate::ipc_types::{IpcResponse, MonitorCommand};
use crate::monitor::{focus_monitor, focus_n_mon};
use crate::types::MonitorDirection;
use crate::wm::Wm;

pub fn handle_monitor_command(wm: &mut Wm, cmd: MonitorCommand) -> IpcResponse {
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
