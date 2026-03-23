//! Client termination: graceful close and forceful kill.
//!
//! # The three entry points
//!
//! * [`kill_client`]  – kill the currently selected window.  Tries a graceful
//!   `WM_DELETE_WINDOW` message first; falls back to `XKillClient` if the
//!   protocol is not supported.
//!
//! * [`shut_kill`]   – like [`kill_client`], but if the monitor has no clients
//!   at all it shuts the whole session down instead.
//!
//! * [`close_win`]   – close an arbitrary window identified by its `Window` ID.
//!   Used by the close button drawn in the bar.
//!
//! # Graceful vs. forceful termination
//!
//! The WM first attempts to send a `WM_DELETE_WINDOW` `ClientMessage`.  If
//! [`send_event`] returns `false` (the window does not support the protocol),
//! we fall back to `conn.kill_client()` wrapped in a server grab so that no
//! other requests from the dying client are processed between the kill and the
//! expected `DestroyNotify`.

use crate::client::focus::send_event_x11;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::types::WindowId;
use x11rb::CURRENT_TIME;
use x11rb::protocol::xproto::{ConnectionExt, Window};

// ---------------------------------------------------------------------------
// kill_client
// ---------------------------------------------------------------------------

/// Backend-agnostic client kill.
///
/// Attempts a graceful close via `WM_DELETE_WINDOW` (X11) or
/// `close` request (Wayland), falling back to `XKillClient` on X11
/// if the protocol is not supported.
pub fn kill_client(ctx: &mut WmCtx, win: WindowId) {
    let Some(client) = ctx.client(win) else {
        return;
    };

    if client.is_locked {
        return;
    }

    force_close(ctx, win);
}

/// Backend-specific force close operation.
fn force_close(ctx: &mut WmCtx, win: WindowId) {
    match ctx {
        WmCtx::X11(ctx_x11) => {
            let wmatom_delete = ctx_x11.x11_runtime.wmatom.delete;
            force_close_x11(ctx_x11, win, wmatom_delete);
        }
        WmCtx::Wayland(wl) => {
            let _ = wl.wayland.backend.close_window(win);
        }
    }
}

// ---------------------------------------------------------------------------
// shut_kill
// ---------------------------------------------------------------------------

/// Kill the selected window, or shut down the session if there are no clients.
///
/// This is bound to the "power" key: pressing it on an empty monitor triggers
/// an orderly system shutdown; pressing it when windows are open closes the
/// focused window instead.
pub fn shut_kill(ctx: &mut WmCtx) {
    let has_clients = !ctx.core().globals().selected_monitor().clients.is_empty();

    if has_clients {
        if let Some(win) = ctx.selected_client() {
            kill_client(ctx, win);
        }
    } else {
        crate::util::spawn(ctx, &["instantshutdown"]);
    }
}

// ---------------------------------------------------------------------------
// close_win
// ---------------------------------------------------------------------------

/// Close an arbitrary window by its Window ID.
///
/// Like `kill_client` but without the kill fallback - only attempts graceful close.
pub fn close_win(ctx: &mut WmCtx, win: WindowId) {
    let is_locked = ctx.core().globals().clients.is_locked(win);

    if is_locked {
        return;
    }

    // Use the same close path as kill_client (graceful close for both backends)
    force_close(ctx, win);
}

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

/// Attempt a graceful `WM_DELETE_WINDOW`, falling back to `XKillClient` (X11-specific).
fn force_close_x11(ctx_x11: &mut WmCtxX11<'_>, win: WindowId, wmatom_delete: u32) {
    let x11_win: Window = win.into();
    let sent = send_event_x11(
        &mut ctx_x11.core,
        &ctx_x11.x11,
        ctx_x11.x11_runtime,
        win,
        wmatom_delete,
        0,
        wmatom_delete as i64,
        CURRENT_TIME as i64,
        0,
        0,
        0,
    );

    if !sent {
        let _grab = crate::backend::x11::ServerGrab::new(ctx_x11.x11.conn);
        let _ = ctx_x11.x11.conn.kill_client(x11_win);
    }
}
