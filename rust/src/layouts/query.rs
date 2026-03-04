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

// ── per-monitor counts ────────────────────────────────────────────────────────

// ── visibility walk ───────────────────────────────────────────────────────────

/// Walk the client list starting at `start_win` and return the first
/// client that passes [`Client::is_visible_on_tags`].
pub fn find_visible_client(g: &Globals, start_win: Option<WindowId>) -> Option<WindowId> {
    let selected = g.selected_monitor().selected_tags();

    let m = g.selected_monitor();
    let start_idx = start_win.and_then(|w| m.clients.iter().position(|&x| x == w));
    let iter_start = start_idx.map(|i| i + 1).unwrap_or(0);

    for i in iter_start..m.clients.len() {
        let win = m.clients[i];
        if let Some(c) = g.clients.get(&win) {
            if c.is_visible_on_tags(selected) {
                return Some(win);
            }
        }
    }

    None
}

// ── layout query ──────────────────────────────────────────────────────────────

/// Returns `hi` if animation is enabled and client count exceeds threshold,
/// otherwise returns `lo`.
pub fn framecount_for_layout(g: &Globals, threshold: usize, hi: i32, lo: i32) -> i32 {
    if g.animated && g.selected_monitor().tiled_client_count(&*g.clients) > threshold {
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
