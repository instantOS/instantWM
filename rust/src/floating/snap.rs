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

use crate::animation::check_animate;
use crate::client::{restore_border_width, save_border_width};
use crate::contexts::WmCtx;
use crate::focus::warp_cursor_to_client;
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
pub fn change_snap(ctx: &mut WmCtx, win: Window, direction: SnapDir) {
    let snapstatus = match ctx.g.clients.get(&win) {
        Some(c) => c.snapstatus,
        None => return,
    };

    // Save geometry before entering snap for the first time.
    if snapstatus == SnapPosition::None && super::helpers::check_floating(ctx, win) {
        super::state::save_floating_win(ctx, win);
    }

    let new_snap = {
        let row = snap_pos_to_index(snapstatus);
        let col = direction as usize;
        SNAP_MATRIX[row][col]
    };

    let mon_id = if let Some(client) = ctx.g.clients.get_mut(&win) {
        client.snapstatus = new_snap;
        client.mon_id
    } else {
        return;
    };

    apply_snap(ctx, win, mon_id);
    warp_cursor_to_client(ctx, win);
    crate::focus::focus(ctx, Some(win));
}

/// Apply the window's current [`SnapPosition`] by animating it into the
/// corresponding screen region on monitor `mon_id`.
///
/// - [`SnapPosition::None`] restores the saved floating geometry.
/// - [`SnapPosition::Maximized`] zeroes the border width and fills the monitor.
/// - All other positions split the monitor into halves or quarters.
pub fn apply_snap(ctx: &mut WmCtx, win: Window, mon_id: Option<usize>) {
    let (snapstatus, saved_geo, border_width) = match ctx.g.clients.get(&win) {
        Some(c) => (c.snapstatus, c.float_geo, c.border_width),
        None => return,
    };

    let Some(mid) = mon_id else { return };

    // Geometry of the target monitor.
    let (m_mx, m_mw, m_mh, m_wh, mony) = match ctx.g.monitors.get(mid) {
        Some(m) => {
            let mony = m.monitor_rect.y + if m.showbar { ctx.g.cfg.bh } else { 0 };
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
    if snapstatus != SnapPosition::Maximized {
        restore_border_width(win);
    }

    match snapstatus {
        SnapPosition::None => {
            check_animate(
                ctx,
                win,
                &Rect {
                    x: saved_geo.x,
                    y: saved_geo.y,
                    w: saved_geo.w,
                    h: saved_geo.h,
                },
                7,
                0,
            );
        }
        SnapPosition::Top => {
            check_animate(
                ctx,
                win,
                &Rect {
                    x: m_mx,
                    y: mony,
                    w: m_mw,
                    h: m_mh / 2,
                },
                7,
                0,
            );
        }
        SnapPosition::TopRight => {
            check_animate(
                ctx,
                win,
                &Rect {
                    x: m_mx + m_mw / 2,
                    y: mony,
                    w: m_mw / 2,
                    h: m_mh / 2,
                },
                7,
                0,
            );
        }
        SnapPosition::Right => {
            check_animate(
                ctx,
                win,
                &Rect {
                    x: m_mx + m_mw / 2,
                    y: mony,
                    w: m_mw / 2 - border_width * 2,
                    h: m_wh - border_width * 2,
                },
                7,
                0,
            );
        }
        SnapPosition::BottomRight => {
            check_animate(
                ctx,
                win,
                &Rect {
                    x: m_mx + m_mw / 2,
                    y: mony + m_mh / 2,
                    w: m_mw / 2,
                    h: m_wh / 2,
                },
                7,
                0,
            );
        }
        SnapPosition::Bottom => {
            check_animate(
                ctx,
                win,
                &Rect {
                    x: m_mx,
                    y: mony + m_mh / 2,
                    w: m_mw,
                    h: m_mh / 2,
                },
                7,
                0,
            );
        }
        SnapPosition::BottomLeft => {
            check_animate(
                ctx,
                win,
                &Rect {
                    x: m_mx,
                    y: mony + m_mh / 2,
                    w: m_mw / 2,
                    h: m_wh / 2,
                },
                7,
                0,
            );
        }
        SnapPosition::Left => {
            check_animate(
                ctx,
                win,
                &Rect {
                    x: m_mx,
                    y: mony,
                    w: m_mw / 2,
                    h: m_wh,
                },
                7,
                0,
            );
        }
        SnapPosition::TopLeft => {
            check_animate(
                ctx,
                win,
                &Rect {
                    x: m_mx,
                    y: mony,
                    w: m_mw / 2,
                    h: m_mh / 2,
                },
                7,
                0,
            );
        }
        SnapPosition::Maximized => {
            save_border_width(win);
            if let Some(client) = ctx.g.clients.get_mut(&win) {
                client.border_width = 0;
            }
            check_animate(
                ctx,
                win,
                &Rect {
                    x: m_mx,
                    y: mony,
                    w: m_mw - border_width * 2,
                    h: m_mh + border_width * 2,
                },
                7,
                0,
            );

            // Raise the window if it is the focused one.
            let is_sel = ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.sel) == Some(win);
            if is_sel {
                if true { let conn = ctx.x11.conn;
                    let _ = configure_window(
                        conn,
                        win,
                        &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                    );
                    let _ = conn.flush();
                }
            }
        }
    }
}

/// Cancel the current snap and animate the window back to its saved floating
/// geometry.
///
/// Does nothing if the window is not snapped or if it is in a tiling layout
/// while being a tiled client.
pub fn reset_snap(ctx: &mut WmCtx, win: Window) {
    let (is_floating, snapstatus) = match ctx.g.clients.get(&win) {
        Some(c) => (c.isfloating, c.snapstatus),
        None => return,
    };

    if snapstatus == SnapPosition::None {
        return;
    }

    let tiling = super::helpers::has_tiling_layout(ctx);

    if is_floating || !tiling {
        if let Some(client) = ctx.g.clients.get_mut(&win) {
            client.snapstatus = SnapPosition::None;
        }
        restore_border_width(win);
        super::state::restore_floating_win(ctx, win);
        super::helpers::apply_size(ctx, win);
    }
}

/// Lightweight snap-state reconciliation used inside layout passes.
///
/// Unlike [`apply_snap`] this does **not** animate; it only updates in-memory
/// fields on the [`Client`] struct (e.g. zeroing `border_width` for maximized
/// windows) so the layout engine sees consistent state during arrange.
pub fn apply_snap_mut(c: &mut Client, _m: &Monitor) {
    if c.snapstatus == SnapPosition::Maximized {
        c.border_width = 0;
    }
}
