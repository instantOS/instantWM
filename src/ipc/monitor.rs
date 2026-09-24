use crate::backend::OutputOps;
use crate::config::config_toml::MonitorConfig;
use crate::ipc_types::{MonitorCommand, Response};
use crate::monitor::{focus_monitor, resolve_monitor_selector};
use crate::output_mirror::{MirrorConfigError, MirrorMap};
use crate::types::{MonitorDirection, MonitorSelector};
use crate::wm::Wm;
use std::collections::HashMap;
pub fn handle_monitor_command(wm: &mut Wm, cmd: MonitorCommand) -> Response {
    match cmd {
        MonitorCommand::List => list_monitors(wm),
        MonitorCommand::Switch { monitor } => switch_monitor(wm, monitor),
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
            mirror,
            mirror_fit,
        } => {
            let patch = MonitorConfig {
                resolution,
                refresh_rate,
                position,
                scale,
                transform: transform.map(|t| t.to_string()),
                enable,
                vrr,
                mirror,
                mirror_fit,
            };
            set_monitor_config(wm, identifier, patch)
        }
        MonitorCommand::Modes { identifier } => list_modes(wm, identifier),
    }
}

fn list_monitors(wm: &Wm) -> Response {
    let selected_id = wm.core.model.selected_monitor_id();
    let mirror_map = &wm.core.derived.monitor_policy.mirrors;
    // The same discovery the monitor layout uses, so each monitor finds its
    // own output (with the heads presenting it) by name.
    let output_info: HashMap<_, _> =
        crate::monitor::logical_outputs(wm.backend.get_outputs(), mirror_map, &wm.core.model)
            .into_iter()
            .map(|output| (output.name.clone(), output))
            .collect();

    let monitors: Vec<crate::ipc_types::MonitorInfo> = wm
        .core
        .model
        .monitors_iter()
        .enumerate()
        .map(|(pos, (id, m))| {
            let output = output_info.get(&m.name);
            let requested_mirrors: Vec<String> = mirror_map
                .iter()
                .filter(|(_, target)| target.source == m.name)
                .map(|(name, _)| name.clone())
                .collect();
            crate::ipc_types::MonitorInfo {
                id: id.get(),
                position: pos,
                backend_index: m.num,
                name: m.name.clone(),
                width: m.monitor_rect.w,
                height: m.monitor_rect.h,
                x: m.monitor_rect.x,
                y: m.monitor_rect.y,
                is_selected: id == selected_id,
                vrr_support: output
                    .map(|o| o.vrr_support)
                    .unwrap_or(crate::backend::BackendVrrSupport::Unsupported),
                vrr_mode: output.and_then(|o| o.vrr_mode),
                vrr_enabled: output.is_some_and(|o| o.vrr_enabled),
                mirrors: output.map(|o| o.mirrors.clone()).unwrap_or_default(),
                requested_mirrors,
            }
        })
        .collect();

    Response::MonitorList(monitors)
}

fn switch_monitor(wm: &mut Wm, selector: MonitorSelector) -> Response {
    if matches!(selector, MonitorSelector::Any) {
        return Response::err(
            "monitor switch needs a concrete monitor: name, position, \"focused\" or \"primary\"",
        );
    }
    match resolve_monitor_selector(&wm.core.model, &selector) {
        Some(target) => {
            let changed = crate::focus::select_monitor(&mut wm.ctx(), target);
            if changed {
                crate::mouse::warp::warp_pointer_to_monitor(&mut wm.ctx(), target);
            }
            Response::ok()
        }
        None => Response::err(format!(
            "monitor '{selector}' does not match any connected monitor"
        )),
    }
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

/// Merge a `monitor set` patch onto a stored monitor config.
///
/// `Some` patch fields overwrite the stored value, `None` keeps it. The
/// `mirror` field follows the IPC sentinel protocol: `None` keeps the
/// current mirror, `Some("")` (the CLI `--mirror none` mapping) clears it,
/// and `Some(target)` declares a new mirror, dropping the shadowed `position`
/// and `scale`: a mirror head presents its source's region and owns neither.
/// Its mode and transform stay, since they describe the physical head.
///
/// `mirror_fit` merges like the other kept fields and is deliberately NOT
/// part of the shadowed set, so it survives mirror re-declarations. Clearing
/// the mirror also clears the fit: a fit without a mirror is meaningless
/// (apply-time sanitization would drop it anyway).
fn merge_monitor_config(existing: Option<&MonitorConfig>, patch: MonitorConfig) -> MonitorConfig {
    let mut candidate = existing.cloned().unwrap_or_default();
    if patch.resolution.is_some() {
        candidate.resolution = patch.resolution;
    }
    if patch.refresh_rate.is_some() {
        candidate.refresh_rate = patch.refresh_rate;
    }
    if patch.position.is_some() {
        candidate.position = patch.position;
    }
    if patch.scale.is_some() {
        candidate.scale = patch.scale;
    }
    if patch.transform.is_some() {
        candidate.transform = patch.transform;
    }
    if patch.enable.is_some() {
        candidate.enable = patch.enable;
    }
    if patch.vrr.is_some() {
        candidate.vrr = patch.vrr;
    }
    if patch.mirror_fit.is_some() {
        candidate.mirror_fit = patch.mirror_fit;
    }
    match patch.mirror {
        None => {}
        Some(target) if target.is_empty() => {
            candidate.mirror = None;
            candidate.mirror_fit = None;
        }
        Some(target) => {
            candidate.mirror = Some(target);
            candidate.position = None;
            candidate.scale = None;
        }
    }
    candidate
}

/// Merge `patch` into the stored entry for `identifier` and validate the
/// result before committing it.
///
/// Validation runs against the prospective config map: fatal
/// [`MirrorConfigError`]s keyed to the edited entry (including the wildcard
/// `"*"` entry when the identifier resolves to it) reject the command, and a
/// mirror target declared by the patch must name a connected output. Nothing
/// is stored and no apply is queued until both checks pass; fatal findings
/// about *other* entries are left to degrade at apply time, where
/// sanitization logs them.
fn set_monitor_config(wm: &mut Wm, identifier: String, patch: MonitorConfig) -> Response {
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

    // The merge consumes the patch; remember whether it declares a mirror,
    // since only that case is connectivity-checked below.
    let declared_mirror = patch.mirror.clone();
    let candidate = merge_monitor_config(wm.core.config.monitors.get(&resolved_id), patch);

    let mut prospective = wm.core.config.monitors.clone();
    prospective.insert(resolved_id.clone(), candidate.clone());
    let (_, errors) = MirrorMap::build(&prospective);
    for error in errors {
        if error.is_fatal() && error.declaration_key() == Some(resolved_id.as_str()) {
            return Response::err(format!("{error}"));
        }
    }

    // Only a target declared by this patch is checked for connectivity: a
    // command that leaves the mirror untouched or clears it must keep
    // working while the current source is unplugged.
    if let Some(target) = declared_mirror.as_deref()
        && !target.is_empty()
    {
        let connected = wm.backend.connected_output_names();
        if !connected.iter().any(|name| name == target) {
            let error = MirrorConfigError::SourceNotConnected {
                output: resolved_id.clone(),
                r#source: target.to_string(),
            };
            return Response::err(format!("{error} (connected: {})", connected.join(", ")));
        }
    }

    wm.core.config.monitors.insert(resolved_id, candidate);
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
    use crate::ipc_types::{MonitorCommand, Response};
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

    #[test]
    fn monitor_switch_resolves_output_name() {
        use crate::ipc_types::MonitorCommand;
        use crate::types::MonitorSelector;

        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.model.monitors.push(Monitor::default());
        let side_id = wm.core.model.monitors.push(Monitor {
            name: "DP-1".to_owned(),
            ..Monitor::default()
        });

        let resp = super::handle_monitor_command(
            &mut wm,
            MonitorCommand::Switch {
                monitor: MonitorSelector::Name("DP-1".to_owned()),
            },
        );
        assert!(matches!(resp, Response::Ok), "{resp:?}");
        assert_eq!(wm.core.model.selected_monitor_id(), side_id);

        let resp = super::handle_monitor_command(
            &mut wm,
            MonitorCommand::Switch {
                monitor: MonitorSelector::Name("HDMI-9".to_owned()),
            },
        );
        assert!(matches!(resp, Response::Err(_)), "{resp:?}");
    }

    #[test]
    fn focused_switch_requires_a_connected_monitor() {
        use crate::ipc_types::MonitorCommand;
        use crate::types::MonitorSelector;

        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let resp = super::handle_monitor_command(
            &mut wm,
            MonitorCommand::Switch {
                monitor: MonitorSelector::Focused,
            },
        );
        assert!(matches!(resp, Response::Err(_)), "{resp:?}");
    }

    /// All-`None` `monitor set` command with just the fields the tests vary.
    fn set_cmd(
        identifier: &str,
        resolution: Option<&str>,
        scale: Option<f32>,
        mirror: Option<&str>,
    ) -> MonitorCommand {
        MonitorCommand::Set {
            identifier: identifier.to_owned(),
            resolution: resolution.map(str::to_string),
            refresh_rate: None,
            position: None,
            scale,
            transform: None,
            enable: None,
            vrr: None,
            mirror: mirror.map(str::to_string),
            mirror_fit: None,
        }
    }

    /// All-`None` `monitor set` command varying only the mirror fields.
    fn set_mirror_cmd(
        identifier: &str,
        mirror: Option<&str>,
        mirror_fit: Option<crate::ipc_types::MirrorFit>,
    ) -> MonitorCommand {
        MonitorCommand::Set {
            identifier: identifier.to_owned(),
            resolution: None,
            refresh_rate: None,
            position: None,
            scale: None,
            transform: None,
            enable: None,
            vrr: None,
            mirror: mirror.map(str::to_string),
            mirror_fit,
        }
    }

    #[test]
    fn monitor_set_merges_instead_of_replacing() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));

        let resp = super::handle_monitor_command(&mut wm, set_cmd("DP-1", None, Some(2.0), None));
        assert!(matches!(resp, Response::Ok), "{resp:?}");
        let resp =
            super::handle_monitor_command(&mut wm, set_cmd("DP-1", Some("2560x1440"), None, None));
        assert!(matches!(resp, Response::Ok), "{resp:?}");

        let config = wm.core.config.monitors.get("DP-1").expect("entry");
        assert_eq!(config.resolution.as_deref(), Some("2560x1440"));
        assert_eq!(config.scale, Some(2.0), "omitted field was dropped");
        assert!(wm.work.monitor_config);
    }

    #[test]
    fn monitor_set_mirror_to_unplugged_output_is_rejected() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let resp = super::handle_monitor_command(&mut wm, set_cmd("DP-1", None, Some(2.0), None));
        assert!(matches!(resp, Response::Ok), "{resp:?}");
        wm.work.monitor_config = false;

        // A fresh WaylandBackend reports no outputs, so no target connects.
        let resp =
            super::handle_monitor_command(&mut wm, set_cmd("DP-1", None, None, Some("HDMI-1")));
        let Response::Err(message) = resp else {
            panic!("expected error, got {resp:?}");
        };
        assert!(message.contains("not connected"), "{message}");
        assert!(message.contains("connected:"), "{message}");

        // The rejected command must not touch config or queue an apply.
        let config = wm.core.config.monitors.get("DP-1").expect("entry");
        assert_eq!(config.mirror, None);
        assert_eq!(config.scale, Some(2.0));
        assert!(!wm.work.monitor_config);
    }

    #[test]
    fn monitor_set_keeps_working_while_a_stored_mirror_source_is_unplugged() {
        use crate::config::config_toml::MonitorConfig;

        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.config.monitors.insert(
            "DP-1".to_owned(),
            MonitorConfig {
                mirror: Some("HDMI-1".to_owned()),
                ..MonitorConfig::default()
            },
        );
        wm.work.monitor_config = false;

        // The stored source is unplugged (a fresh WaylandBackend reports no
        // outputs), but this command leaves the mirror untouched: only a
        // patch declaring a mirror target is connectivity-checked, so an
        // unrelated field set must still succeed.
        let resp = super::handle_monitor_command(&mut wm, set_cmd("DP-1", None, Some(2.0), None));
        assert!(matches!(resp, Response::Ok), "{resp:?}");
        let config = wm.core.config.monitors.get("DP-1").expect("entry");
        assert_eq!(config.mirror.as_deref(), Some("HDMI-1"));
        assert_eq!(config.scale, Some(2.0));
        assert!(wm.work.monitor_config);
    }

    #[test]
    fn monitor_set_self_reference_is_rejected() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));

        let resp =
            super::handle_monitor_command(&mut wm, set_cmd("DP-1", None, None, Some("DP-1")));
        let Response::Err(message) = resp else {
            panic!("expected error, got {resp:?}");
        };
        // Fatal build error keyed to the edited entry, reported before the
        // connectivity check ever runs.
        assert!(message.contains("cannot mirror itself"), "{message}");
        assert!(!wm.core.config.monitors.contains_key("DP-1"));
        assert!(!wm.work.monitor_config);
    }

    #[test]
    fn monitor_set_clear_sentinel_drops_an_existing_mirror() {
        use crate::config::config_toml::MonitorConfig;

        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.config.monitors.insert(
            "DP-1".to_owned(),
            MonitorConfig {
                mirror: Some("HDMI-1".to_owned()),
                ..MonitorConfig::default()
            },
        );

        // Clearing skips the connectivity check: the source may simply be
        // unplugged right now.
        let resp = super::handle_monitor_command(&mut wm, set_cmd("DP-1", None, None, Some("")));
        assert!(matches!(resp, Response::Ok), "{resp:?}");
        assert_eq!(wm.core.config.monitors["DP-1"].mirror, None);
        assert!(wm.work.monitor_config);
    }

    #[test]
    fn monitor_set_mirror_fit_survives_unrelated_sets() {
        use crate::config::config_toml::{MirrorFit, MonitorConfig};

        // Stored mirror + fit, then an unrelated field arrives. Merge-level:
        // the unplugged-source IPC path has its own regression test
        // (`monitor_set_keeps_working_while_a_stored_mirror_source_is_unplugged`).
        let mirrored = MonitorConfig {
            mirror: Some("HDMI-1".into()),
            mirror_fit: Some(MirrorFit::Cover),
            ..MonitorConfig::default()
        };

        // An unrelated field set keeps both the mirror and its fit.
        let merged = super::merge_monitor_config(
            Some(&mirrored),
            MonitorConfig {
                scale: Some(2.0),
                ..MonitorConfig::default()
            },
        );
        assert_eq!(merged.mirror.as_deref(), Some("HDMI-1"));
        assert_eq!(merged.mirror_fit, Some(MirrorFit::Cover));
        assert_eq!(merged.scale, Some(2.0));

        // A fit-only patch overwrites just the fit, keeping the mirror.
        let merged = super::merge_monitor_config(
            Some(&mirrored),
            MonitorConfig {
                mirror_fit: Some(MirrorFit::Contain),
                ..MonitorConfig::default()
            },
        );
        assert_eq!(merged.mirror.as_deref(), Some("HDMI-1"));
        assert_eq!(merged.mirror_fit, Some(MirrorFit::Contain));
    }

    #[test]
    fn monitor_set_fit_without_mirror_is_stored_at_the_ipc_layer() {
        use crate::ipc_types::MirrorFit;

        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));

        // A fit on a non-mirror output is allowed (non-fatal by design);
        // sanitization clears it later at apply time, so the IPC layer must
        // store, not reject, it.
        let resp = super::handle_monitor_command(
            &mut wm,
            set_mirror_cmd("DP-1", None, Some(MirrorFit::Cover)),
        );
        assert!(matches!(resp, Response::Ok), "{resp:?}");
        let config = wm.core.config.monitors.get("DP-1").expect("entry");
        assert_eq!(config.mirror, None);
        assert_eq!(config.mirror_fit, Some(MirrorFit::Cover));
        assert!(wm.work.monitor_config);
    }

    #[test]
    fn monitor_set_clear_sentinel_also_drops_the_mirror_fit() {
        use crate::config::config_toml::MonitorConfig;
        use crate::ipc_types::MirrorFit;

        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        wm.core.config.monitors.insert(
            "DP-1".to_owned(),
            MonitorConfig {
                mirror: Some("HDMI-1".to_owned()),
                mirror_fit: Some(MirrorFit::Cover),
                ..MonitorConfig::default()
            },
        );
        wm.work.monitor_config = false;

        // The clear sentinel takes the now-meaningless fit with it; this
        // keeps working while the source is unplugged (no connectivity check).
        let resp = super::handle_monitor_command(&mut wm, set_mirror_cmd("DP-1", Some(""), None));
        assert!(matches!(resp, Response::Ok), "{resp:?}");
        let config = wm.core.config.monitors.get("DP-1").expect("entry");
        assert_eq!(config.mirror, None);
        assert_eq!(config.mirror_fit, None);
        assert!(wm.work.monitor_config);
    }

    #[test]
    fn merge_keeps_unset_fields_and_mirror_shadows_presentation() {
        use crate::config::config_toml::{MirrorFit, MonitorConfig, VrrMode};

        let existing = MonitorConfig {
            resolution: Some("2560x1440".into()),
            refresh_rate: Some(144.0),
            position: Some("0,0".into()),
            scale: Some(2.0),
            transform: Some("90".into()),
            enable: Some(false),
            vrr: Some(VrrMode::On),
            mirror: None,
            mirror_fit: None,
        };

        // Some overwrites, None keeps.
        let merged = super::merge_monitor_config(
            Some(&existing),
            MonitorConfig {
                resolution: Some("1920x1080".into()),
                ..MonitorConfig::default()
            },
        );
        assert_eq!(merged.resolution.as_deref(), Some("1920x1080"));
        assert_eq!(merged.scale, Some(2.0));
        assert_eq!(merged.position.as_deref(), Some("0,0"));
        assert_eq!(merged.refresh_rate, Some(144.0));

        // mirror_fit merges like the other kept fields.
        let merged = super::merge_monitor_config(
            Some(&existing),
            MonitorConfig {
                mirror_fit: Some(MirrorFit::Cover),
                ..MonitorConfig::default()
            },
        );
        assert_eq!(merged.mirror_fit, Some(MirrorFit::Cover));
        let merged = super::merge_monitor_config(
            Some(&existing),
            MonitorConfig {
                mirror_fit: None,
                ..MonitorConfig::default()
            },
        );
        assert_eq!(merged.mirror_fit, None);

        // Mirror and fit land together in a single patch; the fit is not
        // shadowed by the mirror declaration.
        let merged = super::merge_monitor_config(
            Some(&existing),
            MonitorConfig {
                mirror: Some("eDP-1".into()),
                mirror_fit: Some(MirrorFit::Cover),
                ..MonitorConfig::default()
            },
        );
        assert_eq!(merged.mirror.as_deref(), Some("eDP-1"));
        assert_eq!(merged.mirror_fit, Some(MirrorFit::Cover));
        assert_eq!(merged.position, None);
        assert_eq!(merged.scale, None);

        // Declaring a mirror clears the shadowed position and scale on the
        // candidate. The head's own mode and transform and the policy fields
        // (including mirror_fit) stay.
        let mirrored_existing = MonitorConfig {
            mirror_fit: Some(MirrorFit::Cover),
            ..existing
        };
        let merged = super::merge_monitor_config(
            Some(&mirrored_existing),
            MonitorConfig {
                mirror: Some("eDP-1".into()),
                ..MonitorConfig::default()
            },
        );
        assert_eq!(merged.mirror.as_deref(), Some("eDP-1"));
        assert_eq!(merged.position, None);
        assert_eq!(merged.scale, None);
        assert_eq!(merged.resolution.as_deref(), Some("2560x1440"));
        assert_eq!(merged.refresh_rate, Some(144.0));
        assert_eq!(merged.transform.as_deref(), Some("90"));
        assert_eq!(merged.enable, Some(false));
        assert_eq!(merged.vrr, Some(VrrMode::On));
        assert_eq!(merged.mirror_fit, Some(MirrorFit::Cover));

        // The clear sentinel drops an existing mirror — and its fit, which
        // would be meaningless without it — and nothing else.
        let mirrored = MonitorConfig {
            mirror: Some("eDP-1".into()),
            mirror_fit: Some(MirrorFit::Cover),
            scale: Some(2.0),
            ..MonitorConfig::default()
        };
        let merged = super::merge_monitor_config(
            Some(&mirrored),
            MonitorConfig {
                mirror: Some(String::new()),
                ..MonitorConfig::default()
            },
        );
        assert_eq!(merged.mirror, None);
        assert_eq!(merged.mirror_fit, None);
        assert_eq!(merged.scale, Some(2.0));

        // No stored entry starts from the default.
        let merged = super::merge_monitor_config(None, MonitorConfig::default());
        assert_eq!(merged.mirror, None);
    }
}
