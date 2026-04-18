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
            let title = if w.title.len() > 50 {
                format!("{}...", &w.title[..47])
            } else {
                w.title.clone()
            };
            println!(
                "{:<8} {:<50} {:<10} {:<8} {:<15} {:<20}",
                w.id,
                title,
                format_window_protocol(w.protocol),
                w.monitor,
                tags,
                state
            );
        }
    }
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
    match state.mode {
        instantwm::types::ClientMode::Tiling => parts.push("Tiling"),
        instantwm::types::ClientMode::Floating => parts.push("Floating"),
        instantwm::types::ClientMode::TrueFullscreen { .. } => parts.push("Fullscreen"),
        instantwm::types::ClientMode::FakeFullscreen { .. } => parts.push("FakeFullscreen"),
        instantwm::types::ClientMode::Maximized { .. } => parts.push("Maximized"),
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
        println!("monitor: {}", window.monitor);
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
            let marker = if m.is_primary { "*" } else { " " };
            let vrr_mode = m
                .vrr_mode
                .map(|mode| format!("{mode:?}").to_lowercase())
                .unwrap_or_else(|| "-".to_string());
            let vrr_enabled = if m.vrr_enabled { "on" } else { "off" };
            println!(
                "{}{} {}: {}x{}+{}+{} vrr[support={:?} mode={} enabled={}]",
                marker,
                m.index,
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
            match sp.mode {
                instantwm::types::ClientMode::TrueFullscreen { .. }
                | instantwm::types::ClientMode::FakeFullscreen { .. } => flags.push("fullscreen"),
                instantwm::types::ClientMode::Floating => flags.push("floating"),
                instantwm::types::ClientMode::Tiling => flags.push("tiled"),
                instantwm::types::ClientMode::Maximized { .. } => flags.push("maximized"),
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
