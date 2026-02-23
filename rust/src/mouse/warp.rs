//! Cursor-warping utilities.
//!
//! These functions move the X11 pointer so that it stays near the focused
//! window.  Call-sites should prefer the named wrappers over the internal
//! `warp_impl` function.
//!
//! # Overview
//!
//! | Function                    | When to use                                             |
//! |-----------------------------|---------------------------------------------------------|
//! | [`warp`]                    | Warp into a client only if the cursor is outside it     |
//! | [`warp_cursor_to_client_win`]| Same as `warp`, taking a `&Client` directly             |
//! | [`force_warp`]              | Unconditionally warp to the top-centre of a client      |
//! | [`warp_to_focus`]           | Keybinding handler – warp to the selected window        |
//! | [`reset_cursor`]            | Restore the normal (arrow) X11 root cursor              |

use crate::globals::{get_globals, get_x11};
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Query the current root-window pointer position.
///
/// Returns `None` when the X11 connection is unavailable or the request fails.
pub(super) fn get_root_ptr() -> Option<(i32, i32)> {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if let Ok(cookie) = query_pointer(conn, globals.root) {
            if let Ok(reply) = cookie.reply() {
                return Some((reply.root_x as i32, reply.root_y as i32));
            }
        }
    }
    None
}

/// Core warp implementation.  Moves the pointer to the centre of `win`.
///
/// If `win` is `0` the pointer is sent to the centre of the selected monitor's
/// work area instead.  The warp is skipped when the pointer is already inside
/// the client's window (including its border) or on the bar belonging to that
/// client's monitor.
pub(super) fn warp_impl(win: Window) {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let globals = get_globals();
    let root = globals.root;
    let bh = globals.bh;

    // No target window – centre on the selected monitor's work area.
    if win == 0 {
        if let Some(mon) = globals.monitors.get(globals.selmon) {
            let _ = conn.warp_pointer(
                CURRENT_TIME,
                root,
                0,
                0,
                0,
                0,
                (mon.work_rect.x + mon.work_rect.w / 2) as i16,
                (mon.work_rect.y + mon.work_rect.h / 2) as i16,
            );
            let _ = conn.flush();
        }
        return;
    }

    let Some(c) = globals.clients.get(&win) else {
        return;
    };

    let Some((ptr_x, ptr_y)) = get_root_ptr() else {
        return;
    };

    // Skip if the pointer is already inside the window (accounting for borders).
    let in_window = c.geo.contains_point(ptr_x, ptr_y)
        || (ptr_x > c.geo.x - c.border_width
            && ptr_y > c.geo.y - c.border_width
            && ptr_x < c.geo.x + c.geo.w + c.border_width * 2
            && ptr_y < c.geo.y + c.geo.h + c.border_width * 2);

    // Skip if the pointer is on the bar belonging to this client's monitor.
    let on_bar = c
        .mon_id
        .and_then(|mid| globals.monitors.get(mid))
        .map_or(false, |mon| {
            (ptr_y > mon.by && ptr_y < mon.by + bh) || (mon.topbar && ptr_y == 0)
        });

    if in_window || on_bar {
        return;
    }

    let _ = conn.warp_pointer(
        CURRENT_TIME,
        c.win,
        0,
        0,
        0,
        0,
        (c.geo.w / 2) as i16,
        (c.geo.h / 2) as i16,
    );
    let _ = conn.flush();
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Warp the pointer into `c` only if it is currently outside the window.
///
/// This is the preferred warp function for focus changes: it avoids jarring
/// pointer jumps when the user deliberately placed the cursor somewhere else.
#[inline]
pub fn warp(c: &Client) {
    warp_impl(c.win);
}

/// Same as [`warp`] but accepts a `&Client` directly – kept for call-sites
/// that already hold a reference to the full struct.
#[inline]
pub fn warp_cursor_to_client_win(c: &Client) {
    warp_impl(c.win);
}

/// Unconditionally move the pointer to the top-centre of `c`.
///
/// Used after operations that deliberately reposition the window (e.g. after
/// an animated move) where the old cursor position is no longer meaningful.
pub fn force_warp(c: &Client) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.warp_pointer(
            x11rb::NONE,
            c.win,
            0i16,
            0i16,
            0u16,
            0u16,
            (c.geo.w / 2) as i16,
            10i16,
        );
        let _ = conn.flush();
    }
}

/// Keybinding handler: warp the cursor to the currently focused window.
///
/// Reads `selmon → sel` and delegates to [`warp_impl`].  Does nothing when no
/// window is selected.
pub fn warp_to_focus(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
    };

    if let Some(win) = sel_win {
        warp_impl(win);
    }
}

/// Restore the root window's default (arrow) cursor.
///
/// Call this after a modal grab ends so that the cursor reverts to normal even
/// if the pointer is not over any client window.
pub fn reset_cursor() {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if let Some(ref cursor) = globals.cursors[0] {
            let _ = change_window_attributes(
                conn,
                globals.root,
                &ChangeWindowAttributesAux::new().cursor(cursor.cursor),
            );
            let _ = conn.flush();
        }
    }
}
