#![allow(dead_code)]
//! Layout-driven client operations.
//!
//! These are small operations that sit at the boundary between the client and
//! the layout engine.  They are collected here so that neither `geometry.rs`
//! nor the layout algorithms need to know about each other's internals.

use crate::backend::BackendOps;
use crate::contexts::WmCtx;
use crate::focus::focus_soft;
use crate::layouts::arrange;

// ---------------------------------------------------------------------------
// zoom
// ---------------------------------------------------------------------------

fn next_tiled(
    ctx: &WmCtx,
    start_win: Option<crate::types::WindowId>,
) -> Option<crate::types::WindowId> {
    let mon = ctx.g().selected_monitor();
    let selected = mon.selected_tags();

    let start_idx = if let Some(win) = start_win {
        mon.clients.iter().position(|&w| w == win)
    } else {
        None
    };

    let clients = &mon.clients;
    let iter_start = start_idx.map(|i| i + 1).unwrap_or(0);

    for &win in clients.iter().skip(iter_start) {
        if let Some(c) = ctx.client(win)
            && !c.is_floating
            && c.is_visible_on_tags(selected)
            && !c.is_hidden
        {
            return Some(win);
        }
    }
    None
}

fn pop(ctx: &mut WmCtx, win: crate::types::WindowId) {
    ctx.g_mut().detach(win);
    ctx.g_mut().attach(win);
    let monitor_id = ctx.g().clients.monitor_id(win);
    focus_soft(ctx, Some(win));

    if let Some(mid) = monitor_id {
        arrange(ctx, Some(mid));
    }
}

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
    let Some(win) = ctx.selected_client() else {
        return;
    };

    // Raise the window immediately so it appears on top while the layout
    // catches up on the next arrange pass.
    ctx.backend().raise_window(win);
    ctx.backend().flush();

    let (is_floating, monitor_id) = ctx
        .g()
        .clients
        .get(&win)
        .map(|c| (c.is_floating, c.monitor_id))
        .unwrap_or((true, 0));

    // Only meaningful in a tiling layout with a non-floating window.
    let is_tiling = ctx
        .g()
        .monitor(monitor_id)
        .map(|mon| mon.is_tiling_layout())
        .unwrap_or(false);

    if !is_tiling || is_floating {
        return;
    }

    // Find the current master (first tiled client on the monitor).
    let first_on_monitor = ctx
        .g()
        .monitor(monitor_id)
        .and_then(|mon| mon.clients.first().copied());
    let first_tiled = first_on_monitor.and_then(|w| next_tiled(ctx, Some(w)));

    if first_tiled == Some(win) {
        let next = next_tiled(ctx, first_tiled);

        // Nothing to promote if there is only one tiled window.
        if next.is_none() {
            return;
        }
    }

    pop(ctx, win);
}
