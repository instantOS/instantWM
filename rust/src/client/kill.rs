//! Client termination: graceful close and forceful kill.
//!
//! # The three entry points
//!
//! * [`kill_client`]  – kill the currently selected window.  Tries a graceful
//!                      `WM_DELETE_WINDOW` message first; falls back to
//!                      `XKillClient` if the protocol is not supported.
//!                      Plays a closing animation unless the window is already
//!                      animating or is fullscreen.
//!
//! * [`shut_kill`]    – like [`kill_client`], but if the monitor has no
//!                      clients at all it shuts the whole session down instead.
//!
//! * [`close_win`]    – close an arbitrary window identified by its `Window`
//!                      ID. Used by the close button drawn in the bar.
//!
//! # Graceful vs. forceful termination
//!
//! The WM first attempts to send a `WM_DELETE_WINDOW` `ClientMessage`.  If
//! [`send_event`] returns `false` (the window does not support the protocol),
//! we fall back to `conn.kill_client()` wrapped in a server grab so that no
//! other requests from the dying client are processed between the kill and the
//! expected `DestroyNotify`.

use crate::animation::animate_client_x11;
use crate::backend::x11::X11BackendRef;
use crate::client::focus::send_event_x11;
use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
use crate::types::{Rect, WindowId};
use x11rb::protocol::xproto::{ConnectionExt, Window};
use x11rb::CURRENT_TIME;

// ---------------------------------------------------------------------------
// kill_client
// ---------------------------------------------------------------------------

/// Kill the given window (X11-specific implementation).
fn kill_client_x11(ctx_x11: &mut WmCtxX11<'_>, win: WindowId) {
    let Some(client) = ctx_x11.core.g.clients.get(&win).clone() else {
        return;
    };

    if client.is_locked {
        return;
    }

    let is_fullscreen = client.is_fullscreen;
    let mon_mh = ctx_x11
        .core
        .g
        .monitor(client.monitor_id)
        .map(|m| m.monitor_rect.h)
        .unwrap_or(0);

    let animated = ctx_x11.core.g.animated;
    let anim_client = ctx_x11.core.focus.anim_client;

    if animated && win != anim_client && !is_fullscreen {
        ctx_x11.core.focus.anim_client = win;
        animate_client_x11(
            &mut ctx_x11.core,
            &ctx_x11.x11,
            ctx_x11.x11_runtime,
            win,
            &Rect {
                x: 0,
                y: mon_mh - 20,
                w: 0,
                h: 0,
            },
            10,
            0,
        );
    }

    let wmatom_delete = ctx_x11.x11_runtime.wmatom.delete;
    force_close_x11(ctx_x11, win, wmatom_delete);
}

/// Kill the given window.
pub fn kill_client(ctx: &mut WmCtx, win: WindowId) {
    match ctx {
        WmCtx::X11(ref mut c) => kill_client_x11(c, win),
        WmCtx::Wayland(c) => {
            let _ = c.wayland.backend.close_window(win);
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
    let has_clients = !ctx.g().selected_monitor().clients.is_empty();

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

/// Close an arbitrary window by its Window ID (X11-specific).
fn close_win_x11(ctx_x11: &mut WmCtxX11<'_>, win: WindowId) {
    let is_locked = ctx_x11
        .core
        .g
        .clients
        .get(&win)
        .map(|c| c.is_locked)
        .unwrap_or(true);

    if is_locked {
        return;
    }

    // Animation not yet supported in X11-specific path
    let wmatom_delete = ctx_x11.x11_runtime.wmatom.delete;
    force_close_x11(ctx_x11, win, wmatom_delete);
}

/// Close an arbitrary window by its Window ID.
pub fn close_win(ctx: &mut WmCtx, win: WindowId) {
    match ctx {
        WmCtx::X11(ref mut c) => close_win_x11(c, win),
        WmCtx::Wayland(c) => {
            let _ = c.wayland.backend.close_window(win);
        }
    }
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
