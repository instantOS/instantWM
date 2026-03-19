//! Classic master-stack tiling layout.
//!
//! The screen is split vertically into:
//!
//! ```text
//! ┌──────────────┬───────────────┐
//! │              │  stack[0]     │
//! │  master[0]   ├───────────────┤
//! │              │  stack[1]     │
//! ├──────────────┼───────────────┤
//! │  master[1]   │  stack[2]     │
//! └──────────────┴───────────────┘
//! ```
//!
//! - The left column (width = `mfact * work_width`) holds the first `nmaster`
//!   tiled clients, each taking an equal share of the column height.
//! - The right column holds all remaining clients in the same fashion.
//! - When there is only one client it expands to fill the entire work area.

use crate::animation::animate_client;
use crate::constants::animation::BORDER_MULTIPLIER;
use crate::constants::animation::{DEFAULT_FRAME_COUNT, FAST_ANIM_THRESHOLD, FAST_FRAME_COUNT};
use crate::contexts::WmCtx;
use crate::layouts::query::framecount_for_layout;
use crate::types::{Monitor, Rect};
use std::cmp::min;

pub fn tile(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    let framecount = framecount_for_layout(
        ctx.g,
        FAST_ANIM_THRESHOLD,
        FAST_FRAME_COUNT,
        DEFAULT_FRAME_COUNT,
    );

    let n = m.tiled_client_count(ctx.g_mut().clients.map()) as u32;

    if n == 0 {
        return;
    }

    let mut mw: i32 = if n > m.nmaster as u32 {
        if m.nmaster > 0 {
            (m.mfact * m.work_rect.w as f32) as i32
        } else {
            0
        }
    } else {
        if n > 1 && n < m.nmaster as u32 {
            m.nmaster = n as i32;
            tile(ctx, m);
            return;
        }
        m.work_rect.w
    };

    // Collect tiled clients first
    let tiled = m.collect_tiled(ctx.g.clients.map());

    let mut master_y_offset: u32 = 0;
    let mut stack_y_offset: u32 = 0;

    for (i, client) in tiled.iter().enumerate() {
        if (i as u32) < (m.nmaster as u32) {
            let h = (m.work_rect.h - master_y_offset as i32)
                / (min(n, m.nmaster as u32) - i as u32) as i32;

            let frames = if n == 2 { 0 } else { framecount };

            animate_client(
                ctx,
                client.win,
                &Rect {
                    x: m.work_rect.x,
                    y: m.work_rect.y + master_y_offset as i32,
                    w: mw - BORDER_MULTIPLIER * client.border_width,
                    h: h - BORDER_MULTIPLIER * client.border_width,
                },
                frames,
                0,
            );

            if m.nmaster == 1
                && n > 1
                && let Some(c) = ctx.client(client.win)
            {
                mw = c.geo.w + c.border_width * 2;
            }

            if let Some(c) = ctx.client(client.win)
                && master_y_offset as i32 + c.total_height() < m.work_rect.h
            {
                master_y_offset += c.total_height() as u32;
            }
        } else {
            let h = (m.work_rect.h - stack_y_offset as i32) / (n - i as u32) as i32;

            animate_client(
                ctx,
                client.win,
                &Rect {
                    x: m.work_rect.x + mw,
                    y: m.work_rect.y + stack_y_offset as i32,
                    w: m.work_rect.w - mw - BORDER_MULTIPLIER * client.border_width,
                    h: h - BORDER_MULTIPLIER * client.border_width,
                },
                framecount,
                0,
            );

            if let Some(c) = ctx.g.clients.get(&client.win)
                && stack_y_offset as i32 + c.total_height() < m.work_rect.h
            {
                stack_y_offset += c.total_height() as u32;
            }
        }
    }
}
