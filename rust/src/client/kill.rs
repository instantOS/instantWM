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
//!                      ID packed into an [`Arg`].  Used by the close button
//!                      drawn in the bar.
//!
//! # Graceful vs. forceful termination
//!
//! The WM first attempts to send a `WM_DELETE_WINDOW` `ClientMessage`.  If
//! [`send_event`] returns `false` (the window does not support the protocol),
//! we fall back to `conn.kill_client()` wrapped in a server grab so that no
//! other requests from the dying client are processed between the kill and the
//! expected `DestroyNotify`.

use crate::animation::animate_client_rect;
use crate::client::focus::{send_event, ANIM_CLIENT};
use crate::globals::{get_globals, get_x11};
use crate::types::{Arg, Rect};
use std::sync::atomic::Ordering;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::CURRENT_TIME;

// ---------------------------------------------------------------------------
// kill_client
// ---------------------------------------------------------------------------

/// Kill the currently selected window.
///
/// Steps:
/// 1. Return immediately if the window is locked (`islocked`).
/// 2. Play a "slide down" animation (skipped when already animating or
///    the window is fullscreen).
/// 3. Send `WM_DELETE_WINDOW`; if unsupported, force-kill via X.
pub fn kill_client(_arg: &Arg) {
    let globals = get_globals();
    let Some(win) = globals.monitors.get(globals.selmon).and_then(|m| m.sel) else {
        return;
    };

    let Some(client) = globals.clients.get(&win) else {
        return;
    };

    if client.islocked {
        return;
    }

    let is_fullscreen = client.is_fullscreen;
    let mon_mh = client
        .mon_id
        .and_then(|mid| globals.monitors.get(mid))
        .map(|m| m.monitor_rect.h)
        .unwrap_or(0);

    let animated = globals.animated;
    let anim_client = ANIM_CLIENT.load(Ordering::Relaxed);

    if animated && win != anim_client && !is_fullscreen {
        ANIM_CLIENT.store(win, Ordering::Relaxed);
        animate_client_rect(
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

    let wmatom_delete = globals.wmatom.delete;
    force_close(win, wmatom_delete);
}

// ---------------------------------------------------------------------------
// shut_kill
// ---------------------------------------------------------------------------

/// Kill the selected window, or shut down the session if there are no clients.
///
/// This is bound to the "power" key: pressing it on an empty monitor triggers
/// an orderly system shutdown; pressing it when windows are open closes the
/// focused window instead.
pub fn shut_kill(arg: &Arg) {
    let globals = get_globals();
    let has_clients = globals
        .monitors
        .get(globals.selmon)
        .is_some_and(|m| m.clients.is_some());

    if has_clients {
        kill_client(arg);
    } else {
        let shut_arg = Arg {
            v: Some(crate::config::commands::Cmd::InstantShutdown as usize),
            ..Default::default()
        };
        crate::util::spawn(&shut_arg);
    }
}

// ---------------------------------------------------------------------------
// close_win
// ---------------------------------------------------------------------------

/// Close an arbitrary window whose ID is packed into `arg.v`.
///
/// Unlike [`kill_client`] this targets any window, not just the selected one.
/// Used by the per-client close button in the bar.
///
/// The window is still animated before the close message is sent.
pub fn close_win(arg: &Arg) {
    let win = match arg.v {
        Some(ptr) => ptr as u32,
        None => return,
    };

    let globals = get_globals();
    let (is_locked, mon_mh) = globals
        .clients
        .get(&win)
        .map(|c| {
            let mh = c
                .mon_id
                .and_then(|mid| globals.monitors.get(mid))
                .map(|m| m.monitor_rect.h)
                .unwrap_or(0);
            (c.islocked, mh)
        })
        .unwrap_or((true, 0));

    if is_locked {
        return;
    }

    animate_client_rect(
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

    let wmatom_delete = get_globals().wmatom.delete;
    force_close(win, wmatom_delete);
}

// ---------------------------------------------------------------------------
// Shared helper
// ---------------------------------------------------------------------------

/// Attempt a graceful `WM_DELETE_WINDOW`, falling back to `XKillClient`.
fn force_close(win: u32, wmatom_delete: u32) {
    let sent = send_event(
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
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = conn.grab_server();
            let _ = conn.kill_client(win);
            let _ = conn.flush();
            let _ = conn.ungrab_server();
        }
    }
}
