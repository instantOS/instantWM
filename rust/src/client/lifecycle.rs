//! Client lifecycle shared helpers.
//!
//! Backend-specific manage/unmanage logic lives under backend modules.

use crate::globals::Globals;

/// Initial tag mask for a newly managed client on `monitor_id`.
///
/// This mirrors DWM semantics: a new client appears on all tags currently
/// visible on its target monitor.
pub fn initial_tags_for_monitor(g: &Globals, monitor_id: Option<usize>) -> u32 {
    monitor_id
        .and_then(|mid| g.monitor(mid))
        .map(|m| m.selected_tags())
        .filter(|tags| *tags != 0)
        .unwrap_or(1)
}
