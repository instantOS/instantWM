//! Client lifecycle shared helpers.
//!
//! Backend-specific manage/unmanage logic lives under backend modules.

use crate::globals::Globals;
use crate::types::TagMask;

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
