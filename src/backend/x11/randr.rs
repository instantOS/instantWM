//! X11 XRandR support for display configuration.

use crate::backend::BackendOutputInfo;
use crate::backend::BackendVrrSupport;
use crate::config::config_toml::MonitorConfig;
use crate::types::{MonitorPosition, Rect};
use x11rb::protocol::randr::{self, ConnectionExt as RandrExt};
use x11rb::protocol::xproto::Window;
use x11rb::rust_connection::RustConnection;

/// Return the fastest active RandR output refresh rate in millihertz.
///
/// X11 has one geometry-update stream for all outputs, so pacing it for the
/// fastest active output avoids undersampling animations on mixed-refresh
/// desktops. Slower outputs simply present the latest available geometry.
pub fn max_active_refresh_millihertz(conn: &RustConnection, root: Window) -> Option<u32> {
    let resources = conn
        .randr_get_screen_resources_current(root)
        .ok()?
        .reply()
        .ok()?;

    resources
        .crtcs
        .iter()
        .filter_map(|crtc| {
            let crtc = conn
                .randr_get_crtc_info(*crtc, resources.config_timestamp)
                .ok()?
                .reply()
                .ok()?;
            let mode = resources.modes.iter().find(|mode| mode.id == crtc.mode)?;
            mode_refresh_millihertz(mode.dot_clock, mode.htotal, mode.vtotal)
        })
        .max()
}

fn mode_refresh_millihertz(dot_clock: u32, htotal: u16, vtotal: u16) -> Option<u32> {
    let divisor = u64::from(htotal).checked_mul(u64::from(vtotal))?;
    if dot_clock == 0 || divisor == 0 {
        return None;
    }
    u32::try_from(u64::from(dot_clock).saturating_mul(1000) / divisor).ok()
}

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

/// Extract output info from already-fetched RandR resources.
fn process_outputs(
    conn: &RustConnection,
    output_ids: &[randr::Output],
    config_timestamp: u32,
    modes: &[randr::ModeInfo],
) -> Option<Vec<BackendOutputInfo>> {
    let mut outputs = Vec::new();

    for output_id in output_ids {
        let output_info = conn
            .randr_get_output_info(*output_id, config_timestamp)
            .ok()?
            .reply()
            .ok()?;

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

            let (w, h) = modes
                .iter()
                .find(|m| m.id == crtc_info.mode)
                .map(|m| (m.width as i32, m.height as i32))
                .unwrap_or((crtc_info.width as i32, crtc_info.height as i32));

            Rect::new(crtc_info.x as i32, crtc_info.y as i32, w, h)
        } else {
            let preferred_mode = find_preferred_mode(&output_info, modes)?;
            Rect::new(
                0,
                0,
                preferred_mode.width as i32,
                preferred_mode.height as i32,
            )
        };

        outputs.push(BackendOutputInfo {
            name,
            rect,
            scale: 1.0,
            vrr_support: BackendVrrSupport::Unsupported,
            vrr_mode: None,
            vrr_enabled: false,
        });
    }

    Some(outputs)
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
    process_outputs(
        conn,
        &resources.outputs,
        resources.config_timestamp,
        &resources.modes,
    )
}

/// Get outputs using GetScreenResources (fallback).
fn get_screen_resources(conn: &RustConnection, root: Window) -> Option<Vec<BackendOutputInfo>> {
    let resources = conn.randr_get_screen_resources(root).ok()?.reply().ok()?;
    process_outputs(
        conn,
        &resources.outputs,
        resources.config_timestamp,
        &resources.modes,
    )
}

/// Set monitor configuration using XRandR.
pub fn set_monitor_config(conn: &RustConnection, root: Window, name: &str, config: &MonitorConfig) {
    if set_monitor_config_inner(conn, root, name, config, true) {
        return;
    }
    let _ = set_monitor_config_inner(conn, root, name, config, false);
}

/// Set monitor configuration for a given resource-fetch strategy.
fn set_monitor_config_inner(
    conn: &RustConnection,
    root: Window,
    name: &str,
    config: &MonitorConfig,
    use_current: bool,
) -> bool {
    let (output_ids, config_timestamp, modes) = if use_current {
        let resources = match conn
            .randr_get_screen_resources_current(root)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(r) => r,
            None => return false,
        };
        (
            resources.outputs,
            resources.config_timestamp,
            resources.modes,
        )
    } else {
        let resources = match conn
            .randr_get_screen_resources(root)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(r) => r,
            None => return false,
        };
        (
            resources.outputs,
            resources.config_timestamp,
            resources.modes,
        )
    };

    let known_outputs = collect_output_rects(conn, &output_ids, config_timestamp, &modes);

    for output_id in &output_ids {
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
            &modes,
            &known_outputs,
            use_current,
        );
    }

    true
}

/// Apply configuration to a specific output.
fn apply_output_config(
    conn: &RustConnection,
    root: Window,
    output_id: randr::Output,
    output_info: &randr::GetOutputInfoReply,
    config: &MonitorConfig,
    config_timestamp: u32,
    modes: &[randr::ModeInfo],
    known_outputs: &[(String, Rect)],
    use_current: bool,
) {
    if let Some(enable) = config.enable
        && !enable
    {
        if output_info.crtc != 0 {
            let _ = conn.randr_set_crtc_config(
                output_info.crtc,
                x11rb::CURRENT_TIME,
                config_timestamp,
                0,
                0,
                0,
                randr::Rotation::ROTATE0,
                &[],
            );
        }
        return;
    }

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

    let position = if let Some(ref position) = config.position {
        MonitorPosition::parse(position)
            .and_then(|p| {
                p.resolve(
                    crate::types::Size::new(mode_info.width as i32, mode_info.height as i32),
                    known_outputs
                        .iter()
                        .map(|(name, rect)| (name.as_str(), *rect)),
                )
            })
            .unwrap_or_default()
    } else {
        crate::types::Point::default()
    };

    let crtc = if output_info.crtc != 0 {
        output_info.crtc
    } else {
        find_available_crtc(conn, output_id, output_info, root, use_current)
    };

    if crtc == 0 {
        return;
    }

    let _ = conn.randr_set_crtc_config(
        crtc,
        x11rb::CURRENT_TIME,
        config_timestamp,
        position.x as i16,
        position.y as i16,
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

/// Find an available CRTC.
fn find_available_crtc(
    conn: &RustConnection,
    _output_id: randr::Output,
    output_info: &randr::GetOutputInfoReply,
    root: Window,
    use_current: bool,
) -> randr::Crtc {
    if output_info.crtc != 0 {
        return output_info.crtc;
    }

    let (crtcs, config_timestamp) = if use_current {
        let resources = match conn
            .randr_get_screen_resources_current(root)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(r) => r,
            None => return 0,
        };
        (resources.crtcs, resources.config_timestamp)
    } else {
        let resources = match conn
            .randr_get_screen_resources(root)
            .ok()
            .and_then(|c| c.reply().ok())
        {
            Some(r) => r,
            None => return 0,
        };
        (resources.crtcs, resources.config_timestamp)
    };

    for crtc_id in &crtcs {
        let crtc_info = match conn
            .randr_get_crtc_info(*crtc_id, config_timestamp)
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

#[cfg(test)]
mod refresh_tests {
    use super::mode_refresh_millihertz;

    #[test]
    fn calculates_standard_and_high_refresh_modes() {
        assert_eq!(
            mode_refresh_millihertz(148_500_000, 2200, 1125),
            Some(60_000)
        );
        assert_eq!(
            mode_refresh_millihertz(585_953_280, 2720, 1496),
            Some(144_000)
        );
    }

    #[test]
    fn rejects_incomplete_mode_timings() {
        assert_eq!(mode_refresh_millihertz(0, 2200, 1125), None);
        assert_eq!(mode_refresh_millihertz(148_500_000, 0, 1125), None);
    }
}
