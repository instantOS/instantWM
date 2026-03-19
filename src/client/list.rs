//! Linked-list management for the per-monitor client lists.
//!
//! All list mutation is now delegated to `ClientManager` methods to maintain
//! invariants in one place.

use crate::contexts::WmCtx;
use crate::layouts::arrange;
use crate::types::WindowId;

// ---------------------------------------------------------------------------
// Traversal helpers
// ---------------------------------------------------------------------------

pub fn next_tiled(ctx: &WmCtx, start_win: Option<WindowId>) -> Option<WindowId> {
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

/// Detach `win` from the client list and re-attach it at the front (master
/// position), then re-focus and re-arrange the monitor.
pub fn pop(ctx: &mut WmCtx, win: WindowId) {
    ctx.g_mut().detach(win);
    ctx.g_mut().attach(win);
    let monitor_id = ctx.g().clients.monitor_id(win);
    crate::focus::focus_soft(ctx, Some(win));

    if let Some(mid) = monitor_id {
        arrange(ctx, Some(mid));
    }
}
