#![allow(dead_code)]
//! Grid layout algorithms.
//!
//! Three related layouts live here:
//!
//! - [`grid`]       — square-ish grid, falls back to tile on wide 2-client monitors
//! - [`horizgrid`]  — grid arranged in columns rather than rows (better for landscape)
//! - [`gaplessgrid`] — alias for [`grid`] (no gaps variant, behaviour identical)
//!
//! ## Visual — `grid` (6 clients, 3 cols × 2 rows)
//!
//! ```text
//! ┌──────┬──────┬──────┐
//! │  0   │  2   │  4   │
//! ├──────┼──────┼──────┤
//! │  1   │  3   │  5   │
//! └──────┴──────┴──────┘
//! ```
//!
//! ## Visual — `horizgrid` (6 clients, 3 cols × 2 rows)
//!
//! ```text
//! ┌──────┬──────┬──────┐
//! │  0   │  2   │  4   │
//! ├──────┼──────┼──────┤
//! │  1   │  3   │  5   │
//! └──────┴──────┴──────┘
//! ```
//!
//! The difference is in how clients are distributed when the count does not
//! divide evenly: `grid` pads the last *row*, while `horizgrid` pads the last
//! *column*.

use crate::animation::animate_client;
use crate::client::next_tiled;
use crate::contexts::WmCtx;
use crate::layouts::query::client_count;
use crate::types::{Monitor, Rect};

// ── grid ─────────────────────────────────────────────────────────────────────

/// Square-ish grid layout.
///
/// Falls back to [`super::tile::tile`] when there are ≤ 2 clients on a
/// landscape monitor, where a master/stack split looks better.
pub fn grid(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    // Two-client landscape shortcut: tile looks nicer.
    if m.clientcount <= 2 && m.monitor_rect.w > m.monitor_rect.h {
        super::tile::tile(ctx, m);
        return;
    }

    let framecount = {
        if ctx.g.animated && client_count(ctx.g) > 5 {
            3
        } else {
            6
        }
    };

    // ── count tiled clients ───────────────────────────────────────────────
    let mut n: i32 = 0;
    let mut c_win = next_tiled(m.clients);
    while c_win.is_some() {
        n += 1;
        c_win = c_win.and_then(|w| ctx.g.clients.get(&w)?.next);
    }

    if n == 0 {
        return;
    }

    // ── find the smallest integer r such that r² ≥ n ─────────────────────
    let mut rows: i32 = 0;
    for r in 0..=n / 2 {
        if r * r >= n {
            rows = r;
            break;
        }
    }

    // Reduce columns if the previous row can absorb the overflow.
    let cols: u32 = if rows > 0 && (rows - 1) * rows >= n {
        (rows - 1) as u32
    } else {
        rows as u32
    };

    let cell_height = m.work_rect.h / if rows > 0 { rows } else { 1 };
    let cell_width = m.work_rect.w / if cols > 0 { cols as i32 } else { 1 };

    // ── place each client ─────────────────────────────────────────────────
    let mut i: i32 = 0;
    let mut c_win = next_tiled(m.clients);
    while let Some(win) = c_win {
        let (border_width, next_client) = ctx
            .g
            .clients
            .get(&win)
            .map(|c| (c.border_width, c.next))
            .unwrap_or((0, None));

        let cell_x = m.work_rect.x + (i / rows) * cell_width;
        let cell_y = m.work_rect.y + (i % rows) * cell_height;

        // Last cell in a row or column gets the remaining pixels to avoid gaps
        // caused by integer division rounding.
        let extra_h = if (i + 1) % rows == 0 {
            m.work_rect.h - cell_height * rows
        } else {
            0
        };
        let extra_w = if i >= rows * (cols as i32 - 1) {
            m.work_rect.w - cell_width * cols as i32
        } else {
            0
        };

        animate_client(
            win,
            &Rect {
                x: cell_x,
                y: cell_y,
                w: cell_width - 2 * border_width + extra_w,
                h: cell_height - 2 * border_width + extra_h,
            },
            framecount,
            0,
        );

        i += 1;
        c_win = next_tiled(next_client);
    }
}

// ── horizgrid ─────────────────────────────────────────────────────────────────

/// Horizontal grid layout.
///
/// Arranges clients in equal-width columns, each column containing an equal
/// share of clients stacked vertically.  The last column absorbs any remainder
/// so there are never empty cells.
pub fn horizgrid(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
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

    let framecount = {
        if ctx.g.animated && client_count(ctx.g) > 5 {
            3
        } else {
            6
        }
    };

    // Number of columns = ceil(sqrt(n)).
    let cols = ((n as f32).sqrt() + 0.5) as u32;

    for col in 0..cols {
        // Clients in this column: last column absorbs any remainder.
        let cn = if col == cols - 1 {
            n - (n / cols) * (cols - 1)
        } else {
            n / cols
        };
        let cell_width = m.work_rect.w / cols as i32;

        // Walk forward to the first client belonging to this column.
        let mut c_win = next_tiled(m.clients);
        let mut skip = col * (n / cols);
        while skip > 0 {
            if let Some(win) = c_win {
                c_win = ctx.g.clients.get(&win).and_then(|c| next_tiled(c.next));
            } else {
                break;
            }
            skip -= 1;
        }

        for row in 0..cn {
            if let Some(win) = c_win {
                let (border_width, next_client) = ctx
                    .g
                    .clients
                    .get(&win)
                    .map(|c| (c.border_width, c.next))
                    .unwrap_or((0, None));

                let cell_height = m.work_rect.h / cn as i32;
                let cell_x = m.work_rect.x + col as i32 * cell_width;
                let cell_y = m.work_rect.y + row as i32 * cell_height;

                // Last column gets any remaining width from rounding.
                let extra_w = if col == cols - 1 {
                    m.work_rect.w - cols as i32 * cell_width + cell_width
                } else {
                    0
                };

                animate_client(
                    win,
                    &Rect {
                        x: cell_x,
                        y: cell_y,
                        w: cell_width - 2 * border_width + extra_w,
                        h: cell_height - 2 * border_width,
                    },
                    framecount,
                    0,
                );

                c_win = next_tiled(next_client);
            }
        }
    }
}

// ── gaplessgrid ───────────────────────────────────────────────────────────────

/// Gapless grid — identical to [`grid`].
///
/// Kept as a named entry point so that layout index tables that reference
/// `gaplessgrid` by name continue to compile without changes.
#[inline]
pub fn gaplessgrid(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    grid(ctx, m);
}
