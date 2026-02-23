//! Slop-based window drawing and geometry validation.
//!
//! "Slop" is an external helper (`instantslop`) that lets the user drag out a
//! rectangle on screen with the mouse.  [`draw_window`] invokes it and resizes
//! the selected window to the chosen rectangle.
//!
//! This module also owns the geometry-validation predicates used by external
//! callers (IPC commands, bar click handlers) that want to resize a window to
//! an arbitrary rectangle without slop.
//!
//! # Call flow for `draw_window`
//!
//! ```text
//! user triggers draw_window keybinding
//!   └─► spawn instantslop → parse stdout → validate rect
//!             └─► handle_monitor_switch   (window may cross monitors)
//!                   └─► apply_window_resize_rect
//!                             └─► toggle_floating (if tiled) + resize
//! ```

use crate::client::resize;
use crate::floating::toggle_floating;
use crate::globals::get_globals;
use crate::mouse::monitor::handle_monitor_switch;
use crate::types::*;
use x11rb::protocol::xproto::*;

use super::constants::{MIN_WINDOW_SIZE, SLOP_MARGIN};

// ── Slop output parsing ───────────────────────────────────────────────────────

/// Parse the output of `instantslop -f x%xx%yx%wx%hx` into a [`Rect`].
///
/// The format is a literal string like `x100x200x800x600x`; the leading field
/// before the first `x` is always empty, and the four values follow in the
/// order `x`, `y`, `w`, `h`.
///
/// Returns `None` when the output is malformed or any field fails to parse as
/// an integer.
pub fn parse_slop_output(output: &str) -> Option<Rect> {
    // Expected tokens after splitting on 'x': ["", x, y, w, h, ""]
    let parts: Vec<&str> = output.split('x').collect();
    if parts.len() < 5 {
        return None;
    }

    let x: i32 = parts.get(1)?.parse().ok()?;
    let y: i32 = parts.get(2)?.parse().ok()?;
    let w: i32 = parts.get(3)?.parse().ok()?;
    let h: i32 = parts.get(4)?.trim_end().parse().ok()?;

    Some(Rect { x, y, w, h })
}

// ── Geometry validation ───────────────────────────────────────────────────────

/// Return `true` when `(x, y, width, height)` describes a rectangle that is
/// large enough to be a useful window size *and* meaningfully different from
/// the window's current geometry.
///
/// The checks performed are:
/// * `width`  and `height` both exceed [`MIN_WINDOW_SIZE`].
/// * `x` and `y` are within [`SLOP_MARGIN`] pixels of the monitor boundary
///   (i.e. not wildly off-screen).
/// * At least one dimension differs by more than 20 px from the current
///   geometry (prevents no-op resizes).
pub fn is_valid_window_size(x: i32, y: i32, width: i32, height: i32, c_win: Window) -> bool {
    let globals = get_globals();
    let Some(c) = globals.clients.get(&c_win) else {
        return false;
    };

    width > MIN_WINDOW_SIZE
        && height > MIN_WINDOW_SIZE
        && x > -SLOP_MARGIN
        && y > -SLOP_MARGIN
        && ((c.geo.w - width).abs() > 20
            || (c.geo.h - height).abs() > 20
            || (c.geo.x - x).abs() > 20
            || (c.geo.y - y).abs() > 20)
}

/// Rect-typed convenience wrapper around [`is_valid_window_size`].
#[inline]
pub fn is_valid_window_size_rect(rect: &Rect, c_win: Window) -> bool {
    is_valid_window_size(rect.x, rect.y, rect.w, rect.h, c_win)
}

// ── Window resize helpers ─────────────────────────────────────────────────────

/// Resize `c_win` to the given rectangle, promoting it to floating first if
/// it is currently tiled.
///
/// This is the single point where all external "place this window here"
/// requests should funnel.
pub fn apply_window_resize(c_win: Window, x: i32, y: i32, width: i32, height: i32) {
    let is_floating = {
        let globals = get_globals();
        globals
            .clients
            .get(&c_win)
            .map(|c| c.isfloating)
            .unwrap_or(false)
    };

    let rect = Rect {
        x,
        y,
        w: width,
        h: height,
    };

    if !is_floating {
        toggle_floating(&Arg::default());
    }

    resize(c_win, &rect, true);
}

/// Rect-typed convenience wrapper around [`apply_window_resize`].
#[inline]
pub fn apply_window_resize_rect(c_win: Window, rect: &Rect) {
    apply_window_resize(c_win, rect.x, rect.y, rect.w, rect.h);
}

// ── draw_window ───────────────────────────────────────────────────────────────

/// Let the user draw a rectangle with `instantslop` and resize the focused
/// window to that rectangle.
///
/// * If the slop process fails or the user cancels, the function returns
///   without making any changes.
/// * If the resulting rectangle is too small or identical to the current
///   geometry, the function also returns early (see [`is_valid_window_size`]).
/// * If the window is tiled it is promoted to floating before being resized.
pub fn draw_window(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
    };
    let Some(win) = sel_win else { return };

    let output = std::process::Command::new("instantslop")
        .arg("-f")
        .arg("x%xx%yx%wx%hx")
        .output();

    let Ok(out) = output else { return };
    let stdout = String::from_utf8_lossy(&out.stdout);

    let Some(rect) = parse_slop_output(&stdout) else {
        return;
    };

    if rect.w <= MIN_WINDOW_SIZE || rect.h <= MIN_WINDOW_SIZE {
        return;
    }

    // Check the rect is meaningfully different from the current geometry.
    let is_different = {
        let globals = get_globals();
        globals.clients.get(&win).is_some_and(|c| {
            (c.geo.w - rect.w).abs() > 20
                || (c.geo.h - rect.h).abs() > 20
                || (c.geo.x - rect.x).abs() > 20
                || (c.geo.y - rect.y).abs() > 20
        })
    };

    if !is_different {
        return;
    }

    // Migrate to the correct monitor if the rect crosses a boundary.
    handle_monitor_switch(win, &rect);

    // Promote to floating if needed, then apply.
    let is_floating = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.isfloating)
            .unwrap_or(false)
    };

    if !is_floating {
        toggle_floating(&Arg::default());
    }

    resize(win, &rect, true);
}
