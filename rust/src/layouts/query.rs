#![allow(dead_code)]
//! Layout query helpers — stateless reads over the global window/monitor state.
//!
//! These functions answer questions like "how many tiled clients are on the
//! selected monitor?" or "what is the active layout index?" without mutating
//! any state.  They are kept separate from the arrange/restack machinery so
//! that both the algorithm modules and the manager can depend on them without
//! creating circular imports.
//!
//! ## Contents
//!
//! | Function                  | Description                                              |
//! |---------------------------|----------------------------------------------------------|
//! [`client_count`]            | Tiled, visible clients on the selected monitor           |
//! [`client_count_mon`]        | Tiled, visible clients on a specific monitor             |
//! [`all_client_count`]        | Total number of tracked clients (all monitors/tags)      |
//! [`find_visible_client`]     | Walk a client linked-list, returning the first visible   |
//! [`get_current_layout_idx`]  | Active layout index for a monitor (via its current tag)  |
//! [`get_current_layout`]      | Active `&dyn Layout` for a monitor                       |

use crate::globals::get_globals;
use crate::types::{Layout, Monitor};
use x11rb::protocol::xproto::Window;

use super::TILE_LAYOUT;

// ── per-monitor counts ────────────────────────────────────────────────────────

/// Number of tiled, visible clients on the *selected* monitor.
///
/// A client is counted only when it is:
/// - visible (passes [`Client::is_visible`]), and
/// - not floating (`!c.isfloating`).
///
/// This is the count used to tune animation frame-rates and decide whether
/// a single-client layout should remove borders.
pub fn client_count() -> i32 {
    let g = get_globals();

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
    let mut c_win = mon.clients;
    while let Some(win) = c_win {
        match g.clients.get(&win) {
            Some(c) => {
                if c.is_visible() && !c.isfloating && !c.is_hidden {
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
pub fn client_count_mon(m: &Monitor) -> i32 {
    let g = get_globals();
    let mut count = 0;

    // Mirror C's nexttiled-based clientcountmon: skip floating AND hidden clients
    // so that m.clientcount only reflects windows that the tiling layout will
    // actually place on screen.
    let mut c_win = m.clients;
    while let Some(win) = c_win {
        match g.clients.get(&win) {
            Some(c) => {
                if c.is_visible() && !c.isfloating && !c.is_hidden {
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
pub fn all_client_count() -> i32 {
    get_globals().clients.len() as i32
}

// ── visibility walk ───────────────────────────────────────────────────────────

/// Walk the client linked-list starting at `start_win` and return the first
/// client that passes [`Client::is_visible`].
///
/// Returns `None` if the list is exhausted without finding a visible client.
pub fn find_visible_client(start_win: Option<Window>) -> Option<Window> {
    let g = get_globals();
    let mut current = start_win;

    while let Some(win) = current {
        match g.clients.get(&win) {
            Some(c) => {
                if c.is_visible() {
                    return Some(win);
                }
                current = c.next;
            }
            None => break,
        }
    }

    None
}

// ── layout index & trait-object resolution ────────────────────────────────────

/// Return the active layout index for monitor `m`.
///
/// The index is stored per-tag: `tags[current_tag - 1].ltidxs[active_layout_slot.as_index()]`.
/// Returns `None` when the tag index is out of range (e.g. during
/// initialisation before any tag has been selected).
pub fn get_current_layout_idx(m: &Monitor) -> Option<usize> {
    let g = get_globals();
    let tag = m.current_tag;

    if tag > 0 && tag <= g.tags.tags.len() {
        let t = &g.tags.tags[tag - 1];
        t.layout_indices.get(t.active_layout_slot)
    } else {
        None
    }
}

/// Return the active [`Layout`] trait object for monitor `m`.
///
/// Falls back to [`TILE_LAYOUT`] when the stored index is out of range or no
/// layout has been selected yet, so callers never have to deal with an
/// `Option`.
pub fn get_current_layout(m: &Monitor) -> &'static dyn Layout {
    let g = get_globals();
    let idx = get_current_layout_idx(m).unwrap_or(0);
    g.layouts.get(idx).copied().unwrap_or(&TILE_LAYOUT)
}

/// Return the active layout index for the *selected* monitor, or `None`.
///
/// Convenience wrapper used by the manager and command handlers that always
/// operate on `selmon`.
pub fn get_selmon_layout_idx() -> Option<usize> {
    let g = get_globals();
    let m = g.monitors.get(g.selmon)?;
    get_current_layout_idx(m)
}

/// Return the layout symbol string for the *selected* monitor, or the
/// first layout's symbol as a fallback.
///
/// Used by the bar renderer to display the current layout indicator.
pub fn get_current_layout_symbol() -> Option<&'static str> {
    let g = get_globals();

    if let Some(m) = g.monitors.get(g.selmon) {
        if let Some(idx) = get_current_layout_idx(m) {
            if let Some(layout) = g.layouts.get(idx) {
                return Some(layout.symbol());
            }
        }
    }

    // Fallback: first registered layout.
    g.layouts.first().map(|l| l.symbol())
}

/// Returns `true` when the active layout for the *selected* monitor is
/// a floating (non-tiling) layout.
///
/// Used by `floating::has_tiling_layout` and a few other callers.
pub fn selmon_has_tiling_layout() -> bool {
    let g = get_globals();
    match g.monitors.get(g.selmon) {
        Some(m) => get_current_layout(m).is_tiling(),
        None => false,
    }
}
