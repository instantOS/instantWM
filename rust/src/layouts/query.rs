#![allow(dead_code)]
//! Layout query helpers — stateless reads over the global window/monitor state.
//!
//! These functions answer questions like "how many tiled clients are on the
//! selected monitor?" or "what is the active layout?" without mutating
//! any state.  They are kept separate from the arrange/restack machinery so
//! that both the algorithm modules and the manager can depend on them without
//! creating circular imports.

use crate::globals::Globals;
use crate::types::{Monitor, WindowId};

use super::LayoutKind;

// ── per-monitor counts ────────────────────────────────────────────────────────

/// Number of tiled, visible clients on the *selected* monitor.
pub fn client_count(g: &Globals) -> i32 {
    let mon = match g.selmon() {
        Some(m) => m,
        None => return 0,
    };

    let selected = mon.selected_tags();
    let mut count = 0;
    for (_win, c) in mon.iter_clients(&g.clients) {
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

    for (_win, c) in m.iter_clients(&g.clients) {
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
    let selected = g.selmon().map(|m| m.selected_tags()).unwrap_or(0);
    for (win, c) in crate::types::ClientListIter::new(start_win, &g.clients) {
        if c.is_visible_on_tags(selected) {
            return Some(win);
        }
    }

    None
}

// ── layout query ──────────────────────────────────────────────────────────────

/// Return the active layout for monitor `m`.
///
/// Reads from the monitor's own `tags` list. Falls back to `Tile` when the
/// tag index is out of range (e.g. monitor not yet fully initialised).
pub fn get_current_layout(_g: &Globals, m: &Monitor) -> LayoutKind {
    let tag = m.current_tag;

    if tag > 0 && tag <= m.tags.len() {
        m.tags[tag - 1].layouts.get_layout()
    } else {
        LayoutKind::Tile
    }
}

/// Return the active layout symbol string for the *selected* monitor.
pub fn get_current_layout_symbol(g: &Globals) -> Option<&'static str> {
    if let Some(m) = g.selmon() {
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
    match g.selmon() {
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
