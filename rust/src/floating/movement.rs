//! Keyboard-driven floating window movement, resize, and scaling.

use crate::animation::animate_client;
use crate::client::resize;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::focus::warp_cursor_to_client_x11;
use crate::types::*;

pub fn moveresize(ctx: &mut WmCtxX11, win: WindowId, dir: Direction) {
    let (is_floating, geo, border_width) = match ctx.core.g.clients.get(&win) {
        Some(c) => (c.isfloating, c.geo, c.border_width),
        None => return,
    };

    if super::helpers::has_tiling_layout(&ctx.core) && !is_floating {
        return;
    }

    const MOVE_STEP: i32 = 40;
    let (dx, dy) = dir.move_delta(MOVE_STEP);
    let mut new_x = geo.x + dx;
    let mut new_y = geo.y + dy;

    let mon_rect = ctx.core.g.selected_monitor().monitor_rect;

    new_x = new_x.max(mon_rect.x);
    new_y = new_y.max(mon_rect.y);
    if new_y + geo.h > mon_rect.y + mon_rect.h {
        new_y = (mon_rect.h + mon_rect.y) - geo.h - border_width * 2;
    }
    if new_x + geo.w > mon_rect.x + mon_rect.w {
        new_x = (mon_rect.w + mon_rect.x) - geo.w - border_width * 2;
    }

    let mut wm_ctx = crate::contexts::WmCtx::X11(ctx.reborrow());
    animate_client(
        &mut wm_ctx,
        win,
        &Rect {
            x: new_x,
            y: new_y,
            w: geo.w,
            h: geo.h,
        },
        5,
        0,
    );
    warp_cursor_to_client_x11(&ctx.core, &ctx.x11, win);
}

pub fn key_resize(ctx: &mut WmCtx, win: WindowId, dir: Direction) {
    let (is_floating, geo) = match ctx.g().clients.get(&win) {
        Some(c) => (c.isfloating, c.geo),
        None => return,
    };

    super::snap::reset_snap(ctx, win);

    if super::helpers::has_tiling_layout(ctx.g()) && !is_floating {
        return;
    }

    const RESIZE_STEP: i32 = 40;
    let (dw, dh) = dir.resize_delta(RESIZE_STEP);
    let nw = geo.w + dw;
    let nh = geo.h + dh;

    ctx.warp_cursor_to_client(win);

    resize(
        ctx,
        win,
        &Rect {
            x: geo.x,
            y: geo.y,
            w: nw,
            h: nh,
        },
        true,
    );
}

pub fn center_window(ctx: &mut WmCtx, win: WindowId) {
    let is_overlay = ctx.g().selected_monitor().overlay == Some(win);
    if is_overlay {
        return;
    }
    let (geo, is_floating) = match ctx.g().clients.get(&win) {
        Some(c) => (c.geo, c.isfloating),
        None => return,
    };

    if super::helpers::has_tiling_layout(ctx.g()) && !is_floating {
        return;
    }

    let mon = ctx.g().selected_monitor();
    let work_rect = mon.work_rect;
    let mon_rect = mon.monitor_rect;
    let showbar = mon.showbar;
    let bar_height = ctx.g().cfg.bar_height;

    if geo.w > work_rect.w || geo.h > work_rect.h {
        return;
    }

    let y_offset = if showbar { bar_height } else { -bar_height };

    resize(
        ctx,
        win,
        &Rect {
            x: mon_rect.x + (work_rect.w / 2) - (geo.w / 2),
            y: mon_rect.y + (work_rect.h / 2) - (geo.h / 2) + y_offset,
            w: geo.w,
            h: geo.h,
        },
        true,
    );
}
