//! X11 XRandR support for display configuration.

use crate::backend::BackendOutputInfo;
use crate::backend::BackendVrrSupport;
use crate::config::config_toml::MonitorConfig;
use crate::types::{MonitorPosition, Rect};
use x11rb::protocol::randr::{self, ConnectionExt as RandrExt};
use x11rb::protocol::xproto::Window;
use x11rb::rust_connection::RustConnection;

/// Get outputs using XRandR.
///
/// Returns a list of connected outputs with their names and geometries.
pub fn get_outputs(conn: &RustConnection, root: Window) -> Vec<BackendOutputInfo> {
    // Try to get screen resources, prefer the current (faster) version
    match get_screen_resources_current(conn, root) {
        Some(outputs) if !outputs.is_empty() => outputs,
        _ => {
            // Fall back to the non-current version
            get_screen_resources(conn, root).unwrap_or_default()
        }
    }
}

/// Get outputs using GetScreenResourcesCurrent.
fn get_screen_resources_current(
    conn: &RustConnection,
    root: Window,
) -> Option<Vec<BackendOutputInfo>> {
    let resources = conn
        .randr_get_screen_resources_current(root)
        .ok()?
        .reply()
        .ok()?;

    let mut outputs = Vec::new();
    let config_timestamp = resources.config_timestamp;

    for output_id in &resources.outputs {
        let output_info = conn
            .randr_get_output_info(*output_id, config_timestamp)
            .ok()?
            .reply()
            .ok()?;

        // Only include connected outputs
        if output_info.connection != randr::Connection::CONNECTED {
            continue;
        }

        let name = String::from_utf8_lossy(&output_info.name).to_string();

        let rect = if output_info.crtc != 0 {
            let crtc_info = conn
                .randr_get_crtc_info(output_info.crtc, config_timestamp)
                .ok()?
                .reply()
                .ok()?;

            let mode_width;
            let mode_height;

            if let Some(mode_info) = resources.modes.iter().find(|m| m.id == crtc_info.mode) {
                mode_width = mode_info.width as i32;
                mode_height = mode_info.height as i32;
            } else {
                mode_width = crtc_info.width as i32;
                mode_height = crtc_info.height as i32;
            }

            Rect {
                x: crtc_info.x as i32,
                y: crtc_info.y as i32,
                w: mode_width,
                h: mode_height,
            }
        } else {
            let preferred_mode = find_preferred_mode(&output_info, &resources.modes)?;
            Rect {
                x: 0,
                y: 0,
                w: preferred_mode.width as i32,
                h: preferred_mode.height as i32,
            }
        };

        outputs.push(BackendOutputInfo {
            name,
            rect,
            vrr_support: BackendVrrSupport::Unsupported,
            vrr_mode: None,
            vrr_enabled: false,
        });
    }

    Some(outputs)
}

/// Get outputs using GetScreenResources (fallback).
fn get_screen_resources(conn: &RustConnection, root: Window) -> Option<Vec<BackendOutputInfo>> {
    let resources = conn.randr_get_screen_resources(root).ok()?.reply().ok()?;

    let mut outputs = Vec::new();
    let config_timestamp = resources.config_timestamp;

    for output_id in &resources.outputs {
        let output_info = conn
            .randr_get_output_info(*output_id, config_timestamp)
            .ok()?
            .reply()
            .ok()?;

        // Only include connected outputs
        if output_info.connection != randr::Connection::CONNECTED {
            continue;
        }

        let name = String::from_utf8_lossy(&output_info.name).to_string();

        let rect = if output_info.crtc != 0 {
            let crtc_info = conn
                .randr_get_crtc_info(output_info.crtc, config_timestamp)
                .ok()?
                .reply()
                .ok()?;

            let mode_width;
            let mode_height;

            if let Some(mode_info) = resources.modes.iter().find(|m| m.id == crtc_info.mode) {
                mode_width = mode_info.width as i32;
                mode_height = mode_info.height as i32;
            } else {
                mode_width = crtc_info.width as i32;
                mode_height = crtc_info.height as i32;
            }

            Rect {
                x: crtc_info.x as i32,
                y: crtc_info.y as i32,
                w: mode_width,
                h: mode_height,
            }
        } else {
            let preferred_mode = find_preferred_mode(&output_info, &resources.modes)?;
            Rect {
                x: 0,
                y: 0,
                w: preferred_mode.width as i32,
                h: preferred_mode.height as i32,
            }
        };

        outputs.push(BackendOutputInfo {
            name,
            rect,
            vrr_support: BackendVrrSupport::Unsupported,
            vrr_mode: None,
            vrr_enabled: false,
        });
    }

    Some(outputs)
}

/// Set monitor configuration using XRandR.
pub fn set_monitor_config(conn: &RustConnection, root: Window, name: &str, config: &MonitorConfig) {
    // Try to use current resources first
    if set_monitor_config_current(conn, root, name, config) {
        return;
    }

    // Fall back to regular resources
    let _ = set_monitor_config_fallback(conn, root, name, config);
}

/// Set monitor configuration using GetScreenResourcesCurrent.
fn set_monitor_config_current(
    conn: &RustConnection,
    root: Window,
    name: &str,
    config: &MonitorConfig,
) -> bool {
    let resources = match conn
        .randr_get_screen_resources_current(root)
        .ok()
        .and_then(|c| c.reply().ok())
    {
        Some(r) => r,
        None => return false,
    };

    let config_timestamp = resources.config_timestamp;
    let known_outputs =
        collect_output_rects(conn, &resources.outputs, config_timestamp, &resources.modes);

    for output_id in &resources.outputs {
        let output_info = match conn
            .randr_get_output_info(*output_id, config_timestamp)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(info) => info,
            None => continue,
        };

        let output_name = String::from_utf8_lossy(&output_info.name);

        if name != "*" && output_name != name {
            continue;
        }

        if output_info.connection != randr::Connection::CONNECTED {
            continue;
        }

        apply_output_config(
            conn,
            root,
            *output_id,
            &output_info,
            config,
            config_timestamp,
            &resources.modes,
            &known_outputs,
        );
    }

    true
}

/// Set monitor configuration using GetScreenResources (fallback).
fn set_monitor_config_fallback(
    conn: &RustConnection,
    root: Window,
    name: &str,
    config: &MonitorConfig,
) -> bool {
    let resources = match conn
        .randr_get_screen_resources(root)
        .ok()
        .and_then(|c| c.reply().ok())
    {
        Some(r) => r,
        None => return false,
    };

    let config_timestamp = resources.config_timestamp;
    let known_outputs =
        collect_output_rects(conn, &resources.outputs, config_timestamp, &resources.modes);

    for output_id in &resources.outputs {
        let output_info = match conn
            .randr_get_output_info(*output_id, config_timestamp)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(info) => info,
            None => continue,
        };

        let output_name = String::from_utf8_lossy(&output_info.name);

        if name != "*" && output_name != name {
            continue;
        }

        if output_info.connection != randr::Connection::CONNECTED {
            continue;
        }

        apply_output_config_fallback(
            conn,
            root,
            *output_id,
            &output_info,
            config,
            config_timestamp,
            &resources.modes,
            &known_outputs,
        );
    }

    true
}

/// Apply configuration to a specific output (current version).
fn apply_output_config(
    conn: &RustConnection,
    root: Window,
    output_id: randr::Output,
    output_info: &randr::GetOutputInfoReply,
    config: &MonitorConfig,
    config_timestamp: u32,
    modes: &[randr::ModeInfo],
    known_outputs: &[(String, Rect)],
) {
    // Handle enable/disable
    if let Some(enable) = config.enable
        && !enable
    {
        // Disable the output by setting CRTC to None
        if output_info.crtc != 0 {
            let _ = conn.randr_set_crtc_config(
                output_info.crtc,
                x11rb::CURRENT_TIME,
                config_timestamp,
                0,
                0,
                0, // No mode
                randr::Rotation::ROTATE0,
                &[], // No outputs
            );
        }
        return;
    }

    // Find the mode to use
    let mode = if let Some(ref resolution) = config.resolution {
        parse_resolution(resolution)
            .and_then(|(w, h)| find_mode_by_resolution(modes, w, h))
            .or_else(|| find_preferred_mode(output_info, modes))
    } else {
        find_preferred_mode(output_info, modes)
    };

    let Some(mode_info) = mode else {
        return;
    };

    // Parse position
    let (x, y) = if let Some(ref position) = config.position {
        MonitorPosition::parse(position)
            .and_then(|p| {
                p.resolve(
                    (mode_info.width as i32, mode_info.height as i32),
                    known_outputs
                        .iter()
                        .map(|(name, rect)| (name.as_str(), *rect)),
                )
            })
            .unwrap_or((0, 0))
    } else {
        (0, 0)
    };

    // Find a CRTC to use
    let crtc = if output_info.crtc != 0 {
        output_info.crtc
    } else {
        find_available_crtc_current(conn, output_id, output_info, root)
    };

    if crtc == 0 {
        return;
    }

    // Set the CRTC configuration
    let _ = conn.randr_set_crtc_config(
        crtc,
        x11rb::CURRENT_TIME,
        config_timestamp,
        x as i16,
        y as i16,
        mode_info.id,
        randr::Rotation::ROTATE0,
        &[output_id],
    );
}

/// Apply configuration to a specific output (fallback version).
fn apply_output_config_fallback(
    conn: &RustConnection,
    root: Window,
    output_id: randr::Output,
    output_info: &randr::GetOutputInfoReply,
    config: &MonitorConfig,
    config_timestamp: u32,
    modes: &[randr::ModeInfo],
    known_outputs: &[(String, Rect)],
) {
    // Handle enable/disable
    if let Some(enable) = config.enable
        && !enable
    {
        // Disable the output by setting CRTC to None
        if output_info.crtc != 0 {
            let _ = conn.randr_set_crtc_config(
                output_info.crtc,
                x11rb::CURRENT_TIME,
                config_timestamp,
                0,
                0,
                0, // No mode
                randr::Rotation::ROTATE0,
                &[], // No outputs
            );
        }
        return;
    }

    // Find the mode to use
    let mode = if let Some(ref resolution) = config.resolution {
        parse_resolution(resolution)
            .and_then(|(w, h)| find_mode_by_resolution(modes, w, h))
            .or_else(|| find_preferred_mode(output_info, modes))
    } else {
        find_preferred_mode(output_info, modes)
    };

    let Some(mode_info) = mode else {
        return;
    };

    // Parse position
    let (x, y) = if let Some(ref position) = config.position {
        MonitorPosition::parse(position)
            .and_then(|p| {
                p.resolve(
                    (mode_info.width as i32, mode_info.height as i32),
                    known_outputs
                        .iter()
                        .map(|(name, rect)| (name.as_str(), *rect)),
                )
            })
            .unwrap_or((0, 0))
    } else {
        (0, 0)
    };

    // Find a CRTC to use
    let crtc = if output_info.crtc != 0 {
        output_info.crtc
    } else {
        find_available_crtc_fallback(conn, output_id, output_info, root)
    };

    if crtc == 0 {
        return;
    }

    // Set the CRTC configuration
    let _ = conn.randr_set_crtc_config(
        crtc,
        x11rb::CURRENT_TIME,
        config_timestamp,
        x as i16,
        y as i16,
        mode_info.id,
        randr::Rotation::ROTATE0,
        &[output_id],
    );
}

/// Find the preferred mode for an output.
///
/// The preferred mode is the first one in the output's modes list
/// (as reported by the EDID).
fn find_preferred_mode(
    output_info: &randr::GetOutputInfoReply,
    modes: &[randr::ModeInfo],
) -> Option<randr::ModeInfo> {
    // The first mode in the list is the preferred one
    output_info
        .modes
        .first()
        .and_then(|mode_id| modes.iter().find(|m| &m.id == mode_id).copied())
}

/// Find a mode by resolution.
fn find_mode_by_resolution(
    modes: &[randr::ModeInfo],
    width: u16,
    height: u16,
) -> Option<randr::ModeInfo> {
    modes
        .iter()
        .find(|m| m.width == width && m.height == height)
        .copied()
}

/// Parse a resolution string like "1920x1080".
fn parse_resolution(res: &str) -> Option<(u16, u16)> {
    let parts: Vec<&str> = res.split('x').collect();
    if parts.len() == 2 {
        let w = parts[0].parse().ok()?;
        let h = parts[1].parse().ok()?;
        Some((w, h))
    } else {
        None
    }
}

fn collect_output_rects(
    conn: &RustConnection,
    output_ids: &[randr::Output],
    config_timestamp: u32,
    modes: &[randr::ModeInfo],
) -> Vec<(String, Rect)> {
    let mut outputs = Vec::new();

    for output_id in output_ids {
        let Some(output_info) = conn
            .randr_get_output_info(*output_id, config_timestamp)
            .ok()
            .and_then(|c| c.reply().ok())
        else {
            continue;
        };

        if output_info.connection != randr::Connection::CONNECTED {
            continue;
        }

        let name = String::from_utf8_lossy(&output_info.name).to_string();
        let rect = if output_info.crtc != 0 {
            let Some(crtc_info) = conn
                .randr_get_crtc_info(output_info.crtc, config_timestamp)
                .ok()
                .and_then(|c| c.reply().ok())
            else {
                continue;
            };

            let (w, h) = modes
                .iter()
                .find(|m| m.id == crtc_info.mode)
                .map(|m| (m.width as i32, m.height as i32))
                .unwrap_or((crtc_info.width as i32, crtc_info.height as i32));

            Rect::new(crtc_info.x as i32, crtc_info.y as i32, w, h)
        } else {
            let Some(mode) = find_preferred_mode(&output_info, modes) else {
                continue;
            };
            Rect::new(0, 0, mode.width as i32, mode.height as i32)
        };

        outputs.push((name, rect));
    }

    outputs
}

/// Find an available CRTC (current version).
fn find_available_crtc_current(
    conn: &RustConnection,
    _output_id: randr::Output,
    output_info: &randr::GetOutputInfoReply,
    root: Window,
) -> randr::Crtc {
    if output_info.crtc != 0 {
        return output_info.crtc;
    }

    let resources = match conn
        .randr_get_screen_resources_current(root)
        .ok()
        .and_then(|c| c.reply().ok())
    {
        Some(r) => r,
        None => return 0,
    };

    for crtc_id in &resources.crtcs {
        let crtc_info = match conn
            .randr_get_crtc_info(*crtc_id, resources.config_timestamp)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(info) => info,
            None => continue,
        };

        if crtc_info.outputs.is_empty() {
            return *crtc_id;
        }
    }

    0
}

/// Find an available CRTC (fallback version).
fn find_available_crtc_fallback(
    conn: &RustConnection,
    _output_id: randr::Output,
    output_info: &randr::GetOutputInfoReply,
    root: Window,
) -> randr::Crtc {
    if output_info.crtc != 0 {
        return output_info.crtc;
    }

    let resources = match conn
        .randr_get_screen_resources(root)
        .ok()
        .and_then(|c| c.reply().ok())
    {
        Some(r) => r,
        None => return 0,
    };

    for crtc_id in &resources.crtcs {
        let crtc_info = match conn
            .randr_get_crtc_info(*crtc_id, resources.config_timestamp)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(info) => info,
            None => continue,
        };

        if crtc_info.outputs.is_empty() {
            return *crtc_id;
        }
    }

    0
}
