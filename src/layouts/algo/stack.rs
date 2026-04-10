//! Stacking layout algorithms: deck, bottom_stack, and bstackhoriz.

use crate::animation::{MoveResizeMode, move_resize_client};
use crate::client::resize;
use crate::constants::animation::{BORDER_MULTIPLIER, DEFAULT_FRAME_COUNT, FAST_FRAME_COUNT};
use crate::contexts::WmCtx;
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

    let master_area_width: u32 = if tiled_client_count > monitor.nmaster as u32 {
        if monitor.nmaster > 0 {
            (monitor.mfact * monitor.work_rect.w as f32) as u32
        } else {
            0
        }
    } else {
        monitor.work_rect.w as u32
    };

    // Collect tiled clients first
    let tiled_clients = monitor.collect_tiled(ctx.core().globals().clients.map());

    let mut master_column_offset: u32 = 0;
    for (index, client) in tiled_clients.iter().enumerate() {
        if (index as u32) < (monitor.nmaster as u32) {
            let master_window_height = (monitor.work_rect.h - master_column_offset as i32)
                / (min(tiled_client_count, monitor.nmaster as u32) - index as u32) as i32;
            resize(
                ctx,
                client.win,
                &Rect {
                    x: monitor.work_rect.x,
                    y: monitor.work_rect.y + master_column_offset as i32,
                    w: master_area_width as i32 - BORDER_MULTIPLIER * client.border_width,
                    h: master_window_height - BORDER_MULTIPLIER * client.border_width,
                },
                false,
            );

            if let Some(c) = ctx.core().globals().clients.get(&client.win) {
                master_column_offset += c.total_height() as u32;
            }
        } else {
            resize(
                ctx,
                client.win,
                &Rect {
                    x: monitor.work_rect.x + master_area_width as i32,
                    y: monitor.work_rect.y,
                    w: monitor.work_rect.w
                        - master_area_width as i32
                        - BORDER_MULTIPLIER * client.border_width,
                    h: monitor.work_rect.h - BORDER_MULTIPLIER * client.border_width,
                },
                false,
            );
        }
    }
}

// ── bottom_stack ───────────────────────────────────────────────────────────────

pub fn bottom_stack(ctx: &mut WmCtx<'_>, monitor: &mut Monitor) {
    let framecount = framecount_for_layout(
        ctx.core().globals(),
        4,
        FAST_FRAME_COUNT,
        DEFAULT_FRAME_COUNT,
    );
    let tiled_client_count =
        monitor.tiled_client_count(ctx.core_mut().globals_mut().clients.map()) as u32;

    if tiled_client_count == 0 {
        return;
    }

    let (master_area_height, stack_window_width, stack_area_y) =
        if tiled_client_count > monitor.nmaster as u32 {
            let master_area_height = if monitor.nmaster > 0 {
                (monitor.mfact * monitor.work_rect.h as f32) as i32
            } else {
                0
            };
            let stack_window_width =
                monitor.work_rect.w / (tiled_client_count - monitor.nmaster as u32) as i32;
            let stack_area_y = monitor.work_rect.y + master_area_height;
            (master_area_height, stack_window_width, stack_area_y)
        } else {
            (
                monitor.work_rect.h,
                monitor.work_rect.w,
                monitor.work_rect.y,
            )
        };

    let tiled_clients = monitor.collect_tiled(ctx.core().globals().clients.map());

    let mut master_row_offset: i32 = 0;
    let mut stack_window_x: i32 = monitor.work_rect.x;

    for (index, client) in tiled_clients.iter().enumerate() {
        if (index as u32) < (monitor.nmaster as u32) {
            let master_window_width = (monitor.work_rect.w - master_row_offset)
                / (min(tiled_client_count, monitor.nmaster as u32) - index as u32) as i32;
            move_resize_client(
                ctx,
                client.win,
                &Rect {
                    x: monitor.work_rect.x + master_row_offset,
                    y: monitor.work_rect.y,
                    w: master_window_width - BORDER_MULTIPLIER * client.border_width,
                    h: master_area_height - BORDER_MULTIPLIER * client.border_width,
                },
                MoveResizeMode::Normal,
                framecount,
            );

            if let Some(c) = ctx.core().globals().clients.get(&client.win) {
                master_row_offset += c.total_width();
            }
        } else {
            let stack_window_height = monitor.work_rect.h - master_area_height;
            move_resize_client(
                ctx,
                client.win,
                &Rect {
                    x: stack_window_x,
                    y: stack_area_y,
                    w: stack_window_width - BORDER_MULTIPLIER * client.border_width,
                    h: stack_window_height - BORDER_MULTIPLIER * client.border_width,
                },
                MoveResizeMode::Normal,
                framecount,
            );

            if stack_window_width != monitor.work_rect.w
                && let Some(c) = ctx.core().globals().clients.get(&client.win)
            {
                stack_window_x += c.total_width();
            }
        }
    }
}

// ── bstackhoriz ───────────────────────────────────────────────────────────────

pub fn bstackhoriz(ctx: &mut WmCtx<'_>, monitor: &mut Monitor) {
    let framecount = framecount_for_layout(
        ctx.core().globals(),
        4,
        FAST_FRAME_COUNT,
        DEFAULT_FRAME_COUNT,
    );
    let tiled_client_count =
        monitor.tiled_client_count(ctx.core_mut().globals_mut().clients.map()) as u32;

    if tiled_client_count == 0 {
        return;
    }

    let (master_area_height, stack_window_height, mut stack_window_y) =
        if tiled_client_count > monitor.nmaster as u32 {
            let master_area_height = if monitor.nmaster > 0 {
                (monitor.mfact * monitor.work_rect.h as f32) as i32
            } else {
                0
            };
            let stack_window_height = (monitor.work_rect.h - master_area_height)
                / (tiled_client_count - monitor.nmaster as u32) as i32;
            let stack_window_y = monitor.work_rect.y + master_area_height;
            (master_area_height, stack_window_height, stack_window_y)
        } else {
            (
                monitor.work_rect.h,
                monitor.work_rect.h,
                monitor.work_rect.y,
            )
        };

    let tiled_clients = monitor.collect_tiled(ctx.core().globals().clients.map());

    let mut master_row_offset: i32 = 0;
    let stack_window_x: i32 = monitor.work_rect.x;

    for (index, client) in tiled_clients.iter().enumerate() {
        if (index as u32) < (monitor.nmaster as u32) {
            let master_window_width = (monitor.work_rect.w - master_row_offset)
                / (min(tiled_client_count, monitor.nmaster as u32) - index as u32) as i32;
            move_resize_client(
                ctx,
                client.win,
                &Rect {
                    x: monitor.work_rect.x + master_row_offset,
                    y: monitor.work_rect.y,
                    w: master_window_width - BORDER_MULTIPLIER * client.border_width,
                    h: master_area_height - BORDER_MULTIPLIER * client.border_width,
                },
                MoveResizeMode::Normal,
                framecount,
            );

            if let Some(c) = ctx.core().globals().clients.get(&client.win) {
                master_row_offset += c.total_width();
            }
        } else {
            move_resize_client(
                ctx,
                client.win,
                &Rect {
                    x: stack_window_x,
                    y: stack_window_y,
                    w: monitor.work_rect.w - BORDER_MULTIPLIER * client.border_width,
                    h: stack_window_height - BORDER_MULTIPLIER * client.border_width,
                },
                MoveResizeMode::Normal,
                framecount,
            );

            if stack_window_height != monitor.work_rect.h
                && let Some(c) = ctx.core().globals().clients.get(&client.win)
            {
                stack_window_y += c.total_height();
            }
        }
    }
}
