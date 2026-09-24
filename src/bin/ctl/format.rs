use instantwm::ipc_types::{
    DisplayModes, KeybindInfo, KeyboardLayoutInfo, LayoutInfo, LayoutStatusInfo, ModeInfo,
    MonitorInfo, PendingTmpRuleInfo, Response, ScratchpadInfo, TagInfo, WindowInfo,
    WindowProtocol, WmStatusInfo,
};
use instantwm::types::KeybindOrigin;
use serde_json::Value;

pub fn format_response(response: &Response, json: bool) {
    match response {
        Response::Ok => {}
        Response::Err(msg) => {
            eprintln!("ERR {}", msg);
            std::process::exit(1);
        }
        Response::Message(msg) => print!("{}", msg),
        _ if json => print_json(&json_payload(response)),
        Response::WindowList(windows) => format_window_list(windows),
        Response::WindowInfo(window) => format_window_info(window),
        Response::MonitorList(monitors) => format_monitor_list(monitors),
        Response::MonitorModes(modes) => format_monitor_modes(modes),
        Response::ScratchpadList(scratchpads) => format_scratchpad_list(scratchpads),
        Response::ModeList(modes) => format_mode_list(modes),
        Response::LayoutList(layouts) => format_layout_list(layouts),
        Response::LayoutStatus(status) => format_layout_status(status),
        Response::Status(status) => format_status(status),
        Response::KeyboardLayoutList(layouts) => format_keyboard_layout_list(layouts),
        Response::TagList(tags) => format_tag_list(tags),
        Response::KeybindList(keybinds) => print!("{}", format_keybind_list_text(keybinds)),
        Response::ConfigValue(value) => println!("{value}"),
        Response::ConfigList(entries) => format_config_list(entries),
        Response::Theme(name) => println!("{name}"),
        Response::ThemeList(themes) => themes.iter().for_each(|theme| println!("{theme}")),
        Response::PendingTmpRuleList(rules) => format_pending_tmp_rule_list(rules),
        Response::PendingTmpRuleAdded { id, timeout_ms } => {
            println!("pending-tmp-rule added: id={id} timeout_ms={timeout_ms}");
        }
    }
}

pub fn print_json(value: &impl serde::Serialize) {
    match serde_json::to_string_pretty(value) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("instantwmctl: JSON serialization failed: {error}"),
    }
}

/// Config values arrive rendered (strings unquoted); recover their JSON form.
fn config_json(value: &str) -> Value {
    serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string()))
}

/// The JSON document for a response: its payload, without the variant tag.
fn json_payload(response: &Response) -> Value {
    match response {
        Response::Theme(name) => serde_json::json!({ "theme": name }),
        Response::ConfigValue(value) => serde_json::json!({ "value": config_json(value) }),
        Response::ConfigList(entries) => entries
            .iter()
            .map(|(key, value)| (key.clone(), config_json(value)))
            .collect::<serde_json::Map<_, _>>()
            .into(),
        other => match serde_json::to_value(other) {
            Ok(Value::Object(tagged)) if tagged.len() == 1 => {
                tagged.into_iter().next().map(|(_, payload)| payload).unwrap_or_default()
            }
            Ok(value) => value,
            Err(_) => Value::Null,
        },
    }
}

fn format_pending_tmp_rule_list(rules: &[PendingTmpRuleInfo]) {
    if rules.is_empty() {
        println!("No pending tmp rules");
        return;
    }
    fn render<T: std::fmt::Display>(opt: Option<T>) -> String {
        opt.map(|v| v.to_string()).unwrap_or_else(|| "-".into())
    }
    fn render_ms(ms: u64) -> String {
        if ms >= 60_000 {
            format!("{}m{}s", ms / 60_000, (ms % 60_000) / 1_000)
        } else if ms >= 1_000 {
            format!("{}.{}s", ms / 1_000, (ms % 1_000) / 100)
        } else {
            format!("{ms}ms")
        }
    }
    println!(
        "{:<5} {:<14} {:<14} {:<14} {:<7} {:<4} {:<10} {:<16} {:<6} {:<8}",
        "ID",
        "CLASS",
        "INSTANCE",
        "TITLE",
        "FLOAT",
        "TAG",
        "MONITOR",
        "GEOMETRY",
        "BORDER",
        "REMAINING"
    );
    for r in rules {
        println!(
            "{:<5} {:<14} {:<14} {:<14} {:<7} {:<4} {:<10} {:<16} {:<6} {:<8}",
            r.id,
            render(r.class.as_ref().map(|s| truncate_with_ellipsis(s, 14))),
            render(r.instance.as_ref().map(|s| truncate_with_ellipsis(s, 14))),
            render(r.title.as_ref().map(|s| truncate_with_ellipsis(s, 14))),
            match r.is_floating {
                Some(true) => "yes",
                Some(false) => "no",
                None => "-",
            },
            render(r.tag),
            render(r.on_monitor.clone()),
            render(r.geometry.clone()),
            if r.borderless { "none" } else { "-" },
            render_ms(r.ms_remaining)
        );
    }
}

fn format_window_list(windows: &[WindowInfo]) {
    if windows.is_empty() {
        println!("No windows");
        return;
    }
    println!(
        " {:<8} {:<50} {:<10} {:<8} {:<15} {:<20}",
        "ID", "TITLE", "PROTOCOL", "MONITOR", "TAGS", "STATE"
    );
    println!(
        " {:<8} {:<50} {:<10} {:<8} {:<15} {:<20}",
        "------",
        "--------------------------------------------------",
        "----------",
        "--------",
        "---------------",
        "--------------------"
    );
    for w in windows {
        let state = format_window_state(&w.state);
        let tags = if w.tags.is_empty() {
            String::from("-")
        } else {
            w.tags
                .iter()
                .map(|t| t.to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        let title = truncate_with_ellipsis(&w.title, 50);
        let marker = if w.is_focused { "*" } else { " " };
        println!(
            "{}{:<8} {:<50} {:<10} {:<8} {:<15} {:<20}",
            marker,
            w.id,
            title,
            format_window_protocol(w.protocol),
            w.monitor_position,
            tags,
            state
        );
    }
}

fn truncate_with_ellipsis(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }

    let ellipsis = "...";
    let prefix_chars = max_chars.saturating_sub(ellipsis.chars().count());
    let mut truncated: String = value.chars().take(prefix_chars).collect();
    truncated.push_str(ellipsis);
    truncated
}

fn format_window_protocol(protocol: WindowProtocol) -> &'static str {
    match protocol {
        WindowProtocol::Unknown => "unknown",
        WindowProtocol::X11 => "x11",
        WindowProtocol::Wayland => "wayland",
        WindowProtocol::XWayland => "xwayland",
    }
}

fn format_window_state(state: &instantwm::ipc_types::WindowState) -> String {
    let mut parts = Vec::new();
    if state.mode.is_true_fullscreen() {
        parts.push("Fullscreen");
    } else if state.mode.is_fake_fullscreen() {
        parts.push("FakeFullscreen");
    } else if state.mode.is_maximized() {
        parts.push("Maximized");
    } else if state.mode.is_normal_floating() {
        parts.push("Floating");
    } else {
        parts.push("Tiling");
    }
    if state.sticky {
        parts.push("sticky");
    }
    if state.hidden {
        parts.push("hidden");
    }
    if state.urgent {
        parts.push("urgent");
    }
    if state.locked {
        parts.push("locked");
    }
    if state.fixed_size {
        parts.push("fixed");
    }
    parts.join(", ")
}

fn format_window_info(window: &WindowInfo) {
    let tags = if window.tags.is_empty() {
        String::from("-")
    } else {
        window
            .tags
            .iter()
            .map(|tag| tag.to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    println!("id: {}", window.id);
    println!("title: {}", window.title);
    println!("protocol: {}", format_window_protocol(window.protocol));
    println!("monitor: {}", window.monitor_position);
    println!("focused: {}", window.is_focused);
    println!("tags: {}", tags);
    println!(
        "geometry: {}x{}+{}+{}",
        window.geometry.width, window.geometry.height, window.geometry.x, window.geometry.y
    );
    println!("border_width: {}", window.border_width);
    println!("state: {}", format_window_state(&window.state));
    if let Some(size_hints) = &window.size_hints {
        println!(
            "size_hints: min={}x{} max={}x{} base={}x{} inc={}x{}",
            size_hints.min_width.unwrap_or(0),
            size_hints.min_height.unwrap_or(0),
            size_hints.max_width.unwrap_or(0),
            size_hints.max_height.unwrap_or(0),
            size_hints.base_width.unwrap_or(0),
            size_hints.base_height.unwrap_or(0),
            size_hints.width_increment.unwrap_or(0),
            size_hints.height_increment.unwrap_or(0)
        );
    }
    if let Some(scratchpad) = &window.scratchpad {
        println!(
            "scratchpad: {} ({})",
            scratchpad.name,
            if scratchpad.visible {
                "visible"
            } else {
                "hidden"
            }
        );
    }
}

fn format_monitor_list(monitors: &[MonitorInfo]) {
    for m in monitors {
        let marker = if m.is_selected { "*" } else { " " };
        let vrr_mode = m
            .vrr_mode
            .map(|mode| format!("{mode:?}").to_lowercase())
            .unwrap_or_else(|| "-".to_string());
        let vrr_enabled = if m.vrr_enabled { "on" } else { "off" };
        let mirrors = if m.mirrors.is_empty() {
            String::new()
        } else {
            format!(" mirror[{}]", m.mirrors.join(","))
        };
        let pending: Vec<_> = m
            .requested_mirrors
            .iter()
            .filter(|name| !m.mirrors.contains(name))
            .cloned()
            .collect();
        let pending = if pending.is_empty() {
            String::new()
        } else {
            format!(" requested-mirror[{}]", pending.join(","))
        };
        println!(
            "{}{} {}: {}x{}+{}+{} vrr[support={:?} mode={} enabled={}]{}{}",
            marker,
            m.position,
            m.name,
            m.width,
            m.height,
            m.x,
            m.y,
            m.vrr_support,
            vrr_mode,
            vrr_enabled,
            mirrors,
            pending
        );
    }
}

fn format_scratchpad_list(scratchpads: &[ScratchpadInfo]) {
    if scratchpads.is_empty() {
        println!("No scratchpads");
        println!("Use 'instantwmctl scratchpad create <name>' to create one");
        return;
    }
    println!(
        "{:<12} {:<8} {:<8} {:<8} {:<20} {:<8}",
        "NAME", "STATUS", "ID", "MONITOR", "GEOMETRY", "FLAGS"
    );
    println!(
        "{:<12} {:<8} {:<8} {:<8} {:<20} {:<8}",
        "-----------", "--------", "--------", "--------", "--------------------", "--------"
    );
    for sp in scratchpads {
        let status = if sp.visible { "visible" } else { "hidden" };
        let id = sp
            .window_id
            .map(|w| w.to_string())
            .unwrap_or_else(|| "-".into());
        let monitor = sp
            .monitor
            .map(|m| m.to_string())
            .unwrap_or_else(|| "-".into());
        let geometry =
            if let (Some(w), Some(h), Some(x), Some(y)) = (sp.width, sp.height, sp.x, sp.y) {
                format!("{}x{}+{}+{}", w, h, x, y)
            } else {
                "-".to_string()
            };
        let mut flags = Vec::new();
        if sp.mode.is_fullscreen() {
            flags.push("fullscreen");
        } else if sp.mode.is_maximized() {
            flags.push("maximized");
        } else if sp.mode.is_normal_floating() {
            flags.push("floating");
        } else {
            flags.push("tiled");
        }
        println!(
            "{:<12} {:<8} {:<8} {:<8} {:<20} {}",
            sp.name,
            status,
            id,
            monitor,
            geometry,
            flags.join(", ")
        );
    }
}

fn format_mode_list(modes: &[ModeInfo]) {
    for m in modes {
        let marker = if m.is_active { "*" } else { " " };
        let desc = m.description.as_deref().unwrap_or("(no description)");
        println!("{} {} - {}", marker, m.name, desc);
    }
}

fn format_layout_list(layouts: &[LayoutInfo]) {
    for layout in layouts {
        let marker = if layout.is_active { "*" } else { " " };
        println!(
            "{} {} ({}) - {}",
            marker, layout.name, layout.symbol, layout.label
        );
    }
}

fn format_layout_status(status: &LayoutStatusInfo) {
    println!("layout: {} ({})", status.layout.name, status.layout.symbol);
    println!("presentation: {}", status.presentation);
    println!("monitor: {}", status.monitor_id);
}

fn format_status(status: &WmStatusInfo) {
    println!("instantWM {} ({})", status.version, status.backend);
    println!("Protocol: {}", status.protocol_version);
    println!("Commit: {}", status.build_commit);
    println!("Running: {}", status.running);
    println!("Monitors: {}", status.monitors);
    println!("Windows: {}", status.windows);
    println!("Tags: {}", status.tags);
}

fn format_keyboard_layout_list(layouts: &[KeyboardLayoutInfo]) {
    for l in layouts {
        let variant = l.variant.as_deref().unwrap_or("");
        let marker = if l.is_active { "*" } else { " " };
        if variant.is_empty() {
            println!("{}{}", marker, l.name);
        } else {
            println!("{} {} ({})", marker, l.name, variant);
        }
    }
}

fn format_tag_list(tags: &[TagInfo]) {
    for t in tags {
        let name = t.name.as_deref().unwrap_or("(unnamed)");
        println!("{}: {}", t.index, name);
    }
}

pub fn format_keybind_list_text(keybinds: &[KeybindInfo]) -> String {
    if keybinds.is_empty() {
        return "No keybindings configured\n".to_string();
    }

    let bind_width = keybinds
        .iter()
        .map(|k| binding_text(k).len())
        .max()
        .unwrap_or(0)
        .max("BINDING".len());
    let action_width = keybinds
        .iter()
        .map(|k| k.action.len())
        .max()
        .unwrap_or(0)
        .max("ACTION".len());
    let mode_width = keybinds
        .iter()
        .map(|k| k.mode.as_deref().unwrap_or("global").len())
        .max()
        .unwrap_or(0)
        .max("MODE".len());

    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<bw$} | {:<aw$} | {:<mw$} | ORIGIN",
        "BINDING",
        "ACTION",
        "MODE",
        bw = bind_width,
        aw = action_width,
        mw = mode_width
    );
    let _ = writeln!(
        out,
        "{:-<bw$}-|-{:-<aw$}-|-{:-<mw$}-|------",
        "",
        "",
        "",
        bw = bind_width,
        aw = action_width,
        mw = mode_width
    );

    for k in keybinds {
        let mode = k.mode.as_deref().unwrap_or("global");
        let origin = match k.origin {
            KeybindOrigin::CompiledDefault => "default",
            KeybindOrigin::User => "config",
        };
        let _ = writeln!(
            out,
            "{:<bw$} | {:<aw$} | {:<mw$} | {}",
            binding_text(k),
            k.action,
            mode,
            origin,
            bw = bind_width,
            aw = action_width,
            mw = mode_width
        );
    }
    out
}

/// Render a chord as the user would type it: empty modifiers → just the key,
/// otherwise `Modifiers + Key`.
///
/// Sibling implementation: `instantCLI::keyhelp::KeybindRow::binding` composes
/// the same string for the fzf UI. Keep them in sync — the wire contract
/// deliberately sends raw `modifiers`/`key`, so each side derives this once.
fn binding_text(k: &KeybindInfo) -> String {
    if k.modifiers.is_empty() {
        k.key.clone()
    } else {
        format!("{} + {}", k.modifiers, k.key)
    }
}

fn format_monitor_modes(displays: &[DisplayModes]) {
    for display in displays {
        println!("{}:", display.name);
        for mode in &display.modes {
            let rate = mode.refresh_mhz as f64 / 1000.0;
            println!("  {}x{} @ {:.3}Hz", mode.width, mode.height, rate);
        }
    }
}

fn format_config_list(entries: &[(String, String)]) {
    let width = entries.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    for (key, value) in entries {
        println!("{key:>width$} = {value}");
    }
}

#[cfg(test)]
mod tests {
    use super::truncate_with_ellipsis;

    #[test]
    fn title_truncation_is_utf8_safe() {
        let title = format!("{}ä{}", "a".repeat(46), "b".repeat(10));

        assert_eq!(
            truncate_with_ellipsis(&title, 50),
            format!("{}ä...", "a".repeat(46))
        );
    }

    #[test]
    fn short_multibyte_title_is_unchanged() {
        assert_eq!(truncate_with_ellipsis("Plüma – Datei", 50), "Plüma – Datei");
    }

    #[test]
    fn keybind_table_formatting_aligns_columns_correctly() {
        use super::format_keybind_list_text;
        use instantwm::ipc_types::KeybindInfo;
        use instantwm::types::KeybindOrigin;

        let keybinds = vec![
            KeybindInfo {
                modifiers: "Super".to_string(),
                key: "Return".to_string(),
                action: "spawn terminal".to_string(),
                mode: None,
                origin: KeybindOrigin::CompiledDefault,
            },
            KeybindInfo {
                modifiers: "".to_string(),
                key: "F1".to_string(),
                action: "quit".to_string(),
                mode: Some("desktop".to_string()),
                origin: KeybindOrigin::User,
            },
        ];

        let text = format_keybind_list_text(&keybinds);
        let lines: Vec<&str> = text.lines().collect();

        assert_eq!(lines.len(), 4);
        assert!(lines[0].starts_with("BINDING"));
        assert!(lines[1].contains("-|-"));
        assert!(lines[2].contains("Super + Return"));
        assert!(lines[2].contains("spawn terminal"));
        assert!(lines[2].contains("global"));
        assert!(lines[2].contains("default"));
        assert!(lines[3].contains("F1"));
        assert!(lines[3].contains("quit"));
        assert!(lines[3].contains("desktop"));
        assert!(lines[3].contains("config"));
    }

    #[test]
    fn keybind_table_formatting_empty_list() {
        use super::format_keybind_list_text;
        assert_eq!(format_keybind_list_text(&[]), "No keybindings configured\n");
    }
}
