#![allow(dead_code)]
//! Stacking layout algorithms: deck, bottom_stack, and bstackhoriz.

use crate::animation::animate_client;
use crate::client::resize;
use crate::constants::animation::{BORDER_MULTIPLIER, DEFAULT_FRAME_COUNT, FAST_FRAME_COUNT};
use crate::contexts::WmCtx;
use crate::layouts::query::framecount_for_layout;
use crate::types::{Monitor, Rect};
use std::cmp::min;

// ── deck ─────────────────────────────────────────────────────────────────────

pub fn deck(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    let n = m.tiled_client_count(ctx.core_mut().globals_mut().clients.map()) as u32;

    if n == 0 {
        return;
    }

    let mw: u32 = if n > m.nmaster as u32 {
        if m.nmaster > 0 {
            (m.mfact * m.work_rect.w as f32) as u32
        } else {
            0
        }
    } else {
        m.work_rect.w as u32
    };

    // Collect tiled clients first
    let tiled = m.collect_tiled(ctx.core().globals().clients.map());

    let mut master_column_offset: u32 = 0;
    for (i, client) in tiled.iter().enumerate() {
        if (i as u32) < (m.nmaster as u32) {
            let h = (m.work_rect.h - master_column_offset as i32)
                / (min(n, m.nmaster as u32) - i as u32) as i32;
            resize(
                ctx,
                client.win,
                &Rect {
                    x: m.work_rect.x,
                    y: m.work_rect.y + master_column_offset as i32,
                    w: mw as i32 - BORDER_MULTIPLIER * client.border_width,
                    h: h - BORDER_MULTIPLIER * client.border_width,
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
                    x: m.work_rect.x + mw as i32,
                    y: m.work_rect.y,
                    w: m.work_rect.w - mw as i32 - BORDER_MULTIPLIER * client.border_width,
                    h: m.work_rect.h - BORDER_MULTIPLIER * client.border_width,
                },
                false,
            );
        }
    }
}

// ── bottom_stack ───────────────────────────────────────────────────────────────

pub fn bottom_stack(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    let framecount = framecount_for_layout(
        ctx.core().globals(),
        4,
        FAST_FRAME_COUNT,
        DEFAULT_FRAME_COUNT,
    );
    let n = m.tiled_client_count(ctx.core_mut().globals_mut().clients.map()) as u32;

    if n == 0 {
        return;
    }

    let (mh, tw, stack_y_offset) = if n > m.nmaster as u32 {
        let mh = if m.nmaster > 0 {
            (m.mfact * m.work_rect.h as f32) as i32
        } else {
            0
        };
        let tw = m.work_rect.w / (n - m.nmaster as u32) as i32;
        let stack_y_offset = m.work_rect.y + mh;
        (mh, tw, stack_y_offset)
    } else {
        (m.work_rect.h, m.work_rect.w, m.work_rect.y)
    };

    let tiled = m.collect_tiled(ctx.core().globals().clients.map());

    let mut master_row_offset: i32 = 0;
    let mut tx: i32 = m.work_rect.x;

    for (i, client) in tiled.iter().enumerate() {
        if (i as u32) < (m.nmaster as u32) {
            let w =
                (m.work_rect.w - master_row_offset) / (min(n, m.nmaster as u32) - i as u32) as i32;
            animate_client(
                ctx,
                client.win,
                &Rect {
                    x: m.work_rect.x + master_row_offset,
                    y: m.work_rect.y,
                    w: w - BORDER_MULTIPLIER * client.border_width,
                    h: mh - BORDER_MULTIPLIER * client.border_width,
                },
                framecount,
                0,
            );

            if let Some(c) = ctx.core().globals().clients.get(&client.win) {
                master_row_offset += c.total_width();
            }
        } else {
            let h = m.work_rect.h - mh;
            animate_client(
                ctx,
                client.win,
                &Rect {
                    x: tx,
                    y: stack_y_offset,
                    w: tw - BORDER_MULTIPLIER * client.border_width,
                    h: h - BORDER_MULTIPLIER * client.border_width,
                },
                framecount,
                0,
            );

            if tw != m.work_rect.w
                && let Some(c) = ctx.core().globals().clients.get(&client.win)
            {
                tx += c.total_width();
            }
        }
    }
}

// ── bstackhoriz ───────────────────────────────────────────────────────────────

pub fn bstackhoriz(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    let framecount = framecount_for_layout(
        ctx.core().globals(),
        4,
        FAST_FRAME_COUNT,
        DEFAULT_FRAME_COUNT,
    );
    let n = m.tiled_client_count(ctx.core_mut().globals_mut().clients.map()) as u32;

    if n == 0 {
        return;
    }

    let (mh, th, mut stack_y_offset) = if n > m.nmaster as u32 {
        let mh = if m.nmaster > 0 {
            (m.mfact * m.work_rect.h as f32) as i32
        } else {
            0
        };
        let th = (m.work_rect.h - mh) / (n - m.nmaster as u32) as i32;
        let stack_y_offset = m.work_rect.y + mh;
        (mh, th, stack_y_offset)
    } else {
        (m.work_rect.h, m.work_rect.h, m.work_rect.y)
    };

    let tiled = m.collect_tiled(ctx.core().globals().clients.map());

    let mut master_row_offset: i32 = 0;
    let tx: i32 = m.work_rect.x;

    for (i, client) in tiled.iter().enumerate() {
        if (i as u32) < (m.nmaster as u32) {
            let w =
                (m.work_rect.w - master_row_offset) / (min(n, m.nmaster as u32) - i as u32) as i32;
            animate_client(
                ctx,
                client.win,
                &Rect {
                    x: m.work_rect.x + master_row_offset,
                    y: m.work_rect.y,
                    w: w - BORDER_MULTIPLIER * client.border_width,
                    h: mh - BORDER_MULTIPLIER * client.border_width,
                },
                framecount,
                0,
            );

            if let Some(c) = ctx.core().globals().clients.get(&client.win) {
                master_row_offset += c.total_width();
            }
        } else {
            animate_client(
                ctx,
                client.win,
                &Rect {
                    x: tx,
                    y: stack_y_offset,
                    w: m.work_rect.w - BORDER_MULTIPLIER * client.border_width,
                    h: th - BORDER_MULTIPLIER * client.border_width,
                },
                framecount,
                0,
            );

            if th != m.work_rect.h
                && let Some(c) = ctx.core().globals().clients.get(&client.win)
            {
                stack_y_offset += c.total_height();
            }
        }
    }
}
