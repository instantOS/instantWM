use crate::backend::x11::events::query_manageable_window_geometry;
use crate::backend::x11::lifecycle::unmanage;
use crate::contexts::WmCtxX11;
use crate::layouts::ArrangeAnimation;
use crate::types::WindowId;
use x11rb::connection::Connection;
use x11rb::errors::ReplyError;
use x11rb::protocol::ErrorKind;
use x11rb::protocol::xproto::*;
use x11rb::x11_utils::X11Error;

#[cfg(test)]
mod tests;

/// Recover a managed client whose destruction notification was missed.
///
/// Errors are asynchronous: by the time we see BadWindow, its XID may belong
/// to a live replacement. Confirm that this specific window is still absent
/// before using the ordinary destroyed-client cleanup path. Connection failures
/// and unrelated protocol errors are not evidence of destruction.
pub fn handle_x11_error(ctx: &mut WmCtxX11<'_>, error: &X11Error) {
    let stale = confirmed_stale_client(&ctx.core.state.model, error, |window| {
        ctx.x11
            .conn
            .get_window_attributes(window)
            .map_err(ReplyError::from)?
            .reply()
            .map(|_| ())
    });
    if let Some(window) = stale {
        log::debug!("Recovering stale X11 client {window:?} after BadWindow");
        unmanage(ctx, window, true);
    }
}

fn confirmed_stale_client(
    model: &crate::model::WmModel,
    error: &X11Error,
    query_attributes: impl FnOnce(Window) -> Result<(), ReplyError>,
) -> Option<WindowId> {
    if error.error_kind != ErrorKind::Window {
        return None;
    }
    let window = WindowId::from(error.bad_value);
    model.client(window)?;
    match query_attributes(error.bad_value) {
        Err(ReplyError::X11Error(reply))
            if reply.error_kind == ErrorKind::Window && reply.bad_value == error.bad_value =>
        {
            Some(window)
        }
        _ => None,
    }
}

pub fn configure_notify(ctx: &mut WmCtxX11<'_>, e: &ConfigureNotifyEvent) {
    let event_win = WindowId::from(e.window);
    let root_win = WindowId::from(ctx.x11_runtime.root);
    if event_win != root_win {
        return;
    };

    ctx.core.state.derived.display.width = e.width as i32;
    ctx.core.state.derived.display.height = e.height as i32;

    crate::monitor::refresh_monitor_layout(&mut ctx.wm_ctx());
    crate::backend::x11::update_ewmh_desktop_props(&ctx.core.state, &ctx.x11, ctx.x11_runtime);
    crate::focus::focus(&mut ctx.wm_ctx(), None);
    ctx.core.queue_layout_for_all_monitors_urgent();
}

pub fn configure_request(ctx: &mut WmCtxX11<'_>, e: &ConfigureRequestEvent) {
    let event_win = WindowId::from(e.window);
    if let Some(current_size) = ctx
        .xembed_tray
        .as_ref()
        .and_then(|tray| tray.icon(event_win))
        .map(|icon| icon.size)
    {
        let requested_size = crate::types::Size::new(
            if e.value_mask.contains(ConfigWindow::WIDTH) {
                e.width as i32
            } else {
                current_size.w
            },
            if e.value_mask.contains(ConfigWindow::HEIGHT) {
                e.height as i32
            } else {
                current_size.h
            },
        );
        crate::backend::x11::systray::update_systray_icon_geom(
            ctx.core.state.config.bar_metrics().height,
            ctx.xembed_tray.as_mut(),
            event_win,
            requested_size,
        );
        crate::backend::x11::bar::sync_top_bar_surfaces(
            ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
    } else if let Some(client) = ctx.core.state.model.client(event_win) {
        let (geo, border_width) = (client.geo, client.border_width);
        crate::backend::x11::focus::configure(&ctx.x11, event_win, geo, border_width);
    } else {
        let conn = ctx.x11.conn;
        let _ = conn.configure_window(
            e.window,
            &ConfigureWindowAux::new()
                .x(e.x as i32)
                .y(e.y as i32)
                .width(e.width as u32)
                .height(e.height as u32)
                .border_width(e.border_width as u32),
        );
        let _ = conn.flush();
    };
}

pub fn destroy_notify(ctx: &mut WmCtxX11<'_>, e: &DestroyNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if crate::backend::x11::systray::is_systray_icon(ctx.xembed_tray.as_ref(), event_win) {
        // Remove tray-owned state before recomputing the paired tray/bar
        // geometry so the destroyed icon no longer reserves a cell.
        crate::backend::x11::systray::remove_systray_icon(ctx.xembed_tray.as_mut(), event_win);
        crate::backend::x11::bar::sync_top_bar_surfaces(
            ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
    } else if ctx.core.state.model.client(event_win).is_some() {
        let mut tmp = ctx.reborrow();
        unmanage(&mut tmp, event_win, true);
    };
}

pub fn expose(ctx: &mut WmCtxX11<'_>, e: &ExposeEvent) {
    if e.count != 0 {
        return;
    };

    let event_win = WindowId::from(e.window);
    if let Some(monitor) = ctx.core.state.model.monitors.find_monitor_for(event_win)
        && event_win == monitor.bar_win
    {
        ctx.core.bar.mark_dirty();
    }
}

pub fn focus_in(ctx: &mut WmCtxX11<'_>, _e: &FocusInEvent) {
    if let Some(selected_window) = ctx.core.state.model.selected_win() {
        crate::backend::x11::focus::set_focus(
            &ctx.core.state,
            &ctx.x11,
            ctx.x11_runtime,
            selected_window,
        );
    };
}

pub fn mapping_notify(ctx: &mut WmCtxX11<'_>, _e: &MappingNotifyEvent) {
    if !crate::backend::x11::keyboard::refresh_keyboard_mapping(&ctx.x11, ctx.x11_runtime) {
        log::warn!("X11 keyboard mapping refresh failed; preserving existing passive grabs");
        return;
    }
    crate::backend::x11::keyboard::grab_keys(&ctx.core.state, &ctx.x11, ctx.x11_runtime);
}

pub fn map_request(ctx: &mut WmCtxX11<'_>, e: &MapRequestEvent) {
    let event_win = WindowId::from(e.window);
    if crate::backend::x11::systray::is_systray_icon(ctx.xembed_tray.as_ref(), event_win) {
        crate::backend::x11::bar::sync_top_bar_surfaces(
            ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
        return;
    };

    if ctx.core.state.model.client(event_win).is_none() {
        let Some(initial_geometry) = query_manageable_window_geometry(&ctx.x11, event_win) else {
            return;
        };
        let mut tmp = ctx.reborrow();
        crate::backend::x11::lifecycle::manage(
            &mut tmp,
            event_win,
            initial_geometry.rect,
            initial_geometry.border_width,
        );
    };
}

pub fn property_notify(ctx: &mut WmCtxX11<'_>, e: &PropertyNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if crate::backend::x11::systray::is_systray_icon(ctx.xembed_tray.as_ref(), event_win) {
        if e.atom == ctx.x11_runtime.xatom.xembed_info {
            crate::backend::x11::systray::update_systray_icon_state(
                &ctx.x11,
                ctx.x11_runtime,
                ctx.xembed_tray.as_mut(),
                event_win,
                Some(e),
            );
        }
        crate::backend::x11::bar::sync_top_bar_surfaces(
            ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
        return;
    };

    if ctx.core.state.model.client(event_win).is_some() {
        match e.atom {
            x if x == ctx.x11_runtime.wmatom.protocols => {
                let protocols = crate::backend::x11::focus::read_wm_protocols(
                    ctx.x11.conn,
                    e.window,
                    ctx.x11_runtime.wmatom.protocols,
                )
                .map(crate::backend::x11::X11ClientProtocols::Known)
                .unwrap_or_default();
                ctx.x11_runtime
                    .client_protocols
                    .insert(event_win, protocols);
            }
            x if x == u32::from(AtomEnum::WM_NORMAL_HINTS) => {
                if let Some(c) = ctx.core.state.model.client_mut(event_win) {
                    c.size_hints_valid = false;
                }
            }
            x if x == u32::from(AtomEnum::WM_HINTS) => {
                crate::backend::x11::update_wm_hints(ctx, event_win);
                ctx.core.bar.mark_dirty();
            }
            x if x == u32::from(AtomEnum::WM_TRANSIENT_FOR) => {
                let parent =
                    crate::backend::x11::lifecycle::get_transient_for_hint(&ctx.x11, event_win);
                if let Some(monitor_id) =
                    crate::client::update_transient_for(&mut ctx.wm_ctx(), event_win, parent)
                {
                    crate::layouts::arrange(
                        &mut ctx.wm_ctx(),
                        Some(monitor_id),
                        ArrangeAnimation::Configured,
                    );
                }
            }
            _ => {}
        }

        let net_wm_name = ctx.x11_runtime.netatom.wm_name;
        if e.atom == u32::from(AtomEnum::WM_NAME)
            || e.atom == net_wm_name
            || e.atom == u32::from(AtomEnum::WM_CLASS)
        {
            let props =
                crate::backend::x11::window_properties(&ctx.x11, ctx.x11_runtime, event_win);
            let previous_focus = ctx.core.state.model.selected_win();
            if crate::client::update_window_properties(ctx.core, event_win, &props) {
                crate::focus::refresh_focus_after_selection(
                    &mut ctx.wm_ctx(),
                    previous_focus,
                    None,
                );
            }
        }
    };
}

pub fn resize_request(ctx: &mut WmCtxX11<'_>, e: &ResizeRequestEvent) {
    let event_win = WindowId::from(e.window);
    if crate::backend::x11::systray::is_systray_icon(ctx.xembed_tray.as_ref(), event_win) {
        crate::backend::x11::systray::update_systray_icon_geom(
            ctx.core.state.config.bar_metrics().height,
            ctx.xembed_tray.as_mut(),
            event_win,
            crate::types::Size::new(e.width as i32, e.height as i32),
        );
        crate::backend::x11::bar::sync_top_bar_surfaces(
            ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
    };
}

pub fn unmap_notify(ctx: &mut WmCtxX11<'_>, e: &UnmapNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if crate::backend::x11::systray::is_systray_icon(ctx.xembed_tray.as_ref(), event_win) {
        // XEmbed icons remain owned by the tray while unmapped. Recompute the
        // paired tray/bar geometry; mapped state comes from _XEMBED_INFO.
        crate::backend::x11::bar::sync_top_bar_surfaces(
            ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
    } else if ctx.core.state.model.client(event_win).is_some() {
        if e.response_type & 0x80 != 0 {
            crate::backend::x11::set_client_state(
                &ctx.x11,
                ctx.x11_runtime,
                event_win,
                crate::backend::x11::constants::WM_STATE_WITHDRAWN,
            );
        } else {
            let mut tmp = ctx.reborrow();
            unmanage(&mut tmp, event_win, false);
        }
    };
}
