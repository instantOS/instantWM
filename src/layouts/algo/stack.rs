//! Stacking layout algorithms: deck, bottom_stack, and bstackhoriz.

use std::collections::HashMap;

use crate::constants::animation::{BORDER_MULTIPLIER, DEFAULT_FRAME_COUNT, FAST_FRAME_COUNT};
use crate::geometry::MoveResizeOptions;
use crate::layouts::query::framecount_for_layout;
use crate::layouts::LayoutOutput;
use crate::types::client::Client;
use crate::types::{Monitor, Rect, WindowId};
use std::cmp::min;

// ── deck ─────────────────────────────────────────────────────────────────────

pub fn deck(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
    _animated: bool,
) -> Vec<LayoutOutput> {
    let tiled_client_count = monitor.tiled_client_count(clients) as u32;

    if tiled_client_count == 0 {
        return vec![];
    }

    let nmaster = monitor.nmaster.max(0) as u32;
    let master_area_width: i32 = if tiled_client_count > nmaster {
        if nmaster > 0 {
            (monitor.mfact * monitor.work_rect.w as f32) as i32
        } else {
            0
        }
    } else {
        monitor.work_rect.w
    };

    // Collect tiled clients first
    let tiled_clients = monitor.collect_tiled(clients);

    let mut result = Vec::new();
    let mut master_column_offset: i32 = 0;

    for (index, client) in tiled_clients.iter().enumerate() {
        if (index as u32) < nmaster {
            let master_window_height = (monitor.work_rect.h - master_column_offset)
                / (min(tiled_client_count, nmaster) - index as u32) as i32;

            result.push(LayoutOutput {
                win: client.win,
                rect: Rect {
                    x: monitor.work_rect.x,
                    y: monitor.work_rect.y + master_column_offset,
                    w: master_area_width - BORDER_MULTIPLIER * client.border_width,
                    h: master_window_height - BORDER_MULTIPLIER * client.border_width,
                },
                options: MoveResizeOptions::hinted_immediate(false),
            });

            master_column_offset += master_window_height;
        } else {
            result.push(LayoutOutput {
                win: client.win,
                rect: Rect {
                    x: monitor.work_rect.x + master_area_width,
                    y: monitor.work_rect.y,
                    w: monitor.work_rect.w
                        - master_area_width
                        - BORDER_MULTIPLIER * client.border_width,
                    h: monitor.work_rect.h - BORDER_MULTIPLIER * client.border_width,
                },
                options: MoveResizeOptions::hinted_immediate(false),
            });
        }
    }

    result
}

// ── bottom_stack ───────────────────────────────────────────────────────────────

pub fn bottom_stack(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
    animated: bool,
) -> Vec<LayoutOutput> {
    let tiled_client_count = monitor.tiled_client_count(clients) as u32;

    if tiled_client_count == 0 {
        return vec![];
    }

    let framecount = framecount_for_layout(
        animated,
        tiled_client_count as usize,
        4,
        FAST_FRAME_COUNT,
        DEFAULT_FRAME_COUNT,
    );

    let nmaster = monitor.nmaster.max(0) as u32;
    let (master_area_height, stack_window_width, stack_area_y) = if tiled_client_count > nmaster {
        let master_area_height = if nmaster > 0 {
            (monitor.mfact * monitor.work_rect.h as f32) as i32
        } else {
            0
        };
        let stack_window_width = monitor.work_rect.w / (tiled_client_count - nmaster) as i32;
        let stack_area_y = monitor.work_rect.y + master_area_height;
        (master_area_height, stack_window_width, stack_area_y)
    } else {
        (
            monitor.work_rect.h,
            monitor.work_rect.w,
            monitor.work_rect.y,
        )
    };

    let tiled_clients = monitor.collect_tiled(clients);

    let mut result = Vec::new();
    let mut master_row_offset: i32 = 0;
    let mut stack_window_x: i32 = monitor.work_rect.x;

    for (index, client) in tiled_clients.iter().enumerate() {
        if (index as u32) < nmaster {
            let master_window_width = (monitor.work_rect.w - master_row_offset)
                / (min(tiled_client_count, nmaster) - index as u32) as i32;

            result.push(LayoutOutput {
                win: client.win,
                rect: Rect {
                    x: monitor.work_rect.x + master_row_offset,
                    y: monitor.work_rect.y,
                    w: master_window_width - BORDER_MULTIPLIER * client.border_width,
                    h: master_area_height - BORDER_MULTIPLIER * client.border_width,
                },
                options: MoveResizeOptions::animate_to(framecount),
            });

            master_row_offset += master_window_width;
        } else {
            let stack_window_height = monitor.work_rect.h - master_area_height;

            result.push(LayoutOutput {
                win: client.win,
                rect: Rect {
                    x: stack_window_x,
                    y: stack_area_y,
                    w: stack_window_width - BORDER_MULTIPLIER * client.border_width,
                    h: stack_window_height - BORDER_MULTIPLIER * client.border_width,
                },
                options: MoveResizeOptions::animate_to(framecount),
            });

            if stack_window_width != monitor.work_rect.w {
                stack_window_x += stack_window_width;
            }
        }
    }

    result
}

// ── bstackhoriz ───────────────────────────────────────────────────────────────

pub fn bstackhoriz(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
    animated: bool,
) -> Vec<LayoutOutput> {
    let tiled_client_count = monitor.tiled_client_count(clients) as u32;

    if tiled_client_count == 0 {
        return vec![];
    }

    let framecount = framecount_for_layout(
        animated,
        tiled_client_count as usize,
        4,
        FAST_FRAME_COUNT,
        DEFAULT_FRAME_COUNT,
    );

    let nmaster = monitor.nmaster.max(0) as u32;
    let (master_area_height, stack_window_height, mut stack_window_y) =
        if tiled_client_count > nmaster {
            let master_area_height = if nmaster > 0 {
                (monitor.mfact * monitor.work_rect.h as f32) as i32
            } else {
                0
            };
            let stack_window_height =
                (monitor.work_rect.h - master_area_height) / (tiled_client_count - nmaster) as i32;
            let stack_window_y = monitor.work_rect.y + master_area_height;
            (master_area_height, stack_window_height, stack_window_y)
        } else {
            (
                monitor.work_rect.h,
                monitor.work_rect.h,
                monitor.work_rect.y,
            )
        };

    let tiled_clients = monitor.collect_tiled(clients);

    let mut result = Vec::new();
    let mut master_row_offset: i32 = 0;
    let stack_window_x: i32 = monitor.work_rect.x;

    for (index, client) in tiled_clients.iter().enumerate() {
        if (index as u32) < nmaster {
            let master_window_width = (monitor.work_rect.w - master_row_offset)
                / (min(tiled_client_count, nmaster) - index as u32) as i32;

            result.push(LayoutOutput {
                win: client.win,
                rect: Rect {
                    x: monitor.work_rect.x + master_row_offset,
                    y: monitor.work_rect.y,
                    w: master_window_width - BORDER_MULTIPLIER * client.border_width,
                    h: master_area_height - BORDER_MULTIPLIER * client.border_width,
                },
                options: MoveResizeOptions::animate_to(framecount),
            });

            master_row_offset += master_window_width;
        } else {
            result.push(LayoutOutput {
                win: client.win,
                rect: Rect {
                    x: stack_window_x,
                    y: stack_window_y,
                    w: monitor.work_rect.w - BORDER_MULTIPLIER * client.border_width,
                    h: stack_window_height - BORDER_MULTIPLIER * client.border_width,
                },
                options: MoveResizeOptions::animate_to(framecount),
            });

            if stack_window_height != monitor.work_rect.h {
                stack_window_y += stack_window_height;
            }
        }
    }

    result
}
