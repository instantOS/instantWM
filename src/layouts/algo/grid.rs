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

use crate::animation::{MoveResizeMode, move_resize_client};
use crate::constants::animation::{
    BORDER_MULTIPLIER, DEFAULT_FRAME_COUNT, FAST_ANIM_THRESHOLD, FAST_FRAME_COUNT,
};
use crate::contexts::WmCtx;
use crate::layouts::query::framecount_for_layout;
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

    let framecount = framecount_for_layout(ctx.core().globals(), FAST_ANIM_THRESHOLD, 3, 6);

    // ── count tiled clients ───────────────────────────────────────────────
    let n = m.tiled_client_count(ctx.core_mut().globals_mut().clients.map()) as i32;

    // A single tiled client fills the whole work area — nothing to grid.
    if n <= 1 {
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

    // ── place each tiled client ─────────────────────────────────────────
    let selected_tags = m.selected_tags();
    let mut i: i32 = 0;

    for &win in &m.clients {
        let Some(c) = ctx.core().client(win) else {
            continue;
        };

        // Skip non-tiled, hidden, or invisible clients
        if !c.is_tiled(selected_tags) {
            continue;
        }

        let border_width = c.border_width();

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

        move_resize_client(
            ctx,
            win,
            &Rect {
                x: cell_x,
                y: cell_y,
                w: cell_width - BORDER_MULTIPLIER * border_width + extra_w,
                h: cell_height - BORDER_MULTIPLIER * border_width + extra_h,
            },
            MoveResizeMode::AnimateTo,
            framecount,
        );

        i += 1;
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
    let n = m.tiled_client_count(ctx.core_mut().globals_mut().clients.map()) as u32;

    if n == 0 {
        return;
    }

    let framecount = framecount_for_layout(
        ctx.core().globals(),
        FAST_ANIM_THRESHOLD,
        FAST_FRAME_COUNT,
        DEFAULT_FRAME_COUNT,
    );

    // Number of columns = ceil(sqrt(n)).
    let cols = ((n as f32).sqrt() + 0.5) as u32;

    // Collect tiled clients first
    let tiled = m.collect_tiled(ctx.core().globals().clients.map());

    for col in 0..cols {
        // Clients in this column: last column absorbs any remainder.
        let cn = if col == cols - 1 {
            n - (n / cols) * (cols - 1)
        } else {
            n / cols
        };
        let cell_width = m.work_rect.w / cols as i32;

        // Start index for this column
        let start_idx = (col * (n / cols)) as usize;

        for row in 0..cn {
            let idx = start_idx + row as usize;
            if idx >= tiled.len() {
                break;
            }
            let win = tiled[idx].win;
            let border_width = tiled[idx].border_width;

            let cell_height = m.work_rect.h / cn as i32;
            let cell_x = m.work_rect.x + col as i32 * cell_width;
            let cell_y = m.work_rect.y + row as i32 * cell_height;

            // Last column gets any remaining width from rounding.
            let extra_w = if col == cols - 1 {
                m.work_rect.w - cols as i32 * cell_width + cell_width
            } else {
                0
            };

            move_resize_client(
                ctx,
                win,
                &Rect {
                    x: cell_x,
                    y: cell_y,
                    w: cell_width - BORDER_MULTIPLIER * border_width + extra_w,
                    h: cell_height - BORDER_MULTIPLIER * border_width,
                },
                MoveResizeMode::AnimateTo,
                framecount,
            );
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
