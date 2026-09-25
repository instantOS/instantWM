//! Window manager configuration.
//!
//! This module is the single place to tune instantWM behaviour.  It is split
//! into focused sub-modules so you can find what you need quickly:
//!
//! | Module            | What lives there                                        |
//! |-------------------|---------------------------------------------------------|
//! | [`appearance`]    | Color palette and per-scheme color tables                |
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
//! - **Change a window rule** → [`rules`]
//! - **Tune WM parameters** (border width, gaps, …) → [`EffectiveConfig`] defaults below

pub mod appearance;
pub mod buttons;
pub mod commands_common;
pub mod config_toml;
pub mod generated_keybinds;
pub mod hooks;
pub mod keybind_config;
pub mod keybindings;
pub mod keysyms;
pub mod rules;
pub mod runtime;

// Re-export modifier key constants (used by backend/wayland/input/modifiers.rs via crate::config::*).
pub use crate::types::{EdgeDirection, SchemeHover, SchemeTag, WindowFocus, WindowRole};
pub use keybindings::{CONTROL, MOD1, MODKEY, SHIFT};

use crate::types::KeybindOrigin;
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

use crate::core_state::{BindingConfig, EffectiveConfig, WindowConfig};
use crate::types::Key;
use std::collections::HashMap;
use std::env;

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
    mut theme: config_toml::UserConfig,
    backend: crate::backend::BackendKind,
) -> Result<EffectiveConfig, String> {
    let layout = theme.layout.validated()?;
    theme.fonts = theme.fonts.validated()?;
    let keys = keybind_config::merge_keybinds(
        keybindings::get_keys(backend),
        &theme.keybinds,
        KeybindOrigin::User,
    );
    let desktop_keybinds = keybind_config::merge_keybinds(
        keybindings::get_desktop_keybinds(),
        &theme.desktop_keybinds,
        KeybindOrigin::User,
    );

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
    let window = WindowConfig {
        // The legacy top-level key and the `[window]` section are merged so
        // either spelling enables the opt-in behaviour.
        raise_floating_on_click: theme.raise_floating_on_click
            || theme.window.raise_floating_on_click,
        ..theme.window
    }
    .validated()?;
    let hooks = hooks::resolve_hooks(std::mem::take(&mut theme.hooks))?;
    let mut keyboard = theme.keyboard;
    if keyboard.layouts.is_empty() {
        let layout = env::var("XKB_DEFAULT_LAYOUT").unwrap_or_default();
        if layout.is_empty() {
            keyboard
                .layouts
                .push(crate::types::KeyboardLayout::new("us"));
        } else {
            keyboard.layouts.push(crate::types::KeyboardLayout {
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
        .map(|(index, name)| crate::types::Tag {
            name,
            alt_name: tag_alt_names.get(index).cloned().unwrap_or_default(),
        })
        .collect();

    Ok(EffectiveConfig {
        window,
        bar,
        systray: theme.systray,
        tags: theme.tags,
        layout,
        animations: theme.animations,
        colors: theme.colors,
        theme: theme.theme,
        bindings: BindingConfig {
            keys,
            desktop_keybinds,
            modes,
            buttons: buttons::get_buttons(),
            rules: rules::merge_rules(rules::get_rules(), theme.rules),
        },
        fonts: theme.fonts,
        tag_template,
        keyboard,
        input: theme.input,
        monitors: theme.monitors,
        status_command: theme.status_command,
        cursor: theme.cursor,
        exec_once: theme.exec_once,
        exec: theme.exec,
        hooks,
    })
}

#[cfg(test)]
mod resolution_tests {
    use super::*;
    use crate::core_state::SystrayConfig;
    use crate::wm::Wm;

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
        user.keyboard.layouts = vec![crate::types::KeyboardLayout {
            name: "de".to_string(),
            variant: Some("nodeadkeys".to_string()),
        }];

        let effective = resolve_config(user, crate::backend::BackendKind::Wayland).unwrap();

        assert_eq!(effective.keyboard.layouts[0].name, "de");
        assert_eq!(effective.tag_template.len(), MAX_TAGS);
        assert_eq!(effective.window, WindowConfig::default());
        assert_eq!(effective.systray, SystrayConfig::default());
    }

    #[test]
    fn window_section_resolves_into_the_effective_config() {
        let user: config_toml::UserConfig =
            toml::from_str("[window]\ndecor_hints = false\nborder_width_px = 5").unwrap();

        let effective = resolve_config(user, crate::backend::BackendKind::X11).unwrap();

        assert!(!effective.window.decor_hints);
        assert_eq!(effective.window.border_width_px, 5);
        assert_eq!(effective.window.snap_threshold, 32);
        assert!(!effective.window.raise_floating_on_click);
    }

    #[test]
    fn legacy_top_level_click_raise_merges_with_the_window_section() {
        let user: config_toml::UserConfig =
            toml::from_str("raise_floating_on_click = true").unwrap();
        let effective = resolve_config(user, crate::backend::BackendKind::X11).unwrap();
        assert!(effective.window.raise_floating_on_click);
    }

    #[test]
    fn invalid_window_settings_are_rejected_during_resolution() {
        let user: config_toml::UserConfig =
            toml::from_str("[window]\nborder_width_px = -2").unwrap();

        let error = match resolve_config(user, crate::backend::BackendKind::X11) {
            Ok(_) => panic!("invalid window.border_width_px must be rejected"),
            Err(error) => error,
        };

        assert!(error.contains("window.border_width_px"), "{error}");
    }

    #[test]
    fn apply_config_applies_tag_and_bar_defaults() {
        use crate::backend::Backend;
        use crate::backend::wayland::WaylandBackend;
        use crate::types::{Monitor, Rect};

        let user: config_toml::UserConfig = toml::from_str(
            "[tags]\nshow_alt_names = true\n[bar]\nshow_tags = false\nshow_bottom = true",
        )
        .unwrap();
        let config = crate::config::resolve_config(user, crate::backend::BackendKind::Wayland)
            .expect("valid config");

        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let monitor_id = wm.core.model.monitors.push(Monitor {
            monitor_rect: Rect::new(0, 0, 800, 600),
            ..Monitor::default()
        });
        wm.core.apply_config(config);

        // Alt-tag display is read live from config; bar states are seeded
        // into each monitor.
        assert!(wm.core.config.tags.show_alt_names);
        let monitor = wm.core.model.monitor(monitor_id).unwrap();
        assert!(monitor.hide_tags);
        assert!(monitor.show_bottom_bar);
    }

    #[test]
    fn user_keybinds_override_generated_defaults() {
        use crate::actions::{KeyAction, NamedAction};
        use crate::config::keybind_config::{ActionSpec, KeybindSpec};
        use crate::config::keysyms::XK_RETURN;

        let mut user = config_toml::UserConfig::default();
        user.keybinds.push(KeybindSpec {
            modifiers: vec!["super".to_string()],
            key: "return".to_string(),
            action: ActionSpec::WithArgs(vec!["spawn".into(), "alacritty".into()]),
        });

        let effective = resolve_config(user, crate::backend::BackendKind::X11).unwrap();
        let bound: Vec<_> = effective
            .bindings
            .keys
            .iter()
            .filter(|key| key.mod_mask == MODKEY && key.keysym == XK_RETURN)
            .collect();

        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].origin, KeybindOrigin::User);
        assert!(matches!(
            &bound[0].action,
            KeyAction::Named(NamedAction::Spawn(argv)) if argv == &["alacritty"]
        ));
    }

    #[test]
    fn systray_settings_resolve_from_the_user_schema() {
        let mut user = config_toml::UserConfig::default();
        user.systray.menu_backend = crate::core_state::TrayMenuBackend::InstantMenu;
        user.systray.spacing = 6;

        let effective = resolve_config(user, crate::backend::BackendKind::Wayland).unwrap();

        assert_eq!(
            effective.systray.menu_backend,
            crate::core_state::TrayMenuBackend::InstantMenu
        );
        assert_eq!(effective.systray.spacing, 6);
    }
}
