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
use crate::client::next_tiled;
use crate::constants::animation::BORDER_MULTIPLIER;
use crate::constants::animation::{DEFAULT_FRAME_COUNT, FAST_ANIM_THRESHOLD, FAST_FRAME_COUNT};
use crate::contexts::WmCtx;
use crate::layouts::query::{count_tiled_clients, framecount_for_layout};
use crate::types::{Monitor, Rect};
use std::cmp::min;

pub fn tile(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    let framecount = framecount_for_layout(
        ctx.g,
        FAST_ANIM_THRESHOLD,
        FAST_FRAME_COUNT,
        DEFAULT_FRAME_COUNT,
    );

    // ── count tiled clients ───────────────────────────────────────────────
    let n = count_tiled_clients(ctx, m);

    if n == 0 {
        return;
    }

    // ── master-column width ───────────────────────────────────────────────
    let mut mw: i32 = if n > m.nmaster as u32 {
        if m.nmaster > 0 {
            (m.mfact * m.work_rect.w as f32) as i32
        } else {
            0
        }
    } else {
        // All clients fit in the master column — handle degeneracy.
        if n > 1 && n < m.nmaster as u32 {
            m.nmaster = n as i32;
            tile(ctx, m);
            return;
        }
        m.work_rect.w
    };

    // ── place each client ─────────────────────────────────────────────────
    let mut master_y_offset: u32 = 0; // running y-offset inside master column
    let mut stack_y_offset: u32 = 0; // running y-offset inside stack column
    let mut i: u32 = 0;
    let mut current_window = m
        .clients
        .first()
        .copied()
        .and_then(|w| next_tiled(ctx, Some(w)));

    while let Some(win) = current_window {
        let border_width = ctx
            .g
            .clients
            .get(&win)
            .map(|c| c.border_width())
            .unwrap_or(0);

        if i < m.nmaster as u32 {
            // ── master client ─────────────────────────────────────────────
            let h =
                (m.work_rect.h - master_y_offset as i32) / (min(n, m.nmaster as u32) - i) as i32;

            // Two-client special-case: no animation to avoid visual glitch.
            let frames = if n == 2 { 0 } else { framecount };

            animate_client(
                ctx,
                win,
                &Rect {
                    x: m.work_rect.x,
                    y: m.work_rect.y + master_y_offset as i32,
                    w: mw - BORDER_MULTIPLIER * border_width,
                    h: h - BORDER_MULTIPLIER * border_width,
                },
                frames,
                0,
            );

            // When there is exactly one master, let its actual rendered width
            // dictate the stack column's x-offset (respects size hints).
            if m.nmaster == 1 && n > 1 {
                if let Some(c) = ctx.g.clients.get(&win) {
                    mw = c.geo.w + c.border_width * 2;
                }
            }

            if let Some(c) = ctx.g.clients.get(&win) {
                if master_y_offset as i32 + c.total_height() < m.work_rect.h {
                    master_y_offset += c.total_height() as u32;
                }
            }
        } else {
            // ── stack client ──────────────────────────────────────────────
            let h = (m.work_rect.h - stack_y_offset as i32) / (n - i) as i32;

            animate_client(
                ctx,
                win,
                &Rect {
                    x: m.work_rect.x + mw,
                    y: m.work_rect.y + stack_y_offset as i32,
                    w: m.work_rect.w - mw - BORDER_MULTIPLIER * border_width,
                    h: h - BORDER_MULTIPLIER * border_width,
                },
                framecount,
                0,
            );

            if let Some(c) = ctx.g.clients.get(&win) {
                if stack_y_offset as i32 + c.total_height() < m.work_rect.h {
                    stack_y_offset += c.total_height() as u32;
                }
            }
        }

        i += 1;
        current_window = next_tiled(ctx, current_window);
    }
}
