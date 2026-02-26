//! Floating state transitions and geometry persistence.

use crate::animation::animate_client;
use crate::client::{resize, restore_border_width};
use crate::contexts::WmCtx;
use crate::layouts::arrange;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

pub fn set_floating_in_place(ctx: &mut WmCtx, win: WindowId) {
    if let Some(client) = ctx.g.clients.get_mut(&win) {
        client.isfloating = true;
    }

    restore_border_width(win);

    let conn = ctx.x11.conn;
    let x11_win: Window = win.into();
    if let Some(ref scheme) = ctx.g.cfg.borderscheme {
        let pixel = scheme.float_focus.bg.color.pixel;
        let _ = change_window_attributes(
            conn,
            x11_win,
            &ChangeWindowAttributesAux::new().border_pixel(Some(pixel as u32)),
        );
        let _ = conn.flush();
    }
}

pub fn save_floating_win(ctx: &mut WmCtx, win: WindowId) {
    if let Some(client) = ctx.g.clients.get_mut(&win) {
        client.float_geo = client.geo;
    }
}

pub fn restore_floating_win(ctx: &mut WmCtx, win: WindowId) {
    let float_geo = ctx.g.clients.get(&win).map(|c| c.float_geo);
    if let Some(rect) = float_geo {
        resize(ctx, win, &rect, false);
    }
}

pub fn apply_float_change(
    ctx: &mut WmCtx,
    win: WindowId,
    floating: bool,
    animate: bool,
    update_borders: bool,
) {
    if floating {
        if let Some(client) = ctx.g.clients.get_mut(&win) {
            client.isfloating = true;
        }

        if update_borders {
            restore_border_width(win);

            let conn = ctx.x11.conn;
            let x11_win: Window = win.into();
            if let Some(ref scheme) = ctx.g.cfg.borderscheme {
                let pixel = scheme.float_focus.bg.color.pixel;
                let _ = change_window_attributes(
                    conn,
                    x11_win,
                    &ChangeWindowAttributesAux::new().border_pixel(Some(pixel as u32)),
                );
                let _ = conn.flush();
            }
        }

        let saved_geo = ctx.g.clients.get(&win).map(|c| c.float_geo);
        let Some(saved_geo) = saved_geo else { return };

        if animate {
            animate_client(
                ctx,
                win,
                &Rect {
                    x: saved_geo.x,
                    y: saved_geo.y,
                    w: saved_geo.w,
                    h: saved_geo.h,
                },
                7,
                0,
            );
        } else {
            resize(ctx, win, &saved_geo, false);
        }
    } else {
        let client_count = ctx.g.clients.len();
        if let Some(client) = ctx.g.clients.get_mut(&win) {
            client.isfloating = false;
            client.float_geo = client.geo;

            if update_borders && client_count <= 1 && client.snapstatus == SnapPosition::None {
                if client.border_width != 0 {
                    client.old_border_width = client.border_width;
                }
                client.border_width = 0;
            }
        }
    }
}

pub fn toggle_floating(ctx: &mut WmCtx) {
    let sel_win = {
        let mon = match ctx.g.selmon() {
            Some(m) => m,
            None => return,
        };
        match mon.sel {
            Some(sel) if Some(sel) != mon.overlay => {
                if let Some(c) = ctx.g.clients.get(&sel) {
                    if c.is_fullscreen && !c.isfakefullscreen {
                        return;
                    }
                }
                Some(sel)
            }
            _ => None,
        }
    };

    let Some(win) = sel_win else { return };

    let (is_floating, is_fixed) = ctx
        .g
        .clients
        .get(&win)
        .map(|c| (c.isfloating, c.isfixed))
        .unwrap_or((false, false));

    let new_state = !is_floating || is_fixed;
    apply_float_change(ctx, win, new_state, true, true);
    arrange(ctx, Some(ctx.g.selmon_id()));
}

pub fn change_floating_win(ctx: &mut WmCtx, win: WindowId) {
    let (is_fullscreen, is_fake_fullscreen, is_floating, is_fixed) = match ctx.g.clients.get(&win) {
        Some(c) => (c.is_fullscreen, c.isfakefullscreen, c.isfloating, c.isfixed),
        None => return,
    };

    if is_fullscreen && !is_fake_fullscreen {
        return;
    }

    let new_state = !is_floating || is_fixed;
    apply_float_change(ctx, win, new_state, false, false);
    arrange(ctx, Some(ctx.g.selmon_id()));
}

pub fn set_floating(ctx: &mut WmCtx, win: WindowId, should_arrange: bool) {
    let (is_fullscreen, is_fake_fullscreen, is_floating) = match ctx.g.clients.get(&win) {
        Some(c) => (c.is_fullscreen, c.isfakefullscreen, c.isfloating),
        None => return,
    };

    if is_fullscreen && !is_fake_fullscreen {
        return;
    }
    if is_floating {
        return;
    }

    apply_float_change(ctx, win, true, false, false);

    if should_arrange {
        arrange(ctx, Some(ctx.g.selmon_id()));
    }
}

pub fn set_tiled(ctx: &mut WmCtx, win: WindowId, should_arrange: bool) {
    let (is_fullscreen, is_fake_fullscreen, is_floating, is_fixed) = match ctx.g.clients.get(&win) {
        Some(c) => (c.is_fullscreen, c.isfakefullscreen, c.isfloating, c.isfixed),
        None => return,
    };

    if is_fullscreen && !is_fake_fullscreen {
        return;
    }
    if !is_floating && !is_fixed {
        return;
    }

    apply_float_change(ctx, win, false, false, false);

    if should_arrange {
        arrange(ctx, Some(ctx.g.selmon_id()));
    }
}

pub fn temp_fullscreen(ctx: &mut WmCtx) {
    let (fullscreen_win, sel_win, animated) = {
        let mon = match ctx.g.selmon() {
            Some(m) => m,
            None => return,
        };
        (mon.fullscreen, mon.sel, ctx.g.animated)
    };

    if let Some(win) = fullscreen_win {
        let is_floating = ctx
            .g
            .clients
            .get(&win)
            .map(|c| c.isfloating)
            .unwrap_or(false);

        if is_floating || !super::helpers::has_tiling_layout(ctx) {
            restore_floating_win(ctx, win);
            super::helpers::apply_size(ctx, win);
        }

        if let Some(mon) = ctx.g.selmon_mut() {
            mon.fullscreen = None;
        }
    } else {
        let Some(win) = sel_win else { return };

        if let Some(mon) = ctx.g.selmon_mut() {
            mon.fullscreen = Some(win);
        }

        if super::helpers::check_floating(ctx, win) {
            save_floating_win(ctx, win);
        }
    }

    if animated {
        ctx.g.animated = false;
        arrange(ctx, Some(ctx.g.selmon_id()));
        ctx.g.animated = true;
    } else {
        arrange(ctx, Some(ctx.g.selmon_id()));
    }

    if let Some(win) = ctx.g.selmon().and_then(|m| m.fullscreen) {
        let conn = ctx.x11.conn;
        let _ = configure_window(
            conn,
            win,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        );
        let _ = conn.flush();
    }
}
