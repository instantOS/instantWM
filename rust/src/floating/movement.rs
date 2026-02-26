//! Keyboard-driven floating window movement, resize, and scaling.

use crate::animation::animate_client;
use crate::backend::BackendKind;
use crate::client::resize;
use crate::contexts::WmCtx;
use crate::focus::warp_cursor_to_client;
use crate::types::*;

pub fn moveresize(ctx: &mut WmCtx, win: WindowId, dir: Direction) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    let (is_floating, geo, border_width) = match ctx.g.clients.get(&win) {
        Some(c) => (c.isfloating, c.geo, c.border_width),
        None => return,
    };

    if super::helpers::has_tiling_layout(ctx) && !is_floating {
        return;
    }

    const MOVE_STEP: i32 = 40;
    let (dx, dy) = dir.move_delta(MOVE_STEP);
    let mut new_x = geo.x + dx;
    let mut new_y = geo.y + dy;

    let mon_rect = match ctx.g.selmon() {
        Some(m) => m.monitor_rect,
        None => return,
    };

    new_x = new_x.max(mon_rect.x);
    new_y = new_y.max(mon_rect.y);
    if new_y + geo.h > mon_rect.y + mon_rect.h {
        new_y = (mon_rect.h + mon_rect.y) - geo.h - border_width * 2;
    }
    if new_x + geo.w > mon_rect.x + mon_rect.w {
        new_x = (mon_rect.w + mon_rect.x) - geo.w - border_width * 2;
    }

    animate_client(
        ctx,
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
    warp_cursor_to_client(ctx, win);
}

pub fn key_resize(ctx: &mut WmCtx, win: WindowId, dir: Direction) {
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    let (is_floating, geo) = match ctx.g.clients.get(&win) {
        Some(c) => (c.isfloating, c.geo),
        None => return,
    };

    super::snap::reset_snap(ctx, win);

    if super::helpers::has_tiling_layout(ctx) && !is_floating {
        return;
    }

    const RESIZE_STEP: i32 = 40;
    let (dw, dh) = dir.resize_delta(RESIZE_STEP);
    let nw = geo.w + dw;
    let nh = geo.h + dh;

    warp_cursor_to_client(ctx, win);
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
    if ctx.backend_kind() == BackendKind::Wayland {
        return;
    }
    let is_overlay = ctx.g.selmon().and_then(|m| m.overlay) == Some(win);
    if is_overlay {
        return;
    }
    let (geo, is_floating) = match ctx.g.clients.get(&win) {
        Some(c) => (c.geo, c.isfloating),
        None => return,
    };

    if super::helpers::has_tiling_layout(ctx) && !is_floating {
        return;
    }

    let (work_rect, mon_rect, showbar, bh) = match ctx.g.selmon() {
        Some(m) => (m.work_rect, m.monitor_rect, m.showbar, ctx.g.cfg.bar_height),
        None => return,
    };

    if geo.w > work_rect.w || geo.h > work_rect.h {
        return;
    }

    let y_offset = if showbar { bh } else { -bh };

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
