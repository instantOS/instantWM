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
//!      └─► change_snap(win, SnapDir::Left)
//!               ├─ saves current float geometry (if entering snap for the first time)
//!               ├─ looks up new position in SNAP_MATRIX
//!               └─ calls apply_snap → check_animate
//! ```
//!
//! To cancel a snap and return to the previous floating geometry call
//! [`reset_snap`].

use crate::animation::check_animate_x11;
use crate::client::{restore_border_width, save_border_width};
use crate::contexts::{WmCtx, WmCtxX11};
use crate::layouts::algo::apply_snap_for_window;
use crate::mouse::warp::warp_to_client_win;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

// ── Snap direction ────────────────────────────────────────────────────────────

/// The four cardinal directions used to navigate the snap graph.
///
/// `SnapDir::index()` returns the column offset into [`SNAP_MATRIX`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapDir {
    Up = 0,
    Right = 1,
    Down = 2,
    Left = 3,
}

impl SnapDir {
    /// Convert a raw integer to a `SnapDir`.
    /// Values outside `0..=3` clamp to `Up`.
    pub fn from_index(i: i32) -> Self {
        match i {
            0 => Self::Up,
            1 => Self::Right,
            2 => Self::Down,
            3 => Self::Left,
            _ => Self::Up,
        }
    }
}

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
pub fn change_snap(ctx: &mut WmCtx, win: WindowId, direction: SnapDir) {
    let (monitor_id, snap_status) = if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
        let status = client.snap_status;

        // Save geometry before entering snap for the first time.
        let new_snap = {
            let row = snap_pos_to_index(status);
            let col = direction as usize;
            SNAP_MATRIX[row][col]
        };

        if status == SnapPosition::None && client.is_floating {
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
            apply_snap(ctx_x11, win, monitor_id);
            ctx_x11.reborrow().warp_cursor_to_client(win);
            crate::focus::focus_soft_x11(
                &mut ctx_x11.core,
                &ctx_x11.x11,
                ctx_x11.x11_runtime,
                Some(win),
            );
        }
        WmCtx::Wayland(_) => {
            // Wayland: use generic snap geometry (no animation)
            let monitor = ctx.g().monitor(monitor_id).cloned().unwrap();
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
pub fn apply_snap(ctx: &mut WmCtxX11, win: WindowId, monitor_id: usize) {
    let (snap_status, saved_geo, border_width) = match ctx.core.client(win) {
        Some(c) => (c.snap_status, c.float_geo, c.border_width),
        None => return,
    };

    // Geometry of the target monitor.
    let (m_mx, m_mw, m_mh, m_wh, mony) = match ctx.core.g.monitor(monitor_id) {
        Some(m) => {
            let mony = m.monitor_rect.y
                + if m.showbar {
                    ctx.core.g.cfg.bar_height
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
        }
        None => return,
    };

    // Restore border width for all positions except Maximized (which needs bw=0).
    if snap_status != SnapPosition::Maximized {
        restore_border_width(&mut ctx.core, win);
    }

    // Compute target rect based on snap position.
    let rect = match snap_status {
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
            save_border_width(&mut ctx.core, win);
            if let Some(client) = ctx.core.g.clients.get_mut(&win) {
                client.border_width = 0;
            }
            Rect {
                x: m_mx,
                y: mony,
                w: m_mw - border_width * 2,
                h: m_mh + border_width * 2,
            }
        }
    };

    check_animate_x11(&mut ctx.core, &ctx.x11, ctx.x11_runtime, win, &rect, 7, 0);

    // Raise the window if it is the focused one (Maximized only).
    if snap_status == SnapPosition::Maximized {
        let is_sel = ctx.selected_client() == Some(win);
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
    let (is_floating, snap_status) = match ctx.client(win) {
        Some(c) => (c.is_floating, c.snap_status),
        None => return,
    };

    if snap_status == SnapPosition::None {
        return;
    }

    let tiling = super::helpers::has_tiling_layout(ctx.core());

    if is_floating || !tiling {
        if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
            client.snap_status = SnapPosition::None;
        }
        restore_border_width(ctx.core_mut(), win);
        super::state::restore_floating_geometry(ctx, win);

        // apply_size is X11-specific
        if let WmCtx::X11(x11) = ctx {
            super::helpers::apply_size(x11, win);
        }
    }
}
