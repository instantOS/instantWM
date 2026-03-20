use crate::ipc_types::{IpcResponse, WindowCommand};
use crate::types::WindowId;
use crate::wm::Wm;

pub fn handle_window_command(wm: &mut Wm, cmd: WindowCommand) -> IpcResponse {
    match cmd {
        WindowCommand::List(window_id) => list_windows(wm, window_id.map(WindowId::from)),
        WindowCommand::Geom(window_id) => window_geometry(wm, window_id.map(WindowId::from)),
        WindowCommand::Close(window_id) => close_window(wm, window_id.map(WindowId::from)),
    }
}

/// Information about a single window for JSON output.
#[derive(Debug, serde::Serialize)]
struct WindowInfo {
    id: u64,
    title: String,
    monitor: usize,
    tags: Vec<u32>,
    geometry: GeometryInfo,
    border_width: i32,
    state: WindowState,
    #[serde(skip_serializing_if = "Option::is_none")]
    scratchpad: Option<ScratchpadInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_hints: Option<SizeHintsInfo>,
}

#[derive(Debug, serde::Serialize)]
struct GeometryInfo {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Debug, serde::Serialize)]
struct WindowState {
    floating: bool,
    fullscreen: bool,
    #[serde(rename = "fake_fullscreen")]
    fake_fullscreen: bool,
    sticky: bool,
    hidden: bool,
    urgent: bool,
    locked: bool,
    fixed_size: bool,
    never_focus: bool,
}

#[derive(Debug, serde::Serialize)]
struct ScratchpadInfo {
    name: String,
    #[serde(rename = "restore_tags")]
    restore_tags: Vec<u32>,
}

#[derive(Debug, serde::Serialize)]
struct SizeHintsInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    min_width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    min_height: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_height: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_width: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_height: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    width_increment: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height_increment: Option<i32>,
}

/// Root structure for window list JSON output.
#[derive(Debug, serde::Serialize)]
struct WindowList {
    windows: Vec<WindowInfo>,
}

/// Convert a tags bitmask to an array of 1-indexed tag numbers.
fn tags_from_mask(tags_mask: u32, valid_mask: u32) -> Vec<u32> {
    (1..=32)
        .filter(|&t| {
            let tag_bit = 1u32 << (t - 1);
            (tags_mask & tag_bit) != 0 && (valid_mask & tag_bit) != 0
        })
        .collect()
}

/// Build scratchpad info from a client if it's a scratchpad window.
fn build_scratchpad_info(c: &crate::types::client::Client) -> Option<ScratchpadInfo> {
    if !c.is_scratchpad() {
        return None;
    }
    Some(ScratchpadInfo {
        name: c.scratchpad_name.clone(),
        restore_tags: tags_from_mask(c.scratchpad_restore_tags, u32::MAX),
    })
}

/// Build size hints info from a client, only including non-default values.
fn build_size_hints(c: &crate::types::client::Client) -> Option<SizeHintsInfo> {
    if c.size_hints_valid <= 0 {
        return None;
    }
    let h = &c.size_hints;
    Some(SizeHintsInfo {
        min_width: (h.minw > 0).then_some(h.minw),
        min_height: (h.minh > 0).then_some(h.minh),
        max_width: (h.maxw > 0).then_some(h.maxw),
        max_height: (h.maxh > 0).then_some(h.maxh),
        base_width: (h.basew > 0).then_some(h.basew),
        base_height: (h.baseh > 0).then_some(h.baseh),
        width_increment: (h.incw > 0).then_some(h.incw),
        height_increment: (h.inch > 0).then_some(h.inch),
    })
}

/// Build window state info from a client.
fn build_window_state(c: &crate::types::client::Client) -> WindowState {
    WindowState {
        floating: c.is_floating,
        fullscreen: c.is_fullscreen,
        fake_fullscreen: c.isfakefullscreen,
        sticky: c.issticky,
        hidden: c.is_hidden,
        urgent: c.is_urgent,
        locked: c.is_locked,
        fixed_size: c.is_fixed_size,
        never_focus: c.never_focus,
    }
}

/// Convert a single client to WindowInfo for JSON output.
fn client_to_window_info(c: &crate::types::client::Client, valid_tag_mask: u32) -> WindowInfo {
    WindowInfo {
        id: c.win.0 as u64,
        title: c.name.clone(),
        monitor: c.monitor_id,
        tags: tags_from_mask(c.tags, valid_tag_mask),
        geometry: GeometryInfo {
            x: c.geo.x,
            y: c.geo.y,
            width: c.geo.w,
            height: c.geo.h,
        },
        border_width: c.border_width,
        state: build_window_state(c),
        scratchpad: build_scratchpad_info(c),
        size_hints: build_size_hints(c),
    }
}

fn list_windows(wm: &Wm, parsed_id: Option<WindowId>) -> IpcResponse {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let mut wins: Vec<_> = if let Some(win) = target {
        wm.g.clients.get(&win).into_iter().collect()
    } else {
        wm.g.clients.values().collect()
    };
    wins.sort_by_key(|c| c.win.0);

    let tag_mask = wm.g.tags.mask();
    let windows: Vec<WindowInfo> = wins
        .iter()
        .map(|c| client_to_window_info(c, tag_mask))
        .collect();

    match serde_json::to_string_pretty(&WindowList { windows }) {
        Ok(json) => IpcResponse::ok(json),
        Err(e) => IpcResponse::err(format!("JSON serialization failed: {}", e)),
    }
}

fn close_window(wm: &mut Wm, parsed_id: Option<WindowId>) -> IpcResponse {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let Some(win) = target else {
        return IpcResponse::err("no target window");
    };
    crate::client::close_win(&mut wm.ctx(), win);
    IpcResponse::ok("")
}

/// Geometry information for a single window (JSON output).
#[derive(Debug, serde::Serialize)]
struct WindowGeometryInfo {
    id: u64,
    geometry: GeometryInfo,
}

fn window_geometry(wm: &Wm, parsed_id: Option<WindowId>) -> IpcResponse {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let Some(win) = target else {
        return IpcResponse::err("no target window");
    };
    let Some(c) = wm.g.clients.get(&win) else {
        return IpcResponse::err("window not found");
    };

    let info = WindowGeometryInfo {
        id: c.win.0 as u64,
        geometry: GeometryInfo {
            x: c.geo.x,
            y: c.geo.y,
            width: c.geo.w,
            height: c.geo.h,
        },
    };

    match serde_json::to_string_pretty(&info) {
        Ok(json) => IpcResponse::ok(json),
        Err(e) => IpcResponse::err(format!("JSON serialization failed: {}", e)),
    }
}
