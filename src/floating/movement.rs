//! Keyboard-driven floating window movement, resize, and scaling.

use crate::animation::{MoveResizeMode, move_resize_client};
use crate::client::resize;
use crate::contexts::WmCtx;
use crate::types::*;

pub fn moveresize(ctx: &mut WmCtx, win: WindowId, dir: Direction) {
    let (is_floating, geo, border_width) = match ctx.client(win) {
        Some(c) => (c.is_floating, c.geo, c.border_width),
        None => return,
    };

    if super::helpers::has_tiling_layout(ctx.core()) && !is_floating {
        return;
    }

    const MOVE_STEP: i32 = 40;
    let (dx, dy) = dir.move_delta(MOVE_STEP);
    let mut new_x = geo.x + dx;
    let mut new_y = geo.y + dy;

    let mon_rect = ctx.core().globals().selected_monitor().monitor_rect;

    new_x = new_x.max(mon_rect.x);
    new_y = new_y.max(mon_rect.y);
    if new_y + geo.h > mon_rect.y + mon_rect.h {
        new_y = (mon_rect.h + mon_rect.y) - geo.h - border_width * 2;
    }
    if new_x + geo.w > mon_rect.x + mon_rect.w {
        new_x = (mon_rect.w + mon_rect.x) - geo.w - border_width * 2;
    }

    move_resize_client(
        ctx,
        win,
        &Rect {
            x: new_x,
            y: new_y,
            w: geo.w,
            h: geo.h,
        },
        MoveResizeMode::AnimateTo,
        5,
    );
    ctx.warp_cursor_to_client(win);
}

pub fn key_resize(ctx: &mut WmCtx, win: WindowId, dir: Direction) {
    let (is_floating, geo) = match ctx.client(win) {
        Some(c) => (c.is_floating, c.geo),
        None => return,
    };

    super::snap::reset_snap(ctx, win);

    if super::helpers::has_tiling_layout(ctx.core()) && !is_floating {
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
    let is_overlay = ctx.core().globals().selected_monitor().overlay == Some(win);
    if is_overlay {
        return;
    }
    let (geo, is_floating) = match ctx.client(win) {
        Some(c) => (c.geo, c.is_floating),
        None => return,
    };

    if super::helpers::has_tiling_layout(ctx.core()) && !is_floating {
        return;
    }

    let bar_height = ctx.core().globals().cfg.bar_height;
    let (work_rect, mon_rect, _showbar) = {
        let mon = ctx.core().globals().selected_monitor();
        (mon.work_rect, mon.monitor_rect, mon.selected_tags())
    };
    let showbar = {
        let mon = ctx.core_mut().globals_mut().selected_monitor_mut();
        mon.pertag_state().showbar
    };

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
