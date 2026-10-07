use crate::contexts::WmCtx;
use crate::ipc_types::{Response, WindowCommand, WindowInfo};
use crate::layouts::ArrangeAnimation;
use crate::layouts::arrange;
use crate::monitor::{TransferFocus, transfer_client};
use crate::mouse::slop::is_valid_window_size;
use crate::types::{Client, MonitorId, MonitorSelector, Rect, WindowId};

pub fn handle_window_command(ctx: &mut WmCtx<'_>, cmd: WindowCommand) -> Response {
    match cmd {
        WindowCommand::List { window_id } => list_windows(ctx, window_id.map(WindowId::from)),
        WindowCommand::Info { window_id } => window_info(ctx, window_id.map(WindowId::from)),
        WindowCommand::Focus { window_id } => focus_window(ctx, window_id.map(WindowId::from)),
        WindowCommand::Resize {
            window_id,
            monitor,
            x,
            y,
            width,
            height,
        } => resize_window(
            ctx,
            window_id.map(WindowId::from),
            monitor,
            Rect::new(x, y, width, height),
        ),
        WindowCommand::Close { window_id } => close_window(ctx, window_id.map(WindowId::from)),
    }
}

fn list_windows(ctx: &WmCtx<'_>, parsed_id: Option<WindowId>) -> Response {
    let core_state = &ctx.core().state;
    let target = parsed_id;
    // Every client is owned by exactly one monitor, so carry the owning
    // monitor's ID alongside the client to resolve its spatial position. Both
    // come from the same single resolve.
    let mut wins: Vec<(MonitorId, &Client)> = if let Some(win) = target {
        core_state
            .model
            .client_view(win)
            .map(|view| (view.monitor.id(), view.client))
            .into_iter()
            .collect()
    } else {
        core_state.model.clients_iter_all().collect()
    };
    wins.sort_by_key(|(_, c)| c.win.0);

    let tag_mask = core_state.model.tags.mask();
    let selected = core_state.model.selected_win();
    let windows: Vec<WindowInfo> = wins
        .iter()
        .filter_map(|(monitor_id, c)| {
            let mon_pos = core_state.model.monitors.position_of(*monitor_id)?;
            Some(WindowInfo::from_client(
                c,
                tag_mask,
                ctx.window_protocol(c.win),
                mon_pos,
                selected == Some(c.win),
            ))
        })
        .collect();

    Response::WindowList(windows)
}

fn close_window(ctx: &mut WmCtx<'_>, parsed_id: Option<WindowId>) -> Response {
    let target = parsed_id.or_else(|| ctx.core().state.model.selected_win());
    let Some(win) = target else {
        return Response::err("no target window");
    };
    crate::client::close_win(ctx, win);
    Response::ok()
}

fn focus_window(ctx: &mut WmCtx<'_>, parsed_id: Option<WindowId>) -> Response {
    let core_state = &ctx.core().state;
    let target = parsed_id.or_else(|| core_state.model.selected_win());
    let Some(win) = target else {
        return Response::err("no target window");
    };

    // Mirror the X11 _NET_ACTIVE_WINDOW handler: an explicit activation
    // request also restores hidden (minimized) windows before focusing.
    if core_state
        .model
        .client(win)
        .is_some_and(|client| client.is_hidden)
    {
        crate::client::show_window(ctx, win);
    }

    if crate::focus::activate_client(ctx, win) {
        Response::ok()
    } else {
        Response::err("window not found")
    }
}

fn window_info(ctx: &WmCtx<'_>, parsed_id: Option<WindowId>) -> Response {
    let core_state = &ctx.core().state;
    let target = parsed_id.or_else(|| core_state.model.selected_win());
    let Some(win) = target else {
        return Response::err("no target window");
    };
    let Some(view) = core_state.model.client_view(win) else {
        return Response::err("window or assigned monitor not found");
    };

    let tag_mask = core_state.model.tags.mask();
    let Some(mon_pos) = core_state.model.monitors.position_of(view.monitor.id()) else {
        return Response::err("assigned monitor has no display position");
    };
    let c = view.client;
    Response::WindowInfo(WindowInfo::from_client(
        c,
        tag_mask,
        ctx.window_protocol(c.win),
        mon_pos,
        ctx.core().state.model.selected_win() == Some(win),
    ))
}

fn resize_window(
    ctx: &mut WmCtx<'_>,
    parsed_id: Option<WindowId>,
    monitor: Option<MonitorSelector>,
    requested_rect: Rect,
) -> Response {
    let core_state = &ctx.core().state;
    let target = parsed_id.or_else(|| core_state.model.selected_win());
    let Some(win) = target else {
        return Response::err("no target window");
    };

    let (current_monitor_id, is_floating) = match core_state.model.client_view(win) {
        Some(view) => (
            view.monitor.id(),
            view.client.placement() == crate::types::ClientPlacement::Floating,
        ),
        None => return Response::err("window not found"),
    };
    let target_monitor_id =
        match resolve_resize_monitor(&core_state.model, current_monitor_id, monitor.as_ref()) {
            Ok(id) => id,
            Err(msg) => return Response::err(msg),
        };
    let Some(target_monitor_rect) = core_state
        .model
        .monitor(target_monitor_id)
        .map(|m| m.monitor_rect)
    else {
        return Response::err("monitor not found");
    };

    let rect = Rect {
        x: target_monitor_rect.x + requested_rect.x,
        y: target_monitor_rect.y + requested_rect.y,
        w: requested_rect.w,
        h: requested_rect.h,
    };

    if !is_valid_window_size(&core_state.model, &rect, win) {
        return Response::err("invalid target geometry");
    }
    crate::client::fullscreen::leave_maximized(ctx, win);

    if !is_floating {
        let _ = crate::floating::set_window_mode(
            ctx,
            win,
            crate::floating::WindowModeRequest::Floating(
                crate::client::geometry::FloatingPlacementIntent::RestoreOrCenter,
            ),
        );
        arrange(ctx, Some(current_monitor_id), ArrangeAnimation::Configured);
    }

    // A geometry command must not steal keyboard focus or switch the
    // selected monitor; Preserve keeps focus where it is and only hands
    // the moved window's focus to a replacement if it was focused.
    let _ = transfer_client(ctx, win, target_monitor_id, TransferFocus::Preserve);
    ctx.move_resize(
        win,
        rect,
        crate::geometry::MoveResizeOptions::hinted_immediate(true),
    );
    Response::ok()
}

fn resolve_resize_monitor(
    model: &crate::model::WmModel,
    current_monitor_id: crate::types::MonitorId,
    monitor: Option<&MonitorSelector>,
) -> Result<crate::types::MonitorId, String> {
    match monitor {
        None => Ok(current_monitor_id),
        Some(MonitorSelector::Any) => Err(
            "window resize needs a concrete monitor: name, position, \"focused\" or \"primary\""
                .to_owned(),
        ),
        Some(selector) => crate::monitor::resolve_monitor_selector(model, selector)
            .ok_or_else(|| format!("monitor '{selector}' does not match any connected monitor")),
    }
}
