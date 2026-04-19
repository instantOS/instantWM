use crate::backend::BackendOps;
use crate::ipc_types::{Response, WindowCommand, WindowInfo};
use crate::layouts::arrange;
use crate::monitor::transfer_client;
use crate::mouse::slop::is_valid_window_size_rect;
use crate::types::{Rect, WindowId};
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
        .map(|c| WindowInfo::from_client(c, tag_mask, backend.window_protocol(c.win)))
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
    Response::WindowInfo(WindowInfo::from_client(
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
        Some(c) => (c.monitor_id, c.mode.is_floating()),
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

    if !is_valid_window_size_rect(&wm.g, &rect, win) {
        return Response::err("invalid target geometry");
    }

    let mut ctx = wm.ctx();

    if !is_floating {
        crate::floating::set_window_mode(&mut ctx, win, crate::types::BaseClientMode::Floating);
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
