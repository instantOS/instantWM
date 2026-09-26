//! Runtime config get/set/toggle/list over IPC.
//!
//! Thin transport shim: the reflection-by-name reads/writes and their
//! validation live in [`crate::config::runtime`], shared with the
//! `config_set`/`config_toggle` named actions, and this module only applies
//! the returned [`ConfigEffect`] with a full [`Wm`].
//!
//! **Persistence:** edits made through this command live in the running
//! WM only — `reload` reloads from disk and discards them.

use crate::config::runtime::{self, ConfigEffect};
use crate::ipc_types::{ConfigCommand, Response};
use crate::wm::Wm;

pub fn handle_config_command(wm: &mut Wm, cmd: ConfigCommand) -> Response {
    match cmd {
        ConfigCommand::Get { key } => match runtime::get_runtime_field(&wm.core, &key) {
            Ok(value) => Response::ConfigValue(value),
            Err(error) => Response::err(error),
        },
        ConfigCommand::Set { key, value } => {
            match runtime::set_runtime_field(&mut wm.core, &key, value) {
                Ok(effect) => {
                    apply_effect(wm, effect);
                    Response::ok()
                }
                Err(error) => Response::err(error),
            }
        }
        ConfigCommand::Toggle { key } => match runtime::toggle_runtime_field(&mut wm.core, &key) {
            Ok((effect, value)) => {
                apply_effect(wm, effect);
                Response::ConfigValue(value)
            }
            Err(error) => Response::err(error),
        },
        ConfigCommand::List { prefix } => {
            match runtime::list_runtime_fields(&wm.core, prefix.as_deref()) {
                Ok(entries) => Response::ConfigList(entries),
                Err(error) => Response::err(error),
            }
        }
    }
}

/// Apply the follow-up work a config edit requires.
///
/// A `WmCtx` is borrowed purely to run [`crate::actions::apply_config_effect`],
/// the single applier shared with the `config_set`/`config_toggle` actions, so
/// IPC edits and keybind edits cannot drift.
fn apply_effect(wm: &mut Wm, effect: ConfigEffect) {
    crate::actions::apply_config_effect(&mut wm.ctx(), effect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, wayland::WaylandBackend};
    use crate::config::runtime::RuntimeConfigSection;
    use crate::test_support::MonitorBuilder;
    use crate::types::{Monitor, Rect};

    fn test_wm() -> Wm {
        Wm::new(Backend::new_wayland(WaylandBackend::new()))
    }

    fn do_get(wm: &mut Wm, key: &str) -> Response {
        handle_config_command(wm, ConfigCommand::Get { key: key.into() })
    }
    fn do_set(wm: &mut Wm, key: &str, value: &str) -> Response {
        handle_config_command(
            wm,
            ConfigCommand::Set {
                key: key.into(),
                value: value.into(),
            },
        )
    }
    fn do_toggle(wm: &mut Wm, key: &str) -> Response {
        handle_config_command(wm, ConfigCommand::Toggle { key: key.into() })
    }
    fn do_list(wm: &mut Wm) -> Response {
        handle_config_command(wm, ConfigCommand::List { prefix: None })
    }
    fn list_keys(wm: &mut Wm, prefix: &str) -> Result<Vec<String>, String> {
        match handle_config_command(
            wm,
            ConfigCommand::List {
                prefix: Some(prefix.into()),
            },
        ) {
            Response::ConfigList(entries) => Ok(entries.into_iter().map(|(k, _)| k).collect()),
            Response::Err(error) => Err(error),
            other => panic!("expected ConfigList, got {other:?}"),
        }
    }

    #[test]
    fn get_returns_value_and_handles_bad_keys() {
        let mut wm = test_wm();
        match do_get(&mut wm, "window.border_width_px") {
            Response::ConfigValue(v) => assert_eq!(v, "3"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        assert!(matches!(
            do_get(&mut wm, "window.nonexistent"),
            Response::Err(_)
        ));
        assert!(matches!(
            do_get(&mut wm, "nonexistent.field"),
            Response::Err(_)
        ));
        assert!(matches!(do_get(&mut wm, "nodot"), Response::Err(_)));
    }

    #[test]
    fn set_updates_and_roundtrips() {
        let mut wm = test_wm();
        assert!(matches!(
            do_set(&mut wm, "layout.inner_gap", "42"),
            Response::Ok
        ));
        assert_eq!(wm.core.config.layout.inner_gap, 42);

        assert!(matches!(
            do_set(&mut wm, "window.resize_hints", "false"),
            Response::Ok
        ));
        assert!(!wm.core.config.window.resize_hints);

        // Plain string fallback when value isn't valid JSON.
        assert!(matches!(
            do_set(&mut wm, "cursor.theme", "my-cursor"),
            Response::Ok
        ));
        assert_eq!(wm.core.config.cursor.theme, "my-cursor");
        assert!(wm.work.cursor_config);

        match do_get(&mut wm, "layout.inner_gap") {
            Response::ConfigValue(v) => assert_eq!(v, "42"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
    }

    #[test]
    fn toggle_flips_boolean_options_and_returns_the_new_value() {
        let mut wm = test_wm();
        // Defaults: decor_hints on, show_icons off.
        match do_toggle(&mut wm, "window.decor_hints") {
            Response::ConfigValue(v) => assert_eq!(v, "false"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        assert!(!wm.core.config.window.decor_hints);
        match do_toggle(&mut wm, "window.decor_hints") {
            Response::ConfigValue(v) => assert_eq!(v, "true"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        assert!(wm.core.config.window.decor_hints);

        match do_toggle(&mut wm, "tags.show_icons") {
            Response::ConfigValue(v) => assert_eq!(v, "true"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        // The bar reads this live from config, so the flip is effective.
        assert!(wm.core.config.tags.show_icons);
    }

    #[test]
    fn focus_horizontal_edge_round_trips_and_rejects_unknown_policies() {
        let mut wm = test_wm();
        // The default keeps the historical "walk the windows, then the tags".
        match do_get(&mut wm, "focus.horizontal_edge") {
            Response::ConfigValue(v) => assert_eq!(v, "overflow"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }

        do_set(&mut wm, "focus.horizontal_edge", "wrap");
        assert_eq!(
            wm.core.config.focus.horizontal_edge,
            crate::config::config_toml::HorizontalEdge::Wrap
        );
        match do_get(&mut wm, "focus.horizontal_edge") {
            Response::ConfigValue(v) => assert_eq!(v, "wrap"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }

        // An unknown policy must be rejected without mutating the config.
        assert!(matches!(
            do_set(&mut wm, "focus.horizontal_edge", "bounce"),
            Response::Err(_)
        ));
        assert_eq!(
            wm.core.config.focus.horizontal_edge,
            crate::config::config_toml::HorizontalEdge::Wrap
        );
    }

    #[test]
    fn focus_vertical_edge_round_trips_and_rejects_overflow() {
        let mut wm = test_wm();
        // The default answers the boundary by jumping to the far edge.
        match do_get(&mut wm, "focus.vertical_edge") {
            Response::ConfigValue(v) => assert_eq!(v, "wrap"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }

        do_set(&mut wm, "focus.vertical_edge", "none");
        assert_eq!(
            wm.core.config.focus.vertical_edge,
            crate::config::config_toml::VerticalEdge::None
        );
        match do_get(&mut wm, "focus.vertical_edge") {
            Response::ConfigValue(v) => assert_eq!(v, "none"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }

        // `overflow` is the horizontal workspace switch, which has no
        // vertical counterpart, so it must be rejected without mutating.
        assert!(matches!(
            do_set(&mut wm, "focus.vertical_edge", "overflow"),
            Response::Err(_)
        ));
        assert_eq!(
            wm.core.config.focus.vertical_edge,
            crate::config::config_toml::VerticalEdge::None
        );
    }

    #[test]
    fn toggle_flips_input_toggle_settings() {
        let mut wm = test_wm();
        assert!(matches!(
            do_set(&mut wm, "input.type:touchpad.tap", "enabled"),
            Response::Ok
        ));
        match do_toggle(&mut wm, "input.type:touchpad.tap") {
            Response::ConfigValue(v) => assert_eq!(v, "disabled"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        assert_eq!(
            wm.core.config.input["type:touchpad"].tap,
            Some(crate::config::config_toml::ToggleSetting::Disabled)
        );
    }

    #[test]
    fn toggle_rejects_non_boolean_keys_without_mutating() {
        let mut wm = test_wm();
        assert!(matches!(
            do_set(&mut wm, "layout.inner_gap", "42"),
            Response::Ok
        ));
        for key in [
            "layout.inner_gap",           // integer
            "window.focus_follows_mouse", // three-state enum
            "window.border_width_px",     // integer
            "window.nonexistent",         // unknown field
            "nonexistent.field",          // unknown section
            "nodot",                      // malformed key
        ] {
            assert!(
                matches!(do_toggle(&mut wm, key), Response::Err(_)),
                "toggle should reject '{key}'"
            );
        }
        assert_eq!(wm.core.config.layout.inner_gap, 42);
    }

    #[test]
    fn tags_set_takes_effect_live() {
        let mut wm = test_wm();
        assert!(matches!(
            do_set(&mut wm, "tags.show_icons", "true"),
            Response::Ok
        ));
        // The bar reads this straight from config, so an IPC set is live
        // immediately — there is no model copy left to fall out of sync.
        assert!(wm.core.config.tags.show_icons);
        match do_get(&mut wm, "tags.show_icons") {
            Response::ConfigValue(v) => assert_eq!(v, "true"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
    }

    #[test]
    fn the_tag_set_itself_is_readable_but_only_applies_on_reload() {
        let mut wm = test_wm();
        let original = wm.core.config.tags.clone();

        // Readable…
        match do_get(&mut wm, "tags.count") {
            Response::ConfigValue(v) => assert_eq!(v, "20"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        // …but not runtime-settable: the tag count lives in the model.
        for key in ["tags.count", "tags.names", "tags.icons"] {
            assert!(
                matches!(
                    do_set(&mut wm, key, "[\"x\"]"),
                    Response::Err(message) if message.contains("applies on reload")
                ),
                "setting {key} should be rejected"
            );
        }
        assert_eq!(wm.core.config.tags, original);
    }

    #[test]
    fn per_output_tag_display_overrides_apply_live() {
        let mut wm = test_wm();
        wm.core
            .model
            .monitors
            .push(MonitorBuilder::new().named("DP-1").build());
        assert_eq!(
            crate::bar::policy::TagBarPolicy::resolve(&wm.core.config, "DP-1").tag_slots,
            crate::types::tag::DEFAULT_TAG_SLOTS
        );

        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.tag_slots", "5"),
            Response::Ok
        ));
        assert!(!wm.work.monitor_config);
        assert_eq!(
            crate::bar::policy::TagBarPolicy::resolve(&wm.core.config, "DP-1").tag_slots,
            5
        );
    }

    #[test]
    fn invalid_per_output_tag_display_is_rejected() {
        let mut wm = test_wm();
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.tag_slots", "0"),
            Response::Err(message) if message.contains("monitors.DP-1.tag_slots")
        ));
        assert!(!wm.core.config.monitors.contains_key("DP-1"));
    }

    #[test]
    fn invalid_layout_updates_are_rejected_without_changing_config() {
        let mut wm = test_wm();
        let original = wm.core.config.layout;
        assert!(matches!(
            do_set(&mut wm, "layout.inner_gap", "-12"),
            Response::Err(message) if message.contains("layout.inner_gap")
        ));
        assert_eq!(wm.core.config.layout.inner_gap, original.inner_gap);

        assert!(matches!(
            do_set(&mut wm, "layout.minimum_weight", "0.8"),
            Response::Err(message) if message.contains("layout.minimum_weight")
        ));
        assert_eq!(
            wm.core.config.layout.minimum_weight,
            original.minimum_weight
        );
    }

    #[test]
    fn invalid_bar_geometry_is_rejected_without_changing_config() {
        let mut wm = test_wm();
        let original = wm.core.config.bar.clone();

        assert!(matches!(
            do_set(&mut wm, "bar.startmenu_size", "-1"),
            Response::Err(message) if message.contains("bar.startmenu_size")
        ));
        assert_eq!(wm.core.config.bar, original);
    }

    #[test]
    fn font_roles_roundtrip_and_invalid_sizes_are_rejected() {
        let mut wm = test_wm();
        assert!(matches!(
            do_set(&mut wm, "fonts.icon_size", "18"),
            Response::Ok
        ));
        assert_eq!(wm.core.config.fonts.icon_size, 18.0);
        assert!(matches!(
            do_get(&mut wm, "fonts.icon_size"),
            Response::ConfigValue(value) if value == "18.0"
        ));

        assert!(matches!(
            do_set(&mut wm, "fonts.icon_size", "0"),
            Response::Err(message) if message.contains("fonts.icon_size")
        ));
        assert_eq!(wm.core.config.fonts.icon_size, 18.0);
    }

    #[test]
    fn placement_policy_roundtrips_through_runtime_config_ipc() {
        let mut wm = test_wm();
        assert!(matches!(
            do_set(&mut wm, "layout.new_window_placement", "force"),
            Response::Ok
        ));
        assert_eq!(
            wm.core.config.layout.new_window_placement,
            crate::config::config_toml::NewWindowPlacement::Force
        );
        assert!(matches!(
            do_get(&mut wm, "layout.new_window_placement"),
            Response::ConfigValue(value) if value == "force"
        ));
        assert!(matches!(
            do_set(&mut wm, "layout.new_window_placement", "not-a-policy"),
            Response::Err(_)
        ));
    }

    #[test]
    fn animation_speed_roundtrips_and_preserves_valid_runtime_state() {
        let mut wm = test_wm();
        assert!(matches!(
            do_set(&mut wm, "animations.speed", "0.1"),
            Response::Ok
        ));
        assert_eq!(wm.core.config.animations.speed.get(), 0.1);
        assert!(matches!(
            do_get(&mut wm, "animations.speed"),
            Response::ConfigValue(value) if value == "0.1"
        ));

        for invalid in ["0", "-1", "101", r#""slow""#] {
            assert!(matches!(
                do_set(&mut wm, "animations.speed", invalid),
                Response::Err(_)
            ));
            assert_eq!(wm.core.config.animations.speed.get(), 0.1);
        }
    }

    #[test]
    fn set_rejects_bad_inputs() {
        let mut wm = test_wm();
        // Type mismatch (serde rejects).
        assert!(matches!(
            do_set(&mut wm, "window.border_width_px", r#""nope""#),
            Response::Err(_)
        ));
        // Unknown field.
        assert!(matches!(
            do_set(&mut wm, "window.nonexistent", "1"),
            Response::Err(_)
        ));
    }

    #[test]
    fn invalid_window_values_do_not_mutate_runtime_config() {
        let mut wm = test_wm();
        let original = wm.core.config.window.clone();

        assert!(matches!(
            do_set(&mut wm, "window.border_width_px", "-1"),
            Response::Err(_)
        ));
        assert!(matches!(
            do_set(&mut wm, "window.snap_threshold", "-1"),
            Response::Err(_)
        ));
        assert_eq!(
            wm.core.config.window.border_width_px,
            original.border_width_px
        );
        assert_eq!(
            wm.core.config.window.snap_threshold,
            original.snap_threshold
        );
    }

    #[test]
    fn get_returns_unquoted_strings() {
        let mut wm = test_wm();
        do_set(&mut wm, "cursor.theme", "my-cursor");
        match do_get(&mut wm, "cursor.theme") {
            Response::ConfigValue(v) => assert_eq!(v, "my-cursor"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
    }

    #[test]
    fn set_string_fallback_is_type_aware() {
        let mut wm = test_wm();
        // Bare non-JSON value into a string field works (fallback path).
        assert!(matches!(
            do_set(&mut wm, "cursor.theme", "my-cursor"),
            Response::Ok
        ));
        assert_eq!(wm.core.config.cursor.theme, "my-cursor");

        // Bare non-JSON value into a numeric field is rejected as parse
        // error, not silently coerced to a string and then mis-typed.
        assert!(matches!(
            do_set(&mut wm, "window.border_width_px", "nope"),
            Response::Err(_)
        ));
    }

    #[test]
    fn set_option_string_field_with_bare_value() {
        let mut wm = test_wm();
        // monitors.DP-1.position is Option<String>; defaults to None.
        // A bare (non-JSON) value should be accepted as the string.
        let resp = do_set(&mut wm, "monitors.DP-1.position", "0,0");
        assert!(matches!(resp, Response::Ok), "got {resp:?}");
        assert_eq!(
            wm.core
                .config
                .monitors
                .get("DP-1")
                .and_then(|m| m.position.as_deref()),
            Some("0,0")
        );
    }

    #[test]
    fn list_filters_by_section_key_and_map_entry_prefix() {
        let mut wm = test_wm();
        do_set(&mut wm, "input.type:touchpad.tap", r#""enabled""#);

        let fonts = list_keys(&mut wm, "fonts").unwrap();
        assert!(!fonts.is_empty());
        assert!(fonts.iter().all(|key| key.starts_with("fonts.")));

        assert_eq!(
            list_keys(&mut wm, "fonts.icon_size").unwrap(),
            ["fonts.icon_size"]
        );
        assert!(
            list_keys(&mut wm, "input.type:touchpad")
                .unwrap()
                .iter()
                .all(|key| key.starts_with("input.type:touchpad."))
        );
        assert!(list_keys(&mut wm, "fonts.nonexistent").unwrap().is_empty());
        assert!(list_keys(&mut wm, "frobnicate").is_err());
    }

    #[test]
    fn list_includes_fixed_and_map_sections() {
        let mut wm = test_wm();
        do_set(&mut wm, "input.type:touchpad.tap", r#""enabled""#);
        do_set(&mut wm, "monitors.DP-1.enable", "true");
        match do_list(&mut wm) {
            Response::ConfigList(entries) => {
                assert!(entries.iter().any(|(k, _)| k == "layout.inner_gap"));
                assert!(
                    entries
                        .iter()
                        .any(|(k, _)| k.starts_with("input.type:touchpad."))
                );
                assert!(entries.iter().any(|(k, _)| k.starts_with("monitors.DP-1.")));
            }
            other => panic!("expected ConfigList, got {other:?}"),
        }
    }

    #[test]
    fn runtime_config_sections_match_list_output() {
        // The const is the single source of truth for listable sections, so it
        // must agree with the sections `list()` actually emits. Populate the
        // map sections first so input/monitors show up.
        let mut wm = test_wm();
        do_set(&mut wm, "input.type:touchpad.pointer_accel", "0.5");
        do_set(&mut wm, "monitors.DP-1.scale", "2.0");

        let emitted: std::collections::BTreeSet<String> = match do_list(&mut wm) {
            Response::ConfigList(entries) => entries
                .iter()
                .map(|(k, _)| k.split('.').next().unwrap().to_string())
                .collect(),
            other => panic!("expected ConfigList, got {other:?}"),
        };
        let expected: std::collections::BTreeSet<String> = RuntimeConfigSection::ALL
            .into_iter()
            .map(|section| section.name().to_string())
            .collect();
        assert_eq!(
            emitted, expected,
            "typed runtime-config registry drifted from list() output"
        );
    }

    #[test]
    fn input_set_creates_entry_and_queues_apply() {
        let mut wm = test_wm();
        assert!(matches!(
            do_set(&mut wm, "input.type:touchpad.pointer_accel", "0.5"),
            Response::Ok
        ));
        assert!(wm.core.config.input.contains_key("type:touchpad"));
        assert!(wm.work.input_config);

        match do_get(&mut wm, "input.type:touchpad.pointer_accel") {
            Response::ConfigValue(v) => assert_eq!(v, "0.5"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        // Unknown device on get.
        assert!(matches!(
            do_get(&mut wm, "input.nonexistent.tap"),
            Response::Err(_)
        ));
    }

    #[test]
    fn touchscreen_output_mapping_is_runtime_configurable() {
        let mut wm = test_wm();
        assert!(matches!(
            do_set(&mut wm, "input.type:touch.map_to_output", "eDP-1"),
            Response::Ok
        ));
        assert_eq!(
            wm.core
                .config
                .input
                .get("type:touch")
                .and_then(|config| config.map_to_output.as_deref()),
            Some("eDP-1")
        );
        assert!(wm.work.input_config);
        match do_get(&mut wm, "input.type:touch.map_to_output") {
            Response::ConfigValue(value) => assert_eq!(value, "eDP-1"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
    }

    #[test]
    fn monitor_set_creates_entry_and_queues_apply() {
        let mut wm = test_wm();
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.scale", "2.0"),
            Response::Ok
        ));
        assert!(wm.core.config.monitors.contains_key("DP-1"));
        assert!(wm.work.monitor_config);
        assert!(matches!(
            do_get(&mut wm, "monitors.nonexistent.scale"),
            Response::Err(_)
        ));
    }

    #[test]
    fn map_set_does_not_create_entry_on_error() {
        let mut wm = test_wm();

        assert!(matches!(
            do_set(&mut wm, "input.type:touchpad.pointer_accel", r#""fast""#),
            Response::Err(_)
        ));
        assert!(!wm.core.config.input.contains_key("type:touchpad"));
        assert!(!wm.work.input_config);

        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.scale", r#""large""#),
            Response::Err(_)
        ));
        assert!(!wm.core.config.monitors.contains_key("DP-1"));
        assert!(!wm.work.monitor_config);
    }

    #[test]
    fn monitor_mirror_self_reference_is_rejected_without_committing() {
        let mut wm = test_wm();

        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.mirror", "DP-1"),
            Response::Err(message) if message.contains("cannot mirror itself")
        ));
        assert!(!wm.core.config.monitors.contains_key("DP-1"));
        assert!(!wm.work.monitor_config);
    }

    #[test]
    fn monitor_mirror_set_validates_against_the_stored_entry() {
        let mut wm = test_wm();
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.mirror", "HDMI-1"),
            Response::Ok
        ));
        let before = serde_json::to_value(&wm.core.config.monitors).unwrap();

        // Self-reference on a populated entry: rejected, map unchanged.
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.mirror", "DP-1"),
            Response::Err(message) if message.contains("cannot mirror itself")
        ));
        let after = serde_json::to_value(&wm.core.config.monitors).unwrap();
        assert_eq!(before, after);

        // A fatal mirror error on ANOTHER entry must not block edits to this
        // one: seed a broken entry directly, bypassing this command's filter.
        wm.core.config.monitors.insert(
            "HDMI-2".to_owned(),
            crate::config::config_toml::MonitorConfig {
                mirror: Some("HDMI-2".to_owned()),
                ..crate::config::config_toml::MonitorConfig::default()
            },
        );
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.scale", "2.0"),
            Response::Ok
        ));
        assert_eq!(wm.core.config.monitors["DP-1"].scale, Some(2.0));
        assert_eq!(
            wm.core.config.monitors["DP-1"].mirror.as_deref(),
            Some("HDMI-1")
        );
    }

    #[test]
    fn monitor_mirror_empty_value_clears_the_declaration() {
        let mut wm = test_wm();
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.mirror", "HDMI-1"),
            Response::Ok
        ));

        // Bare empty string clears instead of tripping EmptyTarget.
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.mirror", ""),
            Response::Ok
        ));
        assert_eq!(wm.core.config.monitors["DP-1"].mirror, None);

        // JSON string form of the empty value clears as well.
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.mirror", "HDMI-1"),
            Response::Ok
        ));
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.mirror", r#""""#),
            Response::Ok
        ));
        assert_eq!(wm.core.config.monitors["DP-1"].mirror, None);
        assert!(wm.work.monitor_config);
    }

    #[test]
    fn monitor_mirror_fit_set_roundtrips_and_rejects_bad_values() {
        use crate::config::config_toml::MirrorFit;

        let mut wm = test_wm();

        // Bare enum value via the serde string fallback.
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.mirror_fit", "cover"),
            Response::Ok
        ));
        assert_eq!(
            wm.core.config.monitors["DP-1"].mirror_fit,
            Some(MirrorFit::Cover)
        );
        // A fit-only change cannot produce a fatal mirror error, so it
        // commits and queues an apply like any other monitors field.
        assert!(wm.work.monitor_config);

        // Invalid enum values are rejected with the config untouched.
        let before = serde_json::to_value(&wm.core.config.monitors).unwrap();
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.mirror_fit", "sideways"),
            Response::Err(_)
        ));
        assert_eq!(
            serde_json::to_value(&wm.core.config.monitors).unwrap(),
            before
        );

        // "" is not an enum value: unlike `mirror`, fit has no string clear
        // sentinel (clearing happens via the JSON null below or by clearing
        // the mirror itself).
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.mirror_fit", ""),
            Response::Err(_)
        ));
        assert_eq!(
            serde_json::to_value(&wm.core.config.monitors).unwrap(),
            before
        );

        // JSON null clears the fit through the ordinary Option path.
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.mirror_fit", "null"),
            Response::Ok
        ));
        assert_eq!(wm.core.config.monitors["DP-1"].mirror_fit, None);
        assert!(wm.work.monitor_config);
    }

    #[test]
    fn monitor_mirror_fit_set_survives_with_the_mirror_declaration() {
        use crate::config::config_toml::MirrorFit;

        let mut wm = test_wm();

        // Fit set first, mirror second: the mirror validation must not reject
        // or clear the already-stored fit.
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.mirror_fit", "contain"),
            Response::Ok
        ));
        assert!(matches!(
            do_set(&mut wm, "monitors.DP-1.mirror", "HDMI-1"),
            Response::Ok
        ));
        assert_eq!(
            wm.core.config.monitors["DP-1"].mirror.as_deref(),
            Some("HDMI-1")
        );
        assert_eq!(
            wm.core.config.monitors["DP-1"].mirror_fit,
            Some(MirrorFit::Contain)
        );
    }

    #[test]
    fn bar_set_recomputes_monitor_bar_geometry() {
        let mut wm = test_wm();
        let mut monitor = Monitor::new_with_values();
        monitor.monitor_rect = Rect::new(0, 0, 800, 600);
        monitor.available_rect = monitor.monitor_rect;
        wm.core.model.monitors.push(monitor);

        assert!(matches!(do_set(&mut wm, "bar.height", "32"), Response::Ok));

        let monitor = wm.core.model.monitors_iter().next().unwrap().1;
        assert_eq!(monitor.bar_height, 32);
        assert_eq!(monitor.bar_y(), 0);
        assert_eq!(monitor.work_rect(), Rect::new(0, 32, 800, 568));
    }

    #[test]
    fn bar_show_and_top_apply_to_existing_monitor() {
        let mut wm = test_wm();
        let mut monitor = Monitor::new_with_values();
        monitor.monitor_rect = Rect::new(0, 0, 800, 600);
        monitor.available_rect = monitor.monitor_rect;
        // A session `toggle_bar` override on the current view, which an
        // explicit `config set bar.show` must replace.
        monitor.per_tag_state().show_bar = Some(true);
        wm.core.model.monitors.push(monitor);

        assert!(matches!(do_set(&mut wm, "bar.height", "32"), Response::Ok));
        assert_eq!(
            wm.core
                .model
                .expect_selected_monitor()
                .per_tag()
                .unwrap()
                .show_bar,
            Some(true),
            "bar geometry changes preserve per-view visibility"
        );
        assert!(matches!(
            do_set(&mut wm, "bar.tag_slots", "5"),
            Response::Ok
        ));
        assert_eq!(
            wm.core
                .model
                .expect_selected_monitor()
                .per_tag()
                .unwrap()
                .show_bar,
            Some(true),
            "tag baseline changes preserve per-view visibility"
        );
        assert!(matches!(do_set(&mut wm, "bar.show", "false"), Response::Ok));
        let monitor = wm.core.model.monitors_iter().next().unwrap().1;
        assert!(!monitor.bar_default_show);
        assert!(!monitor.shows_bar());
        assert_eq!(monitor.work_rect(), Rect::new(0, 0, 800, 600));

        assert!(matches!(do_set(&mut wm, "bar.show", "true"), Response::Ok));
        let monitor = wm.core.model.monitors_iter().next().unwrap().1;
        assert!(monitor.bar_default_show);
        assert!(monitor.shows_bar());
        assert_eq!(monitor.bar_y(), 0);
        assert_eq!(monitor.work_rect(), Rect::new(0, 32, 800, 568));
    }
}
