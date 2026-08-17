//! Window manager configuration.
//!
//! This module is the single place to tune instantWM behaviour.  It is split
//! into focused sub-modules so you can find what you need quickly:
//!
//! | Module            | What lives there                                        |
//! |-------------------|---------------------------------------------------------|
//! | [`appearance`]    | Color palette, per-scheme color tables, font list       |
//! | [`commands`]      | External commands (`ExternalCommands`, `Cmd` enum)      |
//! | [`keybindings`]   | Normal-mode key bindings (`get_keys`, `get_desktop_keybinds`)      |
//! | [`buttons`]       | Mouse button bindings (`get_buttons`)                   |
//! | [`rules`]         | Window placement rules (`get_rules`)                    |
//! | [`keysyms`]       | X11 keysym constants (re-exported via `use keysyms::*`) |
//!
//! # Quick-start: changing things
//!
//! - **Add/change a keybinding** → [`keybindings`]
//! - **Add/change a mouse button** → [`buttons`]
//! - **Change colors** → [`appearance::palette`]
//! - **Add an external command** → [`commands`] (add field + `Cmd` variant)
//! - **Change a window rule** → [`rules`]
//! - **Tune WM parameters** (border width, gaps, …) → [`EffectiveConfig`] defaults below

pub mod appearance;
pub mod buttons;
pub mod commands;
pub mod commands_common;
pub mod config_toml;
pub mod generated_keybinds;
pub mod keybind_config;
pub mod keybindings;
pub mod keysyms;
pub mod rules;

// Re-export modifier key constants (used by backend/wayland/input/modifiers.rs via crate::config::*).
pub use crate::types::{EdgeDirection, SchemeClose, SchemeHover, SchemeTag, SchemeWin};
pub use keybindings::{CONTROL, MOD1, MODKEY, SHIFT};

use crate::types::KeybindOrigin;
use commands::default_commands;
// ---------------------------------------------------------------------------
// Module-level constants
// ---------------------------------------------------------------------------

/// Shared constants referenced by multiple sub-modules.
pub mod mod_consts {
    use crate::types::MAX_TAGS;

    /// Default border width in pixels.
    pub const BORDER_PX: i32 = 3;

    /// Maximum tag name length.
    pub const MAX_TAGLEN: usize = 16;

    /// Bitmask covering all valid tags.
    pub const TAG_MASK: u32 = (1 << MAX_TAGS) - 1;
}

// ---------------------------------------------------------------------------
// Tag configuration
// ---------------------------------------------------------------------------

use crate::types::MAX_TAGS;

/// Default tag names (used when no config override is set).
///
/// There are [`MAX_TAGS`] entries — the last one (`"s"`) is the scratchpad tag.
pub fn get_tags_default() -> [&'static str; MAX_TAGS] {
    [
        "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16",
        "17", "18", "19", "20", "s",
    ]
}

/// Build the tag name list as owned `String`s.
pub fn get_tags() -> Vec<String> {
    get_tags_default().iter().map(|&s| s.to_string()).collect()
}

/// Alternative (icon) tag names shown when alt-tag mode is active.
pub fn get_tags_alt() -> Vec<String> {
    vec![
        "".to_string(),
        "{}".to_string(),
        "$".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
        "".to_string(),
    ]
}

// ---------------------------------------------------------------------------
// Effective configuration resolution
// ---------------------------------------------------------------------------

use crate::core_state::{
    BindingConfig, ColorConfig, EffectiveConfig, FontConfig, SystrayConfig, WindowConfig,
};
use crate::types::Key;
use std::collections::HashMap;
use std::env;

use generated_keybinds::build_default_keybinds;

/// Mode configuration with keybinds and optional description.
#[derive(Debug, Clone, Default)]
pub struct ModeConfig {
    /// Optional description shown in status bar when mode is active.
    pub description: Option<String>,
    /// Whether the mode is transient (reset to default after any keybind).
    pub transient: bool,
    /// Keybinds for this mode.
    pub keybinds: Vec<Key>,
}

// ---------------------------------------------------------------------------
// Loading and resolution
// ---------------------------------------------------------------------------

/// Load the user-facing schema and resolve it into the one configuration type
/// consumed by the running window manager.
///
/// Used by both backends at startup and by the shared reload path.
pub fn load_config(backend: crate::backend::BackendKind) -> Result<EffectiveConfig, String> {
    resolve_config(config_toml::load_config_file()?, backend)
}

/// Resolve built-in defaults for startup fallback and default-constructed core
/// state. Built-in values are invariants and therefore must always validate.
pub fn default_config(backend: crate::backend::BackendKind) -> EffectiveConfig {
    resolve_config(config_toml::UserConfig::default(), backend)
        .expect("built-in configuration must be valid")
}

/// Load configuration for initial startup. Unlike reload, startup has no
/// previous valid snapshot to retain, so a reported error falls back to the
/// built-in configuration and allows the WM to start.
pub fn load_startup_config(backend: crate::backend::BackendKind) -> EffectiveConfig {
    load_config(backend).unwrap_or_else(|error| {
        eprintln!("instantwm: {error}; using built-in configuration");
        default_config(backend)
    })
}

/// Resolve a parsed user configuration into the complete effective snapshot.
/// This is the sole user-to-runtime conversion boundary.
pub fn resolve_config(
    theme: config_toml::UserConfig,
    backend: crate::backend::BackendKind,
) -> Result<EffectiveConfig, String> {
    let layout = theme.layout.validated()?;
    let defaults = build_default_keybinds(backend, &theme);

    // Merge TOML keybinds over compiled defaults
    let keys = if theme.keybinds.is_empty() {
        defaults.keys
    } else {
        keybind_config::merge_keybinds(defaults.keys, &theme.keybinds, KeybindOrigin::User)
    };
    let desktop_keybinds = if theme.desktop_keybinds.is_empty() {
        defaults.desktop_keybinds
    } else {
        keybind_config::merge_keybinds(
            defaults.desktop_keybinds,
            &theme.desktop_keybinds,
            KeybindOrigin::User,
        )
    };

    let mut modes = HashMap::new();

    // Helper for merging mode keybinds
    let merge_mode = |spec: Option<&config_toml::ModeSpec>,
                      default_desc: &str,
                      default_transient: bool,
                      default_keybinds: Vec<Key>|
     -> ModeConfig {
        if let Some(spec) = spec {
            let keybinds = keybind_config::merge_keybinds(
                default_keybinds,
                &spec.keybinds,
                KeybindOrigin::User,
            );
            ModeConfig {
                description: spec
                    .description
                    .clone()
                    .or_else(|| Some(default_desc.to_string())),
                transient: spec.transient.unwrap_or(default_transient),
                keybinds,
            }
        } else {
            ModeConfig {
                description: Some(default_desc.to_string()),
                transient: default_transient,
                keybinds: default_keybinds,
            }
        }
    };

    // Special handling for default modes: prefix and desktop
    modes.insert(
        "prefix".to_string(),
        merge_mode(theme.modes.get("prefix"), "prefix", true, Vec::new()),
    );

    modes.insert(
        "desktop".to_string(),
        merge_mode(theme.modes.get("desktop"), "desktop", false, Vec::new()),
    );

    let mut placement_mode = merge_mode(
        theme.modes.get(crate::core_state::TREE_PLACEMENT_MODE_NAME),
        "place window",
        false,
        keybindings::get_tree_placement_keybinds(),
    );
    // Placement has a transactional apply/cancel lifecycle; treating one
    // command as transient would discard that transaction mid-navigation.
    placement_mode.transient = false;
    modes.insert(
        crate::core_state::TREE_PLACEMENT_MODE_NAME.to_string(),
        placement_mode,
    );

    // Add all other user-defined modes
    for (name, spec) in &theme.modes {
        if name == "prefix"
            || name == "desktop"
            || name == crate::core_state::TREE_PLACEMENT_MODE_NAME
        {
            continue;
        }
        let keybinds =
            keybind_config::merge_keybinds(Vec::new(), &spec.keybinds, KeybindOrigin::User);
        modes.insert(
            name.clone(),
            ModeConfig {
                description: spec.description.clone(),
                transient: spec.transient.unwrap_or(false),
                keybinds,
            },
        );
    }

    let bar = theme.bar.validated()?;
    let mut keyboard = theme.keyboard;
    if keyboard.layouts.is_empty() {
        let layout = env::var("XKB_DEFAULT_LAYOUT").unwrap_or_default();
        if layout.is_empty() {
            keyboard.layouts.push(config_toml::KeyboardLayoutConfig {
                name: "us".to_string(),
                variant: None,
            });
        } else {
            keyboard.layouts.push(config_toml::KeyboardLayoutConfig {
                name: layout,
                variant: env::var("XKB_DEFAULT_VARIANT").ok(),
            });
        }
    }
    keyboard.options = keyboard
        .options
        .or_else(|| env::var("XKB_DEFAULT_OPTIONS").ok());
    keyboard.model = keyboard
        .model
        .or_else(|| env::var("XKB_DEFAULT_MODEL").ok());

    let tag_alt_names = get_tags_alt();
    let tag_template = get_tags()
        .into_iter()
        .enumerate()
        .map(|(index, name)| crate::types::monitor::TagNames {
            name,
            alt_name: tag_alt_names.get(index).cloned().unwrap_or_default(),
        })
        .collect();

    Ok(EffectiveConfig {
        window: WindowConfig {
            raise_floating_on_click: theme.raise_floating_on_click,
            ..WindowConfig::default()
        },
        bar,
        systray: SystrayConfig::default(),
        layout,
        animations: theme.animations,
        colors: ColorConfig {
            window: theme.colors.window,
            close_button: theme.colors.close_button,
            border: theme.colors.border,
            status_bar: theme.colors.status,
        },
        theme: theme.theme,
        bindings: BindingConfig {
            keys,
            desktop_keybinds,
            modes,
            buttons: buttons::get_buttons(),
            rules: rules::merge_rules(rules::get_rules(), theme.rules),
        },
        fonts: FontConfig {
            fonts: theme.fonts,
            ..FontConfig::default()
        },
        external_commands: default_commands(),
        tag_template,
        tag_colors: theme.colors.tag,
        keyboard,
        input: theme.input,
        monitors: theme.monitors,
        status_command: theme.status_command,
        cursor: theme.cursor,
        exec_once: theme.exec_once,
        exec: theme.exec,
    })
}

#[cfg(test)]
mod resolution_tests {
    use super::*;

    #[test]
    fn resolution_rejects_invalid_layout_before_building_effective_config() {
        let mut user = config_toml::UserConfig::default();
        user.layout.inner_gap = -10;

        let error = match resolve_config(user, crate::backend::BackendKind::Wayland) {
            Ok(_) => panic!("invalid layout must be rejected"),
            Err(error) => error,
        };

        assert!(error.contains("layout.inner_gap"));
    }

    #[test]
    fn valid_resolution_produces_a_complete_effective_config() {
        let mut user = config_toml::UserConfig::default();
        user.keyboard.layouts = vec![config_toml::KeyboardLayoutConfig {
            name: "de".to_string(),
            variant: Some("nodeadkeys".to_string()),
        }];

        let effective = resolve_config(user, crate::backend::BackendKind::Wayland).unwrap();

        assert_eq!(effective.keyboard.layouts[0].name, "de");
        assert_eq!(effective.tag_template.len(), MAX_TAGS);
        assert_eq!(effective.window, WindowConfig::default());
        assert_eq!(effective.systray, SystrayConfig::default());
    }
}
