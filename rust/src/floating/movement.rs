//! Keyboard-driven floating window movement, resize, and scaling.
//!
//! All functions in this module operate on floating windows only; they are
//! no-ops when the selected monitor has a tiling layout active and the window
//! is not explicitly floating.
//!
//! # Functions
//!
//! | Function            | What it does                                              |
//! |---------------------|-----------------------------------------------------------|
//! | `moveresize`        | Move a floating window in a cardinal direction            |
//! | `key_resize`        | Grow / shrink a floating window in a cardinal direction   |
//! | `center_window`     | Center a floating window on the work area                 |
//! | `upscale_client`    | Uniformly grow a floating window by a fixed step          |
//! | `downscale_client`  | Uniformly shrink a floating window, floating it first     |
//! | `scale_client_win`  | Core scale implementation (used by up/downscale)          |

use crate::animation::animate_client_rect;
use crate::client::resize;
use crate::focus::warp_cursor_to_client;
use crate::globals::get_globals;
use crate::types::*;
use x11rb::protocol::xproto::Window;

// ── Move ──────────────────────────────────────────────────────────────────────

/// Move a floating window in a cardinal direction using the keyboard.
///
/// `arg.i` selects the direction:
///
/// | Value | Direction |
/// |-------|-----------|
/// | 0     | Down      |
/// | 1     | Up        |
/// | 2     | Right     |
/// | 3     | Left      |
///
/// The window is clamped to the monitor bounds after the move.
/// Movement is animated with a short 5-step animation.
pub fn moveresize(arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
    };
    let Some(win) = sel_win else { return };

    let (is_floating, c_x, c_y, c_w, c_h, border_width) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (
                c.isfloating,
                c.geo.x,
                c.geo.y,
                c.geo.w,
                c.geo.h,
                c.border_width,
            ),
            None => return,
        }
    };

    if super::helpers::has_tiling_layout() && !is_floating {
        return;
    }

    const MOVE_STEP: i32 = 40;
    // [Down, Up, Right, Left] → [dx, dy]
    const DELTAS: [[i32; 2]; 4] = [
        [0, MOVE_STEP],  // Down
        [0, -MOVE_STEP], // Up
        [MOVE_STEP, 0],  // Right
        [-MOVE_STEP, 0], // Left
    ];

    let dir = arg.i.max(0).min(3) as usize;
    let mut nx = c_x + DELTAS[dir][0];
    let mut ny = c_y + DELTAS[dir][1];

    let (mon_mx, mon_my, mon_mw, mon_mh) = {
        let globals = get_globals();
        match globals.monitors.get(globals.selmon) {
            Some(m) => (
                m.monitor_rect.x,
                m.monitor_rect.y,
                m.monitor_rect.w,
                m.monitor_rect.h,
            ),
            None => return,
        }
    };

    // Clamp to monitor bounds.
    nx = nx.max(mon_mx);
    ny = ny.max(mon_my);
    if ny + c_h > mon_my + mon_mh {
        ny = (mon_mh + mon_my) - c_h - border_width * 2;
    }
    if nx + c_w > mon_mx + mon_mw {
        nx = (mon_mw + mon_mx) - c_w - border_width * 2;
    }

    animate_client_rect(
        win,
        &Rect {
            x: nx,
            y: ny,
            w: c_w,
            h: c_h,
        },
        5,
        0,
    );
    warp_cursor_to_client(win);
}

// ── Resize ────────────────────────────────────────────────────────────────────

/// Resize a floating window in a cardinal direction using the keyboard.
///
/// `arg.i` selects the resize direction:
///
/// | Value | Effect       |
/// |-------|--------------|
/// | 0     | Taller (down)|
/// | 1     | Shorter (up) |
/// | 2     | Wider (right)|
/// | 3     | Narrower     |
///
/// Any active snap is cancelled before resizing so the window reverts to free
/// floating, then the new size is applied immediately (no animation).
pub fn key_resize(arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
    };
    let Some(win) = sel_win else { return };

    let (is_floating, c_x, c_y, c_w, c_h) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.isfloating, c.geo.x, c.geo.y, c.geo.w, c.geo.h),
            None => return,
        }
    };

    // Cancel snap first so the window is free to be resized arbitrarily.
    super::snap::reset_snap(win);

    if super::helpers::has_tiling_layout() && !is_floating {
        return;
    }

    const RESIZE_STEP: i32 = 40;
    // [TallerDown, ShorterUp, WiderRight, NarrowerLeft] → [dw, dh]
    const DELTAS: [[i32; 2]; 4] = [
        [0, RESIZE_STEP],  // Taller (down)
        [0, -RESIZE_STEP], // Shorter (up)
        [RESIZE_STEP, 0],  // Wider (right)
        [-RESIZE_STEP, 0], // Narrower (left)
    ];

    let dir = arg.i.max(0).min(3) as usize;
    let nw = c_w + DELTAS[dir][0];
    let nh = c_h + DELTAS[dir][1];

    warp_cursor_to_client(win);
    resize(
        win,
        &Rect {
            x: c_x,
            y: c_y,
            w: nw,
            h: nh,
        },
        true,
    );
}

// ── Center ────────────────────────────────────────────────────────────────────

/// Center the selected floating window within the monitor's work area.
///
/// Does nothing when:
/// - no window is selected, or the selected window is the overlay
/// - a tiling layout is active and the window is not explicitly floating
/// - the window is larger than the work area (centering would clip it)
pub fn center_window(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        let mon = match globals.monitors.get(globals.selmon) {
            Some(m) => m,
            None => return,
        };
        match mon.sel {
            Some(sel) if Some(sel) != mon.overlay => Some(sel),
            _ => None,
        }
    };
    let Some(win) = sel_win else { return };

    let (w, h, is_floating) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.geo.w, c.geo.h, c.isfloating),
            None => return,
        }
    };

    if super::helpers::has_tiling_layout() && !is_floating {
        return;
    }

    let (mw, mh, showbar, mx, my, bh) = {
        let globals = get_globals();
        let mon = match globals.monitors.get(globals.selmon) {
            Some(m) => m,
            None => return,
        };
        (
            mon.work_rect.w,
            mon.work_rect.h,
            mon.showbar,
            mon.monitor_rect.x,
            mon.monitor_rect.y,
            globals.bh,
        )
    };

    if w > mw || h > mh {
        return; // window larger than work area — centering would clip it
    }

    // When the bar is visible its height shifts the vertical midpoint.
    let y_offset = if showbar { bh } else { -bh };

    resize(
        win,
        &Rect {
            x: mx + (mw / 2) - (w / 2),
            y: my + (mh / 2) - (h / 2) + y_offset,
            w,
            h,
        },
        true,
    );
}

// ── Scale ─────────────────────────────────────────────────────────────────────

/// Uniformly grow the selected (or specified) floating window by 30 pixels on
/// each side.
///
/// If `arg.v` is non-null it is interpreted as a `Window` to operate on;
/// otherwise the currently selected window is used.
pub fn upscale_client(arg: &Arg) {
    let win = resolve_scale_target(arg);
    if let Some(win) = win {
        scale_client_win(win, 30);
    }
}

/// Uniformly shrink the selected (or specified) window by 30 pixels on each
/// side.
///
/// If the window is not floating it is floated first (with focus) so that the
/// scale operation makes sense.
///
/// If `arg.v` is non-null it is interpreted as a `Window` to operate on;
/// otherwise the currently selected window is used.
pub fn downscale_client(arg: &Arg) {
    let Some(win) = resolve_scale_target(arg) else {
        return;
    };

    let is_floating = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.isfloating)
            .unwrap_or(false)
    };

    if !is_floating {
        crate::focus::focus(Some(win));
        super::state::toggle_floating(&Arg::default());
    }

    scale_client_win(win, -30);
}

/// Uniformly scale a floating window by `scale` pixels (positive = grow,
/// negative = shrink).
///
/// The window is expanded/contracted equally on all sides (`scale/2` per
/// edge), then clamped so it stays within the monitor bounds and above the
/// bar.
///
/// Does nothing if the window is not floating.
pub fn scale_client_win(win: Window, scale: i32) {
    let (is_floating, c_x, c_y, c_w, c_h) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.isfloating, c.geo.x, c.geo.y, c.geo.w, c.geo.h),
            None => return,
        }
    };

    if !is_floating {
        return;
    }

    let (mon_mx, mon_my, mon_mw, mon_mh, bh) = {
        let globals = get_globals();
        match globals.monitors.get(globals.selmon) {
            Some(m) => (
                m.monitor_rect.x,
                m.monitor_rect.y,
                m.monitor_rect.w,
                m.monitor_rect.h,
                globals.bh,
            ),
            None => return,
        }
    };

    let mut w = c_w + scale;
    let mut h = c_h + scale;
    let mut x = c_x - scale / 2;
    let mut y = c_y - scale / 2;

    // Clamp position and size to monitor area.
    x = x.max(mon_mx);
    w = w.min(mon_mw);
    h = h.min(mon_mh);
    if h + y > mon_my + mon_mh {
        y = mon_mh - h;
    }
    y = y.max(bh); // don't overlap the bar

    animate_client_rect(win, &Rect { x, y, w, h }, 3, 0);
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Resolve the target window for a scale operation.
///
/// If `arg.v` contains a window ID that window is used; otherwise the
/// currently selected window on `selmon` is returned.
fn resolve_scale_target(arg: &Arg) -> Option<Window> {
    if let Some(v) = arg.v {
        Some(v as Window)
    } else {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
    }
}
