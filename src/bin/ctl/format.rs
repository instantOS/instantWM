use instantwm::ipc_types::{
    ActionInfo, DisplayModes, KeyboardLayoutInfo, ModeInfo, MonitorInfo, Response, ScratchpadInfo,
    TagInfo, WindowInfo, WindowProtocol, WmStatusInfo,
};

pub fn format_response(response: &Response, json: bool) {
    match response {
        Response::Ok => {}
        Response::Err(msg) => {
            eprintln!("ERR {}", msg);
            std::process::exit(1);
        }
        Response::WindowList(windows) => format_window_list(windows, json),
        Response::WindowInfo(window) => format_window_info(window, json),
        Response::MonitorList(monitors) => format_monitor_list(monitors, json),
        Response::MonitorModes(modes) => format_monitor_modes(modes, json),
        Response::ScratchpadList(scratchpads) => format_scratchpad_list(scratchpads, json),
        Response::ModeList(modes) => format_mode_list(modes, json),
        Response::Status(status) => format_status(status, json),
        Response::KeyboardLayoutList(layouts) => format_keyboard_layout_list(layouts, json),
        Response::TagList(tags) => format_tag_list(tags, json),
        Response::ActionList(actions) => format_action_list(actions, json),
        Response::ConfigValue(val) => format_config_value(val, json),
        Response::ConfigList(entries) => format_config_list(entries, json),
        Response::Message(msg) => print!("{}", msg),
        Response::Theme(name) => format_theme(name, json),
        Response::ThemeList(themes) => format_theme_list(themes, json),
        Response::PendingTmpRuleList(rules) => format_pending_tmp_rule_list(rules, json),
        Response::PendingTmpRuleAdded { id, timeout_ms } => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "id": id,
                        "timeout_ms": timeout_ms,
                    })
                );
            } else {
                println!("pending-tmp-rule added: id={id} timeout_ms={timeout_ms}");
            }
        }
    }
}

fn format_pending_tmp_rule_list(rules: &[instantwm::ipc_types::PendingTmpRuleInfo], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(rules).unwrap());
        return;
    }
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
        "{:<5} {:<14} {:<14} {:<14} {:<7} {:<4} {:<10} {:<8}",
        "ID", "CLASS", "INSTANCE", "TITLE", "FLOAT", "TAG", "MONITOR", "REMAINING"
    );
    for r in rules {
        println!(
            "{:<5} {:<14} {:<14} {:<14} {:<7} {:<4} {:<10} {:<8}",
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
            render(r.on_monitor),
            render_ms(r.ms_remaining)
        );
    }
}

fn format_theme(name: &str, json: bool) {
    if json {
        println!("{}", serde_json::json!({ "theme": name }));
    } else {
        println!("{}", name);
    }
}

fn format_theme_list(themes: &[String], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(themes).unwrap());
    } else {
        for theme in themes {
            println!("{}", theme);
        }
    }
}

fn format_window_list(windows: &[WindowInfo], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(windows).unwrap());
    } else {
        if windows.is_empty() {
            println!("No windows");
            return;
        }
        println!(
            "{:<8} {:<50} {:<10} {:<8} {:<15} {:<20}",
            "ID", "TITLE", "PROTOCOL", "MONITOR", "TAGS", "STATE"
        );
        println!(
            "{:<8} {:<50} {:<10} {:<8} {:<15} {:<20}",
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
            println!(
                "{:<8} {:<50} {:<10} {:<8} {:<15} {:<20}",
                w.id,
                title,
                format_window_protocol(w.protocol),
                w.monitor_position,
                tags,
                state
            );
        }
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

fn format_window_info(window: &WindowInfo, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(window).unwrap());
    } else {
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
}

fn format_monitor_list(monitors: &[MonitorInfo], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(monitors).unwrap());
    } else {
        for m in monitors {
            let marker = if m.is_selected { "*" } else { " " };
            let vrr_mode = m
                .vrr_mode
                .map(|mode| format!("{mode:?}").to_lowercase())
                .unwrap_or_else(|| "-".to_string());
            let vrr_enabled = if m.vrr_enabled { "on" } else { "off" };
            println!(
                "{}{} {}: {}x{}+{}+{} vrr[support={:?} mode={} enabled={}]",
                marker,
                m.position,
                m.name,
                m.width,
                m.height,
                m.x,
                m.y,
                m.vrr_support,
                vrr_mode,
                vrr_enabled
            );
        }
    }
}

fn format_scratchpad_list(scratchpads: &[ScratchpadInfo], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(scratchpads).unwrap());
    } else {
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
}

fn format_mode_list(modes: &[ModeInfo], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(modes).unwrap());
    } else {
        for m in modes {
            let marker = if m.is_active { "*" } else { " " };
            let desc = m.description.as_deref().unwrap_or("(no description)");
            println!("{} {} - {}", marker, m.name, desc);
        }
    }
}

fn format_status(status: &WmStatusInfo, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(status).unwrap());
    } else {
        println!("instantWM {} ({})", status.version, status.backend);
        println!("Protocol: {}", status.protocol_version);
        println!("Commit: {}", status.build_commit);
        println!("Running: {}", status.running);
        println!("Monitors: {}", status.monitors);
        println!("Windows: {}", status.windows);
        println!("Tags: {}", status.tags);
    }
}

fn format_keyboard_layout_list(layouts: &[KeyboardLayoutInfo], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(layouts).unwrap());
    } else {
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
}

fn format_tag_list(tags: &[TagInfo], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(tags).unwrap());
    } else {
        for t in tags {
            let name = t.name.as_deref().unwrap_or("(unnamed)");
            println!("{}: {}", t.index, name);
        }
    }
}

fn format_action_list(actions: &[ActionInfo], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(actions).unwrap());
    } else {
        let output = instantwm::config::keybind_config::format_action_list_text(actions);
        print!("{}", output);
    }
}

fn format_monitor_modes(displays: &[DisplayModes], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(displays).unwrap());
    } else {
        for display in displays {
            println!("{}:", display.name);
            for mode in &display.modes {
                let rate = mode.refresh_mhz as f64 / 1000.0;
                println!("  {}x{} @ {:.3}Hz", mode.width, mode.height, rate);
            }
        }
    }
}

fn format_config_value(val: &str, json: bool) {
    // val is a serde_json-serialized fragment, e.g. `"Adwaita"` or `42` or `true`.
    let parsed: serde_json::Value = serde_json::from_str(val).unwrap_or(serde_json::Value::Null);
    if json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({ "value": parsed })).unwrap()
        );
    } else {
        // For strings, Display prints without quotes; for numbers/bools, prints the value.
        println!("{}", display_value(&parsed));
    }
}

fn format_config_list(entries: &[(String, String)], json: bool) {
    if json {
        let map: std::collections::BTreeMap<&str, serde_json::Value> = entries
            .iter()
            .map(|(k, v)| {
                let parsed = serde_json::from_str(v).unwrap_or(serde_json::Value::Null);
                (k.as_str(), parsed)
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&map).unwrap());
    } else {
        let max_key_len = entries.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
        for (key, val) in entries {
            let parsed = serde_json::from_str(val).unwrap_or(serde_json::Value::Null);
            println!(
                "{:>width$} = {}",
                key,
                display_value(&parsed),
                width = max_key_len
            );
        }
    }
}

/// Display a serde_json::Value without quotes for strings.
fn display_value(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
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
}
