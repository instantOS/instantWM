//! Layout-driven client operations.
//!
//! These are small operations that sit at the boundary between the client and
//! the layout engine.  They are collected here so that neither `geometry.rs`
//! nor the layout algorithms need to know about each other's internals.
//!
//! # Contents
//!
//! * [`zoom`] – promote the selected window to the master slot (or, if it
//!              already is master, promote the next tiled window instead).

use crate::client::list::{next_tiled_ctx, pop};
use crate::contexts::WmCtx;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;

// ---------------------------------------------------------------------------
// zoom
// ---------------------------------------------------------------------------

/// Promote the selected window to the master position.
///
/// In a tiling layout the "master" is the first client in the focus-order
/// list (the leftmost / largest slot, depending on the layout algorithm).
/// [`zoom`] moves the selected window there via [`pop`].
///
/// # Edge cases
///
/// * Does nothing when the current layout is not a tiling layout, or when the
///   selected client is floating.
/// * When the selected window **is already** the master, the *next* tiled
///   window is promoted instead (if one exists).  If there is no next tiled
///   window the function returns early.
pub fn zoom(ctx: &mut WmCtx) {
    let Some(win) = ctx.g.selected_win() else {
        return;
    };

    // Raise the window immediately so it appears on top while the layout
    // catches up on the next arrange pass.
    let conn = ctx.x11.conn;
    let _ = conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
    let _ = conn.flush();

    let (is_floating, mon_id) = {
        ctx.g
            .clients
            .get(&win)
            .map(|c| (c.isfloating, c.mon_id))
            .unwrap_or((true, None))
    };

    // Only meaningful in a tiling layout with a non-floating window.
    let is_tiling = mon_id
        .and_then(|mid| ctx.g.monitor(mid))
        .map(|mon| mon.is_tiling_layout())
        .unwrap_or(false);

    if !is_tiling || is_floating {
        return;
    }

    // Find the current master (first tiled client on the monitor).
    let first_tiled = mon_id
        .and_then(|mid| ctx.g.monitor(mid))
        .and_then(|mon| next_tiled_ctx(ctx, mon.clients));

    if first_tiled == Some(win) {
        // The selected window is already master – promote the next one.
        let after_first = first_tiled.and_then(|f| ctx.g.clients.get(&f).and_then(|c| c.next));
        let next = next_tiled_ctx(ctx, after_first);

        // Nothing to promote if there is only one tiled window.
        if next.is_none() {
            return;
        }
    }

    pop(ctx, win);
}
