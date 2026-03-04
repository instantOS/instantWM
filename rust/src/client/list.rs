//! Linked-list management for the per-monitor client lists.
//!
//! All list mutation is now delegated to `ClientManager` methods to maintain
//! invariants in one place.

use crate::contexts::WmCtx;
use crate::globals::Globals;
use crate::layouts::arrange;
use crate::types::WindowId;

// ---------------------------------------------------------------------------
// High-level orchestration (Flat re-exports)
// ---------------------------------------------------------------------------

pub fn attach(ctx: &mut WmCtx, win: WindowId) {
    let mut mgr = std::mem::take(&mut ctx.g.clients);
    mgr.attach(&mut ctx.g.monitors, win);
    ctx.g.clients = mgr;
}

pub fn detach(ctx: &mut WmCtx, win: WindowId) {
    let mut mgr = std::mem::take(&mut ctx.g.clients);
    mgr.detach(&mut ctx.g.monitors, win);
    ctx.g.clients = mgr;
}

pub fn attach_stack(ctx: &mut WmCtx, win: WindowId) {
    let mut mgr = std::mem::take(&mut ctx.g.clients);
    mgr.attach_stack(&mut ctx.g.monitors, win);
    ctx.g.clients = mgr;
}

pub fn detach_stack(ctx: &mut WmCtx, win: WindowId) {
    let mut mgr = std::mem::take(&mut ctx.g.clients);
    mgr.detach_stack(&mut ctx.g.monitors, win);
    ctx.g.clients = mgr;
}

// ---------------------------------------------------------------------------
// Traversal helpers
// ---------------------------------------------------------------------------

pub fn next_tiled(ctx: &WmCtx, start_win: Option<WindowId>) -> Option<WindowId> {
    let mut current = start_win;
    while let Some(win) = current {
        if let Some(c) = ctx.g.clients.get(&win) {
            let selected = c
                .mon_id
                .and_then(|mid| ctx.g.monitor(mid))
                .map(|m| m.selected_tags())
                .unwrap_or(0);
            if !c.isfloating && c.is_visible_on_tags(selected) && !c.is_hidden {
                return Some(win);
            }
            current = c.next;
        } else {
            break;
        }
    }
    None
}

/// Detach `win` from the client list and re-attach it at the front (master
/// position), then re-focus and re-arrange the monitor.
pub fn pop(ctx: &mut WmCtx, win: WindowId) {
    detach(ctx, win);
    attach(ctx, win);
    let mon_id = ctx.g.clients.get(&win).and_then(|c| c.mon_id);
    crate::focus::focus_soft(ctx, Some(win));

    if let Some(mid) = mon_id {
        arrange(ctx, Some(mid));
    }
}

// ---------------------------------------------------------------------------
// Lookup helpers
// ---------------------------------------------------------------------------

/// Returns `Some(win)` if `win` is a currently managed client, `None` otherwise.
pub fn win_to_client(g: &Globals, win: WindowId) -> Option<WindowId> {
    g.clients.win_to_client(win)
}
