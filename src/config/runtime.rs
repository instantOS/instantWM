//! Runtime config edits on the live [`CoreState`].
//!
//! Each fixed section (`window`, `bar`, ...) round-trips through serde_json
//! to read/write fields by name. The two HashMap sections (`input`,
//! `monitors`) take a `<section>.<id>.<field>` key and auto-create missing
//! entries so users can add new device/monitor configs at runtime.
//!
//! Writes return a [`ConfigEffect`] describing what the caller must apply
//! afterwards. The effect vocabulary is deliberately free of backend and
//! `Wm` types so both callers — the IPC handler ([`crate::ipc::config`]) and
//! the `config_set`/`config_toggle` named actions — run the exact same
//! validated write.
//!
//! **Persistence:** edits made through this module live in the running WM
//! only — `reload` reloads from disk and discards them.

use crate::core_state::CoreState;
use crate::types::Monitor;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;

/// Follow-up work a config edit requires.
///
/// Each applier lives with its caller: the IPC handler applies effects with
/// a full [`crate::wm::Wm`], the named actions with a
/// [`crate::contexts::WmCtx`]. Values are backend-agnostic so both can.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigEffect {
    /// Nothing beyond the state write.
    None,
    /// Full bar config change: sync per-monitor bar state, rebuild bar
    /// resources, redraw, re-arrange.
    Bar,
    /// An explicit bar visibility write also clears per-view overrides.
    BarVisibility,
    /// Redraw the bar and re-arrange windows.
    Rearrange,
    /// Rebuild bar resources (colours/fonts), redraw, re-arrange.
    Recolor,
    /// Request a bar redraw.
    BarUpdate,
    /// Re-apply input device configuration.
    Input,
    /// Re-apply monitor configuration.
    Monitors,
    /// Re-apply cursor configuration and refresh the bar.
    Cursor,
}

/// Typed registry of the runtime-config sections; `config list` must emit
/// exactly these names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RuntimeConfigSection {
    Window,
    Bar,
    Systray,
    Tags,
    Layout,
    Animations,
    Colors,
    Cursor,
    Fonts,
    Input,
    Monitors,
    Focus,
}

impl RuntimeConfigSection {
    pub(crate) const ALL: [Self; 12] = [
        Self::Window,
        Self::Bar,
        Self::Systray,
        Self::Tags,
        Self::Layout,
        Self::Animations,
        Self::Colors,
        Self::Cursor,
        Self::Fonts,
        Self::Input,
        Self::Monitors,
        Self::Focus,
    ];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Window => "window",
            Self::Bar => "bar",
            Self::Systray => "systray",
            Self::Tags => "tags",
            Self::Layout => "layout",
            Self::Animations => "animations",
            Self::Colors => "colors",
            Self::Cursor => "cursor",
            Self::Fonts => "fonts",
            Self::Input => "input",
            Self::Monitors => "monitors",
            Self::Focus => "focus",
        }
    }

    fn parse(name: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|section| section.name() == name)
            .ok_or_else(|| {
                let known: Vec<_> = Self::ALL.into_iter().map(Self::name).collect();
                format!("unknown section '{name}' (known: {})", known.join(", "))
            })
    }

    /// The follow-up work an edit to this section requires.
    fn effect(self, field: &str) -> ConfigEffect {
        match self {
            Self::Bar if field == "show" => ConfigEffect::BarVisibility,
            Self::Bar if field == "tag_slots" => ConfigEffect::BarUpdate,
            Self::Bar => ConfigEffect::Bar,
            Self::Window | Self::Layout => ConfigEffect::Rearrange,
            Self::Colors | Self::Fonts => ConfigEffect::Recolor,
            // `request_bar_update` is the backend-agnostic "mark dirty".
            Self::Systray | Self::Tags => ConfigEffect::BarUpdate,
            Self::Cursor => ConfigEffect::Cursor,
            Self::Input => ConfigEffect::Input,
            Self::Monitors if field.ends_with(".tag_slots") => ConfigEffect::BarUpdate,
            Self::Monitors => ConfigEffect::Monitors,
            // Focus policy is read fresh on every key press, so a write only
            // has to land in the config.
            Self::Animations | Self::Focus => ConfigEffect::None,
        }
    }
}

fn split_key(key: &str) -> Result<(&str, &str), String> {
    key.split_once('.')
        .ok_or_else(|| "key must be 'section.field' (e.g. layout.inner_gap)".to_string())
}

fn unknown_field(section: &str, field: &str) -> String {
    format!("unknown field '{field}' on section '{section}'")
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Read a runtime config value by key.
pub fn get_runtime_field(core: &CoreState, key: &str) -> Result<String, String> {
    let (section_name, rest) = split_key(key)?;
    let section = RuntimeConfigSection::parse(section_name)?;
    let state = &core.config;
    match section {
        RuntimeConfigSection::Window => {
            field_get(&state.window, rest).ok_or_else(|| unknown_field(section.name(), rest))
        }
        RuntimeConfigSection::Bar => {
            field_get(&state.bar, rest).ok_or_else(|| unknown_field(section.name(), rest))
        }
        RuntimeConfigSection::Systray => {
            field_get(&state.systray, rest).ok_or_else(|| unknown_field(section.name(), rest))
        }
        RuntimeConfigSection::Tags => {
            field_get(&state.tags, rest).ok_or_else(|| unknown_field(section.name(), rest))
        }
        RuntimeConfigSection::Layout => {
            field_get(&state.layout, rest).ok_or_else(|| unknown_field(section.name(), rest))
        }
        RuntimeConfigSection::Animations => {
            field_get(&state.animations, rest).ok_or_else(|| unknown_field(section.name(), rest))
        }
        RuntimeConfigSection::Colors => {
            field_get(&state.colors, rest).ok_or_else(|| unknown_field(section.name(), rest))
        }
        RuntimeConfigSection::Cursor => {
            field_get(&state.cursor, rest).ok_or_else(|| unknown_field(section.name(), rest))
        }
        RuntimeConfigSection::Fonts => {
            field_get(&state.fonts, rest).ok_or_else(|| unknown_field(section.name(), rest))
        }
        RuntimeConfigSection::Focus => {
            field_get(&state.focus, rest).ok_or_else(|| unknown_field(section.name(), rest))
        }
        RuntimeConfigSection::Input => map_get(&state.input, section.name(), rest),
        RuntimeConfigSection::Monitors => map_get(&state.monitors, section.name(), rest),
    }
}

/// Write a runtime config value by key, returning the follow-up work the
/// caller must apply. Invalid values leave the config untouched.
pub fn set_runtime_field(
    core: &mut CoreState,
    key: &str,
    value: String,
) -> Result<ConfigEffect, String> {
    let (section_name, rest) = split_key(key)?;
    let section = RuntimeConfigSection::parse(section_name)?;
    let state = &mut core.config;
    match section {
        RuntimeConfigSection::Window => set_field_from_raw(&state.window, rest, value)
            .and_then(crate::core_state::WindowConfig::validated)
            .map(|candidate| state.window = candidate),
        RuntimeConfigSection::Bar => set_field_from_raw(&state.bar, rest, value)
            .and_then(crate::config::config_toml::BarConfig::validated)
            .map(|candidate| state.bar = candidate),
        RuntimeConfigSection::Systray => parse_then_set(&mut state.systray, rest, value),
        RuntimeConfigSection::Tags => {
            // Tag count and labels are load-time decisions: changing them
            // live would re-seed every monitor and resize the tag space.
            if rest != "show_icons" {
                return Err(format!(
                    "tags.{rest} defines the tag set and applies on reload; only tags.show_icons is runtime-settable"
                ));
            }
            parse_then_set(&mut state.tags, rest, value)
        }
        RuntimeConfigSection::Layout => set_field_from_raw(&state.layout, rest, value)
            .and_then(crate::config::config_toml::LayoutConfig::validated)
            .map(|candidate| state.layout = candidate),
        RuntimeConfigSection::Animations => parse_then_set(&mut state.animations, rest, value),
        RuntimeConfigSection::Colors => parse_then_set(&mut state.colors, rest, value),
        RuntimeConfigSection::Cursor => parse_then_set(&mut state.cursor, rest, value),
        RuntimeConfigSection::Fonts => set_field_from_raw(&state.fonts, rest, value)
            .and_then(crate::core_state::FontConfig::validated)
            .map(|candidate| state.fonts = candidate),
        RuntimeConfigSection::Focus => parse_then_set(&mut state.focus, rest, value),
        RuntimeConfigSection::Input => {
            map_set(&mut state.input, section.name(), rest, value)?;
            Ok(())
        }
        RuntimeConfigSection::Monitors => {
            set_monitor_field(&mut state.monitors, section.name(), rest, value)
        }
    }?;
    Ok(section.effect(rest))
}

/// Flip a boolean option by key, returning the follow-up work and the new
/// value.
///
/// Implemented as read-then-[`set_runtime_field`], so validation is exactly
/// that of an explicit set.
pub fn toggle_runtime_field(
    core: &mut CoreState,
    key: &str,
) -> Result<(ConfigEffect, String), String> {
    let current = get_runtime_field(core, key)?;
    let flipped = match current.as_str() {
        "true" => "false",
        "false" => "true",
        // Input toggles are `ToggleSetting` enums that render as these two
        // strings; flip them so every boolean-like option is covered.
        "enabled" => "disabled",
        "disabled" => "enabled",
        _ => {
            return Err(format!(
                "config toggle only works on boolean options; '{key}' is '{current}'"
            ));
        }
    };
    let effect = set_runtime_field(core, key, flipped.to_string())?;
    Ok((effect, flipped.to_string()))
}

/// List runtime config keys and their current values, optionally only those
/// at or beneath `prefix`.
pub fn list_runtime_fields(
    core: &CoreState,
    prefix: Option<&str>,
) -> Result<Vec<(String, String)>, String> {
    let mut entries = Vec::new();
    match prefix {
        None => {
            for section in RuntimeConfigSection::ALL {
                collect_section(&core.config, section, &mut entries);
            }
        }
        Some(prefix) => {
            let section_name = prefix
                .split_once('.')
                .map_or(prefix, |(section, _)| section);
            let section = RuntimeConfigSection::parse(section_name)?;
            collect_section(&core.config, section, &mut entries);
            let nested = format!("{prefix}.");
            entries.retain(|(key, _)| key == prefix || key.starts_with(&nested));
        }
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(entries)
}

/// Apply `bar` config values to every monitor. An explicit `bar.show` write
/// also drops per-view bar overrides; unrelated bar edits preserve them.
pub fn sync_bar_config_to_monitors(core: &mut CoreState, clear_overrides: bool) {
    let show_bottom_bar = core.config.bar.show_bottom;
    for monitor in core.model.monitors_iter_all_mut() {
        monitor.show_bottom_bar = show_bottom_bar;
        let policy = crate::bar::policy::TagBarPolicy::resolve(&core.config, &monitor.name);
        policy.apply_to(monitor);
        if clear_overrides {
            clear_bar_overrides(monitor);
        }
    }
}

/// Drop every per-view `toggle_bar` override on one monitor, so each tag
/// mask falls back to the configured visibility again.
pub fn clear_bar_overrides(monitor: &mut Monitor) {
    for state in monitor.per_tag.values_mut() {
        state.show_bar = None;
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

fn map_get<T: Serialize>(
    map: &HashMap<String, T>,
    section: &str,
    rest: &str,
) -> Result<String, String> {
    let Some((id, field)) = rest.split_once('.') else {
        return Err(format!("{section} key must be '{section}.<name>.<field>'"));
    };
    let Some(cfg) = map.get(id) else {
        return Err(format!("unknown {section} entry '{id}'"));
    };
    field_get(cfg, field)
        .ok_or_else(|| format!("unknown field '{field}' on {section} entry '{id}'"))
}

fn map_set<T: Serialize + DeserializeOwned + Default>(
    map: &mut HashMap<String, T>,
    section: &str,
    rest: &str,
    raw: String,
) -> Result<(), String> {
    let Some((id, field)) = rest.split_once('.') else {
        return Err(format!("{section} key must be '{section}.<name>.<field>'"));
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
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// `monitors` needs extra validation against a clone first: a fatal mirror
/// error keyed to this entry must reject the command without touching config.
fn set_monitor_field(
    map: &mut HashMap<String, crate::config::config_toml::MonitorConfig>,
    section: &str,
    rest: &str,
    raw: String,
) -> Result<(), String> {
    let mut prospective = map.clone();
    map_set(&mut prospective, section, rest, raw)?;
    if let Some((id, field)) = rest.split_once('.') {
        // Reject values the WM cannot present before committing.
        if let Some(entry) = prospective.get(id) {
            entry.validated(id)?;
        }
        if field == "mirror"
            && let Some(config) = prospective.get_mut(id)
            && config.mirror.as_deref() == Some("")
        {
            // `config set monitors.X.mirror ""` clears the mirror
            // instead of tripping EmptyTarget at apply time.
            config.mirror = None;
        }
        let (_, errors) = crate::output_mirror::MirrorMap::build(&prospective);
        if let Some(error) = errors
            .into_iter()
            .find(|error| error.is_fatal() && error.declaration_key() == Some(id))
        {
            return Err(format!("{error}"));
        }
    }
    *map = prospective;
    Ok(())
}

fn collect_section(
    config: &crate::core_state::EffectiveConfig,
    section: RuntimeConfigSection,
    entries: &mut Vec<(String, String)>,
) {
    let prefix = section.name();
    match section {
        RuntimeConfigSection::Window => collect(&config.window, prefix, entries),
        RuntimeConfigSection::Bar => collect(&config.bar, prefix, entries),
        RuntimeConfigSection::Systray => collect(&config.systray, prefix, entries),
        RuntimeConfigSection::Tags => collect(&config.tags, prefix, entries),
        RuntimeConfigSection::Layout => collect(&config.layout, prefix, entries),
        RuntimeConfigSection::Animations => collect(&config.animations, prefix, entries),
        RuntimeConfigSection::Colors => collect(&config.colors, prefix, entries),
        RuntimeConfigSection::Cursor => collect(&config.cursor, prefix, entries),
        RuntimeConfigSection::Fonts => collect(&config.fonts, prefix, entries),
        RuntimeConfigSection::Focus => collect(&config.focus, prefix, entries),
        RuntimeConfigSection::Input => {
            for (id, config) in &config.input {
                collect(config, &format!("{prefix}.{id}"), entries);
            }
        }
        RuntimeConfigSection::Monitors => {
            for (id, config) in &config.monitors {
                collect(config, &format!("{prefix}.{id}"), entries);
            }
        }
    }
}
