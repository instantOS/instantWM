use crate::backend::BackendOps;
use crate::ipc_types::{
    GeometryInfo, Response, SizeHintsInfo, WindowCommand, WindowInfo, WindowState,
};
use crate::layouts::arrange;
use crate::monitor::transfer_client;
use crate::mouse::slop::is_valid_window_size_rect;
use crate::types::{Rect, TagMask, WindowId};
use crate::wm::Wm;

pub fn handle_window_command(wm: &mut Wm, cmd: WindowCommand) -> Response {
    match cmd {
        WindowCommand::List(window_id) => list_windows(wm, window_id.map(WindowId::from)),
        WindowCommand::Info(window_id) => window_info(wm, window_id.map(WindowId::from)),
        WindowCommand::Resize {
            window_id,
            monitor,
            x,
            y,
            width,
            height,
        } => resize_window(
            wm,
            window_id.map(WindowId::from),
            monitor,
            x,
            y,
            width,
            height,
        ),
        WindowCommand::Close(window_id) => close_window(wm, window_id.map(WindowId::from)),
    }
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
        visible: c.is_sticky,
        window_id: Some(c.win.0),
        monitor: Some(c.monitor_id.index()),
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
        sticky: c.is_sticky,
        hidden: c.is_hidden,
        urgent: c.is_urgent,
        locked: c.is_locked,
        fixed_size: c.is_fixed_size,
        never_focus: c.never_focus,
    }
}

fn client_to_window_info(
    c: &crate::types::client::Client,
    valid_tag_mask: TagMask,
    protocol: crate::backend::WindowProtocol,
) -> WindowInfo {
    WindowInfo {
        id: c.win.0 as u64,
        title: c.name.clone(),
        protocol,
        monitor: c.monitor_id.index(),
        tags: c.tags & valid_tag_mask,
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
    let backend = crate::backend::BackendRef::from_backend(&wm.backend);
    let windows: Vec<WindowInfo> = wins
        .iter()
        .map(|c| client_to_window_info(c, tag_mask, backend.window_protocol(c.win)))
        .collect();

    Response::WindowList(windows)
}

fn close_window(wm: &mut Wm, parsed_id: Option<WindowId>) -> Response {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let Some(win) = target else {
        return Response::err("no target window");
    };
    crate::client::close_win(&mut wm.ctx(), win);
    Response::ok()
}

fn window_info(wm: &Wm, parsed_id: Option<WindowId>) -> Response {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let Some(win) = target else {
        return Response::err("no target window");
    };
    let Some(c) = wm.g.clients.get(&win) else {
        return Response::err("window not found");
    };

    let tag_mask = wm.g.tags.mask();
    let backend = crate::backend::BackendRef::from_backend(&wm.backend);
    Response::WindowInfo(client_to_window_info(
        c,
        tag_mask,
        backend.window_protocol(c.win),
    ))
}

fn resize_window(
    wm: &mut Wm,
    parsed_id: Option<WindowId>,
    monitor_arg: Option<String>,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Response {
    let target = parsed_id.or_else(|| wm.g.selected_win());
    let Some(win) = target else {
        return Response::err("no target window");
    };

    let (current_monitor_id, is_floating) = match wm.g.clients.get(&win) {
        Some(c) => (c.monitor_id, c.is_floating),
        None => return Response::err("window not found"),
    };
    let target_monitor_id =
        match resolve_resize_monitor(wm, current_monitor_id, monitor_arg.as_deref()) {
            Ok(id) => id,
            Err(msg) => return Response::err(msg),
        };
    let Some(target_monitor_rect) = wm.g.monitor(target_monitor_id).map(|m| m.monitor_rect) else {
        return Response::err("monitor not found");
    };

    let rect = Rect {
        x: target_monitor_rect.x + x,
        y: target_monitor_rect.y + y,
        w: width,
        h: height,
    };

    let mut ctx = wm.ctx();

    if !is_valid_window_size_rect(&ctx, &rect, win) {
        return Response::err("invalid target geometry");
    }

    if !is_floating {
        crate::floating::set_window_mode(&mut ctx, win, crate::floating::WindowMode::Floating);
        arrange(&mut ctx, Some(current_monitor_id));
    }

    transfer_window_to_monitor(&mut ctx, win, current_monitor_id, target_monitor_id);
    ctx.move_resize(
        win,
        rect,
        crate::geometry::MoveResizeOptions::hinted_immediate(true),
    );
    Response::ok()
}

fn resolve_resize_monitor(
    wm: &Wm,
    current_monitor_id: crate::types::MonitorId,
    monitor_arg: Option<&str>,
) -> Result<crate::types::MonitorId, String> {
    match monitor_arg {
        None => Ok(current_monitor_id),
        Some("focused") => Ok(wm.g.selected_monitor_id()),
        Some(raw) => {
            let monitor_id = crate::types::MonitorId(
                raw.parse::<usize>()
                    .map_err(|_| format!("invalid monitor '{}'", raw))?,
            );
            if wm.g.monitor(monitor_id).is_some() {
                Ok(monitor_id)
            } else {
                Err(format!("monitor {} not found", monitor_id.index()))
            }
        }
    }
}

fn transfer_window_to_monitor(
    ctx: &mut crate::contexts::WmCtx<'_>,
    win: WindowId,
    current_monitor: crate::types::MonitorId,
    target_monitor: crate::types::MonitorId,
) {
    if current_monitor == target_monitor || ctx.is_wayland() {
        return;
    }

    ctx.core_mut()
        .globals_mut()
        .set_selected_monitor(current_monitor);
    transfer_client(ctx, win, target_monitor);
    ctx.core_mut()
        .globals_mut()
        .set_selected_monitor(target_monitor);
    crate::focus::focus_soft(ctx, Some(win));
}
