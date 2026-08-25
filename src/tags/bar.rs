//! Tag bar rendering helpers.
//!
//! This module resolves which tags should be drawn, including tag-index
//! remapping, skip logic, display names, and estimated fallback widths.

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
    /// Total pixel width of this tag cell (text width + horizontal_padding).
    pub width: i32,
}

pub(crate) fn visible_tags<'a>(
    globals: &crate::core_state::CoreState,
    monitor: &'a Monitor,
    occupied: TagMask,
) -> Vec<VisibleTag<'a>> {
    let horizontal_padding = globals.derived.bar_horizontal_padding;
    let show_alt = globals.model.tags.show_alternative_names;
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
        let label = tag.display_name(show_alt);
        let width =
            ((label.chars().count() as i32) * 8 + horizontal_padding).max(horizontal_padding);

        out.push(VisibleTag {
            slot,
            tag_index,
            label,
            width,
        });
    }

    out
}
