//! Runtime config get/set/list over IPC.
//!
//! Each fixed section (`window`, `bar`, ...) round-trips through serde_json
//! to read/write fields by name. The two HashMap sections (`input`,
//! `monitors`) take a `<section>.<id>.<field>` key and auto-create missing
//! entries so users can add new device/monitor configs at runtime.
//!
//! **Persistence:** edits made through this command live in the running
//! WM only — `reload` reloads from disk and discards them.
//!
//! **Read-only fields:** `display.width`/`display.height` are derived from
//! the actual outputs, so the entire `display` section is hidden from
//! `get`/`set`/`list`.

use crate::ipc_types::{ConfigCommand, Response};
use crate::wm::Wm;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Typed section registry. Parsing strings once and dispatching exhaustively on
// this enum prevents get/set/list and the CLI validator from silently drifting.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeConfigSection {
    Window,
    Bar,
    Systray,
    Layout,
    Animations,
    Colors,
    Cursor,
    Fonts,
    Input,
    Monitors,
    Display,
}

impl RuntimeConfigSection {
    pub const EXPOSED: [Self; 10] = [
        Self::Window,
        Self::Bar,
        Self::Systray,
        Self::Layout,
        Self::Animations,
        Self::Colors,
        Self::Cursor,
        Self::Fonts,
        Self::Input,
        Self::Monitors,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Window => "window",
            Self::Bar => "bar",
            Self::Systray => "systray",
            Self::Layout => "layout",
            Self::Animations => "animations",
            Self::Colors => "colors",
            Self::Cursor => "cursor",
            Self::Fonts => "fonts",
            Self::Input => "input",
            Self::Monitors => "monitors",
            Self::Display => "display",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::EXPOSED
            .into_iter()
            .chain([Self::Display])
            .find(|section| section.name() == name)
    }
}

/// How a top-level section name is exposed by this IPC surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionStatus {
    /// Normal section: listed and read/written (`window`, `layout`, …).
    Exposed,
    /// Real section, but derived from outputs — hidden from get/set/list.
    Hidden,
    /// Not a section this IPC surface knows about.
    Unknown,
}

/// Classify a top-level section name. Lets the client produce helpful errors
/// for `config list <bad-section>` without duplicating the section list.
pub fn section_status(name: &str) -> SectionStatus {
    match RuntimeConfigSection::parse(name) {
        Some(RuntimeConfigSection::Display) => SectionStatus::Hidden,
        Some(_) => SectionStatus::Exposed,
        None => SectionStatus::Unknown,
    }
}

pub fn handle_config_command(wm: &mut Wm, cmd: ConfigCommand) -> Response {
    match cmd {
        ConfigCommand::Get { key } => get(wm, &key),
        ConfigCommand::Set { key, value } => set(wm, &key, value),
        ConfigCommand::List => list(wm),
    }
}

fn get(wm: &Wm, key: &str) -> Response {
    let Some((section_name, rest)) = key.split_once('.') else {
        return Response::err("key must be 'section.field' (e.g. layout.inner_gap)");
    };
    let Some(section) = RuntimeConfigSection::parse(section_name) else {
        return Response::err(format!("unknown section '{section_name}'"));
    };
    let state = &wm.core;
    let val = match section {
        RuntimeConfigSection::Window => field_get(&state.config.window, rest),
        RuntimeConfigSection::Bar => field_get(&state.config.bar, rest),
        RuntimeConfigSection::Systray => field_get(&state.config.systray, rest),
        RuntimeConfigSection::Layout => field_get(&state.config.layout, rest),
        RuntimeConfigSection::Animations => field_get(&state.config.animations, rest),
        RuntimeConfigSection::Colors => field_get(&state.config.colors, rest),
        RuntimeConfigSection::Cursor => field_get(&state.config.cursor, rest),
        RuntimeConfigSection::Fonts => field_get(&state.config.fonts, rest),
        RuntimeConfigSection::Input => return map_get(&state.config.input, section.name(), rest),
        RuntimeConfigSection::Monitors => {
            return map_get(&state.config.monitors, section.name(), rest);
        }
        RuntimeConfigSection::Display => {
            return Response::err("display.* is derived from outputs and not exposed at runtime");
        }
    };
    val.map(Response::ConfigValue).unwrap_or_else(|| {
        Response::err(format!(
            "unknown field '{rest}' on section '{}'",
            section.name()
        ))
    })
}

fn set(wm: &mut Wm, key: &str, value: String) -> Response {
    let Some((section_name, rest)) = key.split_once('.') else {
        return Response::err("key must be 'section.field' (e.g. layout.inner_gap)");
    };
    let Some(section) = RuntimeConfigSection::parse(section_name) else {
        return Response::err(format!("unknown section '{section_name}'"));
    };

    let state = &mut wm.core;
    let result = match section {
        RuntimeConfigSection::Window => set_field_from_raw(&state.config.window, rest, value)
            .and_then(crate::core_state::WindowConfig::validated)
            .map(|candidate| state.config.window = candidate),
        RuntimeConfigSection::Bar => set_field_from_raw(&state.config.bar, rest, value)
            .and_then(crate::config::config_toml::BarConfig::validated)
            .map(|candidate| state.config.bar = candidate),
        RuntimeConfigSection::Systray => parse_then_set(&mut state.config.systray, rest, value),
        RuntimeConfigSection::Layout => set_field_from_raw(&state.config.layout, rest, value)
            .and_then(crate::config::config_toml::LayoutConfig::validated)
            .map(|candidate| state.config.layout = candidate),
        RuntimeConfigSection::Animations => {
            parse_then_set(&mut state.config.animations, rest, value)
        }
        RuntimeConfigSection::Colors => parse_then_set(&mut state.config.colors, rest, value),
        RuntimeConfigSection::Cursor => parse_then_set(&mut state.config.cursor, rest, value),
        RuntimeConfigSection::Fonts => set_field_from_raw(&state.config.fonts, rest, value)
            .and_then(crate::core_state::FontConfig::validated)
            .map(|candidate| state.config.fonts = candidate),
        RuntimeConfigSection::Input => {
            let resp = map_set(&mut state.config.input, section.name(), rest, value);
            if matches!(resp, Response::Ok) {
                wm.work.queue_input_config_apply();
            }
            return resp;
        }
        RuntimeConfigSection::Monitors => {
            let resp = map_set(&mut state.config.monitors, section.name(), rest, value);
            if matches!(resp, Response::Ok) {
                wm.work.queue_monitor_config_apply();
            }
            return resp;
        }
        RuntimeConfigSection::Display => {
            return Response::err("display.* is derived from outputs and cannot be set at runtime");
        }
    };
    if let Err(e) = result {
        return Response::err(e);
    }
    apply_side_effects(wm, section);
    Response::ok()
}

fn list(wm: &Wm) -> Response {
    let state = &wm.core;
    let mut entries = Vec::new();
    for section in RuntimeConfigSection::EXPOSED {
        collect_section(state, section, &mut entries);
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Response::ConfigList(entries)
}

fn collect_section(
    core: &crate::core_state::CoreState,
    section: RuntimeConfigSection,
    entries: &mut Vec<(String, String)>,
) {
    let prefix = section.name();
    match section {
        RuntimeConfigSection::Window => collect(&core.config.window, prefix, entries),
        RuntimeConfigSection::Bar => collect(&core.config.bar, prefix, entries),
        RuntimeConfigSection::Systray => collect(&core.config.systray, prefix, entries),
        RuntimeConfigSection::Layout => collect(&core.config.layout, prefix, entries),
        RuntimeConfigSection::Animations => collect(&core.config.animations, prefix, entries),
        RuntimeConfigSection::Colors => collect(&core.config.colors, prefix, entries),
        RuntimeConfigSection::Cursor => collect(&core.config.cursor, prefix, entries),
        RuntimeConfigSection::Fonts => collect(&core.config.fonts, prefix, entries),
        RuntimeConfigSection::Input => {
            for (id, config) in &core.config.input {
                collect(config, &format!("{prefix}.{id}"), entries);
            }
        }
        RuntimeConfigSection::Monitors => {
            for (id, config) in &core.config.monitors {
                collect(config, &format!("{prefix}.{id}"), entries);
            }
        }
        RuntimeConfigSection::Display => unreachable!("hidden runtime-config section"),
    }
}

// ---------------------------------------------------------------------------
// Field-level get/set via serde round-tripping (reflection-by-name).
// ---------------------------------------------------------------------------

/// Render a config value as a string. Strings come back unquoted so shell
/// users see `my-cursor`, not `"my-cursor"`.
fn render_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

fn field_get<T: Serialize>(obj: &T, field: &str) -> Option<String> {
    let v = serde_json::to_value(obj).ok()?;
    Some(render_value(v.get(field)?))
}

/// Return a copy of `obj` with `field` set from a raw user string.
///
/// We try the value as JSON first (so `12`, `true`, `[1,2,3]` work), and
/// fall back to treating it as a plain string when either:
///   * the JSON parse fails (e.g. `my-cursor`), or
///   * the parsed JSON value can't be deserialised into the target field
///     (e.g. someone wrote `set monitors.DP-1.position 12` and the
///     `Value::Number` was rejected by `Option<String>`).
///
/// The fallback is necessary for `Option<String>` fields too — when the
/// current value is `None`, we can't tell from a serde snapshot that the
/// field expects a string, so we have to actually attempt the set and
/// retry on type error.
fn set_field_from_raw<T: Serialize + DeserializeOwned>(
    obj: &T,
    field: &str,
    raw: String,
) -> Result<T, String> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw)
        && let Ok(new) = field_set_owned(obj, field, value)
    {
        return Ok(new);
    }
    // JSON parsed but didn't fit the field — fall through and retry as
    // a plain string (e.g. a bare value for an `Option<String>` field).
    field_set_owned(obj, field, serde_json::Value::String(raw))
}

fn parse_then_set<T: Serialize + DeserializeOwned>(
    obj: &mut T,
    field: &str,
    raw: String,
) -> Result<(), String> {
    *obj = set_field_from_raw(&*obj, field, raw)?;
    Ok(())
}

fn field_set_owned<T: Serialize + DeserializeOwned>(
    obj: &T,
    field: &str,
    value: serde_json::Value,
) -> Result<T, String> {
    let mut v = serde_json::to_value(obj).map_err(|e| e.to_string())?;
    let map = v.as_object_mut().ok_or("expected object")?;
    if !map.contains_key(field) {
        return Err(format!("unknown field '{field}'"));
    }
    map.insert(field.to_string(), value);
    serde_json::from_value(v).map_err(|e| format!("type error: {e}"))
}

fn collect<T: Serialize>(obj: &T, prefix: &str, entries: &mut Vec<(String, String)>) {
    if let Ok(serde_json::Value::Object(map)) = serde_json::to_value(obj) {
        for (field, val) in map {
            entries.push((format!("{prefix}.{field}"), render_value(&val)));
        }
    }
}

// ---------------------------------------------------------------------------
// HashMap-shaped sections: key format `<section>.<id>.<field>`.
// ---------------------------------------------------------------------------

fn map_get<T: Serialize>(map: &HashMap<String, T>, section: &str, rest: &str) -> Response {
    let Some((id, field)) = rest.split_once('.') else {
        return Response::err(format!("{section} key must be '{section}.<name>.<field>'"));
    };
    let Some(cfg) = map.get(id) else {
        return Response::err(format!("unknown {section} entry '{id}'"));
    };
    field_get(cfg, field)
        .map(Response::ConfigValue)
        .unwrap_or_else(|| {
            Response::err(format!("unknown field '{field}' on {section} entry '{id}'"))
        })
}

fn map_set<T: Serialize + DeserializeOwned + Default>(
    map: &mut HashMap<String, T>,
    section: &str,
    rest: &str,
    raw: String,
) -> Response {
    let Some((id, field)) = rest.split_once('.') else {
        return Response::err(format!("{section} key must be '{section}.<name>.<field>'"));
    };
    let default;
    let existing = match map.get(id) {
        Some(cfg) => cfg,
        None => {
            default = T::default();
            &default
        }
    };
    match set_field_from_raw(existing, field, raw) {
        Ok(cfg) => {
            map.insert(id.to_string(), cfg);
            Response::ok()
        }
        Err(e) => Response::err(e),
    }
}

fn apply_side_effects(wm: &mut Wm, section: RuntimeConfigSection) {
    match section {
        RuntimeConfigSection::Bar => {
            sync_bar_config_to_monitors(wm);
            wm.reinit_bar_resources();
            let mut ctx = wm.ctx();
            ctx.request_bar_update();
            crate::layouts::manager::arrange(&mut ctx, None);
        }
        RuntimeConfigSection::Window | RuntimeConfigSection::Layout => {
            let mut ctx = wm.ctx();
            ctx.request_bar_update();
            crate::layouts::manager::arrange(&mut ctx, None);
        }
        RuntimeConfigSection::Colors | RuntimeConfigSection::Fonts => recolor(wm),
        RuntimeConfigSection::Animations => {}
        RuntimeConfigSection::Cursor => {
            wm.work.queue_cursor_config_apply();
            wm.bar.mark_dirty();
        }
        RuntimeConfigSection::Systray => {
            wm.bar.mark_dirty();
        }
        RuntimeConfigSection::Input
        | RuntimeConfigSection::Monitors
        | RuntimeConfigSection::Display => {}
    }
}

fn sync_bar_config_to_monitors(wm: &mut Wm) {
    let show_bar = wm.core.config.bar.show;
    let show_bottom_bar = wm.core.config.bar.show_bottom;
    for monitor in wm.core.model.monitors_iter_all_mut() {
        monitor.show_bar = show_bar;
        monitor.show_bottom_bar = show_bottom_bar;
        for state in monitor.per_tag.values_mut() {
            state.show_bar = show_bar;
        }
    }
}

/// Push colour/font changes to the screen after `wm.core.config.colors` (or the
/// tag colours) have been mutated.
///
/// Resource rebuilding is owned by [`Wm::reinit_bar_resources`]; on X11 the
/// rebuilt schemes only take effect once the bar redraws, so mark it dirty.
pub(crate) fn recolor(wm: &mut Wm) {
    wm.reinit_bar_resources();
    wm.bar.mark_dirty();
    let mut ctx = wm.ctx();
    ctx.request_bar_update();
    crate::layouts::manager::arrange(&mut ctx, None);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, wayland::WaylandBackend};
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
    fn do_list(wm: &mut Wm) -> Response {
        handle_config_command(wm, ConfigCommand::List)
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
        // display section is hidden — both fields are derived from outputs.
        assert!(matches!(
            do_set(&mut wm, "display.width", "1920"),
            Response::Err(_)
        ));
        assert!(matches!(
            do_set(&mut wm, "display.height", "1080"),
            Response::Err(_)
        ));
        assert!(matches!(do_get(&mut wm, "display.width"), Response::Err(_)));
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
    fn list_excludes_display_section() {
        let mut wm = test_wm();
        match do_list(&mut wm) {
            Response::ConfigList(entries) => {
                assert!(entries.iter().all(|(k, _)| !k.starts_with("display.")));
            }
            other => panic!("expected ConfigList, got {other:?}"),
        }
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
        let expected: std::collections::BTreeSet<String> = RuntimeConfigSection::EXPOSED
            .into_iter()
            .map(|section| section.name().to_string())
            .collect();
        assert_eq!(
            emitted, expected,
            "typed runtime-config registry drifted from list() output"
        );
    }

    #[test]
    fn section_status_classifies_each_kind() {
        assert_eq!(section_status("layout"), SectionStatus::Exposed);
        assert_eq!(section_status("input"), SectionStatus::Exposed);
        assert_eq!(section_status("display"), SectionStatus::Hidden);
        assert_eq!(section_status("nope"), SectionStatus::Unknown);
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
    fn bar_set_recomputes_monitor_bar_geometry() {
        let mut wm = test_wm();
        let mut monitor = Monitor::new_with_values(true);
        monitor.monitor_rect = Rect::new(0, 0, 800, 600);
        monitor.available_rect = monitor.monitor_rect;
        wm.core.model.monitors.push(monitor);

        assert!(matches!(do_set(&mut wm, "bar.height", "32"), Response::Ok));

        let monitor = wm
            .core
            .model
            .monitor(wm.core.model.monitors.first().unwrap())
            .unwrap();
        assert_eq!(monitor.bar_height, 32);
        assert_eq!(monitor.bar_y(), 0);
        assert_eq!(monitor.work_rect(), Rect::new(0, 32, 800, 568));
    }

    #[test]
    fn bar_show_and_top_apply_to_existing_monitor() {
        let mut wm = test_wm();
        let mut monitor = Monitor::new_with_values(true);
        monitor.monitor_rect = Rect::new(0, 0, 800, 600);
        monitor.available_rect = monitor.monitor_rect;
        wm.core.model.monitors.push(monitor);

        assert!(matches!(do_set(&mut wm, "bar.height", "32"), Response::Ok));
        assert!(matches!(do_set(&mut wm, "bar.show", "false"), Response::Ok));
        let monitor = wm
            .core
            .model
            .monitor(wm.core.model.monitors.first().unwrap())
            .unwrap();
        assert!(!monitor.show_bar);
        assert_eq!(monitor.work_rect(), Rect::new(0, 0, 800, 600));

        assert!(matches!(do_set(&mut wm, "bar.show", "true"), Response::Ok));
        let monitor = wm
            .core
            .model
            .monitor(wm.core.model.monitors.first().unwrap())
            .unwrap();
        assert!(monitor.show_bar);
        assert_eq!(monitor.bar_y(), 0);
        assert_eq!(monitor.work_rect(), Rect::new(0, 32, 800, 568));
    }
}
