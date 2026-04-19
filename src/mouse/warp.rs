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
use crate::backend::x11::X11RuntimeConfig;
use crate::contexts::{CoreCtx, WmCtx};
use crate::types::input::AltCursor;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

pub(crate) const WARP_INTO_PADDING: i32 = 10;

// ── Pointer position query ────────────────────────────────────────────────────

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

    let Some(c) = ctx.core().client(win).cloned() else {
        return;
    };

    let (mut tx, mut ty) = ctx
        .pointer_location()
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

    ctx.warp_pointer(tx as f64, ty as f64);
}

/// Keybinding/IPC handler: warp the cursor to the currently focused window.
pub fn warp_to_focus(ctx: &mut WmCtx) {
    if let Some(win) = ctx.core().selected_client() {
        ctx.warp_cursor_to_client(win);
    }
}

// ── Cursor reset ──────────────────────────────────────────────────────────────

/// Restore the root window's default (arrow) cursor and clear the requested
/// WM cursor presentation.
///
/// Call this after a modal grab ends so the cursor reverts to normal even
/// if the pointer is not over any client window.
pub fn reset_cursor_x11(core: &mut CoreCtx, x11: &X11BackendRef, x11_runtime: &X11RuntimeConfig) {
    if core.globals().behavior.requested_cursor == AltCursor::Default {
        return;
    }
    core.globals_mut().behavior.requested_cursor = AltCursor::Default;

    let cursor_idx = AltCursor::Default.to_x11_index();
    if let Some(ref cursor) = x11_runtime.cursors[cursor_idx] {
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
