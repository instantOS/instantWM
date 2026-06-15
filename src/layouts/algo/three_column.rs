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

use std::collections::HashMap;

use crate::config::config_toml::LayoutConfig;
use crate::geometry::MoveResizeOptions;
use crate::layouts::LayoutKind;
use crate::layouts::placement::LayoutPlacement;
use crate::layouts::LayoutOutput;
use crate::types::client::Client;
use crate::types::{Monitor, Rect, WindowId};

pub fn three_column(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
    layout_cfg: &LayoutConfig,
    _animated: bool,
) -> Vec<LayoutOutput> {
    let tiled_client_count = monitor.tiled_client_count(clients) as u32;

    if tiled_client_count == 0 {
        return vec![];
    }

    let tiled_clients = monitor.collect_tiled(clients);

    if tiled_clients.is_empty() {
        return vec![];
    }

    let placement = LayoutPlacement::new(
        layout_cfg,
        monitor,
        LayoutKind::Tile,
        tiled_client_count,
    );
    let work_rect = placement.work_rect();
    let first_client = &tiled_clients[0];

    let mut result = Vec::new();

    // Column geometry
    let master_area_width = (monitor.mfact * work_rect.w as f32) as i32;
    let side_column_width = (work_rect.w - master_area_width) / 2;

    let master_x = if tiled_client_count < 3 {
        work_rect.x
    } else {
        work_rect.x + side_column_width
    };

    let master_window_width = if tiled_client_count == 1 {
        work_rect.w
    } else {
        master_area_width
    };

    let master_slot = Rect {
        x: master_x,
        y: work_rect.y,
        w: master_window_width,
        h: work_rect.h,
    };

    result.push(LayoutOutput {
        win: first_client.win,
        rect: placement.client_rect(master_slot, first_client.border_width),
        options: MoveResizeOptions::hinted_immediate(false),
    });

    if tiled_client_count <= 1 {
        return result;
    }

    // Distribute stack clients
    let stack_client_count = tiled_client_count - 1;
    let stack_column_width = if stack_client_count == 1 {
        work_rect.w - master_area_width
    } else {
        side_column_width
    };

    let right_column_client_count = stack_client_count.div_ceil(2);
    let left_column_client_count = stack_client_count / 2;

    let bar_height = monitor.bar_height;

    // Right column (even indices in stack: 0, 2, 4...)
    if right_column_client_count > 0 {
        let raw_window_height = work_rect.h / right_column_client_count as i32;
        let per_window_height = if raw_window_height < bar_height {
            work_rect.h
        } else {
            raw_window_height
        };

        let column_x = if tiled_client_count < 3 {
            work_rect.x + master_area_width
        } else {
            work_rect.x + master_area_width + side_column_width
        };
        let mut next_window_y = work_rect.y;

        // Stack clients start at index 1
        for stack_position in 0..right_column_client_count {
            let stack_client_index = (1 + stack_position * 2) as usize;
            if stack_client_index >= tiled_clients.len() {
                break;
            }
            let client = &tiled_clients[stack_client_index];

            let window_height = if stack_position + 1 == right_column_client_count {
                work_rect.y + work_rect.h - next_window_y
            } else {
                per_window_height
            };

            let slot = Rect {
                x: column_x,
                y: next_window_y,
                w: stack_column_width,
                h: window_height,
            };

            result.push(LayoutOutput {
                win: client.win,
                rect: placement.client_rect(slot, client.border_width),
                options: MoveResizeOptions::hinted_immediate(false),
            });

            if per_window_height != work_rect.h {
                next_window_y += slot.h;
            }
        }
    }

    // Left column (odd indices in stack: 1, 3, 5...)
    if left_column_client_count > 0 {
        let raw_window_height = work_rect.h / left_column_client_count as i32;
        let per_window_height = if raw_window_height < bar_height {
            work_rect.h
        } else {
            raw_window_height
        };

        let column_x = work_rect.x;
        let mut next_window_y = work_rect.y;

        // Stack clients start at index 2 (second odd)
        for stack_position in 0..left_column_client_count {
            let stack_client_index = (2 + stack_position * 2) as usize;
            if stack_client_index >= tiled_clients.len() {
                break;
            }
            let client = &tiled_clients[stack_client_index];

            let window_height = if stack_position + 1 == left_column_client_count {
                work_rect.y + work_rect.h - next_window_y
            } else {
                per_window_height
            };

            let slot = Rect {
                x: column_x,
                y: next_window_y,
                w: stack_column_width,
                h: window_height,
            };

            result.push(LayoutOutput {
                win: client.win,
                rect: placement.client_rect(slot, client.border_width),
                options: MoveResizeOptions::hinted_immediate(false),
            });

            if per_window_height != work_rect.h {
                next_window_y += slot.h;
            }
        }
    }

    result
}
