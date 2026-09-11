//! Cursor-warping utilities.
//!
//! # Overview
//!
//! | Function                           | When to use                                            |
//!|------------------------------------|--------------------------------------------------------|
//! | [`WmCtx::warp_cursor_to_client`]   | Warp to a client only if the cursor is outside it      |
//! | [`clamp_into`]                     | Clamp a point into window bounds (before a drag/resize)|
//! | [`warp_to_focus`]                  | Keybinding handler – warp to the selected window       |
//! | [`warp_to_resize_corner`]          | Warp to the edge/corner for a resize direction         |
//! | [`warp_pointer_to_monitor`]        | Carry the pointer along on monitor-switch focus        |
//!
//! [`WmCtx::warp_cursor_to_client`]: crate::contexts::WmCtx::warp_cursor_to_client

use crate::contexts::WmCtx;
use crate::types::*;

pub(crate) const WARP_INTO_PADDING: i32 = 10;

// ── Pointer position query ────────────────────────────────────────────────────

// ── Public backend-agnostic API ───────────────────────────────────────────────

/// Clamp `point` into `geo` (with a small inset) if it lies outside.
///
/// Returns the original point when it is already inside the rect.
pub fn clamp_into(point: Point, geo: Rect) -> Point {
    let pad = WARP_INTO_PADDING;
    let mut target = point;
    if target.x < geo.x {
        target.x = geo.x + pad;
    } else if target.x > geo.right() {
        target.x = geo.right() - pad;
    }
    if target.y < geo.y {
        target.y = geo.y + pad;
    } else if target.y > geo.bottom() {
        target.y = geo.bottom() - pad;
    }
    target
}

/// Keybinding/IPC handler: warp the cursor to the currently focused window.
pub fn warp_to_focus(ctx: &mut WmCtx) {
    if let Some(win) = ctx.core().model().selected_win() {
        ctx.warp_cursor_to_client(win);
    }
}

/// Decide where a monitor-switch warp should place the pointer.
///
/// Returns the monitor's center when the pointer is not already inside
/// `monitor_rect`; an unknown pointer position counts as off-monitor.
/// Returns `None` when no warp is needed.
pub fn warp_target_for_monitor(
    monitor_rect: Rect,
    center: Point,
    pointer: Option<Point>,
) -> Option<Point> {
    if pointer.is_some_and(|ptr| monitor_rect.contains_point(ptr)) {
        return None;
    }
    Some(center)
}

/// Carry the pointer to `monitor_id` after an explicit monitor switch.
///
/// Used by monitor-focus actions (`Super+comma`/`Super+period` and the
/// `monitor` IPC commands) so keyboard-driven focus switches also move the
/// cursor, unless it already sits on the target monitor. Pointer-driven
/// selection paths must not use this: they select because of the cursor, so
/// warping there would fight the user's hand.
pub fn warp_pointer_to_monitor(ctx: &mut WmCtx, monitor_id: MonitorId) {
    let Some((monitor_rect, center)) = ctx
        .core()
        .model()
        .monitor(monitor_id)
        .map(|monitor| (monitor.monitor_rect, monitor.center()))
    else {
        return;
    };

    let pointer = ctx.pointer_backend().pointer_location();
    if let Some(target) = warp_target_for_monitor(monitor_rect, center, pointer) {
        ctx.pointer_backend().warp_to_point(target);
    }
}

/// Warp the pointer to the edge or corner of `win` described by `direction`,
/// and return that absolute target point.
///
/// The point is computed from `win`'s current geometry and border width via
/// [`ResizeDirection::warp_offset`].  Use the returned `Point` as the resize
/// `start` anchor for [`begin_resize`] / [`activate_armed_resize`] so the
/// drag math matches the warped cursor position.
///
/// [`begin_resize`]: crate::mouse::drag::lifecycle::begin_resize
/// [`activate_armed_resize`]: crate::mouse::drag::lifecycle::activate_armed_resize
///
/// Returns `None` if `win` is unknown to the model.
pub fn warp_to_resize_corner(
    ctx: &mut WmCtx,
    win: WindowId,
    direction: ResizeDirection,
) -> Option<Point> {
    let c = ctx.core().model().client(win)?;
    let offset = direction.warp_offset(c.geo.size(), c.border_width);
    let target = Point::new(c.geo.x + offset.x, c.geo.y + offset.y);
    ctx.pointer_backend().warp_to_point(target);
    Some(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Side-by-side 1920x1080 monitors: `x = 0` and `x = 1920`.
    fn monitor_at(x: i32) -> (Rect, Point) {
        let rect = Rect {
            x,
            y: 0,
            w: 1920,
            h: 1080,
        };
        (rect, rect.center())
    }

    #[test]
    fn pointer_on_target_monitor_skips_the_warp() {
        let (rect, center) = monitor_at(1920);
        assert_eq!(
            warp_target_for_monitor(rect, center, Some(Point::new(2000, 540))),
            None
        );
    }

    #[test]
    fn pointer_on_another_monitor_warps_to_the_target_center() {
        let (left, _) = monitor_at(0);
        let (right, right_center) = monitor_at(1920);
        // Pointer rests on the left monitor; the target is the right one.
        assert!(left.contains_point(Point::new(100, 540)));
        assert_eq!(
            warp_target_for_monitor(right, right_center, Some(Point::new(100, 540))),
            Some(Point::new(2880, 540))
        );
    }

    #[test]
    fn pointer_next_to_the_target_edge_still_warps() {
        let (right, right_center) = monitor_at(1920);
        // Last column of the left neighbour, directly at the target's edge.
        assert_eq!(
            warp_target_for_monitor(right, right_center, Some(Point::new(1919, 540))),
            Some(right_center)
        );
    }

    #[test]
    fn unknown_pointer_position_warps_to_the_target_center() {
        let (rect, center) = monitor_at(0);
        assert_eq!(warp_target_for_monitor(rect, center, None), Some(center));
    }
}
