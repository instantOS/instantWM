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

use crate::animation::animate_client;
use crate::client::focus::{send_event, ANIM_CLIENT};
use crate::contexts::WmCtx;
use crate::types::Rect;
use std::sync::atomic::Ordering;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, Window};
use x11rb::CURRENT_TIME;

// ---------------------------------------------------------------------------
// kill_client
// ---------------------------------------------------------------------------

/// Kill the given window.
///
/// Steps:
/// 1. Return immediately if the window is locked (`islocked`).
/// 2. Play a "slide down" animation (skipped when already animating or
///    the window is fullscreen).
/// 3. Send `WM_DELETE_WINDOW`; if unsupported, force-kill via X.
pub fn kill_client(ctx: &mut WmCtx, win: Window) {
    let Some(client) = ctx.g.clients.get(&win) else {
        return;
    };

    if client.islocked {
        return;
    }

    let is_fullscreen = client.is_fullscreen;
    let mon_mh = client
        .mon_id
        .and_then(|mid| ctx.g.monitors.get(mid))
        .map(|m| m.monitor_rect.h)
        .unwrap_or(0);

    let animated = ctx.g.animated;
    let anim_client = ANIM_CLIENT.load(Ordering::Relaxed);

    if animated && win != anim_client && !is_fullscreen {
        ANIM_CLIENT.store(win, Ordering::Relaxed);
        animate_client(
            ctx,
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

    let wmatom_delete = ctx.g.cfg.wmatom.delete;
    force_close(ctx, win, wmatom_delete);
}

/// Return the selected window for the current monitor.
pub fn selected_window(ctx: &WmCtx) -> Option<Window> {
    ctx.g.monitors.get(ctx.g.selmon).and_then(|mon| mon.sel)
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
    let has_clients = ctx
        .g
        .monitors
        .get(ctx.g.selmon)
        .is_some_and(|m| m.clients.is_some());

    if has_clients {
        if let Some(win) = selected_window(ctx) {
            kill_client(ctx, win);
        }
    } else {
        crate::util::spawn(ctx, crate::config::commands::Cmd::InstantShutdown);
    }
}

// ---------------------------------------------------------------------------
// close_win
// ---------------------------------------------------------------------------

/// Close an arbitrary window by its Window ID.
///
/// Unlike [`kill_client`] this targets any window, not just the selected one.
/// Used by the per-client close button in the bar.
///
/// The window is still animated before the close message is sent.
pub fn close_win(ctx: &mut WmCtx, win: Window) {
    let (is_locked, mon_mh) = ctx
        .g
        .clients
        .get(&win)
        .map(|c| {
            let mh = c
                .mon_id
                .and_then(|mid| ctx.g.monitors.get(mid))
                .map(|m| m.monitor_rect.h)
                .unwrap_or(0);
            (c.islocked, mh)
        })
        .unwrap_or((true, 0));

    if is_locked {
        return;
    }

    animate_client(
        ctx,
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

    let wmatom_delete = ctx.g.cfg.wmatom.delete;
    force_close(ctx, win, wmatom_delete);
}

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

/// Attempt a graceful `WM_DELETE_WINDOW`, falling back to `XKillClient`.
fn force_close(ctx: &mut WmCtx, win: Window, wmatom_delete: u32) {
    let sent = send_event(
        ctx,
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
        if true { let conn = ctx.x11.conn;
            let _ = conn.grab_server();
            let _ = conn.kill_client(win);
            let _ = conn.flush();
            let _ = conn.ungrab_server();
        }
    }
}
