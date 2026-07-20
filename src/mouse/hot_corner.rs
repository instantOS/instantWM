//! Pointer hot-corner recognition.
//!
//! Recognition is backend-neutral and uses two nested zones: entering the
//! small activation zone fires once, while the larger keep zone holds a latch
//! until the pointer has clearly left the corner.

use crate::contexts::WmCtx;
use crate::floating::scratchpad::{
    DEFAULT_EDGE_SCRATCHPAD_NAME, scratchpad_toggle_from_hot_corner,
};
use crate::types::{MonitorId, Point, Rect};

const ACTIVATION_WIDTH: i32 = 20;
const ACTIVATION_HEIGHT: i32 = 4;
const KEEP_WIDTH: i32 = 40;
const KEEP_HEIGHT: i32 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HotCornerZones {
    activation: Rect,
    keep: Rect,
}

fn top_right_zones(monitor: Rect) -> HotCornerZones {
    fn zone(monitor: Rect, width: i32, height: i32) -> Rect {
        let width = width.min(monitor.w.max(0));
        let height = height.min(monitor.h.max(0));
        Rect::new(monitor.x + monitor.w - width, monitor.y, width, height)
    }

    HotCornerZones {
        activation: zone(monitor, ACTIVATION_WIDTH, ACTIVATION_HEIGHT),
        keep: zone(monitor, KEEP_WIDTH, KEEP_HEIGHT),
    }
}

fn corner_at(ctx: &WmCtx<'_>, root: Point) -> Option<(MonitorId, HotCornerZones)> {
    let point = Rect::new(root.x, root.y, 1, 1);
    let monitor_id = ctx.core().model().monitors.id_intersecting_rect(point)?;
    let monitor = ctx.core().model().monitor(monitor_id)?;
    Some((monitor_id, top_right_zones(monitor.monitor_rect)))
}

/// Update the top-right overlay hot corner and apply a transition if it fires.
///
/// Returns `true` only for the motion sample that toggled the edge scratchpad.
pub fn update_overlay_hot_corner(ctx: &mut WmCtx<'_>, root: Point) -> bool {
    if ctx.core().drag_state().any_drag_active() {
        // Active drags own pointer motion. Rearming here ensures releasing a
        // drag outside the corner cannot leave an old latch behind.
        ctx.core_mut()
            .state_mut()
            .hot_corner
            .update(None, false, false);
        return false;
    }

    let corner = corner_at(ctx, root);
    let (monitor_id, inside_activation, inside_keep) = match corner {
        Some((monitor_id, zones)) => (
            Some(monitor_id),
            zones.activation.contains_point(root),
            zones.keep.contains_point(root),
        ),
        None => (None, false, false),
    };
    let triggered =
        ctx.core_mut()
            .state_mut()
            .hot_corner
            .update(monitor_id, inside_activation, inside_keep);

    let Some(monitor_id) = triggered else {
        return false;
    };
    scratchpad_toggle_from_hot_corner(ctx, DEFAULT_EDGE_SCRATCHPAD_NAME, monitor_id);
    true
}

#[cfg(test)]
mod tests {
    use super::top_right_zones;
    use crate::types::{Point, Rect};

    #[test]
    fn zones_are_anchored_to_monitor_origin() {
        let zones = top_right_zones(Rect::new(100, 200, 1920, 1080));

        assert_eq!(zones.activation, Rect::new(2000, 200, 20, 4));
        assert_eq!(zones.keep, Rect::new(1980, 200, 40, 30));
        assert!(zones.activation.contains_point(Point::new(2019, 203)));
        assert!(!zones.activation.contains_point(Point::new(2020, 203)));
    }

    #[test]
    fn zones_do_not_extend_outside_tiny_monitor() {
        let zones = top_right_zones(Rect::new(10, 20, 8, 3));

        assert_eq!(zones.activation, Rect::new(10, 20, 8, 3));
        assert_eq!(zones.keep, Rect::new(10, 20, 8, 3));
    }
}
