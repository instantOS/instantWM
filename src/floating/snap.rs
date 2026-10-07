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
use crate::backend::PointerOps;

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
    // The owning monitor answers the work area the snap target is resolved
    // against; a client no longer names its own monitor.
    let Some(monitor) = ctx.core_mut().state.model.client_owner_mut(win) else {
        return;
    };
    let work_area = monitor.work_rect();
    let client = monitor
        .client_mut(win)
        .expect("owner was resolved from client membership");
    let status = client.snap_status;
    let new_snap = status.next(direction);
    // Save geometry before entering snap for the first time.
    if status == SnapPosition::None && client.mode().is_normal_floating() {
        client.save_floating_placement(client.geo, work_area);
    }
    let SnapTarget { border_width, rect } = apply_snap(client, new_snap, work_area);

    ctx.raise_client(win);
    ctx.set_border(win, border_width);
    let Some(rect) = rect else {
        return;
    };

    // Animate into place, keep the pointer inside the freshly snapped
    // window (snapping is keyboard-driven), and make the snapped window
    // the focused client. Identical on both backends. Size hints are
    // respected so clients with a minimum size (e.g. terminal grids) are
    // never configured below it by a small snap region.
    ctx.move_resize(
        win,
        rect,
        MoveResizeOptions::animate_to(DEFAULT_ANIMATION_MILLIS).with_size_hints(),
    );
    ctx.warp_to_point(rect.center());
    crate::focus::focus(ctx, Some(win));
}

/// Model outcome of entering a snap position.
struct SnapTarget {
    /// Border width the backend must apply.
    border_width: i32,
    /// Geometry the window should occupy, if the position resolves to one.
    rect: Option<Rect>,
}

/// Enter `new_snap` and resolve the geometry the window should occupy.
///
/// [`SnapPosition::None`] restores the saved floating geometry — and the
/// border width [`SnapPosition::Maximized`] zeroed.
/// [`SnapPosition::Maximized`] saves the current border width and zeroes it
/// so the window fills the work area edge to edge; every other position
/// splits the monitor into halves or quarters around the normal border.
///
/// The caller must push the returned border through [`WmCtx::set_border`] so
/// backends apply it; [`WmCtx::move_resize`] never touches border widths.
fn apply_snap(client: &mut Client, new_snap: SnapPosition, work_area: Rect) -> SnapTarget {
    client.snap_status = new_snap;
    if new_snap == SnapPosition::None {
        client.restore_border_width();
        return SnapTarget {
            border_width: client.border_width,
            rect: Some(client.saved_floating_rect().unwrap_or(client.geo)),
        };
    }

    if new_snap == SnapPosition::Maximized {
        client.save_border_width();
        client.border_width = 0;
    } else {
        client.restore_border_width();
    }
    SnapTarget {
        border_width: client.border_width,
        rect: new_snap.target_rect(client.border_width, work_area),
    }
}

/// Cancel the current snap and animate the window back to its saved floating
/// geometry.
///
/// Does nothing if the window is not snapped or if it is in a tiling layout
/// while being a tiled client.
pub fn reset_snap(ctx: &mut WmCtx, win: WindowId) {
    let core_state = &ctx.core().state;
    let (is_floating, snap_status) = match core_state.model.client(win) {
        Some(c) => (c.mode().is_normal_floating(), c.snap_status),
        None => return,
    };

    if snap_status == SnapPosition::None {
        return;
    }

    let tiling = core_state
        .model
        .expect_selected_monitor()
        .is_tiling_layout();

    if is_floating || !tiling {
        ctx.raise_client(win);
        let restored_border = {
            let Some(client) = ctx.core_mut().state.model.client_mut(win) else {
                return;
            };
            client.snap_status = SnapPosition::None;
            client.restore_border_width();
            client.border_width
        };
        // Push the restored border to the backend; the move_resize in
        // restore_floating_geometry never applies border widths.
        ctx.set_border(win, restored_border);
        super::state::restore_floating_geometry(ctx, win);
    }
}
