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
    /// Display label (name, or icon while icon mode is active).
    pub label: &'a str,
}

pub(crate) fn visible_tags(
    monitor: &Monitor,
    occupied: TagMask,
    show_icons: bool,
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

        let names: Vec<_> = visible_tags(&monitor, TagMask::all(2), false)
            .into_iter()
            .map(|tag| tag.label.to_string())
            .collect();
        assert_eq!(names, vec!["web", "mail"]);

        let icons: Vec<_> = visible_tags(&monitor, TagMask::all(2), true)
            .into_iter()
            .map(|tag| tag.label.to_string())
            .collect();
        assert_eq!(icons, vec!["W", "mail"]);
    }
}
