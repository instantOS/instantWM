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
/// TODO: does this mess with stacking policy? What is a sensible order?
/// Choosing just ANY client should be the last resort
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
    animated: bool,
    tiled_client_count: usize,
    threshold: usize,
    fast_frame_count: i32,
    slow_frame_count: i32,
) -> i32 {
    if animated && tiled_client_count > threshold {
        fast_frame_count
    } else {
        slow_frame_count
    }
}

#[cfg(test)]
mod tests {
    use super::framecount_for_layout;

    #[test]
    fn framecount_uses_given_client_count_only_when_animated() {
        assert_eq!(framecount_for_layout(true, 5, 4, 2, 6), 2);
        assert_eq!(framecount_for_layout(true, 4, 4, 2, 6), 6);
        assert_eq!(framecount_for_layout(false, 5, 4, 2, 6), 6);
    }
}
