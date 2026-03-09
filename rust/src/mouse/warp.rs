//! Cursor-warping utilities.
//!
//! # Overview
//!
//! | Function                           | When to use                                            |
//! |------------------------------------|--------------------------------------------------------|
//! | [`WmCtx::warp_cursor_to_client`]   | Warp to a client only if the cursor is outside it      |
//! | [`warp_into`]                      | Clamp cursor into window bounds (before a drag/resize) |
//! | [`warp_to_focus`]                  | Keybinding handler – warp to the selected window       |
//! | [`reset_cursor`]                   | Restore the normal (arrow) root cursor                 |
//!
//! [`WmCtx::warp_cursor_to_client`]: crate::contexts::WmCtx::warp_cursor_to_client

use crate::backend::x11::X11BackendRef;
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::globals::X11RuntimeConfig;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

pub(crate) const WARP_INTO_PADDING: i32 = 10;

// ── Pointer position query ────────────────────────────────────────────────────

/// Query the current root-window pointer position via X11.
pub(crate) fn get_root_ptr_x11(x11: &X11BackendRef, root: Window) -> Option<(i32, i32)> {
    let cookie = query_pointer(x11.conn, root).ok()?;
    let reply = cookie.reply().ok()?;
    Some((reply.root_x as i32, reply.root_y as i32))
}

/// Query the current pointer position in root (logical) coordinates.
/// Returns `None` when the position is unavailable.
pub fn get_root_ptr(ctx: &WmCtx) -> Option<(i32, i32)> {
    match ctx {
        WmCtx::X11(x11) => get_root_ptr_x11(&x11.x11, x11.x11_runtime.root),
        WmCtx::Wayland(wl) => wl.wayland.backend.pointer_location(),
    }
}

pub fn get_root_ptr_ctx_x11(ctx: &WmCtxX11<'_>) -> Option<(i32, i32)> {
    get_root_ptr_x11(&ctx.x11, ctx.x11_runtime.root)
}

// ── Core X11 warp implementation ──────────────────────────────────────────────

/// Move the X11 pointer to the centre of `win`, skipping if already inside.
///
/// If `win` is the default (zero) `WindowId`, warps to the centre of the
/// selected monitor's work area instead.  The warp is also skipped when the
/// pointer is on the bar of the window's monitor.
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

    // Skip if already inside the window (including border).
    let in_window = c.geo.contains_point(ptr_x, ptr_y)
        || (ptr_x > c.geo.x - c.border_width
            && ptr_y > c.geo.y - c.border_width
            && ptr_x < c.geo.x + c.geo.w + c.border_width * 2
            && ptr_y < c.geo.y + c.geo.h + c.border_width * 2);

    let on_bar = core.g.monitor(c.monitor_id).is_some_and(|mon| {
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

// ── Public backend-agnostic API ───────────────────────────────────────────────

/// Warp the cursor into `win`'s geometry if the cursor is currently outside.
///
/// The cursor is clamped to the window rect with a small inset so subsequent
/// drags/resizes start from inside the client.  On Wayland the warp is
/// deferred; the current pointer position is used to decide the target.
pub fn warp_into(ctx: &mut WmCtx, win: WindowId) {
    if win == WindowId::default() {
        return;
    }
    match ctx {
        WmCtx::X11(x11) => warp_into_x11(&x11.core, &x11.x11, x11.x11_runtime, win),
        WmCtx::Wayland(wl) => {
            let Some(c) = wl.core.g.clients.get(&win) else {
                return;
            };
            let (mut tx, mut ty) = wl
                .wayland
                .backend
                .pointer_location()
                .map(|(px, py)| (px as i32, py as i32))
                .unwrap_or((c.geo.x + c.geo.w / 2, c.geo.y + c.geo.h / 2));

            if tx < c.geo.x {
                tx = c.geo.x + WARP_INTO_PADDING;
            } else if tx > c.geo.x + c.geo.w {
                tx = c.geo.x + c.geo.w - WARP_INTO_PADDING;
            }
            if ty < c.geo.y {
                ty = c.geo.y + WARP_INTO_PADDING;
            } else if ty > c.geo.y + c.geo.h {
                ty = c.geo.y + c.geo.h - WARP_INTO_PADDING;
            }

            wl.wayland.backend.warp_pointer(tx as f64, ty as f64);
        }
    }
}

/// `warp_into` for X11-specific call-sites that already hold a `WmCtxX11`.
pub fn warp_into_ctx_x11(ctx: &WmCtxX11<'_>, win: WindowId) {
    warp_into_x11(&ctx.core, &ctx.x11, ctx.x11_runtime, win);
}

/// Clamp the X11 pointer into `win`'s geometry with padding.
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

/// Keybinding/IPC handler: warp the cursor to the currently focused window.
pub fn warp_to_focus(ctx: &mut WmCtx) {
    if let Some(win) = ctx.selected_client() {
        ctx.warp_cursor_to_client(win);
    }
}

// ── Cursor reset ──────────────────────────────────────────────────────────────

/// Restore the root window's default (arrow) cursor and clear `altcursor`.
///
/// Call this after a modal grab ends so the cursor reverts to normal even
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

/// Backend-agnostic cursor reset.
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
            // Wayland cursor is managed via CursorImageStatus — no root-window cursor to reset.
        }
    }
}
