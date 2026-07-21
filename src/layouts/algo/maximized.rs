//! Maximized-stack presentation — every tiled client occupies the work area.
//!
//! This is deliberately a presentation of the persistent manual tree, not a
//! tree transformation. All tiled leaves receive identical geometry and the
//! normal z-order projection raises the focused tiled leaf. Floating clients
//! remain in the floating layer above the tiled stack.

use std::collections::HashMap;

use crate::config::config_toml::LayoutConfig;
use crate::constants::animation::DEFAULT_FRAME_COUNT;
use crate::geometry::MoveResizeOptions;
use crate::layouts::placement::LayoutPlacement;
use crate::layouts::{LayoutKind, LayoutOutput};
use crate::types::client::Client;
use crate::types::{Monitor, WindowId};

pub fn maximized(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
    layout_cfg: &LayoutConfig,
    animated: bool,
) -> Vec<LayoutOutput> {
    let selected_window = monitor.selected;
    let selected_tags = monitor.selected_tags();
    let tiled_client_count = monitor.tiled_client_count(clients) as u32;
    let placement = LayoutPlacement::new(
        layout_cfg,
        monitor,
        LayoutKind::Maximized,
        tiled_client_count,
    );
    let work_rect = placement.work_rect();

    monitor
        .clients
        .iter()
        .filter_map(|&win| {
            let client = clients.get(&win)?;
            if !client.is_tiled(selected_tags) {
                return None;
            }
            let frames = if animated && Some(win) == selected_window {
                DEFAULT_FRAME_COUNT
            } else {
                0
            };
            Some(LayoutOutput {
                win,
                rect: placement.client_rect(work_rect, client.border_width),
                options: MoveResizeOptions::animate_to(frames),
            })
        })
        .collect()
}
