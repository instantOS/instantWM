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

pub fn three_column(ctx: &mut WmCtx<'_>, monitor: &mut Monitor) {
    let tiled_client_count =
        monitor.tiled_client_count(ctx.core_mut().globals_mut().clients.map()) as u32;

    if tiled_client_count == 0 {
        return;
    }

    // Collect all tiled clients
    let selected_tags = monitor.selected_tags();
    let tiled_clients: Vec<_> = monitor
        .clients
        .iter()
        .filter_map(|&win| {
            let c = ctx.core().client(win)?;
            if !c.is_tiled(selected_tags) {
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
    let master_area_width = (monitor.mfact * monitor.work_rect.w as f32) as i32;
    let side_column_width = (monitor.work_rect.w - master_area_width) / 2;

    // Place master client
    let master_bw = ctx
        .client(first_win)
        .map(|c| BORDER_MULTIPLIER * c.border_width())
        .unwrap_or(0);

    let master_x = if tiled_client_count < 3 {
        monitor.work_rect.x
    } else {
        monitor.work_rect.x + side_column_width
    };

    let master_window_width = if tiled_client_count == 1 {
        monitor.work_rect.w - master_bw
    } else {
        master_area_width - master_bw
    };

    resize(
        ctx,
        first_win,
        &Rect {
            x: master_x,
            y: monitor.work_rect.y,
            w: master_window_width,
            h: monitor.work_rect.h - master_bw,
        },
        false,
    );

    if tiled_client_count <= 1 {
        return;
    }

    // Distribute stack clients
    let stack_client_count = tiled_client_count - 1;
    let stack_column_width = if stack_client_count == 1 {
        monitor.work_rect.w - master_area_width
    } else {
        side_column_width
    };

    let right_column_client_count = stack_client_count.div_ceil(2);
    let left_column_client_count = stack_client_count / 2;

    let bar_height = ctx.core_mut().globals_mut().cfg.bar_height;

    // Right column (even indices in stack: 0, 2, 4...)
    if right_column_client_count > 0 {
        let raw_window_height = monitor.work_rect.h / right_column_client_count as i32;
        let per_window_height = if raw_window_height < bar_height {
            monitor.work_rect.h
        } else {
            raw_window_height
        };

        let column_x = if tiled_client_count < 3 {
            monitor.work_rect.x + master_area_width
        } else {
            monitor.work_rect.x + master_area_width + side_column_width
        };
        let mut next_window_y = monitor.work_rect.y;

        // Stack clients start at index 1
        for stack_position in 0..right_column_client_count {
            let stack_client_index = (1 + stack_position * 2) as usize;
            if stack_client_index >= tiled_clients.len() {
                break;
            }
            let win = tiled_clients[stack_client_index];

            let border_width = ctx
                .client(win)
                .map(|c| c.border_width())
                .unwrap_or(0);

            let window_height = if stack_position + 1 == right_column_client_count {
                monitor.work_rect.y + monitor.work_rect.h
                    - next_window_y
                    - BORDER_MULTIPLIER * border_width
            } else {
                per_window_height - BORDER_MULTIPLIER * border_width
            };

            resize(
                ctx,
                win,
                &Rect {
                    x: column_x,
                    y: next_window_y,
                    w: stack_column_width - BORDER_MULTIPLIER * border_width,
                    h: window_height,
                },
                false,
            );

            if per_window_height != monitor.work_rect.h
                && let Some(c) = ctx.core().client(win)
            {
                next_window_y = c.geo.y + c.total_height();
            }
        }
    }

    // Left column (odd indices in stack: 1, 3, 5...)
    if left_column_client_count > 0 {
        let raw_window_height = monitor.work_rect.h / left_column_client_count as i32;
        let per_window_height = if raw_window_height < bar_height {
            monitor.work_rect.h
        } else {
            raw_window_height
        };

        let column_x = monitor.work_rect.x;
        let mut next_window_y = monitor.work_rect.y;

        // Stack clients start at index 2 (second odd)
        for stack_position in 0..left_column_client_count {
            let stack_client_index = (2 + stack_position * 2) as usize;
            if stack_client_index >= tiled_clients.len() {
                break;
            }
            let win = tiled_clients[stack_client_index];

            let border_width = ctx
                .client(win)
                .map(|c| c.border_width())
                .unwrap_or(0);

            let window_height = if stack_position + 1 == left_column_client_count {
                monitor.work_rect.y + monitor.work_rect.h
                    - next_window_y
                    - BORDER_MULTIPLIER * border_width
            } else {
                per_window_height - BORDER_MULTIPLIER * border_width
            };

            resize(
                ctx,
                win,
                &Rect {
                    x: column_x,
                    y: next_window_y,
                    w: stack_column_width - BORDER_MULTIPLIER * border_width,
                    h: window_height,
                },
                false,
            );

            if per_window_height != monitor.work_rect.h
                && let Some(c) = ctx.core().client(win)
            {
                next_window_y = c.geo.y + c.total_height();
            }
        }
    }
}
