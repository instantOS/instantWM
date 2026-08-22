//! Snap-positioning system for floating windows.
//!
//! A "snap" places a floating window into a named screen region (half/quarter
//! of the monitor, or maximized).  The nine positions plus *None* and
//! *Maximized* form a directed navigation graph encoded in [`snap_next`].
//!
//! # Typical call flow
//!
//! ```text
//! user presses snap-left key
//!      └─► change_snap(win, Direction::Left)
//!               ├─ saves current float geometry (if entering snap for the first time)
//!               ├─ looks up new position via snap_next()
//!               └─ animates the window to the position's target rect
//! ```
//!
//! To cancel a snap and return to the previous floating geometry call
//! [`reset_snap`].

use crate::constants::animation::DEFAULT_ANIMATION_MILLIS;
use crate::contexts::WmCtx;
use crate::geometry::MoveResizeOptions;

use crate::types::*;

// ── Public API ────────────────────────────────────────────────────────────────

/// Navigate the snap graph in `direction` and apply the resulting snap position.
///
/// If the window is not currently snapped, its current geometry is saved first
/// so that [`reset_snap`] can restore it later.
pub fn change_snap(ctx: &mut WmCtx, win: WindowId, direction: Direction) {
    crate::client::fullscreen::leave_maximized(ctx, win);
    let work_area = ctx
        .core()
        .model()
        .client_view(win)
        .map(|view| view.monitor.work_rect());
    let (monitor_id, _snap_status) =
        if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
            let status = client.snap_status;

            // Save geometry before entering snap for the first time.
            let new_snap = status.next(direction);

            if status == SnapPosition::None
                && client.mode().is_normal_floating()
                && let Some(work_area) = work_area
            {
                client.save_floating_placement(client.geo, work_area);
            }
            client.snap_status = new_snap;
            (client.monitor_id, status)
        } else {
            return;
        };

    ctx.raise_client(win);

    let Some(rect) = snap_target_rect(ctx, win, monitor_id) else {
        return;
    };

    // Animate into place, keep the pointer inside the freshly snapped
    // window (snapping is keyboard-driven), and make the snapped window
    // the focused client. Identical on both backends.
    ctx.move_resize(
        win,
        rect,
        MoveResizeOptions::animate_to(DEFAULT_ANIMATION_MILLIS),
    );
    ctx.pointer_backend().warp_to_point(rect.center());
    crate::focus::focus(ctx, Some(win));
}

/// Resolve the geometry a snapped window should occupy.
///
/// [`SnapPosition::None`] restores the saved floating geometry.
/// [`SnapPosition::Maximized`] saves the current border width and zeroes it
/// so the window fills the work area edge to edge; every other position
/// splits the monitor into halves or quarters around the normal border.
fn snap_target_rect(ctx: &mut WmCtx, win: WindowId, monitor_id: MonitorId) -> Option<Rect> {
    let (snap_status, saved_geo) = {
        let c = ctx.core().model().client(win)?;
        (c.snap_status, c.saved_floating_rect().unwrap_or(c.geo))
    };

    if snap_status == SnapPosition::None {
        return Some(saved_geo);
    }

    let border_width = {
        let client = ctx.core_mut().model_mut().client_mut(win)?;
        if snap_status == SnapPosition::Maximized {
            if client.border_width != 0 {
                client.save_border_width();
                client.border_width = 0;
            }
        } else {
            client.restore_border_width();
        }
        client.border_width
    };
    let work_rect = ctx.core().model().monitor(monitor_id)?.work_rect();
    snap_status.target_rect(border_width, work_rect)
}

/// Cancel the current snap and animate the window back to its saved floating
/// geometry.
///
/// Does nothing if the window is not snapped or if it is in a tiling layout
/// while being a tiled client.
pub fn reset_snap(ctx: &mut WmCtx, win: WindowId) {
    let (is_floating, snap_status) = match ctx.core().model().client(win) {
        Some(c) => (c.mode().is_normal_floating(), c.snap_status),
        None => return,
    };

    if snap_status == SnapPosition::None {
        return;
    }

    let tiling = super::helpers::has_tiling_layout(ctx.core().model());

    if is_floating || !tiling {
        ctx.raise_client(win);
        if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
            client.snap_status = SnapPosition::None;
            client.restore_border_width();
        }
        super::state::restore_floating_geometry(ctx, win);
    }
}
