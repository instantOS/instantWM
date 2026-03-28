//! Client lifecycle shared helpers.
//!
//! Backend-specific manage/unmanage logic lives under backend modules.

use crate::globals::Globals;
use crate::types::{TagMask, WindowId};

/// Initial tag mask for a newly managed client on `monitor_id`.
///
/// This mirrors DWM semantics: a new client appears on all tags currently
/// visible on its target monitor.
pub fn initial_tags_for_monitor(g: &Globals, monitor_id: usize) -> TagMask {
    g.monitor(monitor_id)
        .map(|m| m.selected_tags())
        .filter(|tags| !tags.is_empty())
        .unwrap_or(TagMask::single(1).unwrap_or(TagMask::EMPTY))
}

/// Select `win` on its assigned monitor.
///
/// This is WM policy, not backend policy: backends may discover a new window
/// or an activation request, but the choice to make that window the monitor's
/// selected client lives in shared state.
pub fn select_client(g: &mut Globals, win: WindowId) {
    let Some(monitor_id) = g.clients.monitor_id(win) else {
        return;
    };
    if let Some(mon) = g.monitor_mut(monitor_id) {
        mon.sel = Some(win);
    }
}
