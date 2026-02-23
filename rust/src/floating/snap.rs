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
//!               └─ calls apply_snap → check_animate_rect
//! ```
//!
//! To cancel a snap and return to the previous floating geometry call
//! [`reset_snap`].

use crate::animation::check_animate_rect;
use crate::focus::warp_cursor_to_client;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::types::*;
use crate::util::get_sel_win;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

// ── Snap position integer constants ──────────────────────────────────────────
//
// These are kept as plain `i32` constants (mirroring the C header) so that
// existing call-sites in mouse.rs / keyboard.rs that import them by name
// continue to compile.  New code should prefer [`SnapPosition`] from types.rs.

pub const SNAP_NONE: i32 = 0;
pub const SNAP_TOP: i32 = 1;
pub const SNAP_TOP_RIGHT: i32 = 2;
pub const SNAP_RIGHT: i32 = 3;
pub const SNAP_BOTTOM_RIGHT: i32 = 4;
pub const SNAP_BOTTOM: i32 = 5;
pub const SNAP_BOTTOM_LEFT: i32 = 6;
pub const SNAP_LEFT: i32 = 7;
pub const SNAP_TOP_LEFT: i32 = 8;
pub const SNAP_MAXIMIZED: i32 = 9;

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
    /// Convert a raw integer (from an [`Arg`]) to a `SnapDir`.
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
// `SNAP_MATRIX[current_snap_index][direction_index]` → next snap index.
//
// Rows  = current snap position (SNAP_NONE = 0 … SNAP_MAXIMIZED = 9)
// Cols  = direction             (Up = 0, Right = 1, Down = 2, Left = 3)
//
//                                Up              Right           Down            Left
static SNAP_MATRIX: [[i32; 4]; 10] = [
    [SNAP_MAXIMIZED, SNAP_RIGHT, SNAP_BOTTOM, SNAP_LEFT], // None
    [SNAP_MAXIMIZED, SNAP_TOP_RIGHT, SNAP_NONE, SNAP_TOP_LEFT], // Top
    [SNAP_TOP_RIGHT, SNAP_TOP_RIGHT, SNAP_RIGHT, SNAP_TOP], // TopRight
    [SNAP_TOP_RIGHT, SNAP_RIGHT, SNAP_BOTTOM_RIGHT, SNAP_NONE], // Right
    [
        SNAP_RIGHT,
        SNAP_BOTTOM_RIGHT,
        SNAP_BOTTOM_RIGHT,
        SNAP_BOTTOM,
    ], // BottomRight
    [SNAP_NONE, SNAP_BOTTOM_RIGHT, SNAP_BOTTOM, SNAP_BOTTOM_LEFT], // Bottom
    [SNAP_LEFT, SNAP_BOTTOM, SNAP_BOTTOM_LEFT, SNAP_BOTTOM_LEFT], // BottomLeft
    [SNAP_TOP_LEFT, SNAP_NONE, SNAP_BOTTOM_LEFT, SNAP_LEFT], // Left
    [SNAP_TOP_LEFT, SNAP_TOP, SNAP_LEFT, SNAP_TOP],       // TopLeft
    [SNAP_TOP, SNAP_RIGHT, SNAP_NONE, SNAP_LEFT],         // Maximized
];

// ── SnapPosition ↔ integer helpers ───────────────────────────────────────────

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

fn index_to_snap_pos(i: i32) -> SnapPosition {
    match i {
        SNAP_NONE => SnapPosition::None,
        SNAP_TOP => SnapPosition::Top,
        SNAP_TOP_RIGHT => SnapPosition::TopRight,
        SNAP_RIGHT => SnapPosition::Right,
        SNAP_BOTTOM_RIGHT => SnapPosition::BottomRight,
        SNAP_BOTTOM => SnapPosition::Bottom,
        SNAP_BOTTOM_LEFT => SnapPosition::BottomLeft,
        SNAP_LEFT => SnapPosition::Left,
        SNAP_TOP_LEFT => SnapPosition::TopLeft,
        SNAP_MAXIMIZED => SnapPosition::Maximized,
        _ => SnapPosition::None,
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Navigate the snap graph in `direction` and apply the resulting snap position.
///
/// If the window is not currently snapped, its current geometry is saved first
/// so that [`reset_snap`] can restore it later.
pub fn change_snap(win: Window, direction: SnapDir) {
    let snapstatus = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => c.snapstatus,
            None => return,
        }
    };

    // Save geometry before entering snap for the first time.
    if snapstatus == SnapPosition::None && super::helpers::check_floating(win) {
        super::state::save_floating_win(win);
    }

    let new_snap = {
        let row = snap_pos_to_index(snapstatus);
        let col = direction as usize;
        index_to_snap_pos(SNAP_MATRIX[row][col])
    };

    let mon_id = {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.snapstatus = new_snap;
            client.mon_id
        } else {
            return;
        }
    };

    apply_snap(win, mon_id);
    warp_cursor_to_client(win);
    crate::focus::focus(Some(win));
}

/// Apply the window's current [`SnapPosition`] by animating it into the
/// corresponding screen region on monitor `mon_id`.
///
/// - [`SnapPosition::None`] restores the saved floating geometry.
/// - [`SnapPosition::Maximized`] zeroes the border width and fills the monitor.
/// - All other positions split the monitor into halves or quarters.
pub fn apply_snap(win: Window, mon_id: Option<usize>) {
    let (snapstatus, saved_geo, border_width) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.snapstatus, c.float_geo, c.border_width),
            None => return,
        }
    };

    let Some(mid) = mon_id else { return };

    // Geometry of the target monitor.
    let (m_mx, m_mw, m_mh, m_wh, mony) = {
        let globals = get_globals();
        match globals.monitors.get(mid) {
            Some(m) => {
                let mony = m.monitor_rect.y + if m.showbar { globals.bh } else { 0 };
                (
                    m.monitor_rect.x,
                    m.monitor_rect.w,
                    m.monitor_rect.h,
                    m.work_rect.h,
                    mony,
                )
            }
            None => return,
        }
    };

    // Restore border width for all positions except Maximized (which needs bw=0).
    if snapstatus != SnapPosition::Maximized {
        super::state::restore_border_width_win(win);
    }

    match snapstatus {
        SnapPosition::None => {
            check_animate_rect(win, &saved_geo, 7, 0);
        }
        SnapPosition::Top => {
            check_animate_rect(
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
            check_animate_rect(
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
            check_animate_rect(
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
            check_animate_rect(
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
            check_animate_rect(
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
            check_animate_rect(
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
            check_animate_rect(
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
            check_animate_rect(
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
            super::state::save_bw_win(win);
            {
                let globals = get_globals_mut();
                if let Some(client) = globals.clients.get_mut(&win) {
                    client.border_width = 0;
                }
            }
            check_animate_rect(
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
            let is_sel = get_sel_win() == Some(win);
            if is_sel {
                let x11 = get_x11();
                if let Some(ref conn) = x11.conn {
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
pub fn reset_snap(win: Window) {
    let (is_floating, snapstatus) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.isfloating, c.snapstatus),
            None => return,
        }
    };

    if snapstatus == SnapPosition::None {
        return;
    }

    let tiling = super::helpers::has_tiling_layout();

    if is_floating || !tiling {
        {
            let globals = get_globals_mut();
            if let Some(client) = globals.clients.get_mut(&win) {
                client.snapstatus = SnapPosition::None;
            }
        }
        super::state::restore_border_width_win(win);
        super::state::restore_floating_win(win);
        super::helpers::apply_size(win);
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
