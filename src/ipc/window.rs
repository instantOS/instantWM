use crate::ipc_types::{
    GeometryInfo, Response, SizeHintsInfo, WindowCommand, WindowGeometryInfo, WindowInfo,
    WindowState,
};
use crate::types::WindowId;
use crate::wm::Wm;

pub fn handle_window_command(wm: &mut Wm, cmd: WindowCommand) -> Response {
    match cmd {
        WindowCommand::List(window_id) => list_windows(wm, window_id.map(WindowId::from)),
        WindowCommand::Geom(window_id) => window_geometry(wm, window_id.map(WindowId::from)),
        WindowCommand::Close(window_id) => close_window(wm, window_id.map(WindowId::from)),
    }
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
fn build_scratchpad_info(
    c: &crate::types::client::Client,
) -> Option<crate::ipc_types::ScratchpadInfo> {
    if !c.is_scratchpad() {
        return None;
    }
    Some(crate::ipc_types::ScratchpadInfo {
        name: c.scratchpad_name.clone(),
        visible: c.issticky,
        window_id: Some(c.win.0),
        monitor: Some(c.monitor_id),
        x: Some(c.geo.x),
        y: Some(c.geo.y),
        width: Some(c.geo.w),
        height: Some(c.geo.h),
        floating: c.is_floating,
        fullscreen: c.is_fullscreen,
    })
}

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

fn client_to_window_info(c: &crate::types::client::Client, valid_tag_mask: u32) -> WindowInfo {
    WindowInfo {
        id: c.win.0 as u64,
        title: c.name.clone(),
        monitor: c.monitor_id,
        tags: tags_from_mask(c.tags.bits(), valid_tag_mask),
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

fn list_windows(wm: &Wm, parsed_id: Option<WindowId>) -> Response {
    let target = parsed_id;
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

    Response::WindowList(windows)
}

fn format_state(state: &WindowState) -> String {
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

fn close_window(wm: &mut Wm, parsed_id: Option<WindowId>) -> Response {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let Some(win) = target else {
        return Response::err("no target window");
    };
    crate::client::close_win(&mut wm.ctx(), win);
    Response::ok()
}

fn window_geometry(wm: &Wm, parsed_id: Option<WindowId>) -> Response {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let Some(win) = target else {
        return Response::err("no target window");
    };
    let Some(c) = wm.g.clients.get(&win) else {
        return Response::err("window not found");
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

    Response::WindowGeometry(info)
}
