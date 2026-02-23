//! Small stateless helper functions used throughout the floating module.
//!
//! None of these functions mutate floating state – they only inspect it.

use crate::client::resize;
use crate::globals::get_globals;
use x11rb::protocol::xproto::Window;

// ── Layout query ─────────────────────────────────────────────────────────────

/// Returns `true` if the currently selected monitor has a tiling layout active.
///
/// Used as a guard throughout the floating module: floating-only operations
/// should be no-ops when a tiling layout is active and the window is not
/// explicitly floating.
pub fn has_tiling_layout() -> bool {
    let globals = get_globals();
    if let Some(mon) = globals.monitors.get(globals.selmon) {
        return crate::monitor::is_current_layout_tiling(mon, &globals.tags);
    }
    // No monitor → treat as tiling so we don't accidentally float things.
    true
}

// ── Per-client queries ────────────────────────────────────────────────────────

/// Returns `true` if the client should be treated as floating right now.
///
/// A client is considered floating when either:
/// - its `isfloating` flag is set, or
/// - no tiling layout is active on the selected monitor (all windows float in
///   floating-only layouts).
pub fn check_floating(win: Window) -> bool {
    let globals = get_globals();
    if let Some(client) = globals.clients.get(&win) {
        if client.isfloating {
            return true;
        }
        if let Some(mon) = globals.monitors.get(globals.selmon) {
            if !crate::monitor::is_current_layout_tiling(mon, &globals.tags) {
                return true;
            }
        }
    }
    false
}

/// Returns `true` if the client is visible on any monitor.
///
/// A client is visible when it belongs to the currently selected tagset of
/// the monitor it is assigned to.
///
/// Note: this mirrors `Client::is_visible` but operates by window ID rather
/// than by reference, which is convenient for call-sites that only hold a
/// `Window` handle.
//
// TODO: consider deduplicating with `Client::is_visible` in types.rs.
pub fn visible_client(win: Window) -> bool {
    let globals = get_globals();
    if let Some(client) = globals.clients.get(&win) {
        for (idx, mon) in globals.monitors.iter().enumerate() {
            if (client.tags & mon.tagset[mon.seltags as usize]) != 0 && client.mon_id == Some(idx) {
                return true;
            }
        }
    }
    false
}

// ── Geometry helpers ──────────────────────────────────────────────────────────

/// Nudge the client one pixel to the right and back, forcing a layout refresh.
///
/// This is a lightweight way to make the X server re-evaluate size hints and
/// repaint the window frame without triggering a full `arrange()` pass.  It is
/// used after restoring a saved geometry so the window manager picks up the
/// correct position.
pub fn apply_size(win: Window) {
    let geo = {
        let globals = get_globals();
        globals.clients.get(&win).map(|c| c.geo)
    };
    if let Some(mut rect) = geo {
        rect.x += 1;
        resize(win, &rect, false);
    }
}
