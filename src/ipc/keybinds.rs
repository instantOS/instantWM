use crate::ipc_types::{KeybindInfo, Response};
use crate::types::{Key, KeybindOrigin};
use crate::wm::Wm;

use crate::config::keybindings::MODKEY;
use crate::config::keysyms::XK_ESCAPE;

const RESET_MODE_ACTION: &str = "reset_mode";

/// List every active keybinding: global, desktop (no client focused), and
/// per-mode. Bindings are rendered readably and each entry is tagged with its
/// origin (compiled default vs. user config).
pub fn list_keybinds(wm: &mut Wm) -> Response {
    Response::KeybindList(keybind_entries(&wm.core.config.bindings))
}

fn keybind_entries(bindings: &crate::core_state::BindingConfig) -> Vec<KeybindInfo> {
    let mut entries: Vec<KeybindInfo> = Vec::new();

    // Global bindings (always active).
    for key in &bindings.keys {
        entries.push(to_entry(key, None));
    }

    // Desktop bindings (active only when no client is focused).
    for key in &bindings.desktop_keybinds {
        entries.push(to_entry(key, Some("desktop")));
    }

    // Per-mode bindings: the built-in `prefix` and `desktop` modes (empty
    // unless configured), the compositor-owned placement mode, and any
    // user-defined modes.
    let mut mode_names: Vec<&String> = bindings.modes.keys().collect();
    mode_names.sort();
    for name in mode_names {
        if let Some(mode) = bindings.modes.get(name) {
            for key in mode
                .keybinds
                .iter()
                .filter(|key| key.mod_mask != MODKEY || key.keysym != XK_ESCAPE)
            {
                entries.push(to_entry(key, Some(name.as_str())));
            }
            entries.push(reset_mode_entry(name.as_str()));
        }
    }

    // Overview is also a non-default mode, but unlike named and placement
    // modes it has no entry in BindingConfig::modes.
    if !bindings.modes.contains_key("overview") {
        entries.push(reset_mode_entry("overview"));
    }

    entries
}

/// The `Super + Escape` chord that leaves any mode, rendered through the same
/// types as every other entry so the list has one vocabulary. It used to be
/// hand-written as the strings `"Super"` and `"Esc"`, which meant the reserved
/// chord was the one entry a user could not copy back into their config.
fn reset_mode_entry(mode: &str) -> KeybindInfo {
    KeybindInfo {
        modifiers: MODKEY.to_string(),
        key: XK_ESCAPE.to_string(),
        action: RESET_MODE_ACTION.to_string(),
        mode: Some(mode.to_string()),
        origin: KeybindOrigin::CompiledDefault,
    }
}

fn to_entry(key: &Key, mode: Option<&str>) -> KeybindInfo {
    KeybindInfo {
        modifiers: key.mod_mask.to_string(),
        key: key.keysym.to_string(),
        action: key.action.describe(),
        mode: mode.map(str::to_string),
        origin: key.origin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::{KeyAction, NamedAction};
    use crate::config::ModeConfig;
    use crate::core_state::BindingConfig;

    #[test]
    fn reserved_mode_reset_replaces_shadowed_config_binding() {
        let configured = Key {
            mod_mask: MODKEY,
            keysym: XK_ESCAPE,
            action: KeyAction::named(NamedAction::FocusNext),
            origin: KeybindOrigin::User,
        };
        let mut bindings = BindingConfig::default();
        bindings.modes.insert(
            "resize".to_string(),
            ModeConfig {
                description: None,
                transient: false,
                keybinds: vec![configured],
            },
        );

        let entries = keybind_entries(&bindings);
        let resize_entries: Vec<_> = entries
            .iter()
            .filter(|entry| entry.mode.as_deref() == Some("resize"))
            .collect();

        assert_eq!(resize_entries.len(), 1);
        assert_eq!(resize_entries[0].modifiers, "Super");
        assert_eq!(resize_entries[0].key, "Escape");
        assert_eq!(resize_entries[0].action, RESET_MODE_ACTION);
        assert_eq!(resize_entries[0].origin, KeybindOrigin::CompiledDefault);
    }

    #[test]
    fn overview_includes_reserved_mode_reset() {
        let entries = keybind_entries(&BindingConfig::default());

        assert!(entries.iter().any(|entry| {
            entry.mode.as_deref() == Some("overview")
                && entry.modifiers == "Super"
                && entry.key == "Escape"
                && entry.action == RESET_MODE_ACTION
        }));
    }

    #[test]
    fn every_listed_chord_can_be_copied_back_into_a_config() {
        // The listing is the tool a user reads to discover what to write. If its
        // output does not parse as config, it is documentation they cannot use.
        for entry in keybind_entries(&BindingConfig::default()) {
            entry
                .modifiers
                .parse::<crate::types::ModMask>()
                .unwrap_or_else(|error| {
                    panic!("modifiers {:?} do not parse: {error}", entry.modifiers)
                });
            crate::types::Keysym::from_name(&entry.key)
                .unwrap_or_else(|error| panic!("key {:?} does not parse: {error}", entry.key));
        }
    }

    #[test]
    fn per_mode_bindings_are_ordered_deterministically_by_mode_name() {
        let mut bindings = BindingConfig::default();
        bindings.modes.insert(
            "zebra".to_string(),
            ModeConfig {
                description: None,
                transient: false,
                keybinds: Vec::new(),
            },
        );
        bindings.modes.insert(
            "alpha".to_string(),
            ModeConfig {
                description: None,
                transient: false,
                keybinds: Vec::new(),
            },
        );

        let entries = keybind_entries(&bindings);
        let mode_names: Vec<_> = entries.iter().filter_map(|e| e.mode.as_deref()).collect();

        let alpha_pos = mode_names.iter().position(|&m| m == "alpha").unwrap();
        let zebra_pos = mode_names.iter().position(|&m| m == "zebra").unwrap();
        assert!(alpha_pos < zebra_pos);
    }
}
