use instantwm::ipc_types::{
    ActionInfo, KeyboardLayoutInfo, ModeInfo, MonitorInfo, Response, ScratchpadInfo, TagInfo,
    WindowGeometryInfo, WindowInfo, WmStatusInfo,
};

pub fn format_response(response: &Response, json: bool) {
    match response {
        Response::Ok => {}
        Response::Err(msg) => {
            eprintln!("ERR {}", msg);
            std::process::exit(1);
        }
        Response::WindowList(windows) => format_window_list(windows, json),
        Response::WindowGeometry(geom) => format_window_geometry(geom, json),
        Response::MonitorList(monitors) => format_monitor_list(monitors, json),
        Response::ScratchpadList(scratchpads) => format_scratchpad_list(scratchpads, json),
        Response::ModeList(modes) => format_mode_list(modes, json),
        Response::Status(status) => format_status(status, json),
        Response::KeyboardLayoutList(layouts) => format_keyboard_layout_list(layouts, json),
        Response::TagList(tags) => format_tag_list(tags, json),
        Response::ActionList(actions) => format_action_list(actions, json),
        Response::Message(msg) => print!("{}", msg),
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
            "{:<8} {:<50} {:<8} {:<15} {:<20}",
            "ID", "TITLE", "MONITOR", "TAGS", "STATE"
        );
        println!(
            "{:<8} {:<50} {:<8} {:<15} {:<20}",
            "------",
            "--------------------------------------------------",
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
            let title = if w.title.len() > 50 {
                format!("{}...", &w.title[..47])
            } else {
                w.title.clone()
            };
            println!(
                "{:<8} {:<50} {:<8} {:<15} {:<20}",
                w.id, title, w.monitor, tags, state
            );
        }
    }
}

fn format_window_state(state: &instantwm::ipc_types::WindowState) -> String {
    let mut parts = Vec::new();
    if state.fullscreen {
        parts.push("Fullscreen");
    } else if state.floating {
        parts.push("Floating");
    } else {
        parts.push("Normal");
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

fn format_window_geometry(geom: &WindowGeometryInfo, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(geom).unwrap());
    } else {
        println!(
            "{}x{}+{}+{}",
            geom.geometry.width, geom.geometry.height, geom.geometry.x, geom.geometry.y
        );
    }
}

fn format_monitor_list(monitors: &[MonitorInfo], json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(monitors).unwrap());
    } else {
        for m in monitors {
            let marker = if m.is_primary { "*" } else { " " };
            println!(
                "{}{}: {}x{}+{}+{}",
                marker, m.index, m.width, m.height, m.x, m.y
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
            if sp.fullscreen {
                flags.push("fullscreen");
            }
            if sp.floating {
                flags.push("floating");
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
