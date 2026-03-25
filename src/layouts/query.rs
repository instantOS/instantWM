//! Layout query helpers — stateless reads over the global window/monitor state.
//!
//! These functions answer questions like "how many tiled clients are on the
//! selected monitor?" or "what is the active layout?" without mutating
//! any state.  They are kept separate from the arrange/restack machinery so
//! that both the algorithm modules and the manager can depend on them without
//! creating circular imports.

use crate::globals::Globals;
use crate::types::WindowId;

// ── per-monitor counts ────────────────────────────────────────────────────────

// ── visibility walk ───────────────────────────────────────────────────────────

/// Walk the client list starting at `start_win` and return the first
/// client that passes [`Client::is_visible`].
pub fn find_visible_client(g: &Globals, start_win: Option<WindowId>) -> Option<WindowId> {
    let selected = g.selected_monitor().selected_tags();

    let m = g.selected_monitor();
    let start_idx = start_win.and_then(|w| m.clients.iter().position(|&x| x == w));
    let iter_start = start_idx.map(|i| i + 1).unwrap_or(0);

    for i in iter_start..m.clients.len() {
        let win = m.clients[i];
        if let Some(c) = g.clients.get(&win)
            && c.is_visible(selected)
        {
            return Some(win);
        }
    }

    None
}

// ── layout query ──────────────────────────────────────────────────────────────

/// Returns `fast_frame_count` if animation is enabled and client count exceeds threshold,
/// otherwise returns `slow_frame_count`.
pub fn framecount_for_layout(
    g: &Globals,
    threshold: usize,
    fast_frame_count: i32,
    slow_frame_count: i32,
) -> i32 {
    if g.behavior.animated && g.selected_monitor().tiled_client_count(g.clients.map()) > threshold {
        fast_frame_count
    } else {
        slow_frame_count
    }
}
