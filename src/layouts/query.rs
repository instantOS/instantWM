//! Layout query helpers — stateless reads over the global window/monitor state.
//!
//! These functions answer questions like "how many tiled clients are on the
//! selected monitor?" or "what is the active layout?" without mutating
//! any state.  They are kept separate from the arrange/z-order machinery so
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
