//! Tag bar rendering helpers.
//!
//! This module resolves which tags should be drawn, including tag-index
//! remapping, skip logic, and display labels. The number of cells comes
//! from the per-output policy (`bar.tag_slots`, overridable per output in
//! `[monitors.<name>]`); this module stays stateless.

use crate::types::{Monitor, TagMask};

/// A tag that should be drawn in the bar, with all derived data pre-computed.
pub(crate) struct VisibleTag<'a> {
    /// Slot index (0..slots-1). Used for hover/gesture matching.
    pub slot: usize,
    /// Actual tag index into `monitor.tags` / bitmask space.
    pub tag_index: usize,
    /// Display label (name, or icon while icon mode is active).
    pub label: &'a str,
}

pub(crate) fn visible_tags(
    monitor: &Monitor,
    occupied: TagMask,
    show_icons: bool,
    slots: usize,
) -> Vec<VisibleTag<'_>> {
    let slot_count = monitor.tags.len().min(slots.max(1));

    let mut out = Vec::with_capacity(slot_count);
    for slot in 0..slot_count {
        let tag_index = monitor.tag_index_for_slot(slot, slots);
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
            label: tag.display_label(show_icons),
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Tag;

    fn monitor_with(tags: &[(&str, &str)]) -> Monitor {
        Monitor {
            tags: tags
                .iter()
                .map(|(name, icon)| Tag {
                    name: (*name).to_string(),
                    icon: (*icon).to_string(),
                })
                .collect(),
            ..Monitor::default()
        }
    }

    #[test]
    fn labels_follow_icon_mode_and_fall_back_per_tag() {
        let monitor = monitor_with(&[("web", "W"), ("mail", "")]);

        let names: Vec<_> = visible_tags(&monitor, TagMask::all(2), false, 9)
            .into_iter()
            .map(|tag| tag.label.to_string())
            .collect();
        assert_eq!(names, vec!["web", "mail"]);

        let icons: Vec<_> = visible_tags(&monitor, TagMask::all(2), true, 9)
            .into_iter()
            .map(|tag| tag.label.to_string())
            .collect();
        assert_eq!(icons, vec!["W", "mail"]);
    }

    #[test]
    fn the_cell_count_is_capped_by_the_configured_slots() {
        let monitor = monitor_with(&[("a", ""), ("b", ""), ("c", ""), ("d", "")]);

        let all: Vec<_> = visible_tags(&monitor, TagMask::all(4), false, 9)
            .into_iter()
            .map(|tag| tag.label.to_string())
            .collect();
        assert_eq!(all, vec!["a", "b", "c", "d"]);

        let two: Vec<_> = visible_tags(&monitor, TagMask::all(4), false, 2)
            .into_iter()
            .map(|tag| tag.label.to_string())
            .collect();
        assert_eq!(two, vec!["a", "b"]);
    }

    #[test]
    fn the_last_cell_becomes_the_current_tag_when_the_set_is_wider() {
        let mut monitor = monitor_with(&[("1", ""), ("2", ""), ("3", ""), ("4", "")]);
        monitor.set_selected_tags(TagMask::single(4).unwrap());

        let labels: Vec<_> = visible_tags(&monitor, TagMask::all(4), false, 3)
            .into_iter()
            .map(|tag| tag.label.to_string())
            .collect();
        // Slots 0 and 1 stay tags 1 and 2; the overflow cell shows tag 4.
        assert_eq!(labels, vec!["1", "2", "4"]);

        // With enough cells every tag gets its own.
        let labels: Vec<_> = visible_tags(&monitor, TagMask::all(4), false, 4)
            .into_iter()
            .map(|tag| tag.label.to_string())
            .collect();
        assert_eq!(labels, vec!["1", "2", "3", "4"]);
    }
}
