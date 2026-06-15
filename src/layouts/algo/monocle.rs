//! Monocle layout — every tiled client occupies the full work area.
//!
//! ```text
//! ┌─────────────────────────────┐
//! │                             │
//! │   client[0]  (on top)       │
//! │                             │
//! └─────────────────────────────┘
//! ```
//!
//! All tiled clients are resized to fill `work_rect` exactly.  Only the
//! selected client is raised to the top of the stack, so cycling through
//! clients feels like flipping through full-screen cards.
//!
//! The selected window is animated with the normal frame-count; every other
//! window is snapped into place instantly (0 frames) to avoid mid-air ghost
//! windows appearing during the animation.

use std::collections::HashMap;

use crate::constants::animation::{BORDER_MULTIPLIER, DEFAULT_FRAME_COUNT};
use crate::geometry::MoveResizeOptions;
use crate::layouts::LayoutOutput;
use crate::types::client::Client;
use crate::types::{Monitor, Rect, WindowId};

pub fn monocle(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
    animated: bool,
) -> Vec<LayoutOutput> {
    let selected_window = monitor.sel;
    let selected_tags = monitor.selected_tags();

    let mut result = Vec::new();

    for &win in &monitor.clients {
        let Some(c) = clients.get(&win) else {
            continue;
        };

        if !c.is_tiled(selected_tags) {
            continue;
        }

        let border_width = c.border_width;

        let frames = if animated && Some(win) == selected_window {
            DEFAULT_FRAME_COUNT
        } else {
            0
        };

        result.push(LayoutOutput {
            win,
            rect: Rect {
                x: monitor.work_rect.x,
                y: monitor.work_rect.y,
                w: monitor.work_rect.w - BORDER_MULTIPLIER * border_width,
                h: monitor.work_rect.h - BORDER_MULTIPLIER * border_width,
            },
            options: MoveResizeOptions::animate_to(frames),
        });
    }

    result
}
