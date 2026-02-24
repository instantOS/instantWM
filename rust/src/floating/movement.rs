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
use crate::util::get_sel_win;
use x11rb::protocol::xproto::Window;

// ── Move ──────────────────────────────────────────────────────────────────────

/// Move a floating window in a cardinal direction using the keyboard.
///
/// `arg.direction` (or `arg.i` for backward compat) selects the direction:
///
/// | Value | Direction |
/// |-------|-----------|
/// | Down  | Down      |
/// | Up    | Up        |
/// | Right | Right     |
/// | Left  | Left      |
///
/// The window is clamped to the monitor bounds after the move.
/// Movement is animated with a short 5-step animation.
pub fn moveresize(arg: &Arg) {
    let sel_win = get_sel_win();
    let Some(win) = sel_win else { return };

    let dir = arg.direction.or_else(|| CardinalDirection::from_i32(arg.i));
    let Some(dir) = dir else { return };

    let (is_floating, geo, border_width) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.isfloating, c.geo, c.border_width),
            None => return,
        }
    };

    if super::helpers::has_tiling_layout() && !is_floating {
        return;
    }

    const MOVE_STEP: i32 = 40;
    let (dx, dy) = dir.move_delta(MOVE_STEP);
    let mut new_x = geo.x + dx;
    let mut new_y = geo.y + dy;

    let mon_rect = {
        let globals = get_globals();
        match globals.monitors.get(globals.selmon) {
            Some(m) => m.monitor_rect,
            None => return,
        }
    };

    // Clamp to monitor bounds.
    new_x = new_x.max(mon_rect.x);
    new_y = new_y.max(mon_rect.y);
    if new_y + geo.h > mon_rect.y + mon_rect.h {
        new_y = (mon_rect.h + mon_rect.y) - geo.h - border_width * 2;
    }
    if new_x + geo.w > mon_rect.x + mon_rect.w {
        new_x = (mon_rect.w + mon_rect.x) - geo.w - border_width * 2;
    }

    animate_client_rect(
        win,
        &Rect {
            x: new_x,
            y: new_y,
            w: geo.w,
            h: geo.h,
        },
        5,
        0,
    );
    warp_cursor_to_client(win);
}

// ── Resize ────────────────────────────────────────────────────────────────────

/// Resize a floating window in a cardinal direction using the keyboard.
///
/// `arg.direction` (or `arg.i` for backward compat) selects the resize direction:
///
/// | Direction | Effect       |
/// |-----------|--------------|
/// | Down      | Taller       |
/// | Up        | Shorter      |
/// | Right     | Wider        |
/// | Left      | Narrower     |
///
/// An active snap is cancelled before resizing so the window reverts to free
/// floating, then the new size is applied immediately (no animation).
pub fn key_resize(arg: &Arg) {
    let sel_win = get_sel_win();
    let Some(win) = sel_win else { return };

    let dir = arg.direction.or_else(|| CardinalDirection::from_i32(arg.i));
    let Some(dir) = dir else { return };

    let (is_floating, geo) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.isfloating, c.geo),
            None => return,
        }
    };

    // Cancel snap first so the window is free to be resized arbitrarily.
    super::snap::reset_snap(win);

    if super::helpers::has_tiling_layout() && !is_floating {
        return;
    }

    const RESIZE_STEP: i32 = 40;
    let (dw, dh) = dir.resize_delta(RESIZE_STEP);
    let nw = geo.w + dw;
    let nh = geo.h + dh;

    warp_cursor_to_client(win);
    resize(
        win,
        &Rect {
            x: geo.x,
            y: geo.y,
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
        let mon = match get_globals().monitors.get(get_globals().selmon) {
            Some(m) => m,
            None => return,
        };
        match mon.sel {
            Some(sel) if Some(sel) != mon.overlay => Some(sel),
            _ => None,
        }
    };
    let Some(win) = sel_win else { return };

    let (geo, is_floating) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.geo, c.isfloating),
            None => return,
        }
    };

    if super::helpers::has_tiling_layout() && !is_floating {
        return;
    }

    let (work_rect, mon_rect, showbar, bh) = {
        let globals = get_globals();
        let mon = match globals.monitors.get(globals.selmon) {
            Some(m) => m,
            None => return,
        };
        (mon.work_rect, mon.monitor_rect, mon.showbar, globals.bh)
    };

    if geo.w > work_rect.w || geo.h > work_rect.h {
        return; // window larger than work area — centering would clip it
    }

    // When the bar is visible its height shifts the vertical midpoint.
    let y_offset = if showbar { bh } else { -bh };

    resize(
        win,
        &Rect {
            x: mon_rect.x + (work_rect.w / 2) - (geo.w / 2),
            y: mon_rect.y + (work_rect.h / 2) - (geo.h / 2) + y_offset,
            w: geo.w,
            h: geo.h,
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
    let (is_floating, geo) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.isfloating, c.geo),
            None => return,
        }
    };

    if !is_floating {
        return;
    }

    let (mon_rect, bh) = {
        let globals = get_globals();
        match globals.monitors.get(globals.selmon) {
            Some(m) => (m.monitor_rect, globals.bh),
            None => return,
        }
    };

    let mut w = geo.w + scale;
    let mut h = geo.h + scale;
    let mut x = geo.x - scale / 2;
    let mut y = geo.y - scale / 2;

    // Clamp position and size to monitor area.
    x = x.max(mon_rect.x);
    w = w.min(mon_rect.w);
    h = h.min(mon_rect.h);
    if h + y > mon_rect.y + mon_rect.h {
        y = mon_rect.h - h;
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
