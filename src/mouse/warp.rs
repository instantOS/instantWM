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
//! | [`WmCtx::set_cursor_style`]        | Restore the normal (arrow) root cursor                 |
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
