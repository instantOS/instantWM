#![allow(dead_code)]
//! Three-column layout.
//!
//! Arranges tiled clients into three vertical columns:
//!
//! ```text
//! ┌──────────┬──────────────┬──────────┐
//! │          │              │          │
//! │  left   │   master     │  right   │
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

use crate::client::resize;
use crate::constants::animation::BORDER_MULTIPLIER;
use crate::contexts::WmCtx;
use crate::types::{Monitor, Rect};

pub fn three_column(ctx: &mut WmCtx<'_>, m: &mut Monitor) {
    let n = m.tiled_client_count(&ctx.g.clients) as u32;

    if n == 0 {
        return;
    }

    // Collect all tiled clients
    let selected_tags = m.selected_tags();
    let tiled_clients: Vec<_> = m
        .clients
        .iter()
        .filter_map(|&win| {
            let c = ctx.g.clients.get(&win)?;
            if c.isfloating || !c.is_visible_on_tags(selected_tags) || c.is_hidden {
                return None;
            }
            Some(win)
        })
        .collect();

    if tiled_clients.is_empty() {
        return;
    }

    let first_win = tiled_clients[0];

    // Column geometry
    let mw = (m.mfact * m.work_rect.w as f32) as i32;
    let sw = (m.work_rect.w - mw) / 2;

    // Place master client
    let master_bw = ctx
        .g
        .clients
        .get(&first_win)
        .map(|c| BORDER_MULTIPLIER * c.border_width())
        .unwrap_or(0);

    let master_x = if n < 3 {
        m.work_rect.x
    } else {
        m.work_rect.x + sw
    };

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

    // Distribute stack clients
    let stack_n = n - 1;
    let col_w = if stack_n == 1 { m.work_rect.w - mw } else { sw };

    let right_n = stack_n.div_ceil(2);
    let left_n = stack_n / 2;

    let bar_height = ctx.g.cfg.bar_height;

    // Right column (even indices in stack: 0, 2, 4...)
    if right_n > 0 {
        let raw_h = m.work_rect.h / right_n as i32;
        let cell_h = if raw_h < bar_height {
            m.work_rect.h
        } else {
            raw_h
        };

        let col_x = m.work_rect.x + mw + sw;
        let mut y = m.work_rect.y;

        // Stack clients start at index 1
        for i in 0..right_n {
            let stack_idx = 1 + i * 2;
            let stack_idx = stack_idx as usize;
            if stack_idx >= tiled_clients.len() {
                break;
            }
            let win = tiled_clients[stack_idx];

            let border_width = ctx
                .g
                .clients
                .get(&win)
                .map(|c| c.border_width())
                .unwrap_or(0);

            let h = if i + 1 == right_n {
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
                    y = c.geo.y + c.total_height();
                }
            }
        }
    }

    // Left column (odd indices in stack: 1, 3, 5...)
    if left_n > 0 {
        let raw_h = m.work_rect.h / left_n as i32;
        let cell_h = if raw_h < bar_height {
            m.work_rect.h
        } else {
            raw_h
        };

        let col_x = m.work_rect.x;
        let mut y = m.work_rect.y;

        // Stack clients start at index 2 (second odd)
        for i in 0..left_n {
            let stack_idx = 2 + i * 2;
            let stack_idx = stack_idx as usize;
            if stack_idx >= tiled_clients.len() {
                break;
            }
            let win = tiled_clients[stack_idx];

            let border_width = ctx
                .g
                .clients
                .get(&win)
                .map(|c| c.border_width())
                .unwrap_or(0);

            let h = if i + 1 == left_n {
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
                    y = c.geo.y + c.total_height();
                }
            }
        }
    }
}
