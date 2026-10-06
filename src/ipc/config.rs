//! Runtime config get/set/toggle/list over IPC.
//!
//! Thin transport shim: the reflection-by-name reads/writes and their
//! validation live in [`crate::config::runtime`], shared with the
//! `config_set`/`config_toggle` named actions, and this module only applies
//! the returned [`ConfigEffect`] with a borrowed [`WmCtx`].
//!
//! **Persistence:** edits made through this command live in the running
//! WM only — `reload` reloads from disk and discards them.

use crate::config::runtime::{self, ConfigEffect};
use crate::contexts::WmCtx;
use crate::ipc_types::{ConfigCommand, Response};

pub fn handle_config_command(ctx: &mut WmCtx<'_>, cmd: ConfigCommand) -> Response {
    match cmd {
        ConfigCommand::Get { key } => match runtime::get_runtime_field(ctx.state(), &key) {
            Ok(value) => Response::ConfigValue(value),
            Err(error) => Response::err(error),
        },
        ConfigCommand::Set { key, value } => {
            match runtime::set_runtime_field(ctx.state_mut(), &key, value) {
                Ok(effect) => {
                    apply_effect(ctx, effect);
                    Response::ok()
                }
                Err(error) => Response::err(error),
            }
        }
        ConfigCommand::Toggle { key } => {
            match runtime::toggle_runtime_field(ctx.state_mut(), &key) {
                Ok((effect, value)) => {
                    apply_effect(ctx, effect);
                    Response::ConfigValue(value)
                }
                Err(error) => Response::err(error),
            }
        }
        ConfigCommand::List { prefix } => {
            match runtime::list_runtime_fields(ctx.state(), prefix.as_deref()) {
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
fn apply_effect(ctx: &mut WmCtx<'_>, effect: ConfigEffect) {
    crate::actions::apply_config_effect(ctx, effect);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::runtime::RuntimeConfigSection;
    use crate::test_support::MonitorBuilder;
    use crate::test_support::TestWm as Wm;
    use crate::types::{Monitor, Rect};

    fn test_wm() -> Wm {
        Wm::new(crate::backend::WaylandBackendData::default())
    }

    fn do_get(ctx: &mut WmCtx<'_>, key: &str) -> Response {
        handle_config_command(ctx, ConfigCommand::Get { key: key.into() })
    }
    fn do_set(ctx: &mut WmCtx<'_>, key: &str, value: &str) -> Response {
        handle_config_command(
            ctx,
            ConfigCommand::Set {
                key: key.into(),
                value: value.into(),
            },
        )
    }
    fn do_toggle(ctx: &mut WmCtx<'_>, key: &str) -> Response {
        handle_config_command(ctx, ConfigCommand::Toggle { key: key.into() })
    }
    fn do_list(ctx: &mut WmCtx<'_>) -> Response {
        handle_config_command(ctx, ConfigCommand::List { prefix: None })
    }
    fn list_keys(ctx: &mut WmCtx<'_>, prefix: &str) -> Result<Vec<String>, String> {
        match handle_config_command(
            ctx,
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
        match wm.with_ctx(|wm| do_get(wm, "window.border_width_px")) {
            Response::ConfigValue(v) => assert_eq!(v, "3"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        assert!(matches!(
            wm.with_ctx(|wm| do_get(wm, "window.nonexistent")),
            Response::Err(_)
        ));
        assert!(matches!(
            wm.with_ctx(|wm| do_get(wm, "nonexistent.field")),
            Response::Err(_)
        ));
        assert!(matches!(
            wm.with_ctx(|wm| do_get(wm, "nodot")),
            Response::Err(_)
        ));
    }

    #[test]
    fn set_updates_and_roundtrips() {
        let mut wm = test_wm();
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "layout.inner_gap", "42")),
            Response::Ok
        ));
        assert_eq!(wm.core.state.config.layout.inner_gap, 42);

        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "window.resize_hints", "false")),
            Response::Ok
        ));
        assert!(!wm.core.state.config.window.resize_hints);

        // Plain string fallback when value isn't valid JSON.
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "cursor.theme", "my-cursor")),
            Response::Ok
        ));
        assert_eq!(wm.core.state.config.cursor.theme, "my-cursor");
        assert!(wm.core.work.cursor_config);

        match wm.with_ctx(|wm| do_get(wm, "layout.inner_gap")) {
            Response::ConfigValue(v) => assert_eq!(v, "42"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
    }

    #[test]
    fn toggle_flips_boolean_options_and_returns_the_new_value() {
        let mut wm = test_wm();
        // Defaults: decor_hints on, show_icons off.
        match wm.with_ctx(|wm| do_toggle(wm, "window.decor_hints")) {
            Response::ConfigValue(v) => assert_eq!(v, "false"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        assert!(!wm.core.state.config.window.decor_hints);
        match wm.with_ctx(|wm| do_toggle(wm, "window.decor_hints")) {
            Response::ConfigValue(v) => assert_eq!(v, "true"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        assert!(wm.core.state.config.window.decor_hints);

        match wm.with_ctx(|wm| do_toggle(wm, "tags.show_icons")) {
            Response::ConfigValue(v) => assert_eq!(v, "true"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        // The bar reads this live from config, so the flip is effective.
        assert!(wm.core.state.config.tags.show_icons);
    }

    #[test]
    fn focus_horizontal_edge_round_trips_and_rejects_unknown_policies() {
        let mut wm = test_wm();
        // The default keeps the historical "walk the windows, then the tags".
        match wm.with_ctx(|wm| do_get(wm, "focus.horizontal_edge")) {
            Response::ConfigValue(v) => assert_eq!(v, "overflow"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }

        wm.with_ctx(|wm| do_set(wm, "focus.horizontal_edge", "wrap"));
        assert_eq!(
            wm.core.state.config.focus.horizontal_edge,
            crate::config::config_toml::HorizontalEdge::Wrap
        );
        match wm.with_ctx(|wm| do_get(wm, "focus.horizontal_edge")) {
            Response::ConfigValue(v) => assert_eq!(v, "wrap"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }

        // An unknown policy must be rejected without mutating the config.
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "focus.horizontal_edge", "bounce")),
            Response::Err(_)
        ));
        assert_eq!(
            wm.core.state.config.focus.horizontal_edge,
            crate::config::config_toml::HorizontalEdge::Wrap
        );
    }

    #[test]
    fn focus_vertical_edge_round_trips_and_rejects_overflow() {
        let mut wm = test_wm();
        // The default answers the boundary by jumping to the far edge.
        match wm.with_ctx(|wm| do_get(wm, "focus.vertical_edge")) {
            Response::ConfigValue(v) => assert_eq!(v, "wrap"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }

        wm.with_ctx(|wm| do_set(wm, "focus.vertical_edge", "none"));
        assert_eq!(
            wm.core.state.config.focus.vertical_edge,
            crate::config::config_toml::VerticalEdge::None
        );
        match wm.with_ctx(|wm| do_get(wm, "focus.vertical_edge")) {
            Response::ConfigValue(v) => assert_eq!(v, "none"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }

        // `overflow` is the horizontal workspace switch, which has no
        // vertical counterpart, so it must be rejected without mutating.
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "focus.vertical_edge", "overflow")),
            Response::Err(_)
        ));
        assert_eq!(
            wm.core.state.config.focus.vertical_edge,
            crate::config::config_toml::VerticalEdge::None
        );
    }

    #[test]
    fn toggle_flips_input_toggle_settings() {
        let mut wm = test_wm();
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "input.type:touchpad.tap", "enabled")),
            Response::Ok
        ));
        match wm.with_ctx(|wm| do_toggle(wm, "input.type:touchpad.tap")) {
            Response::ConfigValue(v) => assert_eq!(v, "disabled"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        assert_eq!(
            wm.core.state.config.input["type:touchpad"].tap,
            Some(crate::config::config_toml::ToggleSetting::Disabled)
        );
    }

    #[test]
    fn toggle_rejects_non_boolean_keys_without_mutating() {
        let mut wm = test_wm();
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "layout.inner_gap", "42")),
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
                matches!(wm.with_ctx(|wm| do_toggle(wm, key)), Response::Err(_)),
                "toggle should reject '{key}'"
            );
        }
        assert_eq!(wm.core.state.config.layout.inner_gap, 42);
    }

    #[test]
    fn tags_set_takes_effect_live() {
        let mut wm = test_wm();
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "tags.show_icons", "true")),
            Response::Ok
        ));
        // The bar reads this straight from config, so an IPC set is live
        // immediately — there is no model copy left to fall out of sync.
        assert!(wm.core.state.config.tags.show_icons);
        match wm.with_ctx(|wm| do_get(wm, "tags.show_icons")) {
            Response::ConfigValue(v) => assert_eq!(v, "true"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
    }

    #[test]
    fn the_tag_set_itself_is_readable_but_only_applies_on_reload() {
        let mut wm = test_wm();
        let original = wm.core.state.config.tags.clone();

        // Readable…
        match wm.with_ctx(|wm| do_get(wm, "tags.count")) {
            Response::ConfigValue(v) => assert_eq!(v, "20"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        // …but not runtime-settable: the tag count lives in the model.
        for key in ["tags.count", "tags.names", "tags.icons"] {
            assert!(
                matches!(
                    wm.with_ctx(|wm| do_set(wm, key, "[\"x\"]")),
                    Response::Err(message) if message.contains("applies on reload")
                ),
                "setting {key} should be rejected"
            );
        }
        assert_eq!(wm.core.state.config.tags, original);
    }

    #[test]
    fn per_output_tag_display_overrides_apply_live() {
        let mut wm = test_wm();
        wm.core
            .state
            .model
            .monitors
            .push(MonitorBuilder::new().named("DP-1").build());
        assert_eq!(
            crate::bar::policy::TagBarPolicy::resolve(&wm.core.state.config, "DP-1").tag_slots,
            crate::types::tag::DEFAULT_TAG_SLOTS
        );

        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.tag_slots", "5")),
            Response::Ok
        ));
        assert!(!wm.core.work.monitor_config);
        assert_eq!(
            crate::bar::policy::TagBarPolicy::resolve(&wm.core.state.config, "DP-1").tag_slots,
            5
        );
    }

    #[test]
    fn invalid_per_output_tag_display_is_rejected() {
        let mut wm = test_wm();
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.tag_slots", "0")),
            Response::Err(message) if message.contains("monitors.DP-1.tag_slots")
        ));
        assert!(!wm.core.state.config.monitors.contains_key("DP-1"));
    }

    #[test]
    fn invalid_layout_updates_are_rejected_without_changing_config() {
        let mut wm = test_wm();
        let original = wm.core.state.config.layout;
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "layout.inner_gap", "-12")),
            Response::Err(message) if message.contains("layout.inner_gap")
        ));
        assert_eq!(wm.core.state.config.layout.inner_gap, original.inner_gap);

        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "layout.minimum_weight", "0.8")),
            Response::Err(message) if message.contains("layout.minimum_weight")
        ));
        assert_eq!(
            wm.core.state.config.layout.minimum_weight,
            original.minimum_weight
        );
    }

    #[test]
    fn invalid_bar_geometry_is_rejected_without_changing_config() {
        let mut wm = test_wm();
        let original = wm.core.state.config.bar.clone();

        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "bar.startmenu_size", "-1")),
            Response::Err(message) if message.contains("bar.startmenu_size")
        ));
        assert_eq!(wm.core.state.config.bar, original);
    }

    #[test]
    fn font_roles_roundtrip_and_invalid_sizes_are_rejected() {
        let mut wm = test_wm();
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "fonts.icon_size", "18")),
            Response::Ok
        ));
        assert_eq!(wm.core.state.config.fonts.icon_size, 18.0);
        assert!(matches!(
            wm.with_ctx(|wm| do_get(wm, "fonts.icon_size")),
            Response::ConfigValue(value) if value == "18.0"
        ));

        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "fonts.icon_size", "0")),
            Response::Err(message) if message.contains("fonts.icon_size")
        ));
        assert_eq!(wm.core.state.config.fonts.icon_size, 18.0);
    }

    #[test]
    fn placement_policy_roundtrips_through_runtime_config_ipc() {
        let mut wm = test_wm();
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "layout.new_window_placement", "force")),
            Response::Ok
        ));
        assert_eq!(
            wm.core.state.config.layout.new_window_placement,
            crate::config::config_toml::NewWindowPlacement::Force
        );
        assert!(matches!(
            wm.with_ctx(|wm| do_get(wm, "layout.new_window_placement")),
            Response::ConfigValue(value) if value == "force"
        ));
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "layout.new_window_placement", "not-a-policy")),
            Response::Err(_)
        ));
    }

    #[test]
    fn animation_speed_roundtrips_and_preserves_valid_runtime_state() {
        let mut wm = test_wm();
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "animations.speed", "0.1")),
            Response::Ok
        ));
        assert_eq!(wm.core.state.config.animations.speed.get(), 0.1);
        assert!(matches!(
            wm.with_ctx(|wm| do_get(wm, "animations.speed")),
            Response::ConfigValue(value) if value == "0.1"
        ));

        for invalid in ["0", "-1", "101", r#""slow""#] {
            assert!(matches!(
                wm.with_ctx(|wm| do_set(wm, "animations.speed", invalid)),
                Response::Err(_)
            ));
            assert_eq!(wm.core.state.config.animations.speed.get(), 0.1);
        }
    }

    #[test]
    fn set_rejects_bad_inputs() {
        let mut wm = test_wm();
        // Type mismatch (serde rejects).
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "window.border_width_px", r#""nope""#)),
            Response::Err(_)
        ));
        // Unknown field.
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "window.nonexistent", "1")),
            Response::Err(_)
        ));
    }

    #[test]
    fn invalid_window_values_do_not_mutate_runtime_config() {
        let mut wm = test_wm();
        let original = wm.core.state.config.window.clone();

        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "window.border_width_px", "-1")),
            Response::Err(_)
        ));
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "window.snap_threshold", "-1")),
            Response::Err(_)
        ));
        assert_eq!(
            wm.core.state.config.window.border_width_px,
            original.border_width_px
        );
        assert_eq!(
            wm.core.state.config.window.snap_threshold,
            original.snap_threshold
        );
    }

    #[test]
    fn get_returns_unquoted_strings() {
        let mut wm = test_wm();
        wm.with_ctx(|wm| do_set(wm, "cursor.theme", "my-cursor"));
        match wm.with_ctx(|wm| do_get(wm, "cursor.theme")) {
            Response::ConfigValue(v) => assert_eq!(v, "my-cursor"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
    }

    #[test]
    fn set_string_fallback_is_type_aware() {
        let mut wm = test_wm();
        // Bare non-JSON value into a string field works (fallback path).
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "cursor.theme", "my-cursor")),
            Response::Ok
        ));
        assert_eq!(wm.core.state.config.cursor.theme, "my-cursor");

        // Bare non-JSON value into a numeric field is rejected as parse
        // error, not silently coerced to a string and then mis-typed.
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "window.border_width_px", "nope")),
            Response::Err(_)
        ));
    }

    #[test]
    fn set_option_string_field_with_bare_value() {
        let mut wm = test_wm();
        // monitors.DP-1.position is Option<String>; defaults to None.
        // A bare (non-JSON) value should be accepted as the string.
        let resp = wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.position", "0,0"));
        assert!(matches!(resp, Response::Ok), "got {resp:?}");
        assert_eq!(
            wm.core
                .state
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
        wm.with_ctx(|wm| do_set(wm, "input.type:touchpad.tap", r#""enabled""#));

        let fonts = wm.with_ctx(|wm| list_keys(wm, "fonts")).unwrap();
        assert!(!fonts.is_empty());
        assert!(fonts.iter().all(|key| key.starts_with("fonts.")));

        assert_eq!(
            wm.with_ctx(|wm| list_keys(wm, "fonts.icon_size")).unwrap(),
            ["fonts.icon_size"]
        );
        assert!(
            wm.with_ctx(|wm| list_keys(wm, "input.type:touchpad"))
                .unwrap()
                .iter()
                .all(|key| key.starts_with("input.type:touchpad."))
        );
        assert!(
            wm.with_ctx(|wm| list_keys(wm, "fonts.nonexistent"))
                .unwrap()
                .is_empty()
        );
        assert!(wm.with_ctx(|wm| list_keys(wm, "frobnicate")).is_err());
    }

    #[test]
    fn list_includes_fixed_and_map_sections() {
        let mut wm = test_wm();
        wm.with_ctx(|wm| do_set(wm, "input.type:touchpad.tap", r#""enabled""#));
        wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.enable", "true"));
        match wm.with_ctx(do_list) {
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
        wm.with_ctx(|wm| do_set(wm, "input.type:touchpad.pointer_accel", "0.5"));
        wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.scale", "2.0"));

        let emitted: std::collections::BTreeSet<String> = match wm.with_ctx(do_list) {
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
            wm.with_ctx(|wm| do_set(wm, "input.type:touchpad.pointer_accel", "0.5")),
            Response::Ok
        ));
        assert!(wm.core.state.config.input.contains_key("type:touchpad"));
        assert!(wm.core.work.input_config);

        match wm.with_ctx(|wm| do_get(wm, "input.type:touchpad.pointer_accel")) {
            Response::ConfigValue(v) => assert_eq!(v, "0.5"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
        // Unknown device on get.
        assert!(matches!(
            wm.with_ctx(|wm| do_get(wm, "input.nonexistent.tap")),
            Response::Err(_)
        ));
    }

    #[test]
    fn touchscreen_output_mapping_is_runtime_configurable() {
        let mut wm = test_wm();
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "input.type:touch.map_to_output", "eDP-1")),
            Response::Ok
        ));
        assert_eq!(
            wm.core
                .state
                .config
                .input
                .get("type:touch")
                .and_then(|config| config.map_to_output.as_deref()),
            Some("eDP-1")
        );
        assert!(wm.core.work.input_config);
        match wm.with_ctx(|wm| do_get(wm, "input.type:touch.map_to_output")) {
            Response::ConfigValue(value) => assert_eq!(value, "eDP-1"),
            other => panic!("expected ConfigValue, got {other:?}"),
        }
    }

    #[test]
    fn monitor_set_creates_entry_and_queues_apply() {
        let mut wm = test_wm();
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.scale", "2.0")),
            Response::Ok
        ));
        assert!(wm.core.state.config.monitors.contains_key("DP-1"));
        assert!(wm.core.work.monitor_config);
        assert!(matches!(
            wm.with_ctx(|wm| do_get(wm, "monitors.nonexistent.scale")),
            Response::Err(_)
        ));
    }

    #[test]
    fn map_set_does_not_create_entry_on_error() {
        let mut wm = test_wm();

        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "input.type:touchpad.pointer_accel", r#""fast""#)),
            Response::Err(_)
        ));
        assert!(!wm.core.state.config.input.contains_key("type:touchpad"));
        assert!(!wm.core.work.input_config);

        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.scale", r#""large""#)),
            Response::Err(_)
        ));
        assert!(!wm.core.state.config.monitors.contains_key("DP-1"));
        assert!(!wm.core.work.monitor_config);
    }

    #[test]
    fn monitor_mirror_self_reference_is_rejected_without_committing() {
        let mut wm = test_wm();

        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.mirror", "DP-1")),
            Response::Err(message) if message.contains("cannot mirror itself")
        ));
        assert!(!wm.core.state.config.monitors.contains_key("DP-1"));
        assert!(!wm.core.work.monitor_config);
    }

    #[test]
    fn monitor_mirror_set_validates_against_the_stored_entry() {
        let mut wm = test_wm();
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.mirror", "HDMI-1")),
            Response::Ok
        ));
        let before = serde_json::to_value(&wm.core.state.config.monitors).unwrap();

        // Self-reference on a populated entry: rejected, map unchanged.
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.mirror", "DP-1")),
            Response::Err(message) if message.contains("cannot mirror itself")
        ));
        let after = serde_json::to_value(&wm.core.state.config.monitors).unwrap();
        assert_eq!(before, after);

        // A fatal mirror error on ANOTHER entry must not block edits to this
        // one: seed a broken entry directly, bypassing this command's filter.
        wm.core.state.config.monitors.insert(
            "HDMI-2".to_owned(),
            crate::config::config_toml::MonitorConfig {
                mirror: Some("HDMI-2".to_owned()),
                ..crate::config::config_toml::MonitorConfig::default()
            },
        );
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.scale", "2.0")),
            Response::Ok
        ));
        assert_eq!(wm.core.state.config.monitors["DP-1"].scale, Some(2.0));
        assert_eq!(
            wm.core.state.config.monitors["DP-1"].mirror.as_deref(),
            Some("HDMI-1")
        );
    }

    #[test]
    fn monitor_mirror_empty_value_clears_the_declaration() {
        let mut wm = test_wm();
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.mirror", "HDMI-1")),
            Response::Ok
        ));

        // Bare empty string clears instead of tripping EmptyTarget.
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.mirror", "")),
            Response::Ok
        ));
        assert_eq!(wm.core.state.config.monitors["DP-1"].mirror, None);

        // JSON string form of the empty value clears as well.
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.mirror", "HDMI-1")),
            Response::Ok
        ));
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.mirror", r#""""#)),
            Response::Ok
        ));
        assert_eq!(wm.core.state.config.monitors["DP-1"].mirror, None);
        assert!(wm.core.work.monitor_config);
    }

    #[test]
    fn monitor_mirror_fit_set_roundtrips_and_rejects_bad_values() {
        use crate::config::config_toml::MirrorFit;

        let mut wm = test_wm();

        // Bare enum value via the serde string fallback.
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.mirror_fit", "cover")),
            Response::Ok
        ));
        assert_eq!(
            wm.core.state.config.monitors["DP-1"].mirror_fit,
            Some(MirrorFit::Cover)
        );
        // A fit-only change cannot produce a fatal mirror error, so it
        // commits and queues an apply like any other monitors field.
        assert!(wm.core.work.monitor_config);

        // Invalid enum values are rejected with the config untouched.
        let before = serde_json::to_value(&wm.core.state.config.monitors).unwrap();
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.mirror_fit", "sideways")),
            Response::Err(_)
        ));
        assert_eq!(
            serde_json::to_value(&wm.core.state.config.monitors).unwrap(),
            before
        );

        // "" is not an enum value: unlike `mirror`, fit has no string clear
        // sentinel (clearing happens via the JSON null below or by clearing
        // the mirror itself).
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.mirror_fit", "")),
            Response::Err(_)
        ));
        assert_eq!(
            serde_json::to_value(&wm.core.state.config.monitors).unwrap(),
            before
        );

        // JSON null clears the fit through the ordinary Option path.
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.mirror_fit", "null")),
            Response::Ok
        ));
        assert_eq!(wm.core.state.config.monitors["DP-1"].mirror_fit, None);
        assert!(wm.core.work.monitor_config);
    }

    #[test]
    fn monitor_mirror_fit_set_survives_with_the_mirror_declaration() {
        use crate::config::config_toml::MirrorFit;

        let mut wm = test_wm();

        // Fit set first, mirror second: the mirror validation must not reject
        // or clear the already-stored fit.
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.mirror_fit", "contain")),
            Response::Ok
        ));
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "monitors.DP-1.mirror", "HDMI-1")),
            Response::Ok
        ));
        assert_eq!(
            wm.core.state.config.monitors["DP-1"].mirror.as_deref(),
            Some("HDMI-1")
        );
        assert_eq!(
            wm.core.state.config.monitors["DP-1"].mirror_fit,
            Some(MirrorFit::Contain)
        );
    }

    #[test]
    fn bar_set_recomputes_monitor_bar_geometry() {
        let mut wm = test_wm();
        let mut monitor = Monitor::new_with_values();
        monitor.monitor_rect = Rect::new(0, 0, 800, 600);
        monitor.available_rect = monitor.monitor_rect;
        wm.core.state.model.monitors.push(monitor);

        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "bar.height", "32")),
            Response::Ok
        ));

        let monitor = wm.core.state.model.monitors_iter().next().unwrap().1;
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
        wm.core.state.model.monitors.push(monitor);

        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "bar.height", "32")),
            Response::Ok
        ));
        assert_eq!(
            wm.core
                .state
                .model
                .expect_selected_monitor()
                .per_tag()
                .unwrap()
                .show_bar,
            Some(true),
            "bar geometry changes preserve per-view visibility"
        );
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "bar.tag_slots", "5")),
            Response::Ok
        ));
        assert_eq!(
            wm.core
                .state
                .model
                .expect_selected_monitor()
                .per_tag()
                .unwrap()
                .show_bar,
            Some(true),
            "tag baseline changes preserve per-view visibility"
        );
        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "bar.show", "false")),
            Response::Ok
        ));
        let monitor = wm.core.state.model.monitors_iter().next().unwrap().1;
        assert!(!monitor.bar_default_show);
        assert!(!monitor.shows_bar());
        assert_eq!(monitor.work_rect(), Rect::new(0, 0, 800, 600));

        assert!(matches!(
            wm.with_ctx(|wm| do_set(wm, "bar.show", "true")),
            Response::Ok
        ));
        let monitor = wm.core.state.model.monitors_iter().next().unwrap().1;
        assert!(monitor.bar_default_show);
        assert!(monitor.shows_bar());
        assert_eq!(monitor.bar_y(), 0);
        assert_eq!(monitor.work_rect(), Rect::new(0, 32, 800, 568));
    }
}
