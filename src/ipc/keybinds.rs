use crate::ipc_types::{KeybindInfo, Response};
use crate::types::KeybindOrigin;
use crate::wm::Wm;

const RESET_MODE_ACTION: &str = "reset_mode";

/// List every active keybinding: global, desktop (no client focused), and
/// per-mode. Bindings are rendered readably and each entry is tagged with its
/// origin (compiled default vs. user config).
pub fn list_keybinds(wm: &mut Wm) -> Response {
    Response::KeybindList(keybind_entries(&wm.core.config.bindings))
}

fn keybind_entries(bindings: &crate::core_state::BindingConfig) -> Vec<KeybindInfo> {
    use crate::config::keybindings::MODKEY;
    use crate::config::keysyms::XK_ESCAPE;

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

fn reset_mode_entry(mode: &str) -> KeybindInfo {
    KeybindInfo {
        modifiers: "Super".to_string(),
        key: "Esc".to_string(),
        action: RESET_MODE_ACTION.to_string(),
        mode: Some(mode.to_string()),
        origin: KeybindOrigin::CompiledDefault,
    }
}

fn to_entry(key: &crate::types::Key, mode: Option<&str>) -> KeybindInfo {
    use crate::config::keybind_config::{format_keysym, format_modifiers};
    KeybindInfo {
        modifiers: format_modifiers(key.mod_mask),
        key: format_keysym(key.keysym),
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
    use crate::config::keybindings::MODKEY;
    use crate::config::keysyms::XK_ESCAPE;
    use crate::core_state::BindingConfig;
    use crate::types::Key;

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
        assert_eq!(resize_entries[0].key, "Esc");
        assert_eq!(resize_entries[0].action, RESET_MODE_ACTION);
        assert_eq!(resize_entries[0].origin, KeybindOrigin::CompiledDefault);
    }

    #[test]
    fn overview_includes_reserved_mode_reset() {
        let entries = keybind_entries(&BindingConfig::default());

        assert!(entries.iter().any(|entry| {
            entry.mode.as_deref() == Some("overview")
                && entry.modifiers == "Super"
                && entry.key == "Esc"
                && entry.action == RESET_MODE_ACTION
        }));
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
