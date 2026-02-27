use crate::client::{apply_size_hints, set_client_state};
use crate::contexts::WmCtx;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;
use x11rb::CURRENT_TIME;

const XEMBED_MAPPED: u32 = 1 << 0;
const XEMBED_WINDOW_ACTIVATE: u32 = 1;
const XEMBED_WINDOW_DEACTIVATE: u32 = 2;
const XEMBED_EMBEDDED_VERSION: u32 = 0;

pub fn get_systray_width(ctx: &WmCtx) -> u32 {
    if !ctx.g.cfg.showsystray {
        return 1;
    }

    let mut w: u32 = 0;
    if let Some(ref systray) = ctx.g.systray {
        for &icon_win in &systray.icons {
            if let Some(c) = ctx.g.clients.get(&icon_win) {
                w += c.geo.w as u32 + ctx.g.cfg.systrayspacing as u32;
            }
        }
    }

    if w > 0 {
        w + ctx.g.cfg.systrayspacing as u32
    } else {
        1
    }
}

/// Remove systray icon using dependency injection.
pub fn remove_systray_icon(ctx: &mut WmCtx, icon_win: WindowId) {
    if !ctx.g.cfg.showsystray {
        return;
    }

    if let Some(ref mut systray) = ctx.g.systray {
        systray.icons.retain(|&w| w != icon_win);
    }

    ctx.g.clients.remove(&icon_win);
}

/// Update systray icon geometry using dependency injection.
pub fn update_systray_icon_geom(ctx: &mut WmCtx, icon_win: WindowId, w: i32, h: i32) {
    let bh = ctx.g.cfg.bar_height;

    // Extract client data first to avoid borrow issues
    let client_data = ctx.g.clients.get(&icon_win).map(|client| {
        (
            client.geo.x,
            client.geo.y,
            client.geo.w,
            client.geo.h,
            client.base_width,
            client.base_height,
            client.min_width,
            client.min_height,
            client.max_width,
            client.max_height,
            client.inc_width,
            client.inc_height,
            client.base_aspect_num,
            client.base_aspect_denom,
            client.min_aspect_num,
            client.min_aspect_denom,
            client.max_aspect_num,
            client.max_aspect_denom,
        )
    });

    if let Some((
        geo_x,
        geo_y,
        _geo_w,
        _geo_h,
        base_w,
        base_h,
        min_w,
        min_h,
        max_w,
        max_h,
        inc_w,
        inc_h,
        base_aspect_num,
        base_aspect_denom,
        min_aspect_num,
        min_aspect_denom,
        max_aspect_num,
        max_aspect_denom,
    )) = client_data
    {
        let new_geo_h = bh;
        let new_geo_w = if w == h {
            bh
        } else if h == bh {
            w
        } else {
            (bh as f32 * (w as f32 / h as f32)) as i32
        };

        let mut x = geo_x;
        let mut y = geo_y;
        let mut client_width = new_geo_w;
        let mut client_height = new_geo_h;

        let _ = apply_size_hints(
            ctx,
            icon_win,
            &mut x,
            &mut y,
            &mut client_width,
            &mut client_height,
            false,
            base_w,
            base_h,
            min_w,
            min_h,
            max_w,
            max_h,
            inc_w,
            inc_h,
            base_aspect_num,
            base_aspect_denom,
            min_aspect_num,
            min_aspect_denom,
            max_aspect_num,
            max_aspect_denom,
        );

        // Now update the client with the computed values
        if let Some(client) = ctx.g.clients.get_mut(&icon_win) {
            client.geo.x = x;
            client.geo.y = y;
            client.geo.w = client_width;
            client.geo.h = client_height;

            if client.geo.h > bh {
                if client.geo.w == client.geo.h {
                    client.geo.w = bh;
                } else {
                    client.geo.w = (bh as f32 * (client.geo.w as f32 / client.geo.h as f32)) as i32;
                }
                client.geo.h = bh;
            }
        }
    }
}

/// Update systray icon state using dependency injection.
pub fn update_systray_icon_state(ctx: &mut WmCtx, icon_win: WindowId, ev: &PropertyNotifyEvent) {
    if !ctx.g.cfg.showsystray {
        return;
    }

    let xembed_info_atom = ctx.g.cfg.xatom.xembed_info;
    if ev.atom != xembed_info_atom {
        return;
    }

    let x11_icon_win: Window = icon_win.into();

    let flags = get_atom_prop(ctx, icon_win, xembed_info_atom);

    if flags == 0 {
        return;
    }

    let (current_tags, _has_systray) = {
        if let Some(client) = ctx.g.clients.get(&icon_win) {
            (client.tags, ctx.g.systray.is_some())
        } else {
            return;
        }
    };

    if (flags & XEMBED_MAPPED) != 0 && current_tags == 0 {
        if let Some(client) = ctx.g.clients.get_mut(&icon_win) {
            client.tags = 1;
        }

        let systray_win = ctx.g.systray.as_ref().map(|s| s.win).unwrap_or_default();
        if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
            let _ = conn.map_window(x11_icon_win);
            let _ = conn.configure_window(
                x11_icon_win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
            send_event(
                conn,
                icon_win,
                xembed_info_atom,
                xembed_info_atom,
                CURRENT_TIME as i64,
                XEMBED_WINDOW_ACTIVATE as i64,
                0,
                u32::from(systray_win) as i64,
                XEMBED_EMBEDDED_VERSION as i64,
            );
        }
        set_client_state(ctx, icon_win, 1);
    } else if (flags & XEMBED_MAPPED) == 0 && current_tags != 0 {
        if let Some(client) = ctx.g.clients.get_mut(&icon_win) {
            client.tags = 0;
        }

        let systray_win = ctx.g.systray.as_ref().map(|s| s.win).unwrap_or_default();
        if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
            let _ = conn.unmap_window(x11_icon_win);
            send_event(
                conn,
                icon_win,
                xembed_info_atom,
                xembed_info_atom,
                CURRENT_TIME as i64,
                XEMBED_WINDOW_DEACTIVATE as i64,
                0,
                u32::from(systray_win) as i64,
                XEMBED_EMBEDDED_VERSION as i64,
            );
        }
        set_client_state(ctx, icon_win, 0);
    }
}

/// Update systray using dependency injection.
pub fn update_systray(ctx: &mut WmCtx) {
    if !ctx.g.cfg.showsystray {
        return;
    }

    if ctx.g.cfg.xlibdisplay.0.is_null() {
        return;
    }

    // Flush Xlib display to ensure all Xlib requests are sent before using x11rb
    unsafe {
        crate::drw::XFlush(ctx.g.cfg.xlibdisplay.0);
    }

    let (x, by, _showbar, barwin) = {
        let m = systray_to_mon(ctx, None);
        let mon = match ctx.g.monitors.get(m) {
            Some(mon) => mon,
            None => return,
        };
        (
            mon.monitor_rect.x + mon.monitor_rect.w,
            mon.by,
            mon.showbar,
            mon.barwin,
        )
    };

    let mut w: u32 = 1;

    let systray_exists = ctx.g.systray.is_some();

    if !systray_exists {
        let root = ctx.g.cfg.root;
        let bh = ctx.g.cfg.bar_height;
        let net_system_tray = ctx.g.cfg.netatom.system_tray;
        let net_system_tray_horz = ctx.g.cfg.netatom.system_tray_orientation_horz;
        let manager_atom = ctx.g.cfg.xatom.manager;
        let bg_pixel = ctx
            .g
            .cfg
            .statusscheme
            .as_ref()
            .map(|scheme| scheme.bg.color.pixel as u32)
            .unwrap_or(0);

        let systray_win = ctx.x11_conn().map(|x11| x11.conn).and_then(|conn| {
            let systray_win = conn.generate_id().ok()?;

            let result = conn.create_window(
                x11rb::COPY_FROM_PARENT as u8,
                systray_win,
                root,
                x as i16,
                by as i16,
                w as u16,
                bh as u16,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new()
                    .event_mask(EventMask::BUTTON_PRESS | EventMask::EXPOSURE)
                    .override_redirect(1)
                    .background_pixel(bg_pixel),
            );

            if result.is_err() {
                return None;
            }

            let _ = result.and_then(|cookie| {
                cookie
                    .check()
                    .map_err(|_| x11rb::errors::ConnectionError::UnknownError)
            });

            let _ = conn.change_property32(
                PropMode::REPLACE,
                systray_win,
                net_system_tray,
                AtomEnum::CARDINAL,
                &[net_system_tray_horz],
            );

            let _ = conn.change_window_attributes(
                systray_win,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
            );

            let _ = conn.map_window(systray_win);

            let _ = conn.change_window_attributes(
                systray_win,
                &ChangeWindowAttributesAux::new().background_pixel(bg_pixel),
            );

            let _ = conn.set_selection_owner(systray_win, net_system_tray, CURRENT_TIME);

            // Send MANAGER event to root window to announce systray
            // Use non-blocking approach
            let event = ClientMessageEvent {
                response_type: CLIENT_MESSAGE_EVENT,
                format: 32,
                sequence: 0,
                window: root,
                type_: manager_atom,
                data: ClientMessageData::from([CURRENT_TIME, net_system_tray, systray_win, 0, 0]),
            };
            let _ = conn.send_event(false, root, EventMask::STRUCTURE_NOTIFY, event);

            Some(systray_win)
        });

        let Some(systray_win) = systray_win else {
            return;
        };

        ctx.g.systray = Some(Systray {
            win: WindowId::from(systray_win),
            icons: Vec::new(),
        });
    }

    let (systray_win, icons) = match ctx.g.systray.as_ref() {
        Some(s) => (s.win, s.icons.clone()),
        None => return,
    };

    let bh = ctx.g.cfg.bar_height;
    let systrayspacing = ctx.g.cfg.systrayspacing;
    let bg_pixel = ctx
        .g
        .cfg
        .statusscheme
        .as_ref()
        .map(|s| s.bg.color.pixel as u32)
        .unwrap_or(0);

    let icon_layout: Vec<(WindowId, i32, i32)> = icons
        .iter()
        .filter_map(|icon_win| {
            ctx.g
                .clients
                .get(icon_win)
                .map(|client| (*icon_win, client.geo.w, client.geo.h))
        })
        .collect();

    let mut systray_width = 0u32;
    for _ in 0..icon_layout.len() {
        systray_width += systrayspacing as u32;
    }
    for (_, icon_w, _) in &icon_layout {
        systray_width += *icon_w as u32;
    }

    {
        let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
            return;
        };

        w = 0;
        for (icon_win, icon_w, icon_h) in icon_layout {
            let x11_icon_win: Window = icon_win.into();
            let _ = conn.change_window_attributes(
                x11_icon_win,
                &ChangeWindowAttributesAux::new().background_pixel(bg_pixel),
            );
            let _ = conn.map_window(x11_icon_win);

            w += systrayspacing as u32;

            let _ = conn.configure_window(
                x11_icon_win,
                &ConfigureWindowAux::new()
                    .x(w as i32)
                    .y(0)
                    .width(icon_w as u32)
                    .height(icon_h as u32),
            );

            w += icon_w as u32;
        }
    }

    let systray_win = systray_win;
    let x11_systray_win: Window = systray_win.into();
    let x11_barwin: Window = barwin.into();

    w = if systray_width > 0 {
        systray_width + systrayspacing as u32
    } else {
        1
    };
    let x = x - w as i32;

    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return;
    };

    let _ = conn.configure_window(
        x11_systray_win,
        &ConfigureWindowAux::new()
            .x(x)
            .y(by)
            .width(w)
            .height(bh as u32),
    );

    let _ = conn.configure_window(
        x11_systray_win,
        &ConfigureWindowAux::new()
            .stack_mode(StackMode::ABOVE)
            .sibling(x11_barwin),
    );

    let _ = conn.map_window(x11_systray_win);

    let _ = conn.flush();
}

/// Convert window to systray icon using dependency injection.
pub fn win_to_systray_icon(ctx: &mut WmCtx, win: WindowId) -> Option<WindowId> {
    if !ctx.g.cfg.showsystray {
        return None;
    }

    if let Some(ref systray) = ctx.g.systray {
        for &icon_win in &systray.icons {
            if icon_win == win {
                return Some(win);
            }
        }
    }
    None
}

/// Get monitor for systray using dependency injection.
pub fn systray_to_mon(ctx: &mut WmCtx, m: Option<MonitorId>) -> MonitorId {
    if ctx.g.cfg.systraypinning == 0 {
        return match m {
            Some(id) => {
                if id == ctx.g.selmon_id() {
                    id
                } else {
                    ctx.g.selmon_id()
                }
            }
            None => ctx.g.selmon_id(),
        };
    }

    let n = ctx.g.monitors.len();
    let target = ctx.g.cfg.systraypinning.min(n);

    if ctx.g.cfg.systraypinning > n {
        0
    } else {
        target.saturating_sub(1)
    }
}

/// Get atom property using dependency injection.
fn get_atom_prop(ctx: &mut WmCtx, win: WindowId, atom: u32) -> u32 {
    let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) else {
        return 0;
    };
    let x11_win: Window = win.into();
    if let Ok(cookie) = conn.get_property(false, x11_win, atom, AtomEnum::CARDINAL, 0, 2) {
        if let Ok(reply) = cookie.reply() {
            if let Some(val) = reply.value32().and_then(|mut v| v.next()) {
                return val;
            }
        }
    }
    0
}

/// Send X event using dependency injection.
fn send_event(
    conn: &impl Connection,
    win: WindowId,
    proto: u32,
    mask: u32,
    d0: i64,
    d1: i64,
    d2: i64,
    d3: i64,
    d4: i64,
) {
    let x11_win: Window = win.into();
    let event = ClientMessageEvent {
        response_type: CLIENT_MESSAGE_EVENT,
        format: 32,
        sequence: 0,
        window: x11_win,
        type_: proto,
        data: ClientMessageData::from([d0 as u32, d1 as u32, d2 as u32, d3 as u32, d4 as u32]),
    };
    let _ = conn.send_event(false, x11_win, EventMask::from(mask), event);
}
