//! Tag bar rendering helpers.
//!
//! This module resolves which tags should be drawn, including tag-index
//! remapping, skip logic, and display names.

use crate::types::{Monitor, TagMask};

/// Maximum number of tag slots rendered in the bar.
const MAX_BAR_SLOTS: usize = 9;

/// A tag that should be drawn in the bar, with all derived data pre-computed.
pub(crate) struct VisibleTag<'a> {
    /// Slot index (0..MAX_BAR_SLOTS-1). Used for hover/gesture matching.
    pub slot: usize,
    /// Actual tag index into `monitor.tags` / bitmask space.
    pub tag_index: usize,
    /// Display label (regular or alt name).
    pub label: &'a str,
}

pub(crate) fn visible_tags(
    monitor: &Monitor,
    occupied: TagMask,
    show_alt: bool,
) -> Vec<VisibleTag<'_>> {
    let slot_count = monitor.tags.len().min(MAX_BAR_SLOTS);

    let mut out = Vec::with_capacity(slot_count);
    for slot in 0..slot_count {
        let tag_index = monitor.tag_index_for_slot(slot);
        if tag_index >= monitor.tags.len() {
            continue;
        }
        if monitor.should_hide_tag(tag_index, occupied) {
            continue;
        }

        let tag = &monitor.tags[tag_index];
        out.push(VisibleTag {
            slot,
            tag_index,
            label: tag.display_name(show_alt),
        });
    }

    out
}
