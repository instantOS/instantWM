//! X11 XRandR support for display configuration.

use crate::backend::BackendOutputInfo;
use crate::backend::BackendVrrSupport;
use crate::config::config_toml::MonitorConfig;
use crate::types::{MonitorPosition, Rect};
use std::collections::HashMap;
use x11rb::connection::Connection;
use x11rb::protocol::randr::{self, ConnectionExt as RandrExt};
use x11rb::protocol::xproto::{ConnectionExt as XprotoExt, Window};
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

    let requests: Vec<_> = resources
        .crtcs
        .iter()
        .filter_map(|crtc| {
            conn.randr_get_crtc_info(*crtc, resources.config_timestamp)
                .ok()
        })
        .collect();
    requests
        .into_iter()
        .filter_map(|request| {
            let crtc = request.reply().ok()?;
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
/// Returns active outputs with their names and geometries.
///
/// A connected output without a CRTC is a physical head, not a logical
/// monitor. Publishing it at an invented `(0, 0)` position creates a phantom
/// monitor overlapping the real desktop. Output policy may enable such a head;
/// monitor discovery only reports what is actually being scanned out.
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

fn fetch_output_infos(
    conn: &RustConnection,
    output_ids: &[randr::Output],
    config_timestamp: u32,
) -> Vec<(randr::Output, randr::GetOutputInfoReply)> {
    let requests: Vec<_> = output_ids
        .iter()
        .filter_map(|output_id| {
            Some((
                *output_id,
                conn.randr_get_output_info(*output_id, config_timestamp)
                    .ok()?,
            ))
        })
        .collect();
    requests
        .into_iter()
        .filter_map(|(id, request)| Some((id, request.reply().ok()?)))
        .collect()
}

fn fetch_crtc_infos(
    conn: &RustConnection,
    crtc_ids: &[randr::Crtc],
    config_timestamp: u32,
) -> HashMap<randr::Crtc, randr::GetCrtcInfoReply> {
    let requests: Vec<_> = crtc_ids
        .iter()
        .filter_map(|crtc| {
            Some((
                *crtc,
                conn.randr_get_crtc_info(*crtc, config_timestamp).ok()?,
            ))
        })
        .collect();
    requests
        .into_iter()
        .filter_map(|(id, request)| Some((id, request.reply().ok()?)))
        .collect()
}

/// Extract output info from already-fetched RandR resources.
fn process_outputs(
    conn: &RustConnection,
    output_ids: &[randr::Output],
    config_timestamp: u32,
    modes: &[randr::ModeInfo],
) -> Option<Vec<BackendOutputInfo>> {
    let output_infos: Vec<_> = fetch_output_infos(conn, output_ids, config_timestamp)
        .into_iter()
        .filter(|(_, info)| info.connection == randr::Connection::CONNECTED && info.crtc != 0)
        .collect();
    let crtc_ids: Vec<_> = output_infos
        .iter()
        .filter(|(_, info)| info.crtc != 0)
        .map(|(_, info)| info.crtc)
        .collect();
    let crtc_infos = fetch_crtc_infos(conn, &crtc_ids, config_timestamp);

    let mut outputs = Vec::with_capacity(output_infos.len());
    for (_, output_info) in output_infos {
        let name = String::from_utf8_lossy(&output_info.name).to_string();

        let crtc_info = crtc_infos.get(&output_info.crtc)?;
        let (w, h) = modes
            .iter()
            .find(|m| m.id == crtc_info.mode)
            .map(|m| (m.width as i32, m.height as i32))
            .unwrap_or((crtc_info.width as i32, crtc_info.height as i32));
        let rect = Rect::new(crtc_info.x as i32, crtc_info.y as i32, w, h);

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
fn set_monitor_config(conn: &RustConnection, root: Window, name: &str, config: &MonitorConfig) {
    if set_monitor_config_inner(conn, root, name, config, true) {
        return;
    }
    let _ = set_monitor_config_inner(conn, root, name, config, false);
}

/// Apply exactly one effective policy per connected output. A named entry
/// shadows the wildcard instead of relying on two order-dependent modesets.
pub fn apply_monitor_configs(
    conn: &RustConnection,
    root: Window,
    configs: &HashMap<String, MonitorConfig>,
) {
    let Some(resources) = conn
        .randr_get_screen_resources_current(root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return;
    };
    let output_infos = fetch_output_infos(conn, &resources.outputs, resources.config_timestamp);
    for (_, output) in output_infos
        .iter()
        .filter(|(_, output)| output.connection == randr::Connection::CONNECTED)
    {
        let name = String::from_utf8_lossy(&output.name);
        if let Some(config) = effective_monitor_config(configs, &name) {
            set_monitor_config(conn, root, &name, config);
        }
    }
}

/// Configure newly connected heads according to the same policy hierarchy as
/// normal monitor configuration: exact name, wildcard, then automatic default.
/// Explicitly disabled heads never undergo a temporary enabling modeset.
pub fn configure_connected_outputs(
    conn: &RustConnection,
    root: Window,
    configs: &HashMap<String, MonitorConfig>,
) {
    let Some(resources) = conn
        .randr_get_screen_resources_current(root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return;
    };
    let output_infos = fetch_output_infos(conn, &resources.outputs, resources.config_timestamp);
    for (_, output) in output_infos
        .iter()
        .filter(|(_, output)| output.connection == randr::Connection::CONNECTED && output.crtc == 0)
    {
        let name = String::from_utf8_lossy(&output.name);
        let config = effective_monitor_config(configs, &name)
            .cloned()
            .unwrap_or_default();
        if config.enable == Some(false) {
            continue;
        }
        set_monitor_config(conn, root, &name, &config);
    }
}

fn effective_monitor_config<'a>(
    configs: &'a HashMap<String, MonitorConfig>,
    output_name: &str,
) -> Option<&'a MonitorConfig> {
    configs.get(output_name).or_else(|| configs.get("*"))
}

/// Close holes left by removed automatically positioned outputs. Outputs with
/// an explicit named or wildcard position anchor the layout and are never
/// moved by this policy.
pub fn compact_automatic_output_layout(
    conn: &RustConnection,
    root: Window,
    configs: &HashMap<String, MonitorConfig>,
) {
    let mut outputs = get_outputs(conn, root);
    outputs.sort_by(|a, b| (a.rect.x, &a.name).cmp(&(b.rect.x, &b.name)));
    for (name, position) in planned_automatic_positions(&outputs, configs) {
        let config = MonitorConfig {
            position: Some(format!("{},{}", position.x, position.y)),
            ..MonitorConfig::default()
        };
        set_monitor_config(conn, root, &name, &config);
    }
}

fn planned_automatic_positions(
    outputs: &[BackendOutputInfo],
    configs: &HashMap<String, MonitorConfig>,
) -> Vec<(String, crate::types::Point)> {
    let mut cursor = 0;
    let mut moves = Vec::new();
    for output in outputs {
        let explicitly_positioned = effective_monitor_config(configs, &output.name)
            .is_some_and(|config| config.position.is_some());
        if explicitly_positioned {
            cursor = cursor.max(output.rect.x.saturating_add(output.rect.w));
            continue;
        }

        if output.rect.x != cursor {
            moves.push((
                output.name.clone(),
                crate::types::Point::new(cursor, output.rect.y),
            ));
        }
        cursor = cursor.saturating_add(output.rect.w);
    }
    moves
}

/// Set monitor configuration for a given resource-fetch strategy.
fn set_monitor_config_inner(
    conn: &RustConnection,
    root: Window,
    name: &str,
    config: &MonitorConfig,
    use_current: bool,
) -> bool {
    let (output_ids, crtc_ids, config_timestamp, modes) = if use_current {
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
            resources.crtcs,
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
            resources.crtcs,
            resources.config_timestamp,
            resources.modes,
        )
    };

    let output_infos = fetch_output_infos(conn, &output_ids, config_timestamp);
    let crtc_infos = fetch_crtc_infos(conn, &crtc_ids, config_timestamp);
    let mut known_outputs = collect_output_rects(&output_infos, &crtc_infos, &modes);
    let mut claimed_crtcs = std::collections::HashSet::new();

    for (output_id, output_info) in &output_infos {
        let output_name = String::from_utf8_lossy(&output_info.name);

        if name != "*" && output_name != name {
            continue;
        }

        if output_info.connection != randr::Connection::CONNECTED {
            continue;
        }
        let crtc = if output_info.crtc != 0 {
            output_info.crtc
        } else {
            output_info
                .crtcs
                .iter()
                .copied()
                .find(|crtc| {
                    !claimed_crtcs.contains(crtc)
                        && crtc_infos
                            .get(crtc)
                            .is_some_and(|info| info.outputs.is_empty())
                })
                .unwrap_or(0)
        };
        if config.enable != Some(false) && crtc != 0 {
            claimed_crtcs.insert(crtc);
        }
        if let Some(rect) = apply_output_config(
            conn,
            root,
            *output_id,
            output_info,
            crtc_infos.get(&crtc),
            crtc,
            config,
            config_timestamp,
            &modes,
            &known_outputs,
        ) && !known_outputs
            .iter()
            .any(|(known, _)| known == &*output_name)
        {
            known_outputs.push((output_name.to_string(), rect));
        }
    }

    true
}

/// Apply configuration to a specific output.
#[allow(clippy::too_many_arguments)]
fn apply_output_config(
    conn: &RustConnection,
    root: Window,
    output_id: randr::Output,
    output_info: &randr::GetOutputInfoReply,
    current_crtc: Option<&randr::GetCrtcInfoReply>,
    crtc: randr::Crtc,
    config: &MonitorConfig,
    config_timestamp: u32,
    modes: &[randr::ModeInfo],
    known_outputs: &[(String, Rect)],
) -> Option<Rect> {
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
        return None;
    }

    let mode = if let Some(ref resolution) = config.resolution {
        parse_resolution(resolution)
            .and_then(|(w, h)| find_mode_by_resolution(modes, w, h))
            .or_else(|| find_preferred_mode(output_info, modes))
    } else {
        current_crtc
            .and_then(|current| modes.iter().find(|mode| mode.id == current.mode).copied())
            .or_else(|| find_preferred_mode(output_info, modes))
    };

    let mode_info = mode?;

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
    } else if let Some((_, rect)) = known_outputs
        .iter()
        .find(|(name, _)| name.as_bytes() == output_info.name.as_slice())
    {
        crate::types::Point::new(rect.x, rect.y)
    } else {
        automatic_output_position(known_outputs)
    };

    if crtc == 0 {
        return None;
    }

    let (Ok(x), Ok(y)) = (i16::try_from(position.x), i16::try_from(position.y)) else {
        log::warn!("RandR output position is outside the protocol range: {position:?}");
        return None;
    };

    let desired_rect = Rect::new(
        position.x,
        position.y,
        i32::from(mode_info.width),
        i32::from(mode_info.height),
    );
    if current_crtc
        .is_some_and(|current| crtc_configuration_matches(current, x, y, mode_info.id, output_id))
    {
        return Some(desired_rect);
    }

    ensure_framebuffer_contains(conn, root, position, mode_info);

    let applied = conn
        .randr_set_crtc_config(
            crtc,
            x11rb::CURRENT_TIME,
            config_timestamp,
            x,
            y,
            mode_info.id,
            randr::Rotation::ROTATE0,
            &[output_id],
        )
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|reply| reply.status == randr::SetConfig::SUCCESS);
    applied.then_some(desired_rect)
}

fn crtc_configuration_matches(
    current: &randr::GetCrtcInfoReply,
    x: i16,
    y: i16,
    mode: randr::Mode,
    output: randr::Output,
) -> bool {
    current.x == x
        && current.y == y
        && current.mode == mode
        && current.rotation == randr::Rotation::ROTATE0
        && current.outputs.as_slice() == [output]
}

/// Unconfigured heads extend the logical desktop without overlapping an
/// existing output. Keep this policy backend-independent in meaning even
/// though RandR performs the native commit here.
fn automatic_output_position(known_outputs: &[(String, Rect)]) -> crate::types::Point {
    crate::types::Point::new(
        known_outputs
            .iter()
            .map(|(_, rect)| rect.x.saturating_add(rect.w))
            .max()
            .unwrap_or(0),
        0,
    )
}

fn ensure_framebuffer_contains(
    conn: &RustConnection,
    root: Window,
    position: crate::types::Point,
    mode: randr::ModeInfo,
) {
    let Some(geometry) = conn
        .get_geometry(root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return;
    };
    let required_width = position
        .x
        .saturating_add(i32::from(mode.width))
        .max(i32::from(geometry.width));
    let required_height = position
        .y
        .saturating_add(i32::from(mode.height))
        .max(i32::from(geometry.height));
    let (Ok(width), Ok(height)) = (
        u16::try_from(required_width),
        u16::try_from(required_height),
    ) else {
        return;
    };
    if width == geometry.width && height == geometry.height {
        return;
    }
    set_framebuffer_size(conn, root, width, height);
}

/// Resize the RandR framebuffer to exactly contain all active outputs. This is
/// called after topology configuration, so unplugging an edge output does not
/// leave applications observing a permanently oversized root window.
pub fn fit_framebuffer_to_active_outputs(conn: &RustConnection, root: Window) {
    let outputs = get_outputs(conn, root);
    if outputs.is_empty() {
        return;
    }
    let width = outputs
        .iter()
        .map(|output| output.rect.x.saturating_add(output.rect.w))
        .max()
        .unwrap_or(1)
        .max(1);
    let height = outputs
        .iter()
        .map(|output| output.rect.y.saturating_add(output.rect.h))
        .max()
        .unwrap_or(1)
        .max(1);
    let (Ok(width), Ok(height)) = (u16::try_from(width), u16::try_from(height)) else {
        return;
    };
    let Some(current) = conn
        .get_geometry(root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return;
    };
    if current.width != width || current.height != height {
        set_framebuffer_size(conn, root, width, height);
    }
}

fn set_framebuffer_size(conn: &RustConnection, root: Window, width: u16, height: u16) {
    let Some(screen) = conn.setup().roots.iter().find(|screen| screen.root == root) else {
        return;
    };
    let mm_width = u32::from(screen.width_in_millimeters)
        .saturating_mul(u32::from(width))
        .checked_div(u32::from(screen.width_in_pixels).max(1))
        .unwrap_or(u32::from(screen.width_in_millimeters));
    let mm_height = u32::from(screen.height_in_millimeters)
        .saturating_mul(u32::from(height))
        .checked_div(u32::from(screen.height_in_pixels).max(1))
        .unwrap_or(u32::from(screen.height_in_millimeters));
    if let Ok(cookie) = conn.randr_set_screen_size(root, width, height, mm_width, mm_height) {
        let _ = cookie.check();
    }
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
    output_infos: &[(randr::Output, randr::GetOutputInfoReply)],
    crtc_infos: &HashMap<randr::Crtc, randr::GetCrtcInfoReply>,
    modes: &[randr::ModeInfo],
) -> Vec<(String, Rect)> {
    let mut outputs = Vec::new();

    for (_, output_info) in output_infos {
        if output_info.connection != randr::Connection::CONNECTED || output_info.crtc == 0 {
            continue;
        }

        let name = String::from_utf8_lossy(&output_info.name).to_string();
        let rect = {
            let Some(crtc_info) = crtc_infos.get(&output_info.crtc) else {
                continue;
            };

            let (w, h) = modes
                .iter()
                .find(|m| m.id == crtc_info.mode)
                .map(|m| (m.width as i32, m.height as i32))
                .unwrap_or((crtc_info.width as i32, crtc_info.height as i32));

            Rect::new(crtc_info.x as i32, crtc_info.y as i32, w, h)
        };

        outputs.push((name, rect));
    }

    outputs
}

#[cfg(test)]
mod refresh_tests {
    use super::{
        automatic_output_position, crtc_configuration_matches, effective_monitor_config,
        mode_refresh_millihertz, planned_automatic_positions,
    };
    use crate::backend::{BackendOutputInfo, BackendVrrSupport};
    use crate::config::config_toml::MonitorConfig;
    use crate::types::{Point, Rect};
    use std::collections::HashMap;

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

    #[test]
    fn automatic_hotplug_layout_extends_right_of_all_active_outputs() {
        let outputs = vec![
            ("DP-1".to_string(), Rect::new(-1280, 0, 1280, 1024)),
            ("eDP-1".to_string(), Rect::new(0, 0, 1920, 1080)),
        ];
        assert_eq!(automatic_output_position(&outputs), Point::new(1920, 0));
        assert_eq!(automatic_output_position(&[]), Point::new(0, 0));
    }

    #[test]
    fn named_monitor_policy_shadows_wildcard_disable() {
        let mut configs = HashMap::new();
        configs.insert(
            "*".to_string(),
            MonitorConfig {
                enable: Some(true),
                ..MonitorConfig::default()
            },
        );
        configs.insert(
            "DP-1".to_string(),
            MonitorConfig {
                enable: Some(false),
                ..MonitorConfig::default()
            },
        );

        assert_eq!(
            effective_monitor_config(&configs, "DP-1").and_then(|config| config.enable),
            Some(false)
        );
        assert_eq!(
            effective_monitor_config(&configs, "HDMI-1").and_then(|config| config.enable),
            Some(true)
        );
    }

    #[test]
    fn unchanged_crtc_configuration_is_a_noop() {
        let current = x11rb::protocol::randr::GetCrtcInfoReply {
            status: x11rb::protocol::randr::SetConfig::SUCCESS,
            sequence: 0,
            length: 0,
            timestamp: 0,
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
            mode: 7,
            rotation: x11rb::protocol::randr::Rotation::ROTATE0,
            rotations: x11rb::protocol::randr::Rotation::ROTATE0,
            outputs: vec![9],
            possible: vec![9],
        };

        assert!(crtc_configuration_matches(&current, 1920, 0, 7, 9));
        assert!(!crtc_configuration_matches(&current, 0, 0, 7, 9));
        assert!(!crtc_configuration_matches(&current, 1920, 0, 8, 9));
    }

    #[test]
    fn automatic_layout_closes_holes_but_preserves_explicit_anchors() {
        let output = |name: &str, rect: Rect| BackendOutputInfo {
            name: name.to_string(),
            rect,
            scale: 1.0,
            vrr_support: BackendVrrSupport::Unsupported,
            vrr_mode: None,
            vrr_enabled: false,
        };
        let outputs = vec![
            output("DP-1", Rect::new(1920, 0, 1920, 1080)),
            output("HDMI-1", Rect::new(5000, 0, 1920, 1080)),
        ];
        assert_eq!(
            planned_automatic_positions(&outputs, &HashMap::new()),
            vec![
                ("DP-1".to_string(), Point::new(0, 0)),
                ("HDMI-1".to_string(), Point::new(1920, 0)),
            ]
        );

        let mut configs = HashMap::new();
        configs.insert(
            "DP-1".to_string(),
            MonitorConfig {
                position: Some("1920,0".to_string()),
                ..MonitorConfig::default()
            },
        );
        assert_eq!(
            planned_automatic_positions(&outputs, &configs),
            vec![("HDMI-1".to_string(), Point::new(3840, 0))]
        );
    }
}
