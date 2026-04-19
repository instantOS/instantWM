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

use crate::constants::animation::BORDER_MULTIPLIER;
use crate::constants::animation::{DEFAULT_FRAME_COUNT, FAST_ANIM_THRESHOLD, FAST_FRAME_COUNT};
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;
use crate::layouts::query::framecount_for_layout;
use crate::types::{Monitor, Rect};
use std::cmp::min;

fn effective_nmaster(monitor: &Monitor, tiled_client_count: u32) -> u32 {
    min(monitor.nmaster.max(0) as u32, tiled_client_count)
}

fn master_width(monitor: &Monitor, tiled_client_count: u32, nmaster: u32) -> i32 {
    if tiled_client_count > nmaster {
        if nmaster > 0 {
            (monitor.mfact * monitor.work_rect.w as f32) as i32
        } else {
            0
        }
    } else {
        monitor.work_rect.w
    }
}

pub fn tile(ctx: &mut WmCtx<'_>, monitor: &mut Monitor) {
    let tiled_clients = monitor.collect_tiled(ctx.core().globals().clients.map());
    let tiled_client_count = tiled_clients.len() as u32;

    if tiled_client_count == 0 {
        return;
    }

    let framecount = framecount_for_layout(
        ctx.core().globals().behavior.animated,
        tiled_client_count as usize,
        FAST_ANIM_THRESHOLD,
        FAST_FRAME_COUNT,
        DEFAULT_FRAME_COUNT,
    );

    let nmaster = effective_nmaster(monitor, tiled_client_count);
    let master_area_width = master_width(monitor, tiled_client_count, nmaster);

    let mut master_y_offset: u32 = 0;
    let mut stack_y_offset: u32 = 0;

    for (index, client) in tiled_clients.iter().enumerate() {
        if (index as u32) < nmaster {
            let master_window_height =
                (monitor.work_rect.h - master_y_offset as i32) / (nmaster - index as u32) as i32;

            let animation_frames = if tiled_client_count == 2 {
                0
            } else {
                framecount
            };

            ctx.move_resize(
                client.win,
                Rect {
                    x: monitor.work_rect.x,
                    y: monitor.work_rect.y + master_y_offset as i32,
                    w: master_area_width - BORDER_MULTIPLIER * client.border_width,
                    h: master_window_height - BORDER_MULTIPLIER * client.border_width,
                },
                MoveResizeOptions::animate_to(animation_frames),
            );

            if let Some(c) = ctx.core().client(client.win)
                && master_y_offset as i32 + c.total_height() < monitor.work_rect.h
            {
                master_y_offset += c.total_height() as u32;
            }
        } else {
            let stack_window_height = (monitor.work_rect.h - stack_y_offset as i32)
                / (tiled_client_count - index as u32) as i32;

            let animation_frames = if tiled_client_count == 2 {
                0
            } else {
                framecount
            };

            ctx.move_resize(
                client.win,
                Rect {
                    x: monitor.work_rect.x + master_area_width,
                    y: monitor.work_rect.y + stack_y_offset as i32,
                    w: monitor.work_rect.w
                        - master_area_width
                        - BORDER_MULTIPLIER * client.border_width,
                    h: stack_window_height - BORDER_MULTIPLIER * client.border_width,
                },
                MoveResizeOptions::animate_to(animation_frames),
            );

            if let Some(c) = ctx.core().client(client.win)
                && stack_y_offset as i32 + c.total_height() < monitor.work_rect.h
            {
                stack_y_offset += c.total_height() as u32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{effective_nmaster, master_width};
    use crate::types::{Monitor, Rect};

    #[test]
    fn master_width_respects_mfact_when_stack_exists() {
        let mut monitor = Monitor::default();
        monitor.work_rect = Rect::new(0, 0, 1000, 800);
        monitor.mfact = 0.7;
        monitor.nmaster = 1;

        assert_eq!(
            master_width(&monitor, 2, effective_nmaster(&monitor, 2)),
            700
        );
    }

    #[test]
    fn master_width_uses_full_width_when_everything_is_in_master() {
        let mut monitor = Monitor::default();
        monitor.work_rect = Rect::new(0, 0, 1000, 800);
        monitor.mfact = 0.2;
        monitor.nmaster = 2;

        assert_eq!(
            master_width(&monitor, 1, effective_nmaster(&monitor, 1)),
            1000
        );
    }

    #[test]
    fn effective_nmaster_does_not_mutate_configured_nmaster() {
        let mut monitor = Monitor::default();
        monitor.nmaster = 4;

        assert_eq!(effective_nmaster(&monitor, 2), 2);
        assert_eq!(monitor.nmaster, 4);
    }

    #[test]
    fn negative_nmaster_behaves_like_zero_master_clients() {
        let mut monitor = Monitor::default();
        monitor.work_rect = Rect::new(0, 0, 1000, 800);
        monitor.nmaster = -1;

        let nmaster = effective_nmaster(&monitor, 3);

        assert_eq!(nmaster, 0);
        assert_eq!(master_width(&monitor, 3, nmaster), 0);
    }
}
