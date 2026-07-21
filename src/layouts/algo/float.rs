//! Floating layout and snap-position geometry helpers.
//!
//! ## Overview
//!
//! In the floating layout every client is responsible for its own position.
//! The role of [`floating`] is therefore minimal: it applies any pending
//! *snap positions* (e.g. half-screen left, quarter top-right) to clients that
//! have one set, syncs the window z-order in the correct order, and raises the
//! selected client to the top.
//!
//! ## Snap positions
//!
//! A snap position is stored on each client as a [`SnapPosition`] enum
//! variant. When a floating client is dragged to a screen edge the WM sets
//! `client.snap_status`; [`floating`] reads it and computes the target
//! geometry via [`SnapPosition::target_rect`](crate::types::SnapPosition::target_rect).
//!
//! ```text
//! ┌──────────────────────────────────┐
//! │  TopLeft   │   Top   │ TopRight  │
//! ├────────────┼─────────┼───────────┤
//! │    Left    │ (none)  │   Right   │
//! ├────────────┼─────────┼───────────┤
//! │ BottomLeft │ Bottom  │BotRight   │
//! └──────────────────────────────────┘
//!                   ↑ Maximized fills the whole work area
//! ```
//!
//! use std::collections::HashMap;

use std::collections::HashMap;

use crate::geometry::MoveResizeOptions;
use crate::layouts::LayoutOutput;
use crate::types::client::Client;
use crate::types::{Monitor, SnapPosition, WindowId};

// ── floating ─────────────────────────────────────────────────────────────────

/// Floating layout arrange function.
///
/// Called by the [`Floating`](crate::layouts::PresentationMode::Floating) layout
/// — leaves clients at their self-managed positions but still needs snap
/// geometry enforced and the window stack sorted.
pub fn floating(
    monitor: &Monitor,
    clients: &HashMap<WindowId, Client>,
    _animated: bool,
) -> Vec<LayoutOutput> {
    let selected = monitor.selected_tags();

    let mut result: Vec<LayoutOutput> = Vec::new();

    for &win in &monitor.clients {
        let Some(c) = clients.get(&win) else {
            continue;
        };

        if c.is_visible(selected)
            && c.snap_status != SnapPosition::None
            && let Some(rect) = c
                .snap_status
                .target_rect(c.border_width, monitor.work_rect())
        {
            result.push(LayoutOutput {
                win,
                rect,
                options: MoveResizeOptions::hinted_immediate(false),
            });
        }
    }

    result
}
