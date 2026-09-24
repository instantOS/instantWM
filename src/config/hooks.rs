//! Event hooks: run actions in response to window-manager events.
//!
//! ```toml
//! [[hooks]]
//! event = "monitor_connected"
//! monitor = "HDMI-A-1"          # optional: only fire for this output
//! action = ["spawn", "notify-send", "Monitor connected"]
//! ```
//!
//! `action` accepts exactly the same values as a keybind action (a name, a
//! name with arguments, or a `sequence`), so everything a key can do, a hook
//! can do too. Unlike keybinds, invalid hooks are configuration errors instead of
//! being silently skipped. Dispatch lives in [`crate::hooks`].

use serde::{Deserialize, Serialize};

use crate::actions::KeyAction;
use crate::config::keybind_config::{ActionSpec, compile_action};

/// Events a hook can subscribe to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// A monitor appeared at runtime (hot-plugged or enabled).
    MonitorConnected,
    /// A monitor disappeared at runtime (unplugged or disabled).
    MonitorDisconnected,
    /// The monitor setup changed in any way (outputs added or removed, or
    /// their geometry, scale or order changed). Fires once per change.
    MonitorsChanged,
}

impl HookEvent {
    pub const fn name(self) -> &'static str {
        match self {
            Self::MonitorConnected => "monitor_connected",
            Self::MonitorDisconnected => "monitor_disconnected",
            Self::MonitorsChanged => "monitors_changed",
        }
    }

    /// Whether the event concerns one specific monitor, so a `monitor`
    /// filter is meaningful.
    pub const fn is_per_monitor(self) -> bool {
        matches!(self, Self::MonitorConnected | Self::MonitorDisconnected)
    }
}

/// A `[[hooks]]` entry as written by the user.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HookSpec {
    pub event: HookEvent,
    /// Restrict a per-monitor event to one output name (e.g. `"HDMI-A-1"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitor: Option<String>,
    pub action: ActionSpec,
}

/// A validated, executable hook.
#[derive(Debug, Clone)]
pub struct Hook {
    pub event: HookEvent,
    pub monitor: Option<String>,
    pub action: KeyAction,
}

impl Hook {
    /// Whether this hook should run for `event`, which concerns `monitor`
    /// for per-monitor events and nothing otherwise.
    pub fn matches(&self, event: HookEvent, monitor: Option<&str>) -> bool {
        self.event == event
            && self
                .monitor
                .as_deref()
                .is_none_or(|filter| monitor == Some(filter))
    }
}

/// Validate and compile `[[hooks]]` entries.
pub fn resolve_hooks(specs: Vec<HookSpec>) -> Result<Vec<Hook>, String> {
    specs
        .into_iter()
        .enumerate()
        .map(|(index, spec)| {
            let context =
                |error: String| format!("hooks[{index}] ({}): {error}", spec.event.name());
            if let Some(name) = spec.monitor.as_deref() {
                if !spec.event.is_per_monitor() {
                    return Err(context(
                        "'monitor' filter is not supported for this event".to_string(),
                    ));
                }
                if name.trim().is_empty() {
                    return Err(context("'monitor' must not be empty".to_string()));
                }
            }
            let action = compile_action(&spec.action).map_err(context)?;
            Ok(Hook {
                event: spec.event,
                monitor: spec.monitor,
                action,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::config_toml::UserConfig;

    fn parse(source: &str) -> Result<Vec<Hook>, String> {
        let user: UserConfig = toml::from_str(source).map_err(|e| e.to_string())?;
        resolve_hooks(user.hooks)
    }

    #[test]
    fn hooks_accept_keybind_actions() {
        let hooks = parse(
            r#"
            [[hooks]]
            event = "monitor_connected"
            monitor = "HDMI-A-1"
            action = ["spawn", "notify-send", "hi"]

            [[hooks]]
            event = "monitor_disconnected"
            action = { sequence = ["toggle_bar", ["set_layout", "tile"]] }

            [[hooks]]
            event = "monitors_changed"
            action = "toggle_bar"
            "#,
        )
        .unwrap();

        assert_eq!(hooks.len(), 3);
        assert!(hooks[0].matches(HookEvent::MonitorConnected, Some("HDMI-A-1")));
        assert!(!hooks[0].matches(HookEvent::MonitorConnected, Some("DP-1")));
        assert!(!hooks[0].matches(HookEvent::MonitorDisconnected, Some("HDMI-A-1")));
        assert!(hooks[1].matches(HookEvent::MonitorDisconnected, Some("anything")));
        assert!(matches!(hooks[1].action, KeyAction::Sequence(ref a) if a.len() == 2));
        assert!(hooks[2].matches(HookEvent::MonitorsChanged, None));
    }

    #[test]
    fn invalid_hooks_are_errors() {
        for source in [
            // unknown event
            "[[hooks]]\nevent = \"monitor_plugged\"\naction = \"toggle_bar\"",
            // typo'd field
            "[[hooks]]\nevent = \"monitor_connected\"\nmonitors = \"DP-1\"\naction = \"toggle_bar\"",
            // unknown action
            "[[hooks]]\nevent = \"monitor_connected\"\naction = \"does_not_exist\"",
            // not executable
            "[[hooks]]\nevent = \"monitor_connected\"\naction = \"none\"",
            // missing required argument
            "[[hooks]]\nevent = \"monitor_connected\"\naction = [\"spawn\"]",
            "[[hooks]]\nevent = \"monitor_connected\"\naction = \"set_layout\"",
            // empty filter
            "[[hooks]]\nevent = \"monitor_connected\"\nmonitor = \"\"\naction = \"toggle_bar\"",
            // filter on a whole-setup event
            "[[hooks]]\nevent = \"monitors_changed\"\nmonitor = \"DP-1\"\naction = \"toggle_bar\"",
        ] {
            assert!(parse(source).is_err(), "should reject: {source}");
        }
    }
}
