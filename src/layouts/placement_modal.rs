//! Pure target selection for the keyboard tree-placement interaction.
//!
//! Backend effects and authoritative mode transitions remain in `manager`;
//! this module owns the navigation policy and can be tested without a WM.

use crate::layouts::tree::{PlacementTarget, Side};
use crate::types::Point;

pub(super) fn nearest_target(targets: &[PlacementTarget], point: Point) -> usize {
    targets
        .iter()
        .enumerate()
        .min_by_key(|(_, target)| {
            let dx = i64::from(target.position.x - point.x);
            let dy = i64::from(target.position.y - point.y);
            dx * dx + dy * dy
        })
        .map_or(0, |(index, _)| index)
}

pub(super) fn directional_target(
    targets: &[PlacementTarget],
    selected: usize,
    side: Side,
) -> Option<usize> {
    let current = targets.get(selected)?.position;
    targets
        .iter()
        .enumerate()
        .filter_map(|(index, target)| {
            if index == selected {
                return None;
            }
            let dx = target.position.x - current.x;
            let dy = target.position.y - current.y;
            let primary = match side {
                Side::Left => -dx,
                Side::Right => dx,
                Side::Top => -dy,
                Side::Bottom => dy,
            };
            if primary <= 0 {
                return None;
            }
            let cross = match side {
                Side::Left | Side::Right => dy.abs(),
                Side::Top | Side::Bottom => dx.abs(),
            };
            let score = i64::from(primary)
                + i64::from(cross) * 2
                + i64::from(cross) * i64::from(cross) / i64::from(primary + 1);
            Some((index, score))
        })
        .min_by_key(|(index, score)| (*score, *index))
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::{directional_target, nearest_target};
    use crate::layouts::tree::{PlacementTarget, Side};
    use crate::types::{Point, WindowId};

    fn target(window: u32, x: i32, y: i32) -> PlacementTarget {
        PlacementTarget {
            target: WindowId(window),
            side: None,
            candidate_index: 0,
            position: Point::new(x, y),
        }
    }

    #[test]
    fn nearest_target_is_stable_for_equal_distances() {
        let targets = [target(1, -10, 0), target(2, 10, 0)];
        assert_eq!(nearest_target(&targets, Point::new(0, 0)), 0);
    }

    #[test]
    fn directional_navigation_rejects_targets_behind_the_requested_side() {
        let targets = [target(1, 0, 0), target(2, 100, 0), target(3, -100, 0)];
        assert_eq!(directional_target(&targets, 0, Side::Right), Some(1));
        assert_eq!(directional_target(&targets, 0, Side::Left), Some(2));
        assert_eq!(directional_target(&targets, 0, Side::Top), None);
    }

    #[test]
    fn directional_navigation_prefers_alignment_over_a_large_cross_offset() {
        let targets = [target(1, 0, 0), target(2, 80, 100), target(3, 100, 5)];
        assert_eq!(directional_target(&targets, 0, Side::Right), Some(2));
    }
}
