//! Client lifecycle shared helpers.
//!
//! Backend-specific manage/unmanage logic lives under backend modules.

use crate::globals::Globals;
use crate::types::WindowId;

/// Backend-agnostic cleanup for a client that is being destroyed or unmanaged.
/// This detaches the window from the monitor's stack and client list, clears
/// any overlay/fullscreen references, and removes it from the global client map.
/// It also recalculates the fallback focus for the monitor if the window was selected.
pub fn unmanage_client_state(g: &mut Globals, win: WindowId) {
    for mon in g.monitors_iter_all_mut() {
        if mon.overlay == Some(win) {
            mon.overlay = None;
        }
        if mon.fullscreen == Some(win) {
            mon.fullscreen = None;
        }
    }

    if g.clients.contains_key(&win) {
        g.detach(win);
        g.detach_stack(win);
        g.clients.remove(&win);
    }
}

/// Initial tag mask for a newly managed client on `monitor_id`.
///
/// This mirrors DWM semantics: a new client appears on all tags currently
/// visible on its target monitor.
pub fn initial_tags_for_monitor(g: &Globals, monitor_id: usize) -> u32 {
    g.monitor(monitor_id)
        .map(|m| m.selected_tags())
        .filter(|tags| *tags != 0)
        .unwrap_or(1)
}
