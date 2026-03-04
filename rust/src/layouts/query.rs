#![allow(dead_code)]
//! Layout query helpers — stateless reads over the global window/monitor state.
//!
//! These functions answer questions like "how many tiled clients are on the
//! selected monitor?" or "what is the active layout?" without mutating
//! any state.  They are kept separate from the arrange/restack machinery so
//! that both the algorithm modules and the manager can depend on them without
//! creating circular imports.

use crate::client::next_tiled;
use crate::contexts::WmCtx;
use crate::globals::Globals;
use crate::types::{Monitor, WindowId};

use super::LayoutKind;

// ── per-monitor counts ────────────────────────────────────────────────────────

/// Number of tiled, visible clients on the *selected* monitor.
pub fn client_count(g: &Globals) -> i32 {
    let mon = match g.selected_monitor() {
        Some(m) => m,
        None => return 0,
    };

    let selected = mon.selected_tags();
    let mut count = 0;
    for (_win, c) in mon.iter_clients(g.clients.map()) {
        if c.is_visible_on_tags(selected) && !c.isfloating && !c.is_hidden {
            count += 1;
        }
    }

    count
}

/// Number of tiled, visible clients on an *arbitrary* monitor `m`.
pub fn client_count_mon(g: &Globals, m: &Monitor) -> i32 {
    let selected = m.selected_tags();
    let mut count = 0;

    for (_win, c) in m.iter_clients(g.clients.map()) {
        if c.is_visible_on_tags(selected) && !c.isfloating && !c.is_hidden {
            count += 1;
        }
    }

    count
}

/// Total number of tracked clients across *all* monitors and tags.
pub fn all_client_count(g: &Globals) -> i32 {
    g.clients.len() as i32
}

// ── visibility walk ───────────────────────────────────────────────────────────

/// Walk the client linked-list starting at `start_win` and return the first
/// client that passes [`Client::is_visible_on_tags`].
pub fn find_visible_client(g: &Globals, start_win: Option<WindowId>) -> Option<WindowId> {
    let selected = g.selected_monitor().map(|m| m.selected_tags()).unwrap_or(0);
    for (win, c) in crate::types::ClientListIter::new(start_win, g.clients.map()) {
        if c.is_visible_on_tags(selected) {
            return Some(win);
        }
    }

    None
}

// ── layout query ──────────────────────────────────────────────────────────────

/// Return the active layout symbol string for the *selected* monitor.
pub fn get_current_layout_symbol(g: &Globals) -> Option<&'static str> {
    if let Some(m) = g.selected_monitor() {
        let tag = m.current_tag;
        if tag > 0 && tag <= m.tags.len() {
            return Some(m.tags[tag - 1].layouts.symbol());
        }
    }

    Some(LayoutKind::Tile.symbol())
}

/// Returns `true` when the active layout for the *selected* monitor is
/// a tiling layout.
pub fn selmon_has_tiling_layout(g: &Globals) -> bool {
    match g.selected_monitor() {
        Some(m) => {
            let tag = m.current_tag;
            if tag > 0 && tag <= m.tags.len() {
                m.tags[tag - 1].layouts.is_tiling()
            } else {
                true
            }
        }
        None => false,
    }
}

/// Returns `hi` if animation is enabled and client count exceeds threshold,
/// otherwise returns `lo`.
pub fn framecount_for_layout(g: &Globals, threshold: usize, hi: i32, lo: i32) -> i32 {
    if g.animated && client_count(g) > threshold as i32 {
        hi
    } else {
        lo
    }
}

/// Counts tiled clients by walking the linked list using `next_tiled`.
pub fn count_tiled_clients(ctx: &WmCtx, mon: &Monitor) -> u32 {
    let mut count = 0;
    let mut c_win = mon
        .clients
        .first()
        .copied()
        .and_then(|w| next_tiled(ctx, Some(w)));
    while let Some(win) = c_win {
        count += 1;
        c_win = next_tiled(ctx, Some(win));
    }
    count
}
