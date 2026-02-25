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
use crate::client::{client_height, next_tiled};
use crate::contexts::WmCtx;
use crate::types::{Monitor, Rect};
use std::cmp::min;

pub fn tile(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    let framecount = {
        if ctx.g.animated && crate::layouts::query::client_count(ctx.g) > 5 {
            4
        } else {
            7
        }
    };

    // ── count tiled clients ───────────────────────────────────────────────
    let mut n: u32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        c_win = c_win.and_then(|w| ctx.g.clients.get(&w)?.next);
    }

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
    let mut ty: u32 = 0; // running y-offset inside stack  column
    let mut i: u32 = 0;
    let mut c_win = next_tiled(m.clients);

    while let Some(win) = c_win {
        let (border_width, next_client) = ctx
            .g
            .clients
            .get(&win)
            .map(|c| (c.border_width, c.next))
            .unwrap_or((0, None));

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
                    w: mw - 2 * border_width,
                    h: h - 2 * border_width,
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
                if master_y_offset as i32 + client_height(c) < m.work_rect.h {
                    master_y_offset += client_height(c) as u32;
                }
            }
        } else {
            // ── stack client ──────────────────────────────────────────────
            let h = (m.work_rect.h - ty as i32) / (n - i) as i32;

            animate_client(
                ctx,
                win,
                &Rect {
                    x: m.work_rect.x + mw,
                    y: m.work_rect.y + ty as i32,
                    w: m.work_rect.w - mw - 2 * border_width,
                    h: h - 2 * border_width,
                },
                framecount,
                0,
            );

            if let Some(c) = ctx.g.clients.get(&win) {
                if ty as i32 + client_height(c) < m.work_rect.h {
                    ty += client_height(c) as u32;
                }
            }
        }

        i += 1;
        c_win = next_tiled(next_client);
    }
}
