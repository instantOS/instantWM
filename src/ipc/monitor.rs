use crate::ipc_types::{MonitorCommand, Response};
use crate::monitor::{focus_monitor, focus_n_mon};
use crate::types::MonitorDirection;
use crate::wm::Wm;

pub fn handle_monitor_command(wm: &mut Wm, cmd: MonitorCommand) -> Response {
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
            transform,
            enable,
        } => set_monitor_config(
            wm,
            identifier,
            resolution,
            refresh_rate,
            position,
            scale,
            transform,
            enable,
        ),
    }
}

fn list_monitors(wm: &Wm) -> Response {
    let selected_id = wm.g.selected_monitor_id();

    let monitors: Vec<crate::ipc_types::MonitorInfo> =
        wm.g.monitors_iter()
            .map(|(id, m)| crate::ipc_types::MonitorInfo {
                id,
                index: m.num,
                width: m.monitor_rect.w,
                height: m.monitor_rect.h,
                x: m.monitor_rect.x,
                y: m.monitor_rect.y,
                is_primary: id == selected_id,
            })
            .collect();

    Response::MonitorList(monitors)
}

fn switch_monitor(wm: &mut Wm, index: i32) -> Response {
    focus_n_mon(&mut wm.ctx(), index);
    Response::ok()
}

fn next_monitor(wm: &mut Wm, count: i32) -> Response {
    let direction = MonitorDirection::new(count.max(1));
    for _ in 0..count.max(1) {
        focus_monitor(&mut wm.ctx(), direction);
    }
    Response::ok()
}

fn prev_monitor(wm: &mut Wm, count: i32) -> Response {
    let direction = MonitorDirection::new(-count.max(1));
    for _ in 0..count.max(1) {
        focus_monitor(&mut wm.ctx(), direction);
    }
    Response::ok()
}

fn set_monitor_config(
    wm: &mut Wm,
    identifier: String,
    resolution: Option<String>,
    refresh_rate: Option<f32>,
    position: Option<String>,
    scale: Option<f32>,
    transform: Option<String>,
    enable: Option<bool>,
) -> Response {
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
        transform,
        enable,
    };

    wm.g.cfg.monitors.insert(resolved_id, config);
    wm.g.dirty.monitor_config = true;
    Response::ok()
}
