use crate::backend::WindowOps;
use crate::ipc_types::{Response, WindowCommand, WindowInfo};
use crate::layouts::arrange;
use crate::monitor::{TransferFocus, transfer_client};
use crate::mouse::slop::is_valid_window_size;
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
            Rect::new(x, y, width, height),
        ),
        WindowCommand::Close(window_id) => close_window(wm, window_id.map(WindowId::from)),
    }
}

fn list_windows(wm: &Wm, parsed_id: Option<WindowId>) -> Response {
    let target = parsed_id;
    let mut wins: Vec<_> = if let Some(win) = target {
        wm.core.model.client(win).into_iter().collect()
    } else {
        wm.core.model.clients.values().collect()
    };
    wins.sort_by_key(|c| c.win.0);

    let tag_mask = wm.core.model.tags.mask();
    let windows: Vec<WindowInfo> = wins
        .iter()
        .filter_map(|c| {
            let mon_pos = wm.core.model.monitors.position_of(c.monitor_id)?;
            Some(WindowInfo::from_client(
                c,
                tag_mask,
                wm.backend.window_protocol(c.win),
                mon_pos,
            ))
        })
        .collect();

    Response::WindowList(windows)
}

fn close_window(wm: &mut Wm, parsed_id: Option<WindowId>) -> Response {
    let target = parsed_id.or_else(|| wm.core.selected_win());
    let Some(win) = target else {
        return Response::err("no target window");
    };
    crate::client::close_win(&mut wm.ctx(), win);
    Response::ok()
}

fn window_info(wm: &Wm, parsed_id: Option<WindowId>) -> Response {
    let target = parsed_id.or_else(|| wm.core.selected_win());
    let Some(win) = target else {
        return Response::err("no target window");
    };
    let Some(view) = wm.core.model.client_view(win) else {
        return Response::err("window or assigned monitor not found");
    };

    let tag_mask = wm.core.model.tags.mask();
    let Some(mon_pos) = wm.core.model.monitors.position_of(view.monitor.id()) else {
        return Response::err("assigned monitor has no display position");
    };
    let c = view.client;
    Response::WindowInfo(WindowInfo::from_client(
        c,
        tag_mask,
        wm.backend.window_protocol(c.win),
        mon_pos,
    ))
}

fn resize_window(
    wm: &mut Wm,
    parsed_id: Option<WindowId>,
    monitor_arg: Option<String>,
    requested_rect: Rect,
) -> Response {
    let target = parsed_id.or_else(|| wm.core.selected_win());
    let Some(win) = target else {
        return Response::err("no target window");
    };

    let (current_monitor_id, is_floating) = match wm.core.model.client(win) {
        Some(c) => (c.monitor_id, c.mode().is_floating()),
        None => return Response::err("window not found"),
    };
    let target_monitor_id =
        match resolve_resize_monitor(wm, current_monitor_id, monitor_arg.as_deref()) {
            Ok(id) => id,
            Err(msg) => return Response::err(msg),
        };
    let Some(target_monitor_rect) = wm.core.monitor(target_monitor_id).map(|m| m.monitor_rect)
    else {
        return Response::err("monitor not found");
    };

    let rect = Rect {
        x: target_monitor_rect.x + requested_rect.x,
        y: target_monitor_rect.y + requested_rect.y,
        w: requested_rect.w,
        h: requested_rect.h,
    };

    if !is_valid_window_size(&wm.core.model, &rect, win) {
        return Response::err("invalid target geometry");
    }

    let mut ctx = wm.ctx();

    if !is_floating {
        let _ = crate::floating::set_window_mode(
            &mut ctx,
            win,
            crate::floating::WindowModeRequest::Floating(
                crate::client::geometry::FloatingPlacementIntent::RestoreOrCenter,
            ),
        );
        arrange(&mut ctx, Some(current_monitor_id));
    }

    if !ctx.is_wayland() {
        let _ = transfer_client(
            &mut ctx,
            win,
            target_monitor_id,
            TransferFocus::FollowWindow,
        );
    }
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
        Some("focused") => Ok(wm.core.selected_monitor_id()),
        Some(raw) => {
            let pos = raw
                .parse::<usize>()
                .map_err(|_| format!("invalid monitor '{}'", raw))?;
            wm.core
                .model
                .monitors
                .id_at_position(pos)
                .ok_or_else(|| format!("monitor {pos} not found"))
        }
    }
}
