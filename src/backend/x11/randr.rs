//! X11 XRandR support for display configuration.

use crate::backend::BackendOutputInfo;
use crate::backend::BackendVrrSupport;
use crate::backend::output::{
    MonitorModeRequest, OutputMode, OutputPlacement, OutputPositionSource,
    plan_automatic_output_positions, position_after,
};
use crate::config::config_toml::{MirrorFit, MonitorConfig};
use crate::output_mirror::MirrorMap;
use crate::types::{MonitorPosition, Rect};
use std::collections::{HashMap, HashSet};
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
            mode_refresh_millihertz(mode)
        })
        .max()
}

fn mode_refresh_millihertz(mode: &randr::ModeInfo) -> Option<u32> {
    let mut numerator = u64::from(mode.dot_clock).saturating_mul(1000);
    let mut divisor = u64::from(mode.htotal).checked_mul(u64::from(mode.vtotal))?;
    if numerator == 0 || divisor == 0 {
        return None;
    }

    let flags = u32::from(mode.mode_flags);
    if flags & u32::from(randr::ModeFlag::INTERLACE) != 0 {
        numerator = numerator.saturating_mul(2);
    }
    if flags & u32::from(randr::ModeFlag::DOUBLE_SCAN) != 0 {
        divisor = divisor.saturating_mul(2);
    }

    u32::try_from(numerator / divisor).ok()
}

/// Return every mode advertised by a connected RandR output.
pub fn get_output_modes(
    conn: &RustConnection,
    root: Window,
    output_name: &str,
) -> Vec<crate::backend::output::OutputMode> {
    if let Some(resources) = conn
        .randr_get_screen_resources_current(root)
        .ok()
        .and_then(|request| request.reply().ok())
    {
        let modes = output_modes_from_resources(
            conn,
            &resources.outputs,
            resources.config_timestamp,
            &resources.modes,
            output_name,
        );
        if !modes.is_empty() {
            return modes;
        }
    }

    let Some(resources) = conn
        .randr_get_screen_resources(root)
        .ok()
        .and_then(|request| request.reply().ok())
    else {
        return Vec::new();
    };
    output_modes_from_resources(
        conn,
        &resources.outputs,
        resources.config_timestamp,
        &resources.modes,
        output_name,
    )
}

fn output_modes_from_resources(
    conn: &RustConnection,
    output_ids: &[randr::Output],
    config_timestamp: u32,
    resource_modes: &[randr::ModeInfo],
    output_name: &str,
) -> Vec<crate::backend::output::OutputMode> {
    let Some(output) = fetch_output_infos(conn, output_ids, config_timestamp)
        .into_iter()
        .map(|(_, output)| output)
        .find(|output| {
            output.connection == randr::Connection::CONNECTED
                && String::from_utf8_lossy(&output.name) == output_name
        })
    else {
        return Vec::new();
    };

    let mut modes: Vec<_> = output
        .modes
        .iter()
        .filter_map(|id| resource_modes.iter().find(|mode| mode.id == *id))
        .filter_map(|mode| {
            Some(crate::backend::output::OutputMode {
                width: i32::from(mode.width),
                height: i32::from(mode.height),
                refresh_millihertz: i32::try_from(mode_refresh_millihertz(mode)?).ok()?,
            })
        })
        .collect();
    modes.sort_by_key(|mode| (mode.width, mode.height, mode.refresh_millihertz));
    modes.dedup();
    modes
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
            mirrors: Vec::new(),
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

/// Apply the complete monitor policy and settle the layout.
///
/// Pass 1 configures every output that is not about to mirror a presenting
/// source, pass 2 glues declared mirrors onto the state pass 1 produced and
/// releases heads that stopped mirroring, then automatic outputs are
/// compacted (re-gluing mirrors whose source moved) and the framebuffer is
/// fitted to the result.
pub fn apply_output_policy(
    conn: &RustConnection,
    runtime: &mut crate::backend::x11::X11RuntimeConfig,
    configs: &HashMap<String, MonitorConfig>,
) {
    let root = runtime.root;
    apply_monitor_configs(conn, root, configs);
    apply_mirror_configs(
        conn,
        root,
        configs,
        &mut runtime.mirror_heads,
        &mut runtime.automatic_outputs,
    );
    if compact_automatic_output_layout(
        conn,
        root,
        configs,
        &runtime.automatic_outputs,
        &runtime.mirror_heads,
    ) {
        apply_mirror_configs(
            conn,
            root,
            configs,
            &mut runtime.mirror_heads,
            &mut runtime.automatic_outputs,
        );
    }
    fit_framebuffer_to_active_outputs(conn, root);
}

/// Declared mirrors whose source will present after pass 1 (connected and
/// not disabled by policy). Pass 2 owns their policy; every other head,
/// including a mirror of an absent or disabled source, is configured by
/// pass 1 as an ordinary output.
fn gluable_mirrors(
    configs: &HashMap<String, MonitorConfig>,
    connected: &HashSet<String>,
) -> HashSet<String> {
    MirrorMap::build(configs)
        .0
        .active_pairs(|name| {
            connected.contains(name) && !output_is_explicitly_disabled(configs, name)
        })
        .map(|(mirror, _)| mirror.to_string())
        .collect()
}

/// Pass 1: apply exactly one effective policy per connected output. A named
/// entry shadows the wildcard instead of relying on two order-dependent
/// modesets.
fn apply_monitor_configs(
    conn: &RustConnection,
    root: Window,
    configs: &HashMap<String, MonitorConfig>,
) {
    let connected = connected_output_names(conn, root);
    let gluable = gluable_mirrors(configs, &connected);
    let mut names: Vec<_> = connected.difference(&gluable).collect();
    names.sort();
    for name in names {
        if let Some(config) = effective_monitor_config(configs, name) {
            set_monitor_config(conn, root, name, config);
        }
    }
}

/// Pass 2: point declared mirrors at their presenting source and release the
/// heads that stopped mirroring.
///
/// `mirror_heads` records the heads this policy glued, so only those are ever
/// released; outputs cloned by other tools (`xrandr --same-as`) are left
/// alone and merely fold into one monitor. A mirror whose source is not
/// presenting behaves as an ordinary output until the source returns.
fn apply_mirror_configs(
    conn: &RustConnection,
    root: Window,
    configs: &HashMap<String, MonitorConfig>,
    mirror_heads: &mut HashSet<String>,
    automatic_outputs: &mut HashSet<String>,
) {
    let (mirrors, _) = MirrorMap::build(configs);
    if mirrors.is_empty() && mirror_heads.is_empty() {
        return;
    }
    let connected = connected_output_names(conn, root);
    mirror_heads.retain(|name| connected.contains(name));
    // Sources are never mirrors, so this pass does not move them.
    let active = get_outputs(conn, root);
    let source_rect = |name: &str| {
        active
            .iter()
            .find(|output| output.name == name)
            .map(|output| output.rect)
    };

    for (mirror, target) in mirrors.iter() {
        if !connected.contains(mirror) {
            continue;
        }
        let Some(rect) = source_rect(&target.source) else {
            continue;
        };
        let modes = get_output_modes(conn, root, mirror);
        let policy = mirror_policy_for(configs, mirror, &target.source, rect, &modes);
        set_monitor_config(conn, root, mirror, &policy);
        automatic_outputs.remove(mirror);
        if policy.enable == Some(false) {
            mirror_heads.remove(mirror);
        } else {
            mirror_heads.insert(mirror.clone());
        }
    }

    let mut released: Vec<String> = mirror_heads
        .iter()
        .filter(|name| {
            mirrors
                .source_of(name)
                .is_none_or(|source| source_rect(source).is_none())
        })
        .cloned()
        .collect();
    released.sort();
    for name in released {
        mirror_heads.remove(&name);
        release_mirror_head(conn, root, configs, &name, mirror_heads, automatic_outputs);
    }
}

/// Turn a head that stopped mirroring back into an ordinary output: its own
/// policy, its preferred mode unless configured, and, without a configured
/// position, an automatic place right of the layout.
fn release_mirror_head(
    conn: &RustConnection,
    root: Window,
    configs: &HashMap<String, MonitorConfig>,
    name: &str,
    mirror_heads: &HashSet<String>,
    automatic_outputs: &mut HashSet<String>,
) {
    let mut config = effective_monitor_config(configs, name)
        .cloned()
        .unwrap_or_default();
    if config.enable == Some(false) {
        automatic_outputs.remove(name);
        set_monitor_config(conn, root, name, &config);
        return;
    }
    if config.resolution.is_none() {
        config.resolution = preferred_resolution(conn, root, name);
        config.refresh_rate = None;
    }
    if config.position.is_none() {
        let position = position_after(
            get_outputs(conn, root)
                .into_iter()
                .filter(|output| output.name != name && !mirror_heads.contains(&output.name))
                .map(|output| output.rect),
        );
        config.position = Some(format!("{},{}", position.x, position.y));
        automatic_outputs.insert(name.to_string());
    }
    log::info!("output {name} stopped mirroring and becomes an independent output");
    set_monitor_config(conn, root, name, &config);
}

/// The EDID-preferred resolution of a connected output, as a policy string.
fn preferred_resolution(conn: &RustConnection, root: Window, name: &str) -> Option<String> {
    let resources = conn
        .randr_get_screen_resources_current(root)
        .ok()?
        .reply()
        .ok()?;
    let (_, output) = fetch_output_infos(conn, &resources.outputs, resources.config_timestamp)
        .into_iter()
        .find(|(_, output)| String::from_utf8_lossy(&output.name) == name)?;
    let mode = find_preferred_mode(&output, &resources.modes)?;
    Some(format!("{}x{}", mode.width, mode.height))
}

/// Pure policy for one declared mirror whose source presents `source_rect`:
/// the configuration the mirror must run to show the source.
///
/// X11 cannot scale mirrored content: Xorg shares one framebuffer across
/// CRTCs and instantWM never touches RandR output transforms, so a mirror
/// CRTC always scans out a 1:1 pixel region of the framebuffer at its
/// position. The policy is therefore a best-effort ladder over the mirror's
/// advertised modes, compared against the source's pixel size:
///
/// - An explicit `enable = false` on the mirror's effective policy wins over
///   mirroring.
/// - An exact source-sized mode clones the source rectangle (the panel's own
///   scaler fills a non-native mode, so this is a true scaled mirror).
/// - Otherwise the same-aspect mode with the nearest pixel area that is no
///   larger than the source (tie: higher refresh) shows a centered 1:1 crop
///   of the framebuffer region the source occupies.
/// - Otherwise only larger same-aspect or different-aspect modes exist,
///   which would display source pixels plus neighboring framebuffer
///   garbage; the mirror head is disabled instead. The source is presenting,
///   so this cannot leave a headless desktop.
///
/// The mirror's own `resolution`, `refresh_rate` and `mirror_fit` select a
/// scanout mode and fit on Wayland; X11 has to derive the mode from the
/// source and ignores them.
fn mirror_policy_for(
    configs: &HashMap<String, MonitorConfig>,
    mirror: &str,
    source: &str,
    source_rect: Rect,
    mirror_modes: &[OutputMode],
) -> MonitorConfig {
    let config = effective_monitor_config(configs, mirror);
    if config.is_some_and(|config| config.enable == Some(false)) {
        return MonitorConfig {
            enable: Some(false),
            ..MonitorConfig::default()
        };
    }
    if config.is_some_and(|config| {
        config.resolution.is_some() || config.mirror_fit == Some(MirrorFit::Cover)
    }) {
        log::debug!(
            "mirror output {mirror} configures a mode or fit, which X11 ignores: RandR mirrors follow the source 1:1"
        );
    }

    // Rung 1: an exact mode clones the source rectangle. The panel's scaler
    // fills a non-native mode, so this mirrors the source scaled.
    let rect = source_rect;
    let (source_width, source_height) = (rect.w, rect.h);
    if mirror_modes
        .iter()
        .any(|mode| mode.width == source_width && mode.height == source_height)
    {
        return MonitorConfig {
            enable: Some(true),
            resolution: Some(format!("{source_width}x{source_height}")),
            position: Some(format!("{},{}", rect.x, rect.y)),
            ..MonitorConfig::default()
        };
    }

    // Rung 2: X11 cannot scale, so the closest lossless approximation is a
    // centered 1:1 crop of the framebuffer region the source occupies.
    if let Some(crop_mode) = find_same_aspect_crop_mode(mirror_modes, source_width, source_height) {
        let (crop_width, crop_height) = (crop_mode.width, crop_mode.height);
        log::warn!(
            "mirror output {mirror} has no {source_width}x{source_height} mode for source {source}; running {crop_width}x{crop_height} as an unscaled center crop (X11 mirrors cannot scale)"
        );
        return MonitorConfig {
            enable: Some(true),
            resolution: Some(format!("{crop_width}x{crop_height}")),
            position: Some(format!(
                "{},{}",
                rect.x + (source_width - crop_width) / 2,
                rect.y + (source_height - crop_height) / 2
            )),
            ..MonitorConfig::default()
        };
    }

    // Rung 3: any remaining mode would show source pixels plus neighboring
    // framebuffer garbage, so switch the mirror head off instead.
    log::error!(
        "mirror output {mirror} has no mode compatible with source {source} ({source_width}x{source_height}); disabling it because X11 cannot scale a larger or different-aspect mode"
    );
    MonitorConfig {
        enable: Some(false),
        ..MonitorConfig::default()
    }
}

/// The [`OutputMode`] a mirror runs for a centered 1:1 crop: the same-aspect
/// mode with the pixel area nearest to the source's, among modes no larger
/// than the source, with the higher refresh breaking area ties.
///
/// The aspect test is an integer cross-multiplication on raw pixel sizes —
/// X11 has no transform or fractional-scale concept (scale is hardcoded to
/// 1.0), so unlike the Wayland ladder there is nothing transform-adjusted to
/// compare. Equal aspect plus an area no larger than the source's forces
/// both mode dimensions to be no larger than the source's, which keeps the
/// crop's centering offset non-negative.
fn find_same_aspect_crop_mode(
    mirror_modes: &[OutputMode],
    source_width: i32,
    source_height: i32,
) -> Option<&OutputMode> {
    let source_area = i64::from(source_width) * i64::from(source_height);
    mirror_modes
        .iter()
        .filter(|mode| {
            i64::from(mode.width) * i64::from(source_height)
                == i64::from(mode.height) * i64::from(source_width)
        })
        .filter(|mode| i64::from(mode.width) * i64::from(mode.height) <= source_area)
        .min_by_key(|mode| {
            (
                (i64::from(mode.width) * i64::from(mode.height)).abs_diff(source_area),
                std::cmp::Reverse(mode.refresh_millihertz),
            )
        })
}

/// Return physical connector identity independently of active CRTC state.
pub fn connected_output_names(conn: &RustConnection, root: Window) -> HashSet<String> {
    let Some(resources) = conn
        .randr_get_screen_resources_current(root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return HashSet::new();
    };
    fetch_output_infos(conn, &resources.outputs, resources.config_timestamp)
        .into_iter()
        .filter(|(_, output)| output.connection == randr::Connection::CONNECTED)
        .map(|(_, output)| String::from_utf8_lossy(&output.name).into_owned())
        .collect()
}

pub fn active_output_names(conn: &RustConnection, root: Window) -> HashSet<String> {
    get_outputs(conn, root)
        .into_iter()
        .map(|output| output.name)
        .collect()
}

/// Attempt automatic activation only for connectors that the runtime has
/// identified as physically new. Returns the newly active outputs whose
/// placement is owned by the automatic policy.
///
/// Declared mirrors of a connected source are left to
/// [`apply_output_policy`], which glues them onto their source instead of
/// giving them a placement.
pub fn configure_new_outputs(
    conn: &RustConnection,
    root: Window,
    configs: &HashMap<String, MonitorConfig>,
    candidates: &HashSet<String>,
) -> HashSet<String> {
    let Some(resources) = conn
        .randr_get_screen_resources_current(root)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return HashSet::new();
    };
    let connected = connected_output_names(conn, root);
    let gluable = gluable_mirrors(configs, &connected);
    let output_infos = fetch_output_infos(conn, &resources.outputs, resources.config_timestamp);
    for (_, output) in output_infos.iter().filter(|(_, output)| {
        let name = String::from_utf8_lossy(&output.name);
        output.connection == randr::Connection::CONNECTED
            && output.crtc == 0
            && candidates.contains(name.as_ref())
            && !gluable.contains(name.as_ref())
    }) {
        let name = String::from_utf8_lossy(&output.name);
        let config = effective_monitor_config(configs, &name)
            .cloned()
            .unwrap_or_default();
        if config.enable == Some(false) {
            continue;
        }
        set_monitor_config(conn, root, &name, &config);
    }

    let active = active_output_names(conn, root);
    candidates
        .iter()
        .filter(|name| active.contains(*name) && !gluable.contains(*name))
        .filter(|name| {
            effective_monitor_config(configs, name).is_none_or(|config| config.position.is_none())
        })
        .cloned()
        .collect()
}

pub fn output_is_explicitly_disabled(configs: &HashMap<String, MonitorConfig>, name: &str) -> bool {
    effective_monitor_config(configs, name).is_some_and(|config| config.enable == Some(false))
}

pub fn new_auto_enable_candidates(
    previous_connected: &HashSet<String>,
    connected: &HashSet<String>,
    active: &HashSet<String>,
    configs: &HashMap<String, MonitorConfig>,
) -> HashSet<String> {
    connected
        .difference(previous_connected)
        .filter(|name| !active.contains(*name) && !output_is_explicitly_disabled(configs, name))
        .cloned()
        .collect()
}

fn effective_monitor_config<'a>(
    configs: &'a HashMap<String, MonitorConfig>,
    output_name: &str,
) -> Option<&'a MonitorConfig> {
    configs.get(output_name).or_else(|| configs.get("*"))
}

/// Close holes left by removed automatically positioned outputs. Outputs with
/// an explicit named or wildcard position anchor the layout and are never
/// moved by this policy. Returns whether any output moved.
fn compact_automatic_output_layout(
    conn: &RustConnection,
    root: Window,
    configs: &HashMap<String, MonitorConfig>,
    automatic_outputs: &HashSet<String>,
    mirror_heads: &HashSet<String>,
) -> bool {
    let outputs = get_outputs(conn, root);
    let moves = planned_automatic_positions(&outputs, configs, automatic_outputs, mirror_heads);
    for (name, position) in &moves {
        let config = MonitorConfig {
            position: Some(format!("{},{}", position.x, position.y)),
            ..MonitorConfig::default()
        };
        set_monitor_config(conn, root, name, &config);
    }
    !moves.is_empty()
}

fn planned_automatic_positions(
    outputs: &[BackendOutputInfo],
    configs: &HashMap<String, MonitorConfig>,
    automatic_outputs: &HashSet<String>,
    mirror_heads: &HashSet<String>,
) -> Vec<(String, crate::types::Point)> {
    let mut placements: Vec<_> = outputs
        .iter()
        // A mirror head presents its source's region; planning it as a
        // placement would mark that region occupied and shift the source.
        .filter(|output| !mirror_heads.contains(&output.name))
        .map(|output| {
            let automatic = automatic_outputs.contains(&output.name)
                && effective_monitor_config(configs, &output.name)
                    .is_none_or(|config| config.position.is_none());
            OutputPlacement {
                id: output.name.clone(),
                rect: output.rect,
                source: if automatic {
                    OutputPositionSource::Automatic
                } else {
                    OutputPositionSource::ClientManaged
                },
            }
        })
        .collect();
    plan_automatic_output_positions(&mut placements)
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
            &crtc_infos,
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
    crtc_infos: &HashMap<randr::Crtc, randr::GetCrtcInfoReply>,
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
            let remaining: Vec<_> = current_crtc
                .map(|info| {
                    info.outputs
                        .iter()
                        .copied()
                        .filter(|output| *output != output_id)
                        .collect()
                })
                .unwrap_or_default();
            let (x, y, mode, rotation) = if let Some(info) = current_crtc
                && !remaining.is_empty()
            {
                (info.x, info.y, info.mode, info.rotation)
            } else {
                (0, 0, 0, randr::Rotation::ROTATE0)
            };
            let _ = conn.randr_set_crtc_config(
                output_info.crtc,
                x11rb::CURRENT_TIME,
                config_timestamp,
                x,
                y,
                mode,
                rotation,
                &remaining,
            );
        }
        return None;
    }

    let mode = select_output_mode(
        output_info,
        current_crtc.map(|current| current.mode),
        config,
        modes,
    );

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
        position_after(known_outputs.iter().map(|(_, rect)| *rect))
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
    let mut crtc = crtc;
    if let Some(current) = current_crtc
        && current.outputs.len() > 1
    {
        if current.x == x
            && current.y == y
            && current.mode == mode_info.id
            && current.rotation == randr::Rotation::ROTATE0
            && current.outputs.contains(&output_id)
        {
            return Some(desired_rect);
        }
        // RandR replaces a CRTC's entire outputs array. Move this output to
        // a free compatible CRTC before changing its mode or location, so the
        // other heads on its current CRTC keep scanning out.
        let Some(spare) = output_info.crtcs.iter().copied().find(|candidate| {
            crtc_infos
                .get(candidate)
                .is_some_and(|info| info.outputs.is_empty())
        }) else {
            log::warn!(
                "cannot reconfigure output {}: it shares CRTC {crtc} and no free compatible CRTC exists",
                String::from_utf8_lossy(&output_info.name)
            );
            return None;
        };
        crtc = spare;
    }
    if crtc == output_info.crtc
        && current_crtc.is_some_and(|current| {
            crtc_configuration_matches(current, x, y, mode_info.id, output_id)
        })
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

/// Select the requested mode while preserving an active mode when the exact
/// request is unavailable. This matches the Wayland backend's behavior and
/// avoids unexpectedly switching to a preferred mode with another resolution.
fn select_output_mode(
    output_info: &randr::GetOutputInfoReply,
    current_mode: Option<randr::Mode>,
    config: &MonitorConfig,
    modes: &[randr::ModeInfo],
) -> Option<randr::ModeInfo> {
    let requested = config
        .resolution
        .as_deref()
        .and_then(|resolution| MonitorModeRequest::parse(resolution, config.refresh_rate))
        .and_then(|request| find_mode_by_resolution(output_info, modes, request));
    let current = current_mode.and_then(|id| modes.iter().find(|mode| mode.id == id).copied());

    requested
        .or(current)
        .or_else(|| find_preferred_mode(output_info, modes))
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

/// Find an advertised mode matching the shared monitor configuration policy.
/// The caller preserves the current mode when no requested mode matches.
fn find_mode_by_resolution(
    output_info: &randr::GetOutputInfoReply,
    modes: &[randr::ModeInfo],
    request: MonitorModeRequest,
) -> Option<randr::ModeInfo> {
    output_info
        .modes
        .iter()
        .filter_map(|id| modes.iter().find(|mode| mode.id == *id))
        .find(|mode| {
            request.matches(
                i32::from(mode.width),
                i32::from(mode.height),
                mode_refresh_millihertz(mode),
            )
        })
        .copied()
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
        crtc_configuration_matches, effective_monitor_config, find_mode_by_resolution,
        find_same_aspect_crop_mode, mirror_policy_for, mode_refresh_millihertz,
        new_auto_enable_candidates, planned_automatic_positions, select_output_mode,
    };
    use crate::backend::output::{MonitorModeRequest, OutputMode};
    use crate::backend::{BackendOutputInfo, BackendVrrSupport};
    use crate::config::config_toml::{MirrorFit, MonitorConfig};
    use crate::types::{Point, Rect};
    use std::collections::{HashMap, HashSet};

    #[test]
    fn calculates_standard_and_high_refresh_modes() {
        assert_eq!(
            mode_refresh_millihertz(&test_mode(1, 1920, 1080, 148_500_000, 2200, 1125)),
            Some(60_000)
        );
        assert_eq!(
            mode_refresh_millihertz(&test_mode(2, 2560, 1440, 585_953_280, 2720, 1496)),
            Some(144_000)
        );
    }

    #[test]
    fn adjusts_refresh_for_interlaced_and_doublescan_modes() {
        let mut mode = test_mode(1, 1920, 1080, 74_250_000, 2200, 1125);
        mode.mode_flags = x11rb::protocol::randr::ModeFlag::INTERLACE;
        assert_eq!(mode_refresh_millihertz(&mode), Some(60_000));

        mode.dot_clock = 148_500_000;
        mode.mode_flags = x11rb::protocol::randr::ModeFlag::DOUBLE_SCAN;
        assert_eq!(mode_refresh_millihertz(&mode), Some(30_000));
    }

    #[test]
    fn rejects_incomplete_mode_timings() {
        assert_eq!(
            mode_refresh_millihertz(&test_mode(1, 1920, 1080, 0, 2200, 1125)),
            None
        );
        assert_eq!(
            mode_refresh_millihertz(&test_mode(1, 1920, 1080, 148_500_000, 0, 1125)),
            None
        );
    }

    fn test_mode(
        id: u32,
        width: u16,
        height: u16,
        dot_clock: u32,
        htotal: u16,
        vtotal: u16,
    ) -> x11rb::protocol::randr::ModeInfo {
        x11rb::protocol::randr::ModeInfo {
            id,
            width,
            height,
            dot_clock,
            htotal,
            vtotal,
            ..Default::default()
        }
    }

    #[test]
    fn refresh_rate_request_selects_matching_mode() {
        // 1920x1080 timings sharing the same blanking: 148.5 MHz is 60 Hz,
        // 356.4 MHz is 144 Hz.
        let modes = vec![
            test_mode(1, 1920, 1080, 148_500_000, 2200, 1125),
            test_mode(2, 1920, 1080, 356_400_000, 2200, 1125),
        ];
        let output_info = x11rb::protocol::randr::GetOutputInfoReply {
            modes: vec![1, 2],
            ..Default::default()
        };
        assert_eq!(
            find_mode_by_resolution(
                &output_info,
                &modes,
                MonitorModeRequest::parse("1920x1080", Some(144.0)).unwrap()
            )
            .map(|mode| mode.id),
            Some(2)
        );
        assert_eq!(
            find_mode_by_resolution(
                &output_info,
                &modes,
                MonitorModeRequest::parse("1920x1080", Some(60.0)).unwrap()
            )
            .map(|mode| mode.id),
            Some(1)
        );
    }

    #[test]
    fn unset_refresh_rate_keeps_first_match_and_unknown_rate_falls_back() {
        let modes = vec![
            test_mode(1, 1920, 1080, 148_500_000, 2200, 1125),
            test_mode(2, 1920, 1080, 356_400_000, 2200, 1125),
        ];
        let output_info = x11rb::protocol::randr::GetOutputInfoReply {
            modes: vec![1, 2],
            ..Default::default()
        };
        assert_eq!(
            find_mode_by_resolution(
                &output_info,
                &modes,
                MonitorModeRequest::parse("1920x1080", None).unwrap()
            )
            .map(|mode| mode.id),
            Some(1)
        );
        assert_eq!(
            find_mode_by_resolution(
                &output_info,
                &modes,
                MonitorModeRequest::parse("1920x1080", Some(165.0)).unwrap()
            )
            .map(|mode| mode.id),
            None
        );
    }

    #[test]
    fn requested_mode_must_be_advertised_by_the_output() {
        let modes = vec![
            test_mode(1, 1920, 1080, 148_500_000, 2200, 1125),
            test_mode(2, 2560, 1440, 585_953_280, 2720, 1496),
        ];
        let output_info = x11rb::protocol::randr::GetOutputInfoReply {
            modes: vec![2],
            ..Default::default()
        };
        let request = MonitorModeRequest::parse("1920x1080", None).unwrap();
        assert!(find_mode_by_resolution(&output_info, &modes, request).is_none());
    }

    #[test]
    fn unavailable_requested_mode_preserves_current_mode() {
        let modes = vec![
            test_mode(1, 1920, 1080, 148_500_000, 2200, 1125),
            test_mode(2, 3840, 2160, 594_000_000, 4400, 2250),
        ];
        let output_info = x11rb::protocol::randr::GetOutputInfoReply {
            modes: vec![2, 1],
            ..Default::default()
        };
        let config = MonitorConfig {
            resolution: Some("1920x1080".to_string()),
            refresh_rate: Some(165.0),
            ..Default::default()
        };

        assert_eq!(
            select_output_mode(&output_info, Some(1), &config, &modes).map(|mode| mode.id),
            Some(1)
        );
        assert_eq!(
            select_output_mode(&output_info, None, &config, &modes).map(|mode| mode.id),
            Some(2)
        );
    }

    #[test]
    fn refresh_rate_tolerance_matches_wayland_backend() {
        let sixty = test_mode(1, 1920, 1080, 148_500_000, 2200, 1125);
        let matches = |refresh| {
            MonitorModeRequest::parse("1920x1080", refresh)
                .unwrap()
                .matches(1920, 1080, mode_refresh_millihertz(&sixty))
        };
        assert!(matches(Some(60.0)));
        assert!(matches(Some(59.94)));
        assert!(!matches(Some(165.0)));
        assert!(matches(None));
        let broken = test_mode(2, 1920, 1080, 0, 2200, 1125);
        assert!(
            !MonitorModeRequest::parse("1920x1080", Some(60.0))
                .unwrap()
                .matches(1920, 1080, mode_refresh_millihertz(&broken))
        );
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
            mirrors: Vec::new(),
        };
        let outputs = vec![
            output("DP-1", Rect::new(1920, 0, 1920, 1080)),
            output("HDMI-1", Rect::new(5000, 0, 1920, 1080)),
        ];
        let automatic: HashSet<_> = ["DP-1".to_string(), "HDMI-1".to_string()]
            .into_iter()
            .collect();
        assert_eq!(
            planned_automatic_positions(&outputs, &HashMap::new(), &automatic, &HashSet::new()),
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
            planned_automatic_positions(&outputs, &configs, &automatic, &HashSet::new()),
            vec![("HDMI-1".to_string(), Point::new(3840, 0))]
        );
    }

    #[test]
    fn external_crtc_disable_is_not_a_new_connector() {
        let connected: HashSet<_> = ["DP-1".to_string()].into_iter().collect();
        assert!(
            new_auto_enable_candidates(&connected, &connected, &HashSet::new(), &HashMap::new(),)
                .is_empty()
        );
        assert_eq!(
            new_auto_enable_candidates(
                &HashSet::new(),
                &connected,
                &HashSet::new(),
                &HashMap::new(),
            ),
            connected
        );
    }

    fn output(name: &str, rect: Rect) -> BackendOutputInfo {
        BackendOutputInfo {
            name: name.to_string(),
            rect,
            scale: 1.0,
            vrr_support: BackendVrrSupport::Unsupported,
            vrr_mode: None,
            vrr_enabled: false,
            mirrors: Vec::new(),
        }
    }

    /// `DP-1` declares `mirror = "eDP-1"` unless overridden by the caller.
    fn mirror_configs(
        extra: impl IntoIterator<Item = (&'static str, MonitorConfig)>,
    ) -> HashMap<String, MonitorConfig> {
        let mut configs: HashMap<String, MonitorConfig> = extra
            .into_iter()
            .map(|(name, config)| (name.to_string(), config))
            .collect();
        configs.entry("DP-1".to_string()).or_insert(MonitorConfig {
            mirror: Some("eDP-1".to_string()),
            ..MonitorConfig::default()
        });
        configs
    }

    fn mode(width: i32, height: i32, refresh_millihertz: i32) -> OutputMode {
        OutputMode {
            width,
            height,
            refresh_millihertz,
        }
    }

    #[test]
    fn mirror_policy_explicit_disable_wins() {
        let configs = mirror_configs([(
            "DP-1",
            MonitorConfig {
                mirror: Some("eDP-1".to_string()),
                enable: Some(false),
                ..MonitorConfig::default()
            },
        )]);
        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            Rect::new(0, 0, 1920, 1080),
            &[mode(1920, 1080, 60_000)],
        );
        assert_eq!(policy.enable, Some(false));
    }

    #[test]
    fn mirror_policy_follows_the_source_rectangle() {
        let configs = mirror_configs([]);
        let modes = [mode(1920, 1080, 60_000), mode(2560, 1440, 144_000)];

        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            Rect::new(1920, 40, 2560, 1440),
            &modes,
        );
        assert_eq!(policy.enable, Some(true));
        assert_eq!(policy.resolution.as_deref(), Some("2560x1440"));
        assert_eq!(policy.position.as_deref(), Some("1920,40"));
    }

    #[test]
    fn mirror_policy_crops_centered_when_only_smaller_same_aspect_modes_exist() {
        let configs = mirror_configs([]);
        let modes = [mode(1280, 720, 144_000), mode(1920, 1080, 60_000)];

        // X11 cannot scale, so the nearest smaller 16:9 mode shows a
        // centered 1:1 crop of the framebuffer region the source occupies:
        // (1920, 40) + ((2560 - 1920) / 2, (1440 - 1080) / 2).
        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            Rect::new(1920, 40, 2560, 1440),
            &modes,
        );
        assert_eq!(policy.enable, Some(true));
        assert_eq!(policy.resolution.as_deref(), Some("1920x1080"));
        assert_eq!(policy.position.as_deref(), Some("2240,220"));
    }

    #[test]
    fn mirror_policy_disables_mirror_without_a_compatible_mode() {
        let configs = mirror_configs([]);

        // Only larger same-aspect modes: a 1:1 CRTC would also show
        // neighboring framebuffer content, so the head is switched off.
        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            Rect::new(0, 0, 1600, 900),
            &[mode(1920, 1080, 60_000)],
        );
        assert_eq!(policy.enable, Some(false));

        // No same-aspect mode at all: same outcome.
        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            Rect::new(0, 0, 2560, 1440),
            &[mode(1920, 1200, 60_000)],
        );
        assert_eq!(policy.enable, Some(false));
    }

    #[test]
    fn mirror_policy_ignores_mode_and_fit_on_x11() {
        // A mirror's own mode and fit apply on Wayland only; on X11 they
        // must neither change the ladder's outcome nor error.
        let configs = mirror_configs([(
            "DP-1",
            MonitorConfig {
                mirror: Some("eDP-1".to_string()),
                mirror_fit: Some(MirrorFit::Cover),
                resolution: Some("1280x720".to_string()),
                ..MonitorConfig::default()
            },
        )]);
        let source = Rect::new(0, 0, 2560, 1440);

        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            source,
            &[mode(1280, 720, 60_000), mode(2560, 1440, 60_000)],
        );
        assert_eq!(policy.resolution.as_deref(), Some("2560x1440"));
        assert_eq!(policy.position.as_deref(), Some("0,0"));

        let policy = mirror_policy_for(
            &configs,
            "DP-1",
            "eDP-1",
            source,
            &[mode(1280, 720, 60_000)],
        );
        assert_eq!(policy.resolution.as_deref(), Some("1280x720"));
        assert_eq!(policy.position.as_deref(), Some("640,360"));
    }

    #[test]
    fn crop_mode_picker_prefers_nearest_area_then_higher_refresh() {
        // The nearest area wins even against a smaller higher-refresh mode.
        let modes = vec![mode(1280, 720, 144_000), mode(1920, 1080, 60_000)];
        let picked = find_same_aspect_crop_mode(&modes, 2560, 1440).unwrap();
        assert_eq!((picked.width, picked.height), (1920, 1080));

        // Area ties (duplicate resolution at two refresh rates) resolve to
        // the higher refresh.
        let modes = vec![mode(1280, 720, 60_000), mode(1280, 720, 144_000)];
        let picked = find_same_aspect_crop_mode(&modes, 2560, 1440).unwrap();
        assert_eq!(picked.refresh_millihertz, 144_000);
    }

    #[test]
    fn planned_positions_exclude_mirror_heads_so_the_source_keeps_its_rect() {
        // The mirror shares its source's rectangle and sorts before it, so
        // planning it would mark the rectangle occupied and shift the source
        // right, displacing whatever sits under the cursor.
        let outputs = vec![
            output("DP-1", Rect::new(0, 0, 1920, 1080)),
            output("eDP-1", Rect::new(0, 0, 1920, 1080)),
            output("HDMI-1", Rect::new(3840, 0, 1920, 1080)),
        ];
        let automatic: HashSet<_> = ["DP-1", "eDP-1", "HDMI-1"]
            .iter()
            .map(|name| name.to_string())
            .collect();
        let mirror_heads: HashSet<_> = ["DP-1".to_string()].into_iter().collect();

        assert_eq!(
            planned_automatic_positions(&outputs, &HashMap::new(), &automatic, &mirror_heads),
            vec![("HDMI-1".to_string(), Point::new(1920, 0))]
        );
    }

    #[test]
    fn a_released_mirror_takes_part_in_placement_again() {
        // Once released, a former mirror is an ordinary automatic output.
        let outputs = vec![
            output("eDP-1", Rect::new(0, 0, 1920, 1080)),
            output("DP-1", Rect::new(3840, 0, 1920, 1080)),
        ];
        let automatic: HashSet<_> = ["DP-1".to_string()].into_iter().collect();

        assert_eq!(
            planned_automatic_positions(&outputs, &HashMap::new(), &automatic, &HashSet::new()),
            vec![("DP-1".to_string(), Point::new(1920, 0))]
        );
    }
}
