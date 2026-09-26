//! Tag bar rendering helpers.
//!
//! This module resolves the leading tag baseline, occupied and selected tags,
//! and the next empty tag. The baseline comes from the per-output policy
//! (`bar.tag_slots`, overridable in `[monitors.<name>]`).

use crate::types::{Monitor, TagMask};

/// A tag that should be drawn in the bar, with all derived data pre-computed.
pub(crate) struct VisibleTag<'a> {
    /// Actual tag index into `monitor.tags` / bitmask space.
    pub tag_index: usize,
    /// Display label (name, or icon while icon mode is active).
    pub label: &'a str,
}

pub(crate) fn visible_tags(
    monitor: &Monitor,
    occupied: TagMask,
    show_icons: bool,
    baseline: usize,
) -> Vec<VisibleTag<'_>> {
    let count = monitor.tags.len();
    let selected = monitor.selected_tags();
    let mut indices: Vec<usize> = (0..count)
        .filter(|&index| {
            let number = index + 1;
            occupied.contains(number) || selected.contains(number) || index < baseline
        })
        .collect();

    // Keep one empty workspace available when all baseline tags are occupied.
    // A selected empty tag farther right does not replace the first empty
    // immediately after the baseline.
    let baseline_has_visible_empty =
        (0..count.min(baseline)).any(|index| !occupied.contains(index + 1));
    if !baseline_has_visible_empty
        && let Some(index) =
            (baseline.min(count)..count).find(|&index| !occupied.contains(index + 1))
    {
        match indices.binary_search(&index) {
            Ok(_) => {}
            Err(position) => indices.insert(position, index),
        }
    }

    let mut out = Vec::with_capacity(indices.len());
    for tag_index in indices {
        let tag = &monitor.tags[tag_index];
        out.push(VisibleTag {
            tag_index,
            label: tag.display_label(show_icons),
        });
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::MonitorBuilder;
    use crate::types::Tag;

    fn monitor_with(tags: &[(&str, &str)]) -> Monitor {
        MonitorBuilder::new()
            .configure(|monitor| {
                monitor.tags = tags
                    .iter()
                    .map(|(name, icon)| Tag {
                        name: (*name).to_string(),
                        icon: (*icon).to_string(),
                    })
                    .collect();
            })
            .build()
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
    fn the_baseline_shows_leading_tags() {
        let monitor = monitor_with(&[("a", ""), ("b", ""), ("c", ""), ("d", "")]);

        let all: Vec<_> = visible_tags(&monitor, TagMask::EMPTY, false, 9)
            .into_iter()
            .map(|tag| tag.label.to_string())
            .collect();
        assert_eq!(all, vec!["a", "b", "c", "d"]);

        let two: Vec<_> = visible_tags(&monitor, TagMask::EMPTY, false, 2)
            .into_iter()
            .map(|tag| tag.label.to_string())
            .collect();
        assert_eq!(two, vec!["a", "b"]);
    }

    #[test]
    fn occupied_and_selected_tags_are_shown_beyond_the_baseline() {
        let mut monitor = monitor_with(&[("1", ""), ("2", ""), ("3", ""), ("4", "")]);
        monitor.set_selected_tags(TagMask::single(4).unwrap());

        let labels: Vec<_> = visible_tags(&monitor, TagMask::single(3).unwrap(), false, 2)
            .into_iter()
            .map(|tag| tag.label.to_string())
            .collect();
        assert_eq!(labels, vec!["1", "2", "3", "4"]);
    }

    #[test]
    fn the_first_empty_tag_after_full_baseline_is_shown() {
        let monitor = monitor_with(&[("1", ""), ("2", ""), ("3", ""), ("4", "")]);
        let labels: Vec<_> = visible_tags(&monitor, TagMask::all(2), false, 2)
            .into_iter()
            .map(|tag| tag.label.to_string())
            .collect();
        assert_eq!(labels, vec!["1", "2", "3"]);
    }

    #[test]
    fn five_occupied_baseline_tags_expose_tag_six() {
        let monitor = monitor_with(&[
            ("1", ""),
            ("2", ""),
            ("3", ""),
            ("4", ""),
            ("5", ""),
            ("6", ""),
        ]);
        let indices: Vec<_> = visible_tags(&monitor, TagMask::all(5), false, 5)
            .into_iter()
            .map(|tag| tag.tag_index)
            .collect();
        assert_eq!(indices, vec![0, 1, 2, 3, 4, 5]);
    }

    #[test]
    fn occupied_tag_beyond_baseline_is_visible_in_tag_order() {
        let monitor = monitor_with(&[
            ("1", ""),
            ("2", ""),
            ("3", ""),
            ("4", ""),
            ("5", ""),
            ("6", ""),
        ]);
        let indices: Vec<_> = visible_tags(&monitor, TagMask::single(6).unwrap(), false, 2)
            .into_iter()
            .map(|tag| tag.tag_index)
            .collect();
        assert_eq!(indices, vec![0, 1, 5]);
    }

    #[test]
    fn next_empty_tag_is_after_the_baseline_even_with_later_occupied_tags() {
        let monitor = monitor_with(&[
            ("1", ""),
            ("2", ""),
            ("3", ""),
            ("4", ""),
            ("5", ""),
            ("6", ""),
        ]);
        let occupied = TagMask::all(2) | TagMask::single(5).unwrap();
        let indices: Vec<_> = visible_tags(&monitor, occupied, false, 2)
            .into_iter()
            .map(|tag| tag.tag_index)
            .collect();
        assert_eq!(indices, vec![0, 1, 2, 4]);
    }

    #[test]
    fn a_selected_empty_tag_farther_right_does_not_replace_the_next_empty_tag() {
        let mut monitor = monitor_with(&[("1", ""), ("2", ""), ("3", ""), ("4", "")]);
        monitor.set_selected_tags(TagMask::single(4).unwrap());
        let tags = visible_tags(&monitor, TagMask::all(2), false, 2);
        assert_eq!(
            tags.iter().map(|tag| tag.tag_index).collect::<Vec<_>>(),
            vec![0, 1, 2, 3]
        );
    }
}
