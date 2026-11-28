//! Window manager configuration.
//!
//! This module is the single place to tune instantWM behaviour.  It is split
//! into focused sub-modules so you can find what you need quickly:
//!
//! | Module            | What lives there                                        |
//! |-------------------|---------------------------------------------------------|
//! | [`appearance`]    | Color palette and per-scheme color tables                |
//! | [`keybindings`]   | Normal-mode key bindings (`default_keybinds`, `get_desktop_keybinds`) |
//! | [`buttons`]       | Mouse button bindings (`get_buttons`)                   |
//! | [`rules`]         | Window rules (`get_rules`)                              |
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

use crate::types::KeybindOrigin;
// ---------------------------------------------------------------------------
// Module-level constants
// ---------------------------------------------------------------------------

/// Shared constants referenced by multiple sub-modules.
pub mod mod_consts {
    /// Default border width in pixels.
    pub const BORDER_PX: i32 = 3;
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

/// Configuration loaded for initial startup, together with diagnostics from the
/// attempt. Startup keeps running with built-in defaults when loading fails.
pub struct StartupConfig {
    /// Fully resolved configuration to install.
    pub config: EffectiveConfig,
    /// Diagnostics from the load attempt. Empty when loading succeeded.
    pub errors: Vec<String>,
}

impl StartupConfig {
    fn from_result(
        backend: crate::backend::BackendKind,
        result: Result<EffectiveConfig, String>,
    ) -> Self {
        match result {
            Ok(config) => Self {
                config,
                errors: Vec::new(),
            },
            Err(error) => {
                eprintln!("instantwm: {error}; using built-in configuration");
                Self {
                    config: default_config(backend),
                    errors: vec![error],
                }
            }
        }
    }
}

/// Load configuration for initial startup. Unlike reload, startup has no
/// previous valid snapshot to retain, so a reported error falls back to the
/// built-in configuration and allows the WM to start.
pub fn load_startup_config(backend: crate::backend::BackendKind) -> StartupConfig {
    StartupConfig::from_result(backend, load_config(backend))
}

/// Merge one keybind table, recording its diagnostics instead of failing
/// immediately so `resolve_config` can report errors from every table at once.
fn merge_configured_keybinds(
    defaults: Vec<Key>,
    specs: &[keybind_config::KeybindSpec],
    context: &str,
    errors: &mut Vec<String>,
) -> Vec<Key> {
    match keybind_config::merge_keybinds(defaults, specs, KeybindOrigin::User, context) {
        Ok(keys) => keys,
        Err(error) => {
            errors.push(error);
            Vec::new()
        }
    }
}

/// Resolve a parsed user configuration into the complete effective snapshot.
/// This is the sole user-to-runtime conversion boundary.
///
/// Invalid keybinds are configuration errors: every invalid entry is collected
/// (with its table and index) and returned as one error instead of being
/// skipped, so a reload cannot silently install a partially bound config.
pub fn resolve_config(
    mut user: config_toml::UserConfig,
    backend: crate::backend::BackendKind,
) -> Result<EffectiveConfig, String> {
    let layout = user.layout.validated()?;
    user.fonts = user.fonts.validated()?;
    let mut keybind_errors = Vec::new();
    let keys = merge_configured_keybinds(
        keybindings::default_keybinds(backend),
        &user.keybinds,
        "keybinds",
        &mut keybind_errors,
    );
    let desktop_defaults = if user.disable_default_desktop_keybinds {
        Vec::new()
    } else {
        keybindings::get_desktop_keybinds()
    };
    let desktop_keybinds = merge_configured_keybinds(
        desktop_defaults,
        &user.desktop_keybinds,
        "desktop_keybinds",
        &mut keybind_errors,
    );

    let mut modes = HashMap::new();

    // Helper for merging mode keybinds
    let merge_mode = |spec: Option<&config_toml::ModeSpec>,
                      default_desc: &str,
                      default_transient: bool,
                      default_keybinds: Vec<Key>,
                      context: &str,
                      errors: &mut Vec<String>|
     -> ModeConfig {
        if let Some(spec) = spec {
            let keybinds =
                merge_configured_keybinds(default_keybinds, &spec.keybinds, context, errors);
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
        merge_mode(
            user.modes.get("prefix"),
            "prefix",
            true,
            Vec::new(),
            "modes.prefix.keybinds",
            &mut keybind_errors,
        ),
    );

    modes.insert(
        "desktop".to_string(),
        merge_mode(
            user.modes.get("desktop"),
            "desktop",
            false,
            Vec::new(),
            "modes.desktop.keybinds",
            &mut keybind_errors,
        ),
    );

    let mut placement_session_mode = merge_mode(
        user.modes
            .get(crate::core_state::PLACEMENT_SESSION_MODE_NAME),
        "place window",
        false,
        keybindings::get_tree_placement_keybinds(),
        "modes.placement.keybinds",
        &mut keybind_errors,
    );
    // The placement session has a transactional apply/cancel lifecycle; treating one
    // command as transient would discard that transaction mid-navigation.
    placement_session_mode.transient = false;
    modes.insert(
        crate::core_state::PLACEMENT_SESSION_MODE_NAME.to_string(),
        placement_session_mode,
    );

    // Add all other user-defined modes
    for (name, spec) in &user.modes {
        if name == "prefix"
            || name == "desktop"
            || name == crate::core_state::PLACEMENT_SESSION_MODE_NAME
        {
            continue;
        }
        let keybinds = merge_configured_keybinds(
            Vec::new(),
            &spec.keybinds,
            &format!("modes.{name}.keybinds"),
            &mut keybind_errors,
        );
        modes.insert(
            name.clone(),
            ModeConfig {
                description: spec.description.clone(),
                transient: spec.transient.unwrap_or(false),
                keybinds,
            },
        );
    }

    if !keybind_errors.is_empty() {
        return Err(keybind_errors.join("\n"));
    }

    let bar = user.bar.validated()?;
    let window = WindowConfig {
        // The legacy top-level key and the `[window]` section are merged so
        // either spelling enables the opt-in behaviour.
        raise_floating_on_click: user.raise_floating_on_click
            || user.window.raise_floating_on_click,
        ..user.window
    }
    .validated()?;
    let hooks = hooks::resolve_hooks(std::mem::take(&mut user.hooks))?;
    let mut keyboard = user.keyboard;
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

    let tags = user.tags.validated()?;
    let tag_template = tags.tag_template();
    for (output, entry) in &user.monitors {
        entry.validated(output)?;
    }

    Ok(EffectiveConfig {
        window,
        bar,
        systray: user.systray,
        tags,
        layout,
        animations: user.animations,
        colors: user.colors,
        theme: user.theme,
        bindings: BindingConfig {
            keys,
            desktop_keybinds,
            modes,
            buttons: buttons::get_buttons(),
            rules: rules::merge_rules(rules::get_rules(), user.rules),
        },
        fonts: user.fonts,
        tag_template,
        keyboard,
        input: user.input,
        monitors: user.monitors,
        status_command: user.status_command,
        cursor: user.cursor,
        focus: user.focus,
        exec_once: user.exec_once,
        exec: user.exec,
        hooks,
    })
}

#[cfg(test)]
mod resolution_tests {
    use super::*;
    use crate::core_state::SystrayConfig;
    use crate::wm::WaylandWm as Wm;

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
    fn startup_failure_falls_back_and_keeps_the_error() {
        let error = "config parse error in config.toml".to_string();

        let outcome =
            StartupConfig::from_result(crate::backend::BackendKind::Wayland, Err(error.clone()));

        assert_eq!(outcome.errors, vec![error]);
        assert_eq!(
            outcome.config.tags.count,
            default_config(crate::backend::BackendKind::Wayland)
                .tags
                .count
        );
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
        assert_eq!(effective.tag_template.len(), crate::types::SCRATCHPAD_TAG);
        // Stock configuration: numbered names, no icons.
        assert_eq!(effective.tag_template[0].name, "1");
        assert!(effective.tag_template.iter().all(|tag| tag.icon.is_empty()));
        assert!(!effective.tags.show_icons);
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
        use crate::types::Rect;

        let user: config_toml::UserConfig = toml::from_str(
            r#"
            [tags]
            count = 3
            names = ["web", "mail", "code"]
            icons = ["W", "", "C"]
            show_icons = true
            [bar]
            tag_slots = 2
            show_bottom = true
            "#,
        )
        .unwrap();
        let config = crate::config::resolve_config(user, crate::backend::BackendKind::Wayland)
            .expect("valid config");

        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        // Only `monitor_rect` is set: the original fixture left
        // `available_rect` at its default, so the builder's `monitor_rect()`
        // (which sets both) would widen the work area and change the fixture.
        let monitor_id = wm.core.state.model.monitors.push(
            crate::test_support::MonitorBuilder::new()
                .configure(|monitor| monitor.monitor_rect = Rect::new(0, 0, 800, 600))
                .build(),
        );
        wm.core.state.apply_config(config).unwrap();

        // Icon display is read live from config; the tag set seeds the
        // model, and bar states are seeded into each monitor.
        assert!(wm.core.state.config.tags.show_icons);
        let monitor = wm.core.state.model.monitor(monitor_id).unwrap();
        assert!(monitor.show_bottom_bar);
        assert_eq!(wm.core.state.model.tags.num_tags, 3);
        assert_eq!(
            monitor
                .tags
                .iter()
                .map(|tag| (tag.name.as_str(), tag.icon.as_str()))
                .collect::<Vec<_>>(),
            vec![("web", "W"), ("mail", ""), ("code", "C")]
        );
    }

    #[test]
    fn invalid_tag_sets_are_rejected_during_resolution() {
        let user: config_toml::UserConfig =
            toml::from_str("[tags]\ncount = 1\nnames = [\"a\"]\nicons = [\"\", \"\"]").unwrap();

        let error = match resolve_config(user, crate::backend::BackendKind::Wayland) {
            Ok(_) => panic!("an oversized tags.icons list must be rejected"),
            Err(error) => error,
        };
        assert!(error.contains("tags.icons"), "{error}");
    }

    #[test]
    fn user_keybinds_override_generated_defaults() {
        use crate::actions::{KeyAction, NamedAction};
        use crate::config::keybind_config::{ActionSpec, KeybindSpec};
        use crate::config::keybindings::MODKEY;
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
    fn default_desktop_keybinds_resolve_unless_disabled() {
        let effective = resolve_config(
            config_toml::UserConfig::default(),
            crate::backend::BackendKind::X11,
        )
        .unwrap();

        assert_eq!(
            effective.bindings.desktop_keybinds.len(),
            keybindings::get_desktop_keybinds().len()
        );
        assert!(
            effective
                .bindings
                .desktop_keybinds
                .iter()
                .all(|key| key.origin == KeybindOrigin::CompiledDefault)
        );
    }

    #[test]
    fn disabling_default_desktop_keybinds_keeps_configured_ones() {
        use crate::actions::{KeyAction, NamedAction};
        use crate::config::keybind_config::{ActionSpec, KeybindSpec};
        use crate::config::keybindings::MODKEY;
        use crate::config::keysyms::XK_RETURN;

        let mut user = config_toml::UserConfig::default();
        user.disable_default_desktop_keybinds = true;
        user.desktop_keybinds.push(KeybindSpec {
            modifiers: vec!["super".to_string()],
            key: "return".to_string(),
            action: ActionSpec::Named("terminal".to_string()),
        });

        let effective = resolve_config(user, crate::backend::BackendKind::Wayland).unwrap();

        assert_eq!(effective.bindings.desktop_keybinds.len(), 1);
        let key = &effective.bindings.desktop_keybinds[0];
        assert_eq!(key.mod_mask, MODKEY);
        assert_eq!(key.keysym, XK_RETURN);
        assert_eq!(key.origin, KeybindOrigin::User);
        assert!(matches!(
            key.action,
            KeyAction::Named(NamedAction::Terminal)
        ));
        // Ordinary keybinds are a separate table and keep their defaults.
        assert!(!effective.bindings.keys.is_empty());
    }

    #[test]
    fn invalid_keybinds_fail_resolution_with_their_mode_context() {
        let user: config_toml::UserConfig = toml::from_str(
            r#"
            [[keybinds]]
            key = "nokey"
            action = "zoom"

            [modes.resize]
            [[modes.resize.keybinds]]
            key = "p"
            action = "does_not_exist"
            "#,
        )
        .unwrap();

        let error = match resolve_config(user, crate::backend::BackendKind::Wayland) {
            Ok(_) => panic!("invalid keybinds must be rejected"),
            Err(error) => error,
        };

        assert!(error.contains("keybinds[0]"), "{error}");
        assert!(error.contains("nokey"), "{error}");
        assert!(error.contains("modes.resize.keybinds[0]"), "{error}");
        assert!(error.contains("does_not_exist"), "{error}");
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
