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

use crate::config::config_toml::LayoutConfig;
use crate::constants::animation::{DEFAULT_FRAME_COUNT, FAST_ANIM_THRESHOLD, FAST_FRAME_COUNT};
use crate::geometry::MoveResizeOptions;
use crate::layouts::LayoutKind;
use crate::layouts::LayoutOutput;
use crate::layouts::placement::LayoutPlacement;
use crate::layouts::query::framecount_for_layout;
use crate::types::client::Client;
use crate::types::{Monitor, Rect, WindowId};
use std::cmp::min;
use std::collections::HashMap;

fn effective_nmaster(monitor: &Monitor, tiled_client_count: u32) -> u32 {
    min(monitor.master_count.max(0) as u32, tiled_client_count)
}

fn master_width(work_width: i32, monitor: &Monitor, tiled_client_count: u32, nmaster: u32) -> i32 {
    if tiled_client_count > nmaster {
        if nmaster > 0 {
            (monitor.master_factor * work_width as f32) as i32
        } else {
            0
        }
    } else {
        work_width
    }
}

pub fn tile(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
    layout_cfg: &LayoutConfig,
    animated: bool,
) -> Vec<LayoutOutput> {
    let tiled_clients = monitor.collect_tiled(clients);
    let tiled_client_count = tiled_clients.len() as u32;

    if tiled_client_count == 0 {
        return vec![];
    }

    let framecount = framecount_for_layout(
        animated,
        tiled_client_count as usize,
        FAST_ANIM_THRESHOLD,
        FAST_FRAME_COUNT,
        DEFAULT_FRAME_COUNT,
    );

    let placement = LayoutPlacement::new(layout_cfg, monitor, LayoutKind::Tile, tiled_client_count);
    let work_rect = placement.work_rect();
    let nmaster = effective_nmaster(monitor, tiled_client_count);
    let master_area_width = master_width(work_rect.w, monitor, tiled_client_count, nmaster);

    let mut master_y_offset: i32 = 0;
    let mut stack_y_offset: i32 = 0;

    let mut result = Vec::new();

    for (index, client) in tiled_clients.iter().enumerate() {
        if (index as u32) < nmaster {
            let master_window_height =
                (work_rect.h - master_y_offset) / (nmaster - index as u32) as i32;

            let animation_frames = if tiled_client_count == 2 {
                0
            } else {
                framecount
            };

            let slot = Rect {
                x: work_rect.x,
                y: work_rect.y + master_y_offset,
                w: master_area_width,
                h: master_window_height,
            };

            result.push(LayoutOutput {
                win: client.win,
                rect: placement.client_rect(slot, client.border_width),
                options: MoveResizeOptions::animate_to(animation_frames),
            });

            if master_y_offset + slot.h < work_rect.h {
                master_y_offset += slot.h;
            }
        } else {
            let stack_window_height =
                (work_rect.h - stack_y_offset) / (tiled_client_count - index as u32) as i32;

            let animation_frames = if tiled_client_count == 2 {
                0
            } else {
                framecount
            };

            let slot = Rect {
                x: work_rect.x + master_area_width,
                y: work_rect.y + stack_y_offset,
                w: work_rect.w - master_area_width,
                h: stack_window_height,
            };

            result.push(LayoutOutput {
                win: client.win,
                rect: placement.client_rect(slot, client.border_width),
                options: MoveResizeOptions::animate_to(animation_frames),
            });

            if stack_y_offset + slot.h < work_rect.h {
                stack_y_offset += slot.h;
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::{effective_nmaster, master_width};
    use crate::types::{Monitor, Rect};

    #[test]
    fn master_width_respects_mfact_when_stack_exists() {
        let mut monitor = Monitor::default();
        monitor.available_rect = Rect::new(0, 0, 1000, 800);
        monitor.master_factor = 0.7;
        monitor.master_count = 1;

        assert_eq!(
            master_width(
                monitor.work_rect().w,
                &monitor,
                2,
                effective_nmaster(&monitor, 2)
            ),
            700
        );
    }

    #[test]
    fn master_width_uses_full_width_when_everything_is_in_master() {
        let mut monitor = Monitor::default();
        monitor.available_rect = Rect::new(0, 0, 1000, 800);
        monitor.master_factor = 0.2;
        monitor.master_count = 2;

        assert_eq!(
            master_width(
                monitor.work_rect().w,
                &monitor,
                1,
                effective_nmaster(&monitor, 1)
            ),
            1000
        );
    }

    #[test]
    fn effective_nmaster_does_not_mutate_configured_nmaster() {
        let mut monitor = Monitor::default();
        monitor.master_count = 4;

        assert_eq!(effective_nmaster(&monitor, 2), 2);
        assert_eq!(monitor.master_count, 4);
    }

    #[test]
    fn negative_nmaster_behaves_like_zero_master_clients() {
        let mut monitor = Monitor::default();
        monitor.available_rect = Rect::new(0, 0, 1000, 800);
        monitor.master_count = -1;

        let nmaster = effective_nmaster(&monitor, 3);

        assert_eq!(nmaster, 0);
        assert_eq!(master_width(monitor.work_rect().w, &monitor, 3, nmaster), 0);
    }

    #[test]
    fn test_tile_layout_calculation() {
        use super::tile;
        use crate::config::config_toml::LayoutConfig;
        use crate::types::{Client, Rect, TagMask, WindowId};
        use std::collections::HashMap;

        let mut monitor = Monitor::default();
        monitor.available_rect = Rect::new(0, 0, 1000, 800);
        monitor.monitor_rect = Rect::new(0, 0, 1000, 800);
        monitor.master_factor = 0.5;
        monitor.master_count = 1;
        monitor.set_selected_tags(TagMask::single(1).unwrap());
        monitor.clients = vec![WindowId(1), WindowId(2)];

        let mut clients = HashMap::new();

        let mut c1 = Client::default();
        c1.win = WindowId(1);
        c1.border_width = 0;
        c1.set_tag_mask(TagMask::single(1).unwrap());
        clients.insert(WindowId(1), c1);

        let mut c2 = Client::default();
        c2.win = WindowId(2);
        c2.border_width = 0;
        c2.set_tag_mask(TagMask::single(1).unwrap());
        clients.insert(WindowId(2), c2);

        let cfg = LayoutConfig::default();
        let outputs = tile(&monitor, &clients, &cfg, false);
        assert_eq!(outputs.len(), 2);

        // Client 1 (master): x=0, y=0, w=500, h=800
        assert_eq!(outputs[0].win, WindowId(1));
        assert_eq!(outputs[0].rect, Rect::new(0, 0, 500, 800));

        // Client 2 (stack): x=500, y=0, w=500, h=800
        assert_eq!(outputs[1].win, WindowId(2));
        assert_eq!(outputs[1].rect, Rect::new(500, 0, 500, 800));
    }
}
