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
            transform.map(|t| t.to_string()),
            enable,
        ),
        MonitorCommand::Modes { identifier } => list_modes(wm, identifier),
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

fn list_modes(wm: &mut Wm, identifier: Option<String>) -> Response {
    // Determine which displays to query
    let display_names: Vec<String> = match identifier.as_deref() {
        Some("focused") | None => {
            let name = wm.g.selected_monitor().name.clone();
            if name.is_empty() {
                // List all displays
                match &wm.backend {
                    crate::backend::Backend::Wayland(data) => data.backend.list_displays(),
                    crate::backend::Backend::X11(_) => {
                        // For X11, get names from monitor list
                        wm.g.monitors_iter()
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
                mode_strings
                    .iter()
                    .filter_map(|s| parse_mode_string(s))
                    .collect()
            }
            crate::backend::Backend::X11(_) => {
                // On X11, use xrandr to get modes
                match get_xrandr_modes(display_name) {
                    Ok(modes) => modes,
                    Err(_) => continue,
                }
            }
        };

        all_modes.push(crate::ipc_types::DisplayModes {
            name: display_name.clone(),
            modes,
        });
    }

    Response::MonitorModes(all_modes)
}

/// Parse a mode string like "1920x1080@60.000" into a MonitorMode
fn parse_mode_string(s: &str) -> Option<crate::ipc_types::MonitorMode> {
    let (res, rate_str) = s.split_once('@')?;
    let (w, h) = res.split_once('x')?;
    let width: u32 = w.parse().ok()?;
    let height: u32 = h.parse().ok()?;
    let rate_hz: f64 = rate_str.parse().ok()?;
    let refresh_mhz = (rate_hz * 1000.0) as u32;
    Some(crate::ipc_types::MonitorMode {
        width,
        height,
        refresh_mhz,
    })
}

/// Get modes for a display using xrandr (X11 fallback)
fn get_xrandr_modes(
    display_name: &str,
) -> Result<Vec<crate::ipc_types::MonitorMode>, std::io::Error> {
    let output = std::process::Command::new("xrandr")
        .arg("--json")
        .output()?;

    if !output.status.success() {
        return Ok(Vec::new());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = match serde_json::from_str(&stdout) {
        Ok(v) => v,
        Err(_) => return Ok(Vec::new()),
    };

    let mut modes = Vec::new();

    if let Some(screens) = json.get("screens").and_then(|v| v.as_array()) {
        for screen in screens {
            if let Some(outputs) = screen.get("outputs").and_then(|v| v.as_array()) {
                for output in outputs {
                    let name = output.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    if name != display_name {
                        continue;
                    }

                    if let Some(modes_json) = output.get("modes").and_then(|v| v.as_array()) {
                        for mode_json in modes_json {
                            let width =
                                mode_json.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                            let height = mode_json
                                .get("height")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as u32;

                            if let Some(freqs) =
                                mode_json.get("frequencies").and_then(|v| v.as_array())
                            {
                                for freq in freqs {
                                    let rate =
                                        freq.get("rate").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                    modes.push(crate::ipc_types::MonitorMode {
                                        width,
                                        height,
                                        refresh_mhz: (rate * 1000.0) as u32,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(modes)
}
