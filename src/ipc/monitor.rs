use crate::backend::OutputOps;
use crate::ipc_types::{MonitorCommand, Response};
use crate::monitor::{focus_monitor, focus_n_mon};
use crate::types::MonitorDirection;
use crate::wm::Wm;
use std::collections::HashMap;

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
            vrr,
        } => {
            let config = crate::config::config_toml::MonitorConfig {
                resolution,
                refresh_rate,
                position,
                scale,
                transform: transform.map(|t| t.to_string()),
                enable,
                vrr,
            };
            set_monitor_config(wm, identifier, config)
        }
        MonitorCommand::Modes { identifier } => list_modes(wm, identifier),
    }
}

fn list_monitors(wm: &Wm) -> Response {
    let selected_id = wm.core.model.selected_monitor_id();
    let output_info: HashMap<_, _> = wm
        .backend
        .get_outputs()
        .into_iter()
        .map(|output| (output.name.clone(), output))
        .collect();

    let monitors: Vec<crate::ipc_types::MonitorInfo> = wm
        .core
        .model
        .monitors_iter()
        .enumerate()
        .map(|(pos, (id, m))| crate::ipc_types::MonitorInfo {
            id: id.get(),
            position: pos,
            backend_index: m.num,
            name: m.name.clone(),
            width: m.monitor_rect.w,
            height: m.monitor_rect.h,
            x: m.monitor_rect.x,
            y: m.monitor_rect.y,
            is_selected: id == selected_id,
            vrr_support: output_info
                .get(&m.name)
                .map(|o| o.vrr_support)
                .unwrap_or(crate::backend::BackendVrrSupport::Unsupported),
            vrr_mode: output_info.get(&m.name).and_then(|o| o.vrr_mode),
            vrr_enabled: output_info.get(&m.name).is_some_and(|o| o.vrr_enabled),
        })
        .collect();

    Response::MonitorList(monitors)
}

fn switch_monitor(wm: &mut Wm, index: i32) -> Response {
    focus_n_mon(&mut wm.ctx(), index.max(0) as usize);
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
    config: crate::config::config_toml::MonitorConfig,
) -> Response {
    let resolved_id = if identifier == "focused" {
        let name = wm.core.model.expect_selected_monitor().name.clone();
        if name.is_empty() {
            "*".to_string()
        } else {
            name
        }
    } else {
        identifier
    };

    wm.core.config.monitors.insert(resolved_id, config);
    wm.work.queue_monitor_config_apply();
    Response::ok()
}

fn list_modes(wm: &mut Wm, identifier: Option<String>) -> Response {
    // Determine which displays to query
    let display_names: Vec<String> = match identifier.as_deref() {
        Some("focused") | None => {
            let name = wm.core.model.expect_selected_monitor().name.clone();
            if name.is_empty() {
                // List all displays
                match &wm.backend {
                    crate::backend::Backend::Wayland(data) => data.backend.list_displays(),
                    crate::backend::Backend::X11(_) => {
                        // For X11, get names from monitor list
                        wm.core
                            .model
                            .monitors_iter()
                            .map(|(_, m)| m.name.clone())
                            .filter(|n| !n.is_empty())
                            .collect()
                    }
                }
            } else {
                vec![name]
            }
        }
        Some(name) => vec![name.to_string()],
    };

    let mut all_modes = Vec::new();

    for display_name in &display_names {
        let modes = match &wm.backend {
            crate::backend::Backend::Wayland(data) => {
                let mode_strings = data.backend.list_display_modes(display_name);
                mode_strings.iter().filter_map(|s| s.parse().ok()).collect()
            }
            crate::backend::Backend::X11(data) => {
                use x11rb::connection::Connection;

                let root = data.conn.setup().roots[data.screen_num].root;
                crate::backend::x11::randr::get_output_modes(&data.conn, root, display_name)
                    .into_iter()
                    .filter_map(|mode| {
                        Some(crate::ipc_types::MonitorMode {
                            width: u32::try_from(mode.width).ok()?,
                            height: u32::try_from(mode.height).ok()?,
                            refresh_mhz: u32::try_from(mode.refresh_millihertz).ok()?,
                        })
                    })
                    .collect()
            }
        };

        all_modes.push(crate::ipc_types::DisplayModes {
            name: display_name.clone(),
            modes,
        });
    }

    Response::MonitorModes(all_modes)
}

#[cfg(test)]
mod tests {
    use super::list_monitors;
    use crate::backend::Backend;
    use crate::backend::wayland::WaylandBackend;
    use crate::ipc_types::Response;
    use crate::types::Monitor;
    use crate::wm::Wm;

    #[test]
    fn monitor_ipc_separates_stable_id_from_spatial_position() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let first = wm.core.model.monitors.push(Monitor::default());
        let second = wm.core.model.monitors.push(Monitor::default());

        let Response::MonitorList(monitors) = list_monitors(&wm) else {
            panic!("monitor list response");
        };

        assert_eq!(monitors[0].id, first.get());
        assert_eq!(monitors[0].position, 0);
        assert_eq!(monitors[1].id, second.get());
        assert_eq!(monitors[1].position, 1);
    }
}
