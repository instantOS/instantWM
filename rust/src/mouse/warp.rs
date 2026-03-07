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

use crate::backend::x11::X11BackendRef;
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::globals::X11RuntimeConfig;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

const FORCE_WARP_Y: i16 = 10;
const WARP_INTO_PADDING: i32 = 10;

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Query the current root-window pointer position.
///
/// Returns `None` when the X11 connection is unavailable or the request fails.
pub(crate) fn get_root_ptr_x11(x11: &X11BackendRef, root: Window) -> Option<(i32, i32)> {
    let conn = x11.conn;
    let cookie = query_pointer(conn, root).ok()?;
    let reply = cookie.reply().ok()?;
    Some((reply.root_x as i32, reply.root_y as i32))
}

pub fn get_root_ptr(ctx: &WmCtx) -> Option<(i32, i32)> {
    match ctx {
        WmCtx::X11(x11) => get_root_ptr_x11(&x11.x11, x11.x11_runtime.root),
        WmCtx::Wayland(_) => None,
    }
}

pub fn get_root_ptr_ctx_x11(ctx: &WmCtxX11<'_>) -> Option<(i32, i32)> {
    get_root_ptr_x11(&ctx.x11, ctx.x11_runtime.root)
}

/// Core warp implementation.  Moves the pointer to the centre of `win`.
///
/// If `win` is `0` the pointer is sent to the centre of the selected monitor's
/// work area instead.  The warp is skipped when the pointer is already inside
/// the client's window (including its border) or on the bar belonging to that
/// client's monitor.
pub(crate) fn warp_to_client_win(
    core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
) {
    let conn = x11.conn;

    let root = x11_runtime.root;
    let bar_height = core.g.cfg.bar_height;

    // No target window – centre on the selected monitor's work area.
    if win == WindowId::default() {
        let mon = core.g.selected_monitor();
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
        return;
    }

    let Some(c) = core.g.clients.get(&win) else {
        return;
    };

    let Some((ptr_x, ptr_y)) = get_root_ptr_x11(x11, root) else {
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
        .monitor_id
        .and_then(|mid| core.g.monitor(mid))
        .is_some_and(|mon| {
            (ptr_y > mon.bar_y && ptr_y < mon.bar_y + bar_height) || (mon.topbar && ptr_y == 0)
        });

    if in_window || on_bar {
        return;
    }

    let x11_win: Window = c.win.into();
    let _ = conn.warp_pointer(
        CURRENT_TIME,
        x11_win,
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
pub fn warp_x11(core: &CoreCtx, x11: &X11BackendRef, x11_runtime: &X11RuntimeConfig, c: &Client) {
    warp_to_client_win(core, x11, x11_runtime, c.win);
}

/// Same as [`warp`] but accepts a `&Client` directly – kept for call-sites
/// that already hold a reference to the full struct.
#[inline]
pub fn warp_cursor_to_client_win_x11(
    core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    c: &Client,
) {
    warp_to_client_win(core, x11, x11_runtime, c.win);
}

/// Unconditionally move the pointer to the top-centre of `c`.
///
/// Used after operations that deliberately reposition the window (e.g. after
/// an animated move) where the old cursor position is no longer meaningful.
pub fn force_warp_x11(x11: &X11BackendRef, c: &Client) {
    let conn = x11.conn;
    let x11_win: Window = c.win.into();
    let _ = conn.warp_pointer(
        x11rb::NONE,
        x11_win,
        0i16,
        0i16,
        0u16,
        0u16,
        (c.geo.w / 2) as i16,
        FORCE_WARP_Y,
    );
    let _ = conn.flush();
}

/// Warp the pointer into the window's geometry if it is currently outside.
///
/// This clamps the pointer into the window rect with a small padding so
/// subsequent drags/resizes start from inside the client.
pub fn warp_into_x11(
    core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
    win: WindowId,
) {
    if win == WindowId::default() {
        return;
    }

    let Some(c) = core.g.clients.get(&win) else {
        return;
    };

    let Some((mut x, mut y)) = get_root_ptr_x11(x11, x11_runtime.root) else {
        return;
    };

    if x < c.geo.x {
        x = c.geo.x + WARP_INTO_PADDING;
    } else if x > c.geo.x + c.geo.w {
        x = c.geo.x + c.geo.w - WARP_INTO_PADDING;
    }

    if y < c.geo.y {
        y = c.geo.y + WARP_INTO_PADDING;
    } else if y > c.geo.y + c.geo.h {
        y = c.geo.y + c.geo.h - WARP_INTO_PADDING;
    }

    let _ = x11.conn.warp_pointer(
        CURRENT_TIME,
        x11_runtime.root,
        0,
        0,
        0,
        0,
        x as i16,
        y as i16,
    );
    let _ = x11.conn.flush();
}

pub fn warp_into(ctx: &WmCtx, win: WindowId) {
    if let WmCtx::X11(x11) = ctx {
        warp_into_x11(&x11.core, &x11.x11, x11.x11_runtime, win);
    }
}

pub fn warp_into_ctx_x11(ctx: &WmCtxX11<'_>, win: WindowId) {
    warp_into_x11(&ctx.core, &ctx.x11, ctx.x11_runtime, win);
}

/// Keybinding handler: warp the cursor to the currently focused window.
///
/// Reads `selmon → sel` and delegates to [`warp_impl`].  Does nothing when no
/// window is selected.
pub fn warp_to_focus_x11(core: &CoreCtx, x11: &X11BackendRef, x11_runtime: &X11RuntimeConfig) {
    if let Some(win) = core.selected_client() {
        warp_to_client_win(core, x11, x11_runtime, win);
    }
}

pub fn warp_to_focus(ctx: &mut WmCtx) {
    if let WmCtx::X11(x11) = ctx {
        warp_to_focus_x11(&x11.core, &x11.x11, x11.x11_runtime);
    }
}

/// Restore the root window's default (arrow) cursor and clear `altcursor`.
///
/// Call this after a modal grab ends so that the cursor reverts to normal even
/// if the pointer is not over any client window.
pub fn reset_cursor_x11(core: &mut CoreCtx, x11: &X11BackendRef, x11_runtime: &X11RuntimeConfig) {
    if core.g.altcursor == AltCursor::None {
        return;
    }
    core.g.altcursor = AltCursor::None;

    if let Some(ref cursor) = core.g.cfg.cursors[0] {
        let _ = change_window_attributes(
            x11.conn,
            x11_runtime.root,
            &ChangeWindowAttributesAux::new().cursor(cursor.cursor as u32),
        );
        let _ = x11.conn.flush();
    }
}

/// Backend-agnostic reset_cursor - dispatches to X11 or is a no-op on Wayland.
pub fn reset_cursor(ctx: &mut crate::contexts::WmCtx) {
    use crate::contexts::{WmCtx::*, WmCtxX11};
    match ctx {
        X11(WmCtxX11 {
            core,
            x11,
            x11_runtime,
            ..
        }) => reset_cursor_x11(core, x11, x11_runtime),
        Wayland(_) => {
            // Wayland handles cursor reset differently - no-op for now
        }
    }
}
