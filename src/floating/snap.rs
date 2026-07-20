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
//!               └─ calls apply_snap → ctx.move_resize(AnimateTo)
//! ```
//!
//! To cancel a snap and return to the previous floating geometry call
//! [`reset_snap`].

use crate::constants::animation::DEFAULT_FRAME_COUNT;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::geometry::MoveResizeOptions;

use crate::types::*;

// ── Public API ────────────────────────────────────────────────────────────────

/// Navigate the snap graph in `direction` and apply the resulting snap position.
///
/// If the window is not currently snapped, its current geometry is saved first
/// so that [`reset_snap`] can restore it later.
pub fn change_snap(ctx: &mut WmCtx, win: WindowId, direction: Direction) {
    let (monitor_id, _snap_status) =
        if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
            let status = client.snap_status;

            // Save geometry before entering snap for the first time.
            let new_snap = status.next(direction);

            if status == SnapPosition::None && client.mode.is_floating() {
                client.float_geo = client.geo;
            }
            client.snap_status = new_snap;
            (client.monitor_id, status)
        } else {
            return;
        };

    // Apply snap geometry (generic) and backend-specific extras.
    match ctx {
        WmCtx::X11(ctx_x11) => {
            let Some(rect) = snap_target_rect(ctx_x11, win, monitor_id) else {
                return;
            };
            apply_snap(ctx_x11, win, &rect);
            let wm_ctx = WmCtx::X11(ctx_x11.reborrow());
            wm_ctx
                .pointer_backend()
                .warp_pointer((rect.x + rect.w / 2) as f64, (rect.y + rect.h / 2) as f64);
            crate::focus::focus(&mut WmCtx::X11(ctx_x11.reborrow()), Some(win));
        }
        WmCtx::Wayland(_) => {
            // Wayland: use generic snap geometry (no animation)
            let monitor = ctx.core().model().monitor(monitor_id).cloned().unwrap();
            apply_snap_for_window(ctx, win, &monitor);
            ctx.warp_cursor_to_client(win);
        }
    }
}

/// Apply the window's current [`SnapPosition`] by animating it into the
/// corresponding screen region on monitor `monitor_id`.
///
/// - [`SnapPosition::None`] restores the saved floating geometry.
/// - [`SnapPosition::Maximized`] zeroes the border width and fills the monitor.
/// - All other positions split the monitor into halves or quarters.
fn snap_target_rect(ctx: &mut WmCtxX11, win: WindowId, monitor_id: MonitorId) -> Option<Rect> {
    let (snap_status, saved_geo) = {
        let c = ctx.core.model().client(win)?;
        (c.snap_status, c.float_geo)
    };

    if snap_status == SnapPosition::None {
        return Some(saved_geo);
    }

    let border_width = {
        let client = ctx.core.model_mut().client_mut(win)?;
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
    let work_rect = ctx.core.model().monitor(monitor_id)?.work_rect();
    snap_status.target_rect(border_width, work_rect)
}

/// Apply the window's current [`SnapPosition`] by animating it into the
/// corresponding screen region on monitor `monitor_id`.
pub fn apply_snap(ctx: &mut WmCtxX11, win: WindowId, rect: &Rect) {
    let snap_status = match ctx.core.model().client(win) {
        Some(c) => c.snap_status,
        None => return,
    };

    WmCtx::X11(ctx.reborrow()).move_resize(
        win,
        *rect,
        MoveResizeOptions::animate_to(DEFAULT_FRAME_COUNT),
    );

    // Raise the window if it is the focused one (Maximized only).
    if snap_status == SnapPosition::Maximized {
        let is_sel = ctx.core.model().selected_win() == Some(win);
        if is_sel {
            let wm_ctx = WmCtx::X11(ctx.reborrow());
            wm_ctx.window_backend().raise_window_visual_only(win);
        }
    }
}

/// Cancel the current snap and animate the window back to its saved floating
/// geometry.
///
/// Does nothing if the window is not snapped or if it is in a tiling layout
/// while being a tiled client.
pub fn reset_snap(ctx: &mut WmCtx, win: WindowId) {
    let (is_floating, snap_status) = match ctx.core().model().client(win) {
        Some(c) => (c.mode.is_floating(), c.snap_status),
        None => return,
    };

    if snap_status == SnapPosition::None {
        return;
    }

    let tiling = super::helpers::has_tiling_layout(ctx.core().model());

    if is_floating || !tiling {
        if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
            client.snap_status = SnapPosition::None;
            client.restore_border_width();
        }
        super::state::restore_floating_geometry(ctx, win);

        // apply_size is X11-specific
        if let WmCtx::X11(x11) = ctx {
            super::helpers::apply_size(x11, win);
        }
    }
}

/// Compute and apply the geometry dictated by a client's [`SnapPosition`].
///
/// This is a pure geometry function: it reads `client.snap_status` and
/// `client.border_width`, derives the target `Rect` from the monitor's
/// `work_rect`, and applies it through `move_resize`. It does *not* modify
/// `snap_status`.
///
/// Returns immediately if `snap_status` is [`SnapPosition::None`] or the
/// client window is not found.
fn apply_snap_for_window(ctx: &mut WmCtx<'_>, win: WindowId, m: &Monitor) {
    let c = match ctx.core().model().client(win) {
        Some(c) => c,
        None => return,
    };

    let Some(rect) = c.snap_status.target_rect(c.border_width, m.work_rect()) else {
        return;
    };

    ctx.move_resize(win, rect, MoveResizeOptions::hinted_immediate(false));
}
