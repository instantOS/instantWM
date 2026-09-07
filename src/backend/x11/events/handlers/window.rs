use crate::backend::x11::events::query_manageable_window_geometry;
use crate::backend::x11::lifecycle::unmanage;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::types::WindowId;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

pub fn configure_notify(ctx: &mut WmCtxX11<'_>, e: &ConfigureNotifyEvent) {
    let event_win = WindowId::from(e.window);
    let root_win = WindowId::from(ctx.x11_runtime.root);
    if event_win != root_win {
        return;
    };

    ctx.core.derived_mut().display.width = e.width as i32;
    ctx.core.derived_mut().display.height = e.height as i32;

    crate::monitor::refresh_monitor_layout(&mut WmCtx::X11(ctx.reborrow()));
    crate::backend::x11::update_ewmh_desktop_props(ctx.core.state, &ctx.x11, ctx.x11_runtime);
    crate::focus::focus(&mut WmCtx::X11(ctx.reborrow()), None);
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
            ctx.core.derived().bar_height,
            ctx.xembed_tray.as_mut(),
            event_win,
            requested_size,
        );
        crate::backend::x11::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
    } else if ctx.core.model().client(event_win).is_some() {
        crate::backend::x11::focus::configure(ctx.core.state, &ctx.x11, event_win);
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
    if crate::backend::x11::systray::is_systray_icon(
        ctx.core.config().systray.show,
        ctx.xembed_tray.as_ref(),
        event_win,
    ) {
        // Remove tray-owned state before recomputing the paired tray/bar
        // geometry so the destroyed icon no longer reserves a cell.
        crate::backend::x11::systray::remove_systray_icon(ctx.xembed_tray.as_mut(), event_win);
        crate::backend::x11::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
    } else if ctx.core.model().client(event_win).is_some() {
        let mut tmp = ctx.reborrow();
        unmanage(&mut tmp, event_win, true);
    };
}

pub fn expose(ctx: &mut WmCtxX11<'_>, e: &ExposeEvent) {
    if e.count != 0 {
        return;
    };

    let event_win = WindowId::from(e.window);
    if let Some(monitor_id) = ctx
        .core
        .state
        .model
        .monitors
        .find_monitor_for(event_win, &ctx.core.model().clients)
    {
        let is_bar_win = ctx
            .core
            .state
            .model
            .monitors
            .get(monitor_id)
            .is_some_and(|m| event_win == m.bar_win);
        if is_bar_win {
            ctx.core.bar.mark_dirty();
        }
    };
}

pub fn focus_in(ctx: &mut WmCtxX11<'_>, _e: &FocusInEvent) {
    if let Some(selected_window) = ctx.core.model().selected_win() {
        crate::backend::x11::focus::set_focus(
            ctx.core.state,
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
    crate::backend::x11::keyboard::grab_keys(ctx.core.state, &ctx.x11, ctx.x11_runtime);
}

pub fn map_request(ctx: &mut WmCtxX11<'_>, e: &MapRequestEvent) {
    let event_win = WindowId::from(e.window);
    if crate::backend::x11::systray::is_systray_icon(
        ctx.core.config().systray.show,
        ctx.xembed_tray.as_ref(),
        event_win,
    ) {
        crate::backend::x11::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
        return;
    };

    if ctx.core.model().client(event_win).is_none() {
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
    if crate::backend::x11::systray::is_systray_icon(
        ctx.core.config().systray.show,
        ctx.xembed_tray.as_ref(),
        event_win,
    ) {
        if e.atom == ctx.x11_runtime.xatom.xembed_info {
            crate::backend::x11::systray::update_systray_icon_state(
                &ctx.x11,
                ctx.x11_runtime,
                ctx.xembed_tray.as_mut(),
                event_win,
                Some(e),
            );
        }
        crate::backend::x11::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
        return;
    };

    if ctx.core.model().client(event_win).is_some() {
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
                if let Some(c) = ctx.core.model_mut().client_mut(event_win) {
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
                let monitor_id = ctx
                    .core
                    .model()
                    .client(event_win)
                    .map(|client| client.monitor_id);
                let needs_float = ctx.core.model().client(event_win).is_some_and(|client| {
                    parent.is_some()
                        && client.placement() != crate::types::ClientPlacement::Floating
                });
                if let Some(client) = ctx.core.model_mut().client_mut(event_win) {
                    client.transient_for = parent;
                }
                if needs_float {
                    let _ = crate::floating::set_window_placement_from_policy(
                        &mut WmCtx::X11(ctx.reborrow()),
                        event_win,
                        crate::floating::WindowModeRequest::Floating(
                            crate::client::geometry::FloatingPlacementIntent::RestoreOrCenter,
                        ),
                    );
                }
                if let Some(monitor_id) = monitor_id {
                    crate::layouts::arrange(&mut WmCtx::X11(ctx.reborrow()), Some(monitor_id));
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
            let previous_focus = ctx.core.model().selected_win();
            if crate::client::update_window_properties(&mut ctx.core, event_win, &props) {
                crate::focus::refresh_focus_after_selection(
                    &mut WmCtx::X11(ctx.reborrow()),
                    previous_focus,
                    None,
                );
            }
        }
    };
}

pub fn resize_request(ctx: &mut WmCtxX11<'_>, e: &ResizeRequestEvent) {
    let event_win = WindowId::from(e.window);
    if crate::backend::x11::systray::is_systray_icon(
        ctx.core.config().systray.show,
        ctx.xembed_tray.as_ref(),
        event_win,
    ) {
        crate::backend::x11::systray::update_systray_icon_geom(
            ctx.core.derived().bar_height,
            ctx.xembed_tray.as_mut(),
            event_win,
            crate::types::Size::new(e.width as i32, e.height as i32),
        );
        crate::backend::x11::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
    };
}

pub fn unmap_notify(ctx: &mut WmCtxX11<'_>, e: &UnmapNotifyEvent) {
    let event_win = WindowId::from(e.window);
    if crate::backend::x11::systray::is_systray_icon(
        ctx.core.config().systray.show,
        ctx.xembed_tray.as_ref(),
        event_win,
    ) {
        // XEmbed icons remain owned by the tray while unmapped. Recompute the
        // paired tray/bar geometry; mapped state comes from _XEMBED_INFO.
        crate::backend::x11::systray::update_systray(
            &mut ctx.core,
            &ctx.x11,
            ctx.x11_runtime,
            ctx.xembed_tray,
        );
    } else if ctx.core.model().client(event_win).is_some() {
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
