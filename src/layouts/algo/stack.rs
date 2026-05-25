//! Stacking layout algorithms: deck, bottom_stack, and bstackhoriz.

use crate::constants::animation::{DEFAULT_FRAME_COUNT, FAST_FRAME_COUNT};
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::layouts::LayoutKind;
use crate::layouts::placement::LayoutPlacement;
use crate::layouts::query::framecount_for_layout;
use crate::types::{Monitor, Rect};
use std::cmp::min;

// ── deck ─────────────────────────────────────────────────────────────────────

pub fn deck(ctx: &mut WmCtx<'_>, monitor: &mut Monitor) {
    let tiled_client_count =
        monitor.tiled_client_count(ctx.core_mut().globals_mut().clients.map()) as u32;

    if tiled_client_count == 0 {
        return;
    }

    let placement = LayoutPlacement::new(
        ctx.core().globals(),
        monitor,
        LayoutKind::Deck,
        tiled_client_count,
    );
    let work_rect = placement.work_rect();
    let nmaster = monitor.nmaster.max(0) as u32;
    let master_area_width: u32 = if tiled_client_count > nmaster {
        if nmaster > 0 {
            (monitor.mfact * work_rect.w as f32) as u32
        } else {
            0
        }
    } else {
        work_rect.w as u32
    };

    // Collect tiled clients first
    let tiled_clients = monitor.collect_tiled(ctx.core().globals().clients.map());

    let mut master_column_offset: i32 = 0;
    for (index, client) in tiled_clients.iter().enumerate() {
        if (index as u32) < nmaster {
            let master_window_height = (work_rect.h - master_column_offset)
                / (min(tiled_client_count, nmaster) - index as u32) as i32;
            let slot = Rect {
                x: work_rect.x,
                y: work_rect.y + master_column_offset,
                w: master_area_width as i32,
                h: master_window_height,
            };
            placement.place(
                ctx,
                client.win,
                slot,
                client.border_width,
                MoveResizeOptions::hinted_immediate(false),
            );

            master_column_offset += slot.h;
        } else {
            placement.place(
                ctx,
                client.win,
                Rect {
                    x: work_rect.x + master_area_width as i32,
                    y: work_rect.y,
                    w: work_rect.w - master_area_width as i32,
                    h: work_rect.h,
                },
                client.border_width,
                MoveResizeOptions::hinted_immediate(false),
            );
        }
    }
}

// ── bottom_stack ───────────────────────────────────────────────────────────────

pub fn bottom_stack(ctx: &mut WmCtx<'_>, monitor: &mut Monitor) {
    let tiled_client_count =
        monitor.tiled_client_count(ctx.core_mut().globals_mut().clients.map()) as u32;

    if tiled_client_count == 0 {
        return;
    }

    let framecount = framecount_for_layout(
        ctx.core().globals().behavior.animated,
        tiled_client_count as usize,
        4,
        FAST_FRAME_COUNT,
        DEFAULT_FRAME_COUNT,
    );

    let placement = LayoutPlacement::new(
        ctx.core().globals(),
        monitor,
        LayoutKind::BottomStack,
        tiled_client_count,
    );
    let work_rect = placement.work_rect();
    let nmaster = monitor.nmaster.max(0) as u32;
    let (master_area_height, stack_window_width, stack_area_y) = if tiled_client_count > nmaster {
        let master_area_height = if nmaster > 0 {
            (monitor.mfact * work_rect.h as f32) as i32
        } else {
            0
        };
        let stack_window_width = work_rect.w / (tiled_client_count - nmaster) as i32;
        let stack_area_y = work_rect.y + master_area_height;
        (master_area_height, stack_window_width, stack_area_y)
    } else {
        (work_rect.h, work_rect.w, work_rect.y)
    };

    let tiled_clients = monitor.collect_tiled(ctx.core().globals().clients.map());

    let mut master_row_offset: i32 = 0;
    let mut stack_window_x: i32 = work_rect.x;

    for (index, client) in tiled_clients.iter().enumerate() {
        if (index as u32) < nmaster {
            let master_window_width = (work_rect.w - master_row_offset)
                / (min(tiled_client_count, nmaster) - index as u32) as i32;
            let slot = Rect {
                x: work_rect.x + master_row_offset,
                y: work_rect.y,
                w: master_window_width,
                h: master_area_height,
            };
            placement.place(
                ctx,
                client.win,
                slot,
                client.border_width,
                MoveResizeOptions::animate_to(framecount),
            );

            master_row_offset += slot.w;
        } else {
            let stack_window_height = work_rect.h - master_area_height;
            let slot = Rect {
                x: stack_window_x,
                y: stack_area_y,
                w: stack_window_width,
                h: stack_window_height,
            };
            placement.place(
                ctx,
                client.win,
                slot,
                client.border_width,
                MoveResizeOptions::animate_to(framecount),
            );

            if stack_window_width != work_rect.w {
                stack_window_x += slot.w;
            }
        }
    }
}

// ── bstackhoriz ───────────────────────────────────────────────────────────────

pub fn bstackhoriz(ctx: &mut WmCtx<'_>, monitor: &mut Monitor) {
    let tiled_client_count =
        monitor.tiled_client_count(ctx.core_mut().globals_mut().clients.map()) as u32;

    if tiled_client_count == 0 {
        return;
    }

    let framecount = framecount_for_layout(
        ctx.core().globals().behavior.animated,
        tiled_client_count as usize,
        4,
        FAST_FRAME_COUNT,
        DEFAULT_FRAME_COUNT,
    );

    let placement = LayoutPlacement::new(
        ctx.core().globals(),
        monitor,
        LayoutKind::BottomStack,
        tiled_client_count,
    );
    let work_rect = placement.work_rect();
    let nmaster = monitor.nmaster.max(0) as u32;
    let (master_area_height, stack_window_height, mut stack_window_y) =
        if tiled_client_count > nmaster {
            let master_area_height = if nmaster > 0 {
                (monitor.mfact * work_rect.h as f32) as i32
            } else {
                0
            };
            let stack_window_height =
                (work_rect.h - master_area_height) / (tiled_client_count - nmaster) as i32;
            let stack_window_y = work_rect.y + master_area_height;
            (master_area_height, stack_window_height, stack_window_y)
        } else {
            (work_rect.h, work_rect.h, work_rect.y)
        };

    let tiled_clients = monitor.collect_tiled(ctx.core().globals().clients.map());

    let mut master_row_offset: i32 = 0;
    let stack_window_x: i32 = work_rect.x;

    for (index, client) in tiled_clients.iter().enumerate() {
        if (index as u32) < nmaster {
            let master_window_width = (work_rect.w - master_row_offset)
                / (min(tiled_client_count, nmaster) - index as u32) as i32;
            let slot = Rect {
                x: work_rect.x + master_row_offset,
                y: work_rect.y,
                w: master_window_width,
                h: master_area_height,
            };
            placement.place(
                ctx,
                client.win,
                slot,
                client.border_width,
                MoveResizeOptions::animate_to(framecount),
            );

            master_row_offset += slot.w;
        } else {
            let slot = Rect {
                x: stack_window_x,
                y: stack_window_y,
                w: work_rect.w,
                h: stack_window_height,
            };
            placement.place(
                ctx,
                client.win,
                slot,
                client.border_width,
                MoveResizeOptions::animate_to(framecount),
            );

            if stack_window_height != work_rect.h {
                stack_window_y += slot.h;
            }
        }
    }
}
