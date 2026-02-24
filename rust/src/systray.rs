use crate::client::{apply_size_hints, set_client_state};
use crate::contexts::WmCtx;
use crate::globals::{get_globals, get_globals_mut, get_x11};
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

/// Get systray width using dependency injection.
pub fn get_systray_width_ctx(ctx: &mut WmCtx) -> u32 {
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

/// Get systray width (wrapper for backward compatibility).
pub fn get_systray_width() -> u32 {
    let mut globals = get_globals();
    let x11 = get_x11();
    let mut ctx = WmCtx::new(&mut globals, x11);
    get_systray_width_ctx(&mut ctx)
}

/// Remove systray icon using dependency injection.
pub fn remove_systray_icon_ctx(ctx: &mut WmCtx, icon_win: Window) {
    if !ctx.g.cfg.showsystray {
        return;
    }

    if let Some(ref mut systray) = ctx.g.systray {
        systray.icons.retain(|&w| w != icon_win);
    }

    ctx.g.clients.remove(&icon_win);
}

/// Remove systray icon (wrapper for backward compatibility).
pub fn remove_systray_icon(icon_win: Window) {
    let mut globals = get_globals_mut();
    let x11 = get_x11();
    let mut ctx = WmCtx::new(&mut globals, x11);
    remove_systray_icon_ctx(&mut ctx, icon_win);
}

/// Update systray icon geometry using dependency injection.
pub fn update_systray_icon_geom_ctx(ctx: &mut WmCtx, icon_win: Window, w: i32, h: i32) {
    let bh = ctx.g.cfg.bh;
    if let Some(client) = ctx.g.clients.get_mut(&icon_win) {
        client.geo.h = bh;
        if w == h {
            client.geo.w = bh;
        } else if h == bh {
            client.geo.w = w;
        } else {
            client.geo.w = (bh as f32 * (w as f32 / h as f32)) as i32;
        }

        let mut x = client.geo.x;
        let mut y = client.geo.y;
        let mut client_width = client.geo.w;
        let mut client_height = client.geo.h;
        let _ = apply_size_hints(
            client,
            &mut x,
            &mut y,
            &mut client_width,
            &mut client_height,
            false,
        );

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

/// Update systray icon geometry (wrapper for backward compatibility).
pub fn update_systray_icon_geom(icon_win: Window, w: i32, h: i32) {
    let mut globals = get_globals_mut();
    let x11 = get_x11();
    let mut ctx = WmCtx::new(&mut globals, x11);
    update_systray_icon_geom_ctx(&mut ctx, icon_win, w, h);
}

/// Update systray icon state using dependency injection.
pub fn update_systray_icon_state_ctx(ctx: &mut WmCtx, icon_win: Window, ev: &PropertyNotifyEvent) {
    if !ctx.g.cfg.showsystray {
        return;
    }

    let xembed_info_atom = ctx.g.cfg.xatom.xembed_info;
    if ev.atom != xembed_info_atom {
        return;
    }

    let Some(ref conn) = ctx.x11.conn else { return };

    let flags = get_atom_prop_ctx(ctx, icon_win, xembed_info_atom);

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

        let _ = conn.map_window(icon_win);
        let _ = conn.configure_window(
            icon_win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        set_client_state(icon_win, 1);

        let systray_win = ctx.g.systray.as_ref().map(|s| s.win).unwrap_or(0);
        send_event_ctx(
            ctx,
            icon_win,
            xembed_info_atom,
            xembed_info_atom,
            CURRENT_TIME as i64,
            XEMBED_WINDOW_ACTIVATE as i64,
            0,
            systray_win as i64,
            XEMBED_EMBEDDED_VERSION as i64,
        );
    } else if (flags & XEMBED_MAPPED) == 0 && current_tags != 0 {
        if let Some(client) = ctx.g.clients.get_mut(&icon_win) {
            client.tags = 0;
        }

        let _ = conn.unmap_window(icon_win);
        set_client_state(icon_win, 0);

        let systray_win = ctx.g.systray.as_ref().map(|s| s.win).unwrap_or(0);
        send_event_ctx(
            ctx,
            icon_win,
            xembed_info_atom,
            xembed_info_atom,
            CURRENT_TIME as i64,
            XEMBED_WINDOW_DEACTIVATE as i64,
            0,
            systray_win as i64,
            XEMBED_EMBEDDED_VERSION as i64,
        );
    }
}

/// Update systray icon state (wrapper for backward compatibility).
pub fn update_systray_icon_state(icon_win: Window, ev: &PropertyNotifyEvent) {
    let mut globals = get_globals();
    let x11 = get_x11();
    let mut ctx = WmCtx::new(&mut globals, x11);
    update_systray_icon_state_ctx(&mut ctx, icon_win, ev);
}

/// Update systray using dependency injection.
pub fn update_systray_ctx(ctx: &mut WmCtx) {
    if !ctx.g.cfg.showsystray {
        return;
    }

    // Flush Xlib display to ensure all Xlib requests are sent before using x11rb
    unsafe {
        crate::drw::XFlush(ctx.g.xlibdisplay.0);
    }

    let (x, by, _showbar, barwin) = {
        let m = systray_to_mon_ctx(ctx, None);
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

    let Some(ref conn) = ctx.x11.conn else {
        return;
    };

    if !systray_exists {
        let root = ctx.g.cfg.root;
        let bh = ctx.g.cfg.bh;

        let net_system_tray = ctx.g.cfg.netatom.system_tray;
        let net_system_tray_horz = ctx.g.cfg.netatom.system_tray_orientation_horz;

        let bg_pixel = if let Some(ref scheme) = ctx.g.cfg.statusscheme {
            scheme.bg.color.pixel as u32
        } else {
            0
        };

        let systray_win = conn.generate_id().ok();
        let Some(systray_win) = systray_win else {
            return;
        };

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
            return;
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

        ctx.g.systray = Some(Systray {
            win: systray_win,
            icons: Vec::new(),
        });

        let manager_atom = ctx.g.cfg.xatom.manager;

        // Send MANAGER event to root window to announce systray
        // Use non-blocking approach
        if let Some(ref conn) = ctx.x11.conn {
            let event = ClientMessageEvent {
                response_type: CLIENT_MESSAGE_EVENT,
                format: 32,
                sequence: 0,
                window: root,
                type_: manager_atom,
                data: ClientMessageData::from([
                    CURRENT_TIME,
                    net_system_tray as u32,
                    systray_win,
                    0,
                    0,
                ]),
            };
            let _ = conn.send_event(false, root, EventMask::STRUCTURE_NOTIFY, event);
        }
    }

    let systray = match &ctx.g.systray {
        Some(s) => s,
        None => return,
    };

    let bh = ctx.g.cfg.bh;
    let systrayspacing = ctx.g.cfg.systrayspacing;
    let bg_pixel = ctx
        .g
        .cfg
        .statusscheme
        .as_ref()
        .map(|s| s.bg.color.pixel as u32)
        .unwrap_or(0);

    w = 0;
    for &icon_win in &systray.icons {
        let _ = conn.change_window_attributes(
            icon_win,
            &ChangeWindowAttributesAux::new().background_pixel(bg_pixel),
        );
        let _ = conn.map_window(icon_win);

        w += systrayspacing as u32;

        if let Some(client) = ctx.g.clients.get(&icon_win) {
            let icon_w = client.geo.w;
            let icon_h = client.geo.h;

            let _ = conn.configure_window(
                icon_win,
                &ConfigureWindowAux::new()
                    .x(w as i32)
                    .y(0)
                    .width(icon_w as u32)
                    .height(icon_h as u32),
            );

            w += icon_w as u32;
        }
    }

    let systray_win = systray.win;

    w = if w > 0 { w + systrayspacing as u32 } else { 1 };
    let x = x - w as i32;

    let _ = conn.configure_window(
        systray_win,
        &ConfigureWindowAux::new()
            .x(x)
            .y(by)
            .width(w)
            .height(bh as u32),
    );

    let _ = conn.configure_window(
        systray_win,
        &ConfigureWindowAux::new()
            .stack_mode(StackMode::ABOVE)
            .sibling(barwin),
    );

    let _ = conn.map_window(systray_win);

    let _ = conn.flush();
}

/// Update systray (wrapper for backward compatibility).
pub fn update_systray() {
    let mut globals = get_globals();
    let x11 = get_x11();
    let mut ctx = WmCtx::new(&mut globals, x11);
    update_systray_ctx(&mut ctx);
}

/// Convert window to systray icon using dependency injection.
pub fn win_to_systray_icon_ctx(ctx: &mut WmCtx, win: Window) -> Option<Window> {
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

/// Convert window to systray icon (wrapper for backward compatibility).
pub fn win_to_systray_icon(win: Window) -> Option<Window> {
    let mut globals = get_globals();
    let x11 = get_x11();
    let mut ctx = WmCtx::new(&mut globals, x11);
    win_to_systray_icon_ctx(&mut ctx, win)
}

/// Get monitor for systray using dependency injection.
pub fn systray_to_mon_ctx(ctx: &mut WmCtx, m: Option<MonitorId>) -> MonitorId {
    if ctx.g.cfg.systraypinning == 0 {
        return match m {
            Some(id) => {
                if id == ctx.g.selmon {
                    id
                } else {
                    ctx.g.selmon
                }
            }
            None => ctx.g.selmon,
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

/// Get monitor for systray (wrapper for backward compatibility).
pub fn systray_to_mon(m: Option<MonitorId>) -> MonitorId {
    let mut globals = get_globals();
    let x11 = get_x11();
    let mut ctx = WmCtx::new(&mut globals, x11);
    systray_to_mon_ctx(&mut ctx, m)
}

/// Get atom property using dependency injection.
fn get_atom_prop_ctx(ctx: &mut WmCtx, win: Window, atom: u32) -> u32 {
    if let Some(ref conn) = ctx.x11.conn {
        if let Ok(cookie) = conn.get_property(false, win, atom, AtomEnum::CARDINAL, 0, 2) {
            if let Ok(reply) = cookie.reply() {
                if let Some(val) = reply.value32().and_then(|mut v| v.next()) {
                    return val;
                }
            }
        }
    }
    0
}

/// Send X event using dependency injection.
fn send_event_ctx(ctx: &mut WmCtx, win: Window, proto: u32, mask: u32, d0: i64, d1: i64, d2: i64, d3: i64, d4: i64) {
    if let Some(ref conn) = ctx.x11.conn {
        let event = ClientMessageEvent {
            response_type: CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window: win,
            type_: proto,
            data: ClientMessageData::from([d0 as u32, d1 as u32, d2 as u32, d3 as u32, d4 as u32]),
        };
        let _ = conn.send_event(false, win, EventMask::from(mask), event);
    }
}
