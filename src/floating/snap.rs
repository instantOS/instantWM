//! Snap-positioning system for floating windows.
//!
//! A "snap" places a floating window into a named screen region (half/quarter
//! of the monitor, or maximized).  The nine positions plus *None* and
//! *Maximized* form a directed navigation graph encoded in [`SNAP_MATRIX`].
//!
//! # Typical call flow
//!
//! ```text
//! user presses snap-left key
//!      └─► change_snap(win, Direction::Left)
//!               ├─ saves current float geometry (if entering snap for the first time)
//!               ├─ looks up new position in SNAP_MATRIX
//!               └─ calls apply_snap → ctx.move_resize(AnimateTo)
//! ```
//!
//! To cancel a snap and return to the previous floating geometry call
//! [`reset_snap`].

use crate::client::{restore_border_width, save_border_width};
use crate::constants::animation::DEFAULT_FRAME_COUNT;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::geometry::MoveResizeOptions;
use crate::layouts::algo::apply_snap_for_window;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

// ── Snap navigation matrix ────────────────────────────────────────────────────
//
// `SNAP_MATRIX[current_snap_index][direction_index]` → next snap position.
//
// Rows  = current snap position (None = 0 … Maximized = 9)
// Cols  = direction             (Up = 0, Right = 1, Down = 2, Left = 3)
//
//                                Up              Right           Down            Left
static SNAP_MATRIX: [[SnapPosition; 4]; 10] = [
    [
        SnapPosition::Maximized,
        SnapPosition::Right,
        SnapPosition::Bottom,
        SnapPosition::Left,
    ], // None
    [
        SnapPosition::Maximized,
        SnapPosition::TopRight,
        SnapPosition::None,
        SnapPosition::TopLeft,
    ], // Top
    [
        SnapPosition::TopRight,
        SnapPosition::TopRight,
        SnapPosition::Right,
        SnapPosition::Top,
    ], // TopRight
    [
        SnapPosition::TopRight,
        SnapPosition::Right,
        SnapPosition::BottomRight,
        SnapPosition::None,
    ], // Right
    [
        SnapPosition::Right,
        SnapPosition::BottomRight,
        SnapPosition::BottomRight,
        SnapPosition::Bottom,
    ], // BottomRight
    [
        SnapPosition::None,
        SnapPosition::BottomRight,
        SnapPosition::Bottom,
        SnapPosition::BottomLeft,
    ], // Bottom
    [
        SnapPosition::Left,
        SnapPosition::Bottom,
        SnapPosition::BottomLeft,
        SnapPosition::BottomLeft,
    ], // BottomLeft
    [
        SnapPosition::TopLeft,
        SnapPosition::None,
        SnapPosition::BottomLeft,
        SnapPosition::Left,
    ], // Left
    [
        SnapPosition::TopLeft,
        SnapPosition::Top,
        SnapPosition::Left,
        SnapPosition::Top,
    ], // TopLeft
    [
        SnapPosition::Top,
        SnapPosition::Right,
        SnapPosition::None,
        SnapPosition::Left,
    ], // Maximized
];

// ── SnapPosition ↔ index helpers ───────────────────────────────────────────────

fn snap_pos_to_index(s: SnapPosition) -> usize {
    match s {
        SnapPosition::None => 0,
        SnapPosition::Top => 1,
        SnapPosition::TopRight => 2,
        SnapPosition::Right => 3,
        SnapPosition::BottomRight => 4,
        SnapPosition::Bottom => 5,
        SnapPosition::BottomLeft => 6,
        SnapPosition::Left => 7,
        SnapPosition::TopLeft => 8,
        SnapPosition::Maximized => 9,
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Navigate the snap graph in `direction` and apply the resulting snap position.
///
/// If the window is not currently snapped, its current geometry is saved first
/// so that [`reset_snap`] can restore it later.
pub fn change_snap(ctx: &mut WmCtx, win: WindowId, direction: Direction) {
    let (monitor_id, _snap_status) =
        if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
            let status = client.snap_status;

            // Save geometry before entering snap for the first time.
            let new_snap = {
                let row = snap_pos_to_index(status);
                let col = direction.snap_matrix_index();
                SNAP_MATRIX[row][col]
            };

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
            wm_ctx.warp_pointer((rect.x + rect.w / 2) as f64, (rect.y + rect.h / 2) as f64);
            crate::focus::focus_soft_x11(
                &mut ctx_x11.core,
                &ctx_x11.x11,
                ctx_x11.x11_runtime,
                Some(win),
            );
        }
        WmCtx::Wayland(_) => {
            // Wayland: use generic snap geometry (no animation)
            let monitor = ctx.core().globals().monitor(monitor_id).cloned().unwrap();
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
    let (snap_status, saved_geo, border_width) = match ctx.core.client(win) {
        Some(c) => (c.snap_status, c.float_geo, c.border_width),
        None => return None,
    };

    // Geometry of the target monitor.
    let (m_mx, m_mw, m_mh, m_wh, mony) = {
        let m = ctx.core.globals().monitor(monitor_id)?;
        let showbar = m.showbar_for_mask(m.selected_tags());
        let mony = m.monitor_rect.y
            + if showbar {
                ctx.core.globals().cfg.bar_height
            } else {
                0
            };
        (
            m.monitor_rect.x,
            m.monitor_rect.w,
            m.monitor_rect.h,
            m.work_rect.h,
            mony,
        )
    };

    // Restore border width for all positions except Maximized (which needs bw=0).
    if snap_status != SnapPosition::Maximized
        && let Some(client) = ctx.core.globals_mut().clients.get_mut(&win)
    {
        restore_border_width(client);
    }

    // Compute target rect based on snap position.
    Some(match snap_status {
        SnapPosition::None => Rect {
            x: saved_geo.x,
            y: saved_geo.y,
            w: saved_geo.w,
            h: saved_geo.h,
        },
        SnapPosition::Top => Rect {
            x: m_mx,
            y: mony,
            w: m_mw,
            h: m_mh / 2,
        },
        SnapPosition::TopRight => Rect {
            x: m_mx + m_mw / 2,
            y: mony,
            w: m_mw / 2,
            h: m_mh / 2,
        },
        SnapPosition::Right => Rect {
            x: m_mx + m_mw / 2,
            y: mony,
            w: m_mw / 2 - border_width * 2,
            h: m_wh - border_width * 2,
        },
        SnapPosition::BottomRight => Rect {
            x: m_mx + m_mw / 2,
            y: mony + m_mh / 2,
            w: m_mw / 2,
            h: m_wh / 2,
        },
        SnapPosition::Bottom => Rect {
            x: m_mx,
            y: mony + m_mh / 2,
            w: m_mw,
            h: m_mh / 2,
        },
        SnapPosition::BottomLeft => Rect {
            x: m_mx,
            y: mony + m_mh / 2,
            w: m_mw / 2,
            h: m_wh / 2,
        },
        SnapPosition::Left => Rect {
            x: m_mx,
            y: mony,
            w: m_mw / 2,
            h: m_wh,
        },
        SnapPosition::TopLeft => Rect {
            x: m_mx,
            y: mony,
            w: m_mw / 2,
            h: m_mh / 2,
        },
        SnapPosition::Maximized => {
            if let Some(client) = ctx.core.globals_mut().clients.get_mut(&win) {
                save_border_width(client);
                client.border_width = 0;
            }
            Rect {
                x: m_mx,
                y: mony,
                w: m_mw - border_width * 2,
                h: m_mh + border_width * 2,
            }
        }
    })
}

/// Apply the window's current [`SnapPosition`] by animating it into the
/// corresponding screen region on monitor `monitor_id`.
pub fn apply_snap(ctx: &mut WmCtxX11, win: WindowId, rect: &Rect) {
    let snap_status = match ctx.core.client(win) {
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
        let is_sel = ctx.core.selected_client() == Some(win);
        if is_sel {
            let conn = ctx.x11.conn;
            let x11_win: Window = win.into();
            let _ = configure_window(
                conn,
                x11_win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
            let _ = conn.flush();
        }
    }
}

/// Cancel the current snap and animate the window back to its saved floating
/// geometry.
///
/// Does nothing if the window is not snapped or if it is in a tiling layout
/// while being a tiled client.
pub fn reset_snap(ctx: &mut WmCtx, win: WindowId) {
    let (is_floating, snap_status) = match ctx.core().client(win) {
        Some(c) => (c.mode.is_floating(), c.snap_status),
        None => return,
    };

    if snap_status == SnapPosition::None {
        return;
    }

    let tiling = super::helpers::has_tiling_layout(ctx.core());

    if is_floating || !tiling {
        if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
            client.snap_status = SnapPosition::None;
            restore_border_width(client);
        }
        super::state::restore_floating_geometry(ctx, win);

        // apply_size is X11-specific
        if let WmCtx::X11(x11) = ctx {
            super::helpers::apply_size(x11, win);
        }
    }
}
