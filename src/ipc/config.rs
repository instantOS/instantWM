//! Runtime config get/set/list over IPC.
//!
//! Each fixed section (`window`, `bar`, ...) round-trips through serde_json
//! to read/write fields by name. The two HashMap sections (`input`,
//! `monitors`) take a `<section>.<id>.<field>` key and auto-create missing
//! entries so users can add new device/monitor configs at runtime.

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
    let g = &wm.g;
    let val = match section {
        "window" => field_get(&g.cfg.window, rest),
        "bar" => field_get(&g.cfg.bar, rest),
        "systray" => field_get(&g.cfg.systray, rest),
        "display" => field_get(&g.cfg.display, rest),
        "layout" => field_get(&g.cfg.layout, rest),
        "colors" => field_get(&g.cfg.colors, rest),
        "cursor" => field_get(&g.cfg.cursor, rest),
        "fonts" => field_get(&g.cfg.fonts, rest),
        "input" => return map_get(&g.cfg.input, "input", rest),
        "monitors" => return map_get(&g.cfg.monitors, "monitors", rest),
        _ => return Response::err(format!("unknown section '{section}'")),
    };
    val.map(Response::ConfigValue).unwrap_or_else(|| {
        Response::err(format!("unknown field '{rest}' on section '{section}'"))
    })
}

fn set(wm: &mut Wm, key: &str, value: String) -> Response {
    let Some((section, rest)) = key.split_once('.') else {
        return Response::err("key must be 'section.field' (e.g. layout.inner_gap)");
    };

    // display.width/height are derived from real outputs — setting them
    // just desyncs config from reality until the next output change.
    if section == "display" && matches!(rest, "width" | "height") {
        return Response::err(format!(
            "display.{rest} is derived from outputs and cannot be set at runtime"
        ));
    }

    let value: serde_json::Value =
        serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value));

    let g = &mut wm.g;
    let result = match section {
        "window" => field_set(&mut g.cfg.window, rest, value),
        "bar" => field_set(&mut g.cfg.bar, rest, value),
        "systray" => field_set(&mut g.cfg.systray, rest, value),
        "display" => field_set(&mut g.cfg.display, rest, value),
        "layout" => field_set(&mut g.cfg.layout, rest, value),
        "colors" => field_set(&mut g.cfg.colors, rest, value),
        "cursor" => field_set(&mut g.cfg.cursor, rest, value),
        "fonts" => field_set(&mut g.cfg.fonts, rest, value),
        "input" => {
            let resp = map_set(&mut g.cfg.input, "input", rest, value);
            if matches!(resp, Response::Ok) {
                g.queue_input_config_apply();
            }
            return resp;
        }
        "monitors" => {
            let resp = map_set(&mut g.cfg.monitors, "monitors", rest, value);
            if matches!(resp, Response::Ok) {
                g.queue_monitor_config_apply();
            }
            return resp;
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
    let g = &wm.g;
    let mut entries = Vec::new();
    collect(&g.cfg.window, "window", &mut entries);
    collect(&g.cfg.bar, "bar", &mut entries);
    collect(&g.cfg.systray, "systray", &mut entries);
    collect(&g.cfg.display, "display", &mut entries);
    collect(&g.cfg.layout, "layout", &mut entries);
    collect(&g.cfg.colors, "colors", &mut entries);
    collect(&g.cfg.cursor, "cursor", &mut entries);
    collect(&g.cfg.fonts, "fonts", &mut entries);
    for (id, cfg) in &g.cfg.input {
        collect(cfg, &format!("input.{id}"), &mut entries);
    }
    for (id, cfg) in &g.cfg.monitors {
        collect(cfg, &format!("monitors.{id}"), &mut entries);
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Response::ConfigList(entries)
}

// ---------------------------------------------------------------------------
// Field-level get/set via serde round-tripping (reflection-by-name).
// ---------------------------------------------------------------------------

fn field_get<T: Serialize>(obj: &T, field: &str) -> Option<String> {
    let v = serde_json::to_value(obj).ok()?;
    Some(serde_json::to_string(v.get(field)?).unwrap_or_default())
}

fn field_set<T: Serialize + DeserializeOwned>(
    obj: &mut T,
    field: &str,
    value: serde_json::Value,
) -> Result<(), String> {
    let mut v = serde_json::to_value(&*obj).map_err(|e| e.to_string())?;
    let map = v.as_object_mut().ok_or("expected object")?;
    if !map.contains_key(field) {
        return Err(format!("unknown field '{field}'"));
    }
    map.insert(field.to_string(), value);
    *obj = serde_json::from_value(v).map_err(|e| format!("type error: {e}"))?;
    Ok(())
}

fn collect<T: Serialize>(obj: &T, prefix: &str, entries: &mut Vec<(String, String)>) {
    if let Ok(serde_json::Value::Object(map)) = serde_json::to_value(obj) {
        for (field, val) in map {
            entries.push((
                format!("{prefix}.{field}"),
                serde_json::to_string(&val).unwrap_or_default(),
            ));
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
    field_get(cfg, field).map(Response::ConfigValue).unwrap_or_else(|| {
        Response::err(format!("unknown field '{field}' on {section} entry '{id}'"))
    })
}

fn map_set<T: Serialize + DeserializeOwned + Default>(
    map: &mut HashMap<String, T>,
    section: &str,
    rest: &str,
    value: serde_json::Value,
) -> Response {
    let Some((id, field)) = rest.split_once('.') else {
        return Response::err(format!("{section} key must be '{section}.<name>.<field>'"));
    };
    // Auto-create missing entries so users can add new device/monitor configs.
    // Caveat: a typo in the identifier silently creates a dead entry.
    let cfg = map.entry(id.to_string()).or_default();
    match field_set(cfg, field, value) {
        Ok(()) => Response::ok(),
        Err(e) => Response::err(e),
    }
}

fn apply_side_effects(wm: &mut Wm, section: &str) {
    match section {
        "window" | "layout" | "display" => {
            wm.bar.mark_dirty();
            let mut ctx = wm.ctx();
            crate::layouts::manager::arrange(&mut ctx, None);
        }
        // TODO: colors/fonts should refresh window decorations beyond the
        // bar; cursor.size/theme needs CursorManager rebuild. For now we
        // just redraw the bar and pick the rest up on next reload.
        "bar" | "systray" | "colors" | "fonts" | "cursor" => {
            wm.bar.mark_dirty();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::{Backend, wayland::WaylandBackend};

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
        assert!(matches!(do_get(&mut wm, "window.nonexistent"), Response::Err(_)));
        assert!(matches!(do_get(&mut wm, "nonexistent.field"), Response::Err(_)));
        assert!(matches!(do_get(&mut wm, "nodot"), Response::Err(_)));
    }

    #[test]
    fn set_updates_and_roundtrips() {
        let mut wm = test_wm();
        assert!(matches!(do_set(&mut wm, "layout.inner_gap", "42"), Response::Ok));
        assert_eq!(wm.g.cfg.layout.inner_gap, 42);

        assert!(matches!(do_set(&mut wm, "window.resizehints", "false"), Response::Ok));
        assert!(!wm.g.cfg.window.resizehints);

        // Plain string fallback when value isn't valid JSON.
        assert!(matches!(do_set(&mut wm, "cursor.theme", "my-cursor"), Response::Ok));
        assert_eq!(wm.g.cfg.cursor.theme, "my-cursor");

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
        assert!(matches!(do_set(&mut wm, "window.nonexistent", "1"), Response::Err(_)));
        // display dimensions are read-only.
        assert!(matches!(do_set(&mut wm, "display.width", "1920"), Response::Err(_)));
        assert!(matches!(do_set(&mut wm, "display.height", "1080"), Response::Err(_)));
    }

    #[test]
    fn list_includes_fixed_and_map_sections() {
        let mut wm = test_wm();
        do_set(&mut wm, "input.type:touchpad.tap", r#""enabled""#);
        do_set(&mut wm, "monitors.DP-1.enable", "true");
        match do_list(&mut wm) {
            Response::ConfigList(entries) => {
                assert!(entries.iter().any(|(k, _)| k == "layout.inner_gap"));
                assert!(entries.iter().any(|(k, _)| k.starts_with("input.type:touchpad.")));
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
        assert!(wm.g.cfg.input.contains_key("type:touchpad"));
        assert!(wm.g.pending.input_config);

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
        assert!(matches!(do_set(&mut wm, "monitors.DP-1.scale", "2.0"), Response::Ok));
        assert!(wm.g.cfg.monitors.contains_key("DP-1"));
        assert!(wm.g.pending.monitor_config);
        assert!(matches!(
            do_get(&mut wm, "monitors.nonexistent.scale"),
            Response::Err(_)
        ));
    }
}
