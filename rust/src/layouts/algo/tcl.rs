#![allow(dead_code)]
//! Three-column layout.
//!
//! Arranges tiled clients into three vertical columns:
//!
//! ```text
//! ┌──────────┬──────────────┬──────────┐
//! │          │              │          │
//! │  left    │   master     │  right   │
//! │ stack[1] │  client[0]   │ stack[0] │
//! │          │              │          │
//! ├──────────┤              ├──────────┤
//! │ stack[3] │              │ stack[2] │
//! └──────────┴──────────────┴──────────┘
//! ```
//!
//! - **Centre column** — the first tiled client (the master), width = `mfact * work_width`.
//! - **Right column**  — the even-indexed stack clients (0, 2, 4 …), each taking an
//!   equal vertical share.
//! - **Left column**   — the odd-indexed stack clients (1, 3, 5 …), each taking an
//!   equal vertical share.
//!
//! Degenerate cases:
//!
//! | clients | behaviour                                           |
//! |---------|-----------------------------------------------------|
//! | 0       | early return                                        |
//! | 1       | master fills the entire work area                   |
//! | 2       | master + one right column client (no left column)   |
//! | ≥ 3     | full three-column layout                            |
//!
//! The column width for the two side columns is `(work_width - mfact_width) / 2`.
//! When there is only one side column (2 clients), it takes the full remaining width.

use crate::client::{client_height, next_tiled, resize};
use crate::constants::animation::BORDER_MULTIPLIER;
use crate::contexts::WmCtx;
use crate::layouts::query::count_tiled_clients;
use crate::types::{Monitor, Rect};

pub fn three_column(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    // ── count tiled clients ───────────────────────────────────────────────
    let n = count_tiled_clients(ctx, m);

    if n == 0 {
        return;
    }

    let first_win = match m
        .clients
        .first()
        .copied()
        .and_then(|w| next_tiled(ctx, Some(w)))
    {
        Some(w) => w,
        None => return,
    };

    // ── column geometry ───────────────────────────────────────────────────
    // mw  — master (centre) column width
    // sw  — each side column width  (only used when n >= 3)
    let mw = (m.mfact * m.work_rect.w as f32) as i32;
    let sw = (m.work_rect.w - mw) / 2;

    // ── place the master client ───────────────────────────────────────────
    let master_bw = ctx
        .g
        .clients
        .get(&first_win)
        .map(|c| c.border_width())
        .map(|bw| BORDER_MULTIPLIER * bw)
        .unwrap_or(0);

    // When n < 3 the two side columns collapse into one, so the master starts
    // at work_rect.x; with three columns it is inset by sw.
    let master_x = if n < 3 {
        m.work_rect.x
    } else {
        m.work_rect.x + sw
    };

    // When n == 1 master fills full width; when n == 2 it fills mw.
    let master_w = if n == 1 {
        m.work_rect.w - master_bw
    } else {
        mw - master_bw
    };

    resize(
        ctx,
        first_win,
        &Rect {
            x: master_x,
            y: m.work_rect.y,
            w: master_w,
            h: m.work_rect.h - master_bw,
        },
        false,
    );

    if n <= 1 {
        return;
    }

    // ── distribute stack clients into two side columns ────────────────────
    // Remaining clients after the master.
    let stack_n = n - 1;

    // Side column width: when there is only one side column (n == 2) it
    // gets the full remaining width; otherwise each column is sw wide.
    let col_w = if stack_n == 1 { m.work_rect.w - mw } else { sw };

    // Right column  — stack clients at positions 0, 2, 4 … (even indices)
    // Left  column  — stack clients at positions 1, 3, 5 … (odd  indices)
    let right_n = stack_n.div_ceil(2); // ceil(stack_n / 2)
    let left_n = stack_n / 2; // floor(stack_n / 2)

    let bar_height = ctx.g.cfg.bar_height;

    // ── right column (even stack indices) ─────────────────────────────────
    if right_n > 0 {
        let raw_h = m.work_rect.h / right_n as i32;
        // Clamp: if the computed height is smaller than the bar height, just
        // give each client the full work height (only one will be visible).
        let cell_h = if raw_h < bar_height {
            m.work_rect.h
        } else {
            raw_h
        };

        let col_x = m.work_rect.x + mw + sw; // right of master, past the gap
        let mut y = m.work_rect.y;
        let mut c_win = next_tiled(ctx, Some(first_win));
        let mut idx: u32 = 0;

        while let Some(win) = c_win {
            if idx >= right_n {
                break;
            }

            let border_width = ctx
                .g
                .clients
                .get(&win)
                .map(|c| c.border_width())
                .unwrap_or(0);

            // Last client in the column absorbs rounding remainder.
            let h = if idx + 1 == right_n {
                m.work_rect.y + m.work_rect.h - y - BORDER_MULTIPLIER * border_width
            } else {
                cell_h - BORDER_MULTIPLIER * border_width
            };

            resize(
                ctx,
                win,
                &Rect {
                    x: col_x,
                    y,
                    w: col_w - BORDER_MULTIPLIER * border_width,
                    h,
                },
                false,
            );

            // Advance y only when cells are a fixed height.
            if cell_h != m.work_rect.h {
                if let Some(c) = ctx.g.clients.get(&win) {
                    y = c.geo.y + client_height(c);
                }
            }

            idx += 1;
            c_win = next_tiled(ctx, c_win);

            // Skip the next client — it belongs to the left column.
            if let Some(skip_win) = c_win {
                c_win = next_tiled(ctx, Some(skip_win));
            }
        }
    }

    // ── left column (odd stack indices) ───────────────────────────────────
    if left_n > 0 {
        let raw_h = m.work_rect.h / left_n as i32;
        let cell_h = if raw_h < bar_height {
            m.work_rect.h
        } else {
            raw_h
        };

        // The left column sits at work_rect.x when n >= 3.
        // When n == 2 there is no left column (left_n == 0), so this block is
        // never reached for that case.
        let col_x = m.work_rect.x;
        let mut y = m.work_rect.y;

        // Walk to the first odd-index stack client by skipping the first
        // right-column client.
        let first_right = next_tiled(ctx, Some(first_win));
        let first_left = first_right.and_then(|w| next_tiled(ctx, Some(w)));

        let mut c_win = first_left;
        let mut idx: u32 = 0;

        while let Some(win) = c_win {
            if idx >= left_n {
                break;
            }

            let border_width = ctx
                .g
                .clients
                .get(&win)
                .map(|c| c.border_width())
                .unwrap_or(0);

            let h = if idx + 1 == left_n {
                m.work_rect.y + m.work_rect.h - y - BORDER_MULTIPLIER * border_width
            } else {
                cell_h - BORDER_MULTIPLIER * border_width
            };

            resize(
                ctx,
                win,
                &Rect {
                    x: col_x,
                    y,
                    w: col_w - BORDER_MULTIPLIER * border_width,
                    h,
                },
                false,
            );

            if cell_h != m.work_rect.h {
                if let Some(c) = ctx.g.clients.get(&win) {
                    y = c.geo.y + client_height(c);
                }
            }

            idx += 1;
            c_win = next_tiled(ctx, c_win);

            // Skip the next client — it belongs to the right column.
            if let Some(skip_win) = c_win {
                c_win = next_tiled(ctx, Some(skip_win));
            }
        }
    }
}
