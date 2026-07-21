//! Stateless policy helpers shared by layout presentations and the tree manager.

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
