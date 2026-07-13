//! Keyboard-driven floating window movement, resize, and scaling.

use crate::constants::animation::FLOAT_MOVE_FRAME_COUNT;
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::types::*;

pub fn moveresize(ctx: &mut WmCtx, win: WindowId, dir: Direction) {
    let Some((is_floating, geo, border_width, mon_rect, has_tiling)) =
        ctx.core().model().client_view(win).map(|view| {
            (
                view.client.mode.is_floating(),
                view.client.geo,
                view.client.border_width,
                view.monitor.monitor_rect,
                view.monitor.is_tiling_layout(),
            )
        })
    else {
        return;
    };

    if has_tiling && !is_floating {
        return;
    }

    const MOVE_STEP: i32 = 40;
    let (dx, dy) = dir.move_delta(MOVE_STEP);
    let mut new_x = geo.x + dx;
    let mut new_y = geo.y + dy;

    new_x = new_x.max(mon_rect.x);
    new_y = new_y.max(mon_rect.y);
    if new_y + geo.h > mon_rect.y + mon_rect.h {
        new_y = (mon_rect.h + mon_rect.y) - geo.h - border_width * 2;
    }
    if new_x + geo.w > mon_rect.x + mon_rect.w {
        new_x = (mon_rect.w + mon_rect.x) - geo.w - border_width * 2;
    }

    ctx.move_resize(
        win,
        Rect {
            x: new_x,
            y: new_y,
            w: geo.w,
            h: geo.h,
        },
        MoveResizeOptions::animate_to(FLOAT_MOVE_FRAME_COUNT),
    );
    ctx.warp_cursor_to_client(win);
}

pub fn key_resize(ctx: &mut WmCtx, win: WindowId, dir: Direction) {
    let Some((is_floating, geo, has_tiling)) = ctx.core().model().client_view(win).map(|view| {
        (
            view.client.mode.is_floating(),
            view.client.geo,
            view.monitor.is_tiling_layout(),
        )
    }) else {
        return;
    };

    super::snap::reset_snap(ctx, win);

    if has_tiling && !is_floating {
        return;
    }

    const RESIZE_STEP: i32 = 40;
    let (dw, dh) = dir.resize_delta(RESIZE_STEP);
    let nw = geo.w + dw;
    let nh = geo.h + dh;

    ctx.warp_cursor_to_client(win);

    ctx.move_resize(
        win,
        Rect {
            x: geo.x,
            y: geo.y,
            w: nw,
            h: nh,
        },
        MoveResizeOptions::hinted_immediate(true),
    );
}

pub fn center_window(ctx: &mut WmCtx, win: WindowId) {
    let Some((
        geo,
        is_floating,
        is_edge_scratchpad,
        work_rect,
        mon_rect,
        bar_height,
        show_bar,
        has_tiling,
    )) = ctx.core().model().client_view(win).map(|view| {
        (
            view.client.geo,
            view.client.mode.is_floating(),
            view.client.is_edge_scratchpad(),
            view.monitor.work_rect,
            view.monitor.monitor_rect,
            view.monitor.bar_height,
            view.monitor.show_bar_for_mask(view.client.tags),
            view.monitor.is_tiling_layout(),
        )
    })
    else {
        return;
    };
    if is_edge_scratchpad {
        return;
    }

    if has_tiling && !is_floating {
        return;
    }

    if geo.w > work_rect.w || geo.h > work_rect.h {
        return;
    }

    let y_offset = if show_bar { bar_height } else { -bar_height };

    ctx.move_resize(
        win,
        Rect {
            x: mon_rect.x + (work_rect.w / 2) - (geo.w / 2),
            y: mon_rect.y + (work_rect.h / 2) - (geo.h / 2) + y_offset,
            w: geo.w,
            h: geo.h,
        },
        MoveResizeOptions::hinted_immediate(true),
    );
}
