//! Runtime dispatch of `[[hooks]]` (see [`crate::config::hooks`]).
//!
//! Monitor hooks compare the current monitor topology against the topology
//! they last observed, once per runtime tick. Detecting changes from the
//! model rather than from backend events keeps hooks backend-agnostic and
//! naturally coalesces bursts: several reconciles while outputs settle, a
//! dock bringing up two screens, or an output that flaps within one tick all
//! yield a single, net change.

use crate::config::hooks::HookEvent;
use crate::model::WmModel;
use crate::types::Rect;
use crate::wm::Wm;

/// The parts of a monitor whose change is visible to monitor hooks.
///
/// UI-only state (bar metrics, tags, selection) is deliberately excluded:
/// a font reload is not a monitor configuration change.
#[derive(Debug, Clone, PartialEq)]
pub struct MonitorSnapshot {
    pub name: String,
    pub rect: Rect,
    pub scale: f64,
}

/// Snapshot the monitor topology in layout order.
pub fn snapshot_monitors(model: &WmModel) -> Vec<MonitorSnapshot> {
    model
        .monitors_iter()
        .map(|(_, m)| MonitorSnapshot {
            name: m.name.clone(),
            rect: m.monitor_rect,
            scale: m.ui_scale,
        })
        .collect()
}

/// Net difference between two topologies.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TopologyChange {
    pub disconnected: Vec<String>,
    pub connected: Vec<String>,
    /// Anything differs, including geometry, scale or order.
    pub changed: bool,
}

pub fn diff_topology(previous: &[MonitorSnapshot], current: &[MonitorSnapshot]) -> TopologyChange {
    let names = |monitors: &[MonitorSnapshot]| -> Vec<String> {
        monitors
            .iter()
            .map(|m| m.name.clone())
            .filter(|name| !name.is_empty())
            .collect()
    };
    let (before, after) = (names(previous), names(current));
    TopologyChange {
        disconnected: before
            .iter()
            .filter(|name| !after.contains(name))
            .cloned()
            .collect(),
        connected: after
            .iter()
            .filter(|name| !before.contains(name))
            .cloned()
            .collect(),
        changed: previous != current,
    }
}

/// Run monitor hooks if the topology changed since the last call.
///
/// The first topology seen (startup) is recorded without firing anything;
/// startup work belongs in `exec`/`exec_once`.
pub fn run_monitor_hooks(wm: &mut Wm) {
    let current = snapshot_monitors(&wm.core.model);
    if current.is_empty() {
        return;
    }
    let previous = std::mem::replace(&mut wm.work.hooked_monitors, current);
    if previous.is_empty() {
        return;
    }
    let change = diff_topology(&previous, &wm.work.hooked_monitors);
    if change.changed {
        dispatch_monitor_hooks(wm, &change);
    }
}

/// Fire hooks for a topology change: per-monitor disconnects, then connects,
/// then one `monitors_changed`.
///
/// Spawned processes receive `INSTANTWM_HOOK_EVENT`, `INSTANTWM_MONITORS`
/// (all current outputs, space separated) and, for per-monitor events,
/// `INSTANTWM_MONITOR`.
fn dispatch_monitor_hooks(wm: &mut Wm, change: &TopologyChange) {
    let monitors = wm
        .work
        .hooked_monitors
        .iter()
        .map(|m| m.name.as_str())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    let events = change
        .disconnected
        .iter()
        .map(|name| (HookEvent::MonitorDisconnected, Some(name.as_str())))
        .chain(
            change
                .connected
                .iter()
                .map(|name| (HookEvent::MonitorConnected, Some(name.as_str()))),
        )
        .chain(std::iter::once((HookEvent::MonitorsChanged, None)));

    for (event, monitor) in events {
        log::info!("instantwm: {} {}", event.name(), monitor.unwrap_or(""));
        let actions: Vec<_> = wm
            .core
            .config
            .hooks
            .iter()
            .filter(|hook| hook.matches(event, monitor))
            .map(|hook| hook.action.clone())
            .collect();
        if actions.is_empty() {
            continue;
        }

        let mut env = vec![
            ("INSTANTWM_HOOK_EVENT", event.name().to_string()),
            ("INSTANTWM_MONITORS", monitors.clone()),
        ];
        if let Some(monitor) = monitor {
            env.push(("INSTANTWM_MONITOR", monitor.to_string()));
        }
        wm.core.hook_env = env;
        for action in &actions {
            if let Err(error) = crate::actions::try_execute_key_action(&mut wm.ctx(), action) {
                log::warn!("instantwm: {} hook failed: {error}", event.name());
            }
        }
        wm.core.hook_env.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{KeyAction, NamedAction};
    use crate::backend::{Backend, wayland::WaylandBackend};
    use crate::config::hooks::Hook;
    use crate::core_state::ActiveWmMode;

    fn monitor(name: &str, x: i32) -> MonitorSnapshot {
        MonitorSnapshot {
            name: name.into(),
            rect: Rect::new(x, 0, 1920, 1080),
            scale: 1.0,
        }
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    #[test]
    fn swap_reports_disconnect_connect_and_change() {
        let change = diff_topology(
            &[monitor("eDP-1", 0), monitor("DP-1", 1920)],
            &[monitor("eDP-1", 0), monitor("HDMI-A-1", 1920)],
        );
        assert_eq!(
            change,
            TopologyChange {
                disconnected: names(&["DP-1"]),
                connected: names(&["HDMI-A-1"]),
                changed: true,
            }
        );
    }

    #[test]
    fn geometry_and_scale_changes_are_changes_without_hotplug() {
        let moved = diff_topology(&[monitor("DP-1", 0)], &[monitor("DP-1", 100)]);
        assert!(moved.changed && moved.connected.is_empty() && moved.disconnected.is_empty());

        let mut scaled = monitor("DP-1", 0);
        scaled.scale = 2.0;
        assert!(diff_topology(&[monitor("DP-1", 0)], &[scaled]).changed);

        assert_eq!(
            diff_topology(&[monitor("DP-1", 0)], &[monitor("DP-1", 0)]),
            TopologyChange::default()
        );
    }

    fn wm_with_mode_hooks(hooks: Vec<(HookEvent, Option<&str>, &str)>) -> Wm {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        for (event, monitor, mode) in hooks {
            wm.core
                .config
                .bindings
                .modes
                .insert(mode.to_string(), crate::config::ModeConfig::default());
            wm.core.config.hooks.push(Hook {
                event,
                monitor: monitor.map(str::to_string),
                action: KeyAction::Named(NamedAction::SetMode(mode.to_string())),
            });
        }
        wm
    }

    fn current_mode(wm: &Wm) -> &ActiveWmMode {
        &wm.core.behavior.current_mode
    }

    #[test]
    fn hooks_run_in_order_and_respect_filters() {
        let mut wm = wm_with_mode_hooks(vec![
            (HookEvent::MonitorsChanged, None, "changed"),
            (HookEvent::MonitorConnected, Some("DP-1"), "docked"),
            (HookEvent::MonitorConnected, Some("HDMI-A-1"), "wrong"),
            (HookEvent::MonitorDisconnected, None, "wrong"),
        ]);
        wm.work.hooked_monitors = vec![monitor("eDP-1", 0), monitor("DP-1", 1920)];

        dispatch_monitor_hooks(
            &mut wm,
            &TopologyChange {
                disconnected: Vec::new(),
                connected: names(&["DP-1"]),
                changed: true,
            },
        );

        // monitors_changed runs last, after the matching connect hook.
        assert_eq!(
            current_mode(&wm),
            &ActiveWmMode::Named("changed".to_string())
        );
        assert!(wm.core.hook_env.is_empty());

        let mut wm = wm_with_mode_hooks(vec![
            (HookEvent::MonitorConnected, Some("DP-1"), "docked"),
            (HookEvent::MonitorConnected, Some("HDMI-A-1"), "wrong"),
            (HookEvent::MonitorDisconnected, None, "wrong"),
        ]);
        dispatch_monitor_hooks(
            &mut wm,
            &TopologyChange {
                disconnected: Vec::new(),
                connected: names(&["DP-1"]),
                changed: true,
            },
        );
        assert_eq!(
            current_mode(&wm),
            &ActiveWmMode::Named("docked".to_string())
        );
    }

    #[test]
    fn startup_topology_is_recorded_and_later_changes_fire_once() {
        use crate::monitor::MonitorManager;
        use crate::types::{Monitor, MonitorId};

        let push = |wm: &mut Wm, id: u64, name: &str, x: i32| {
            wm.core.model.monitors.push(Monitor {
                monitor_id: MonitorId::from_raw(id),
                name: name.into(),
                monitor_rect: Rect::new(x, 0, 1920, 1080),
                ui_scale: 1.0,
                ..Monitor::default()
            });
        };
        let mut wm = wm_with_mode_hooks(vec![(HookEvent::MonitorsChanged, None, "changed")]);
        wm.core.model.monitors = MonitorManager::new();
        push(&mut wm, 0, "eDP-1", 0);
        let initial = current_mode(&wm).clone();

        run_monitor_hooks(&mut wm);
        assert_eq!(current_mode(&wm), &initial, "startup must not fire");
        assert_eq!(wm.work.hooked_monitors.len(), 1);

        push(&mut wm, 1, "DP-1", 1920);
        run_monitor_hooks(&mut wm);
        assert_eq!(
            current_mode(&wm),
            &ActiveWmMode::Named("changed".to_string())
        );

        wm.core.behavior.current_mode = initial.clone();
        run_monitor_hooks(&mut wm);
        assert_eq!(
            current_mode(&wm),
            &initial,
            "unchanged topology must not fire"
        );
    }

    #[test]
    fn no_topology_means_nothing_to_record_or_fire() {
        let mut wm = wm_with_mode_hooks(vec![(HookEvent::MonitorsChanged, None, "changed")]);
        let before = current_mode(&wm).clone();

        run_monitor_hooks(&mut wm);

        assert!(wm.work.hooked_monitors.is_empty());
        assert_eq!(current_mode(&wm), &before);
    }
}
