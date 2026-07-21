//! Cursor-warping utilities.
//!
//! # Overview
//!
//! | Function                           | When to use                                            |
//! |------------------------------------|--------------------------------------------------------|
//! | [`WmCtx::warp_cursor_to_client`]   | Warp to a client only if the cursor is outside it      |
//! | [`warp_into`]                      | Clamp cursor into window bounds (before a drag/resize) |
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

/// Warp the cursor into `win`'s geometry if the cursor is currently outside.
///
/// The cursor is clamped to the window rect with a small inset so subsequent
/// drags/resizes start from inside the client.  On Wayland the warp is
/// deferred; the current pointer position is used to decide the target.
pub fn warp_into(ctx: &mut WmCtx, win: WindowId) {
    if win == WindowId::default() {
        return;
    }

    let Some(c) = ctx.core().model().client(win).cloned() else {
        return;
    };

    let (mut tx, mut ty) = ctx
        .pointer_backend()
        .pointer_location()
        .map(|p| (p.x, p.y))
        .unwrap_or((c.geo.x + c.geo.w / 2, c.geo.y + c.geo.h / 2));

    if tx < c.geo.x {
        tx = c.geo.x + WARP_INTO_PADDING;
    } else if tx > c.geo.x + c.geo.w {
        tx = c.geo.x + c.geo.w - WARP_INTO_PADDING;
    }
    if ty < c.geo.y {
        ty = c.geo.y + WARP_INTO_PADDING;
    } else if ty > c.geo.y + c.geo.h {
        ty = c.geo.y + c.geo.h - WARP_INTO_PADDING;
    }

    ctx.pointer_backend().warp_pointer(tx as f64, ty as f64);
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
    ctx.pointer_backend().warp_pointer(target.x as f64, target.y as f64);
    Some(target)
}
