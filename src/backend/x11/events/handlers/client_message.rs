use crate::backend::x11::events::setup::SYSTEM_TRAY_REQUEST_DOCK;
use crate::backend::x11::systray::XEmbedMessage;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::types::{Rect, TagMask, WindowId};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

/// Handle incoming X11 client messages.
pub fn client_message(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent) {
    let showsystray = ctx.core.config().systray.show;
    let systray_win = ctx.xembed_tray.as_ref().map(|s| s.win).unwrap_or_default();
    let net_system_tray_op = ctx.x11_runtime.netatom.system_tray_op;
    let net_wm_state = ctx.x11_runtime.netatom.wm_state;
    let net_active_window = ctx.x11_runtime.netatom.active_window;
    let net_current_desktop = ctx.x11_runtime.netatom.current_desktop;
    let net_wm_desktop = ctx.x11_runtime.netatom.wm_desktop;
    let event_win = WindowId::from(e.window);

    if showsystray && event_win == systray_win && e.type_ == net_system_tray_op {
        let data = e.data.as_data32();
        if data[1] == SYSTEM_TRAY_REQUEST_DOCK {
            handle_systray_dock_request(ctx, e);
        }
        return;
    };

    if e.type_ == net_current_desktop {
        handle_current_desktop(ctx, e);
        return;
    }

    if ctx.core.model().client(event_win).is_none() {
        return;
    };

    if e.type_ == net_wm_state {
        handle_net_wm_state(ctx, e, event_win);
    } else if e.type_ == net_active_window {
        handle_active_window(ctx, event_win);
    } else if e.type_ == net_wm_desktop {
        handle_wm_desktop(ctx, e, event_win);
    };
}

fn handle_systray_dock_request(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent) {
    let data = e.data.as_data32();
    let icon_win = WindowId::from(data[2]);
    if icon_win == WindowId::default() {
        return;
    };

    let systray_win_opt = ctx.xembed_tray.as_ref().map(|s| s.win);
    let statusescheme_bg_pixel = ctx.x11_runtime.status_scheme.bg.color.pixel as u32;

    let Some(systray_win) = systray_win_opt else {
        return;
    };

    let geo = {
        let conn = ctx.x11.conn;
        let x11_icon_win: Window = icon_win.into();
        conn.get_geometry(x11_icon_win)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|wa| Rect {
                x: 0,
                y: 0,
                w: wa.width as i32,
                h: wa.height as i32,
            })
            .unwrap_or(Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            })
    };

    let mapped =
        crate::backend::x11::systray::xembed_wants_mapped(&ctx.x11, ctx.x11_runtime, icon_win);
    let Some(systray) = ctx.xembed_tray.as_mut() else {
        return;
    };
    if systray.icon(icon_win).is_some() {
        return;
    }
    systray.icons.insert(
        0,
        crate::types::XEmbedIcon {
            win: icon_win,
            size: geo.size(),
            mapped,
        },
    );

    crate::backend::x11::systray::update_systray_icon_geom(
        ctx.core.derived().bar_height,
        ctx.xembed_tray.as_mut(),
        icon_win,
        geo.size(),
    );

    let conn = ctx.x11.conn;
    let x11_icon_win: Window = icon_win.into();
    let x11_systray_win: Window = systray_win.into();

    let _ = conn.change_save_set(SetMode::INSERT, x11_icon_win);
    let _ = conn.configure_window(x11_icon_win, &ConfigureWindowAux::new().border_width(0));

    let mask =
        EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE | EventMask::RESIZE_REDIRECT;
    let _ = conn.change_window_attributes(
        x11_icon_win,
        &ChangeWindowAttributesAux::new().event_mask(mask),
    );

    let _ = conn.reparent_window(x11_icon_win, x11_systray_win, 0, 0);

    let _ = conn.change_window_attributes(
        x11_icon_win,
        &ChangeWindowAttributesAux::new().background_pixel(statusescheme_bg_pixel),
    );

    let _ = conn.flush();

    crate::backend::x11::systray::update_systray(
        &mut ctx.core,
        &ctx.x11,
        ctx.x11_runtime,
        ctx.xembed_tray,
    );
    crate::backend::x11::systray::send_xembed_message(
        &ctx.x11,
        ctx.x11_runtime,
        icon_win,
        XEmbedMessage::EmbeddedNotify {
            embedder: systray_win,
        },
    );
    if mapped {
        crate::backend::x11::systray::send_xembed_message(
            &ctx.x11,
            ctx.x11_runtime,
            icon_win,
            XEmbedMessage::WindowActivate,
        );
        crate::backend::x11::set_client_state(&ctx.x11, ctx.x11_runtime, icon_win, 1);
    }
}

fn handle_net_wm_state(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent, win: WindowId) {
    let data = e.data.as_data32();
    let action = data[0];
    let requested = |current: bool| match action {
        0 => Some(false),
        1 => Some(true),
        2 => Some(!current),
        _ => None,
    };
    let atoms = [data[1], data[2]];
    let netatom = ctx.x11_runtime.netatom;
    if atoms.contains(&netatom.wm_maximized_vert) || atoms.contains(&netatom.wm_maximized_horz) {
        let current = ctx
            .core
            .state
            .model
            .client_protocol_maximized(win)
            .unwrap_or(false);
        if let Some(maximized) = requested(current) {
            crate::client::fullscreen::apply_client_maximize_intent(
                &mut WmCtx::X11(ctx.reborrow()),
                win,
                maximized,
            );
        }
    }

    if atoms.contains(&netatom.wm_fullscreen) {
        let mode = ctx.core.state.model.client(win).map(|client| client.mode());
        let current = mode.is_some_and(|mode| mode.is_fullscreen());
        if let Some(fullscreen) = requested(current) {
            crate::client::set_fullscreen(&mut WmCtx::X11(ctx.reborrow()), win, fullscreen);
        }
    }
}

fn handle_current_desktop(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent) {
    let desktop = e.data.as_data32()[0];
    let Some((monitor_id, tag_index)) =
        crate::backend::x11::properties::monitor_tag_for_desktop(ctx.core.model(), desktop)
    else {
        return;
    };
    let Some(mask) = TagMask::single(tag_index) else {
        return;
    };

    crate::overview::exit_overview(
        &mut WmCtx::X11(ctx.reborrow()),
        crate::overview::ExitMode::RestorePrevious,
    );
    crate::focus::select_monitor(&mut WmCtx::X11(ctx.reborrow()), monitor_id);
    crate::tags::view::view_tags(&mut WmCtx::X11(ctx.reborrow()), mask);
}

fn handle_wm_desktop(ctx: &mut WmCtxX11<'_>, e: &ClientMessageEvent, win: WindowId) {
    let desktop = e.data.as_data32()[0];

    if desktop == u32::MAX {
        if ctx
            .core
            .model()
            .client(win)
            .is_some_and(|client| client.is_scratchpad())
        {
            crate::backend::x11::set_client_tag_prop(
                ctx.core.state,
                &ctx.x11,
                ctx.x11_runtime,
                win,
            );
            return;
        }
        if let Some(client) = ctx.core.model_mut().client_mut(win) {
            client.is_sticky = true;
        }
        crate::backend::x11::set_client_tag_prop(ctx.core.state, &ctx.x11, ctx.x11_runtime, win);
        ctx.core.queue_layout_for_all_monitors_urgent();
        return;
    }

    let Some((target_mon, tag_index)) =
        crate::backend::x11::properties::monitor_tag_for_desktop(ctx.core.model(), desktop)
    else {
        return;
    };
    let Some(target_tags) = TagMask::single(tag_index) else {
        return;
    };

    if ctx
        .core
        .model()
        .client(win)
        .is_some_and(|client| client.is_scratchpad())
    {
        let _ = crate::floating::scratchpad::scratchpad_restore_window(
            &mut WmCtx::X11(ctx.reborrow()),
            win,
            Some((target_mon, target_tags)),
        );
        return;
    }

    let old_mon = ctx.core.model().client(win).map(|client| client.monitor_id);
    let previous_focus = ctx.core.model().selected_win();
    let reassigned = ctx.core.mutate_selection(|model| {
        if let Some(client) = model.client_mut(win) {
            client.is_sticky = false;
            client.set_tag_mask(target_tags);
        } else {
            return false;
        }
        model.reassign_client_monitor(win, target_mon)
    });
    debug_assert!(reassigned, "validated EWMH monitor transfer must succeed");
    if !reassigned {
        return;
    }

    crate::backend::x11::set_client_tag_prop(ctx.core.state, &ctx.x11, ctx.x11_runtime, win);
    crate::focus::refresh_focus_after_selection(
        &mut WmCtx::X11(ctx.reborrow()),
        previous_focus,
        None,
    );

    if old_mon == Some(target_mon) {
        ctx.core.queue_layout_for_monitor_urgent(target_mon);
    } else {
        ctx.core.queue_layout_for_all_monitors_urgent();
    }
}

fn handle_active_window(ctx: &mut WmCtxX11<'_>, win: WindowId) {
    let is_hidden = ctx
        .core
        .model()
        .client(win)
        .is_some_and(|client| client.is_hidden);
    if is_hidden {
        crate::client::show_window(&mut WmCtx::X11(ctx.reborrow()), win);
    };

    let _ = crate::focus::activate_client(&mut WmCtx::X11(ctx.reborrow()), win);
}
