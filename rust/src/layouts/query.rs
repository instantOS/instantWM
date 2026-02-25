#![allow(dead_code)]
//! Layout query helpers — stateless reads over the global window/monitor state.
//!
//! These functions answer questions like "how many tiled clients are on the
//! selected monitor?" or "what is the active layout?" without mutating
//! any state.  They are kept separate from the arrange/restack machinery so
//! that both the algorithm modules and the manager can depend on them without
//! creating circular imports.

use crate::globals::Globals;
use crate::types::Monitor;
use x11rb::protocol::xproto::Window;

use super::LayoutKind;

// ── per-monitor counts ────────────────────────────────────────────────────────

/// Number of tiled, visible clients on the *selected* monitor.
///
/// A client is counted only when it is:
/// - visible (passes [`Client::is_visible`]), and
/// - not floating (`!c.isfloating`).
///
/// This is the count used to tune animation frame-rates and decide whether
/// a single-client layout should remove borders.
pub fn client_count(g: &Globals) -> i32 {
    let mon = match g.monitors.get(g.selmon) {
        Some(m) => m,
        None => return 0,
    };

    // Match C's clientcountmon which uses nexttiled — hidden (iconic) clients
    // are excluded so the count reflects only clients that are actually tiled
    // and on-screen.  Using is_hidden here avoids inflating clientcount when
    // windows are minimized, which would break single-client border-stripping
    // and layout frame-rate heuristics.
    let mut count = 0;
    let selected = mon.selected_tags();
    let mut c_win = mon.clients;
    while let Some(win) = c_win {
        match g.clients.get(&win) {
            Some(c) => {
                if c.is_visible_on_tags(selected) && !c.isfloating && !c.is_hidden {
                    count += 1;
                }
                c_win = c.next;
            }
            None => break,
        }
    }

    count
}

/// Number of tiled, visible clients on an *arbitrary* monitor `m`.
///
/// Identical logic to [`client_count`] but works on any [`Monitor`] rather
/// than always picking the selected one.  Used by [`arrange_monitor`] to
/// update `m.clientcount` before invoking the layout algorithm.
///
/// [`arrange_monitor`]: crate::layouts::manager::arrange_monitor
pub fn client_count_mon(g: &Globals, m: &Monitor) -> i32 {
    let mut count = 0;
    let selected = m.selected_tags();

    // Mirror C's nexttiled-based clientcountmon: skip floating AND hidden clients
    // so that m.clientcount only reflects windows that the tiling layout will
    // actually place on screen.
    let mut c_win = m.clients;
    while let Some(win) = c_win {
        match g.clients.get(&win) {
            Some(c) => {
                if c.is_visible_on_tags(selected) && !c.isfloating && !c.is_hidden {
                    count += 1;
                }
                c_win = c.next;
            }
            None => break,
        }
    }

    count
}

/// Total number of tracked clients across *all* monitors and tags.
///
/// This is simply the length of `globals.clients` and is used by the
/// overview layout to size its grid.
pub fn all_client_count(g: &Globals) -> i32 {
    g.clients.len() as i32
}

// ── visibility walk ───────────────────────────────────────────────────────────

/// Walk the client linked-list starting at `start_win` and return the first
/// client that passes [`Client::is_visible`].
///
/// Returns `None` if the list is exhausted without finding a visible client.
pub fn find_visible_client(g: &Globals, start_win: Option<Window>) -> Option<Window> {
    let selected = g
        .monitors
        .get(g.selmon)
        .map(|m| m.selected_tags())
        .unwrap_or(0);
    let mut current = start_win;

    while let Some(win) = current {
        match g.clients.get(&win) {
            Some(c) => {
                if c.is_visible_on_tags(selected) {
                    return Some(win);
                }
                current = c.next;
            }
            None => break,
        }
    }

    None
}

// ── layout query ───────────────────────────────────────────────────────────────

/// Return the active layout for monitor `m`.
///
/// The layout is stored per-tag in `tags[current_tag - 1].layouts.get_layout()`.
/// Falls back to [`LayoutKind::Tile`] when tag index is invalid.
pub fn get_current_layout(g: &Globals, m: &Monitor) -> LayoutKind {
    let tag = m.current_tag;

    if tag > 0 && tag <= g.tags.tags.len() {
        g.tags.tags[tag - 1].layouts.get_layout()
    } else {
        LayoutKind::Tile
    }
}

/// Return the active layout symbol string for the *selected* monitor, or the
/// tile layout's symbol as a fallback.
///
/// Used by the bar renderer to display the current layout indicator.
pub fn get_current_layout_symbol(g: &Globals) -> Option<&'static str> {
    if let Some(m) = g.monitors.get(g.selmon) {
        let tag = m.current_tag;
        if tag > 0 && tag <= g.tags.tags.len() {
            return Some(g.tags.tags[tag - 1].layouts.symbol());
        }
    }

    Some(LayoutKind::Tile.symbol())
}

/// Returns `true` when the active layout for the *selected* monitor is
/// a tiling layout.
///
/// Used by `floating::has_tiling_layout` and a few other callers.
pub fn selmon_has_tiling_layout(g: &Globals) -> bool {
    match g.monitors.get(g.selmon) {
        Some(m) => {
            let tag = m.current_tag;
            if tag > 0 && tag <= g.tags.tags.len() {
                g.tags.tags[tag - 1].layouts.is_tiling()
            } else {
                true
            }
        }
        None => false,
    }
}
