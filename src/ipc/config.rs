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

pub fn handle_config_command(wm: &mut Wm, cmd: ConfigCommand) -> Response {
    match cmd {
        ConfigCommand::Get { key } => get(wm, &key),
        ConfigCommand::Set { key, value } => set(wm, &key, value),
        ConfigCommand::List => list(wm),
    }
}

fn get(wm: &Wm, key: &str) -> Response {
    let Some((section, rest)) = key.split_once('.') else {
        return Response::err("key must be 'section.field' (e.g. layout.inner_gap)");
    };
    let g = &wm.core;
    let val = match section {
        "window" => field_get(&g.config.window, rest),
        "bar" => field_get(&g.config.bar, rest),
        "systray" => field_get(&g.config.systray, rest),
        "layout" => field_get(&g.config.layout, rest),
        "colors" => field_get(&g.config.colors, rest),
        "cursor" => field_get(&g.config.cursor, rest),
        "fonts" => field_get(&g.config.fonts, rest),
        "input" => return map_get(&g.config.input, "input", rest),
        "monitors" => return map_get(&g.config.monitors, "monitors", rest),
        "display" => {
            return Response::err("display.* is derived from outputs and not exposed at runtime");
        }
        _ => return Response::err(format!("unknown section '{section}'")),
    };
    val.map(Response::ConfigValue)
        .unwrap_or_else(|| Response::err(format!("unknown field '{rest}' on section '{section}'")))
}

fn set(wm: &mut Wm, key: &str, value: String) -> Response {
    let Some((section, rest)) = key.split_once('.') else {
        return Response::err("key must be 'section.field' (e.g. layout.inner_gap)");
    };

    let g = &mut wm.core;
    let result = match section {
        "window" => parse_then_set(&mut g.config.window, rest, value),
        "bar" => parse_then_set(&mut g.config.bar, rest, value),
        "systray" => parse_then_set(&mut g.config.systray, rest, value),
        "layout" => parse_then_set(&mut g.config.layout, rest, value),
        "colors" => parse_then_set(&mut g.config.colors, rest, value),
        "cursor" => parse_then_set(&mut g.config.cursor, rest, value),
        "fonts" => parse_then_set(&mut g.config.fonts, rest, value),
        "input" => {
            let resp = map_set(&mut g.config.input, "input", rest, value);
            if matches!(resp, Response::Ok) {
                wm.work.queue_input_config_apply();
            }
            return resp;
        }
        "monitors" => {
            let resp = map_set(&mut g.config.monitors, "monitors", rest, value);
            if matches!(resp, Response::Ok) {
                wm.work.queue_monitor_config_apply();
            }
            return resp;
        }
        "display" => {
            return Response::err("display.* is derived from outputs and cannot be set at runtime");
        }
        _ => return Response::err(format!("unknown section '{section}'")),
    };
    if let Err(e) = result {
        return Response::err(e);
    }
    apply_side_effects(wm, section);
    Response::ok()
}

fn list(wm: &Wm) -> Response {
    let g = &wm.core;
    let mut entries = Vec::new();
    collect(&g.config.window, "window", &mut entries);
    collect(&g.config.bar, "bar", &mut entries);
    collect(&g.config.systray, "systray", &mut entries);
    collect(&g.config.layout, "layout", &mut entries);
    collect(&g.config.colors, "colors", &mut entries);
    collect(&g.config.cursor, "cursor", &mut entries);
    collect(&g.config.fonts, "fonts", &mut entries);
    for (id, cfg) in &g.config.input {
        collect(cfg, &format!("input.{id}"), &mut entries);
    }
    for (id, cfg) in &g.config.monitors {
        collect(cfg, &format!("monitors.{id}"), &mut entries);
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Response::ConfigList(entries)
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

fn apply_side_effects(wm: &mut Wm, section: &str) {
    match section {
        "bar" => {
            sync_bar_config_to_monitors(wm);
            if matches!(wm.backend, crate::backend::Backend::X11(_)) {
                crate::backend::x11::startup::init_drw_and_schemes(wm);
            }
            if let crate::backend::Backend::Wayland(data) = &mut wm.backend {
                crate::wayland::common::apply_bar_metrics(&mut wm.core, data);
            }
            let mut ctx = wm.ctx();
            ctx.request_bar_update();
            crate::layouts::manager::arrange(&mut ctx, None);
        }
        "window" | "layout" => {
            let mut ctx = wm.ctx();
            ctx.request_bar_update();
            crate::layouts::manager::arrange(&mut ctx, None);
        }
        "colors" | "fonts" => {
            // X11 schemes/fontset are baked into the Drw at startup; rebuild
            // them so the new values are visible without a full reload. On
            // Wayland the bar painter pulls colours/fonts on each redraw, so
            // marking the bar dirty is enough.
            if matches!(wm.backend, crate::backend::Backend::X11(_)) {
                crate::backend::x11::startup::init_drw_and_schemes(wm);
            }
            wm.bar.mark_dirty();
            let mut ctx = wm.ctx();
            ctx.request_bar_update();
            crate::layouts::manager::arrange(&mut ctx, None);
        }
        // TODO: cursor.size/theme needs the Wayland CursorManager to be
        // rebuilt before it takes effect. Until that lands, treat it as a
        // bar-only refresh and rely on the next reload for the real change.
        "systray" | "cursor" => {
            wm.bar.mark_dirty();
        }
        _ => {}
    }
}

fn sync_bar_config_to_monitors(wm: &mut Wm) {
    let show_bar = wm.core.config.bar.show;
    let top_bar = wm.core.config.bar.top;
    for monitor in wm.core.monitors_iter_all_mut() {
        monitor.show_bar = show_bar;
        monitor.top_bar = top_bar;
        for state in monitor.per_tag.values_mut() {
            state.showbar = show_bar;
        }
    }
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
            Response::ConfigValue(v) => assert_eq!(v, "1"),
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
            do_set(&mut wm, "window.resizehints", "false"),
            Response::Ok
        ));
        assert!(!wm.core.config.window.resizehints);

        // Plain string fallback when value isn't valid JSON.
        assert!(matches!(
            do_set(&mut wm, "cursor.theme", "my-cursor"),
            Response::Ok
        ));
        assert_eq!(wm.core.config.cursor.theme, "my-cursor");

        match do_get(&mut wm, "layout.inner_gap") {
            Response::ConfigValue(v) => assert_eq!(v, "42"),
            other => panic!("expected ConfigValue, got {other:?}"),
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
        let mut monitor = Monitor::new_with_values(true, true);
        monitor.monitor_rect = Rect::new(0, 0, 800, 600);
        monitor.available_rect = monitor.monitor_rect;
        monitor.work_rect = monitor.monitor_rect;
        wm.core.model.monitors.push(monitor);

        assert!(matches!(do_set(&mut wm, "bar.height", "32"), Response::Ok));

        let monitor = wm.core.monitor(crate::types::MonitorId(0)).unwrap();
        assert_eq!(monitor.bar_height, 32);
        assert_eq!(monitor.bar_y, 0);
        assert_eq!(monitor.work_rect, Rect::new(0, 32, 800, 568));
    }

    #[test]
    fn bar_show_and_top_apply_to_existing_monitor() {
        let mut wm = test_wm();
        let mut monitor = Monitor::new_with_values(true, true);
        monitor.monitor_rect = Rect::new(0, 0, 800, 600);
        monitor.available_rect = monitor.monitor_rect;
        monitor.work_rect = monitor.monitor_rect;
        wm.core.model.monitors.push(monitor);

        assert!(matches!(do_set(&mut wm, "bar.height", "32"), Response::Ok));
        assert!(matches!(do_set(&mut wm, "bar.show", "false"), Response::Ok));
        let monitor = wm.core.monitor(crate::types::MonitorId(0)).unwrap();
        assert!(!monitor.show_bar);
        assert_eq!(monitor.work_rect, Rect::new(0, 0, 800, 600));

        assert!(matches!(do_set(&mut wm, "bar.show", "true"), Response::Ok));
        assert!(matches!(do_set(&mut wm, "bar.top", "false"), Response::Ok));
        let monitor = wm.core.monitor(crate::types::MonitorId(0)).unwrap();
        assert!(monitor.show_bar);
        assert!(!monitor.top_bar);
        assert_eq!(monitor.bar_y, 568);
        assert_eq!(monitor.work_rect, Rect::new(0, 0, 800, 568));
    }
}
