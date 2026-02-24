//! Type-safe tag operations.
//!
//! This module provides ergonomic wrappers around tag operations using
//! the new `TagMask` and `TagSelection` types.

use crate::globals::get_globals;
use crate::types::{MonitorDirection, TagMask, TagSelection};

/// View tags using a type-safe selection.
///
/// This is the preferred way to switch tag views as it provides
/// semantic meaning and type safety.
///
/// # Examples
///
/// ```
/// use crate::types::{TagMask, TagSelection};
/// use crate::tags::tag_ops;
///
/// // View a single tag
/// tag_ops::view_selection(TagSelection::Single(3));
///
/// // View all tags (overview)
/// tag_ops::view_selection(TagSelection::All);
///
/// // View specific tags
/// let mask = TagMask::single(1).unwrap() | TagMask::single(2).unwrap();
/// tag_ops::view_selection(TagSelection::Multi(mask));
/// ```
pub fn view_selection(selection: TagSelection) {
    let globals = get_globals();
    let num_tags = globals.tags.count();
    let current_mask = globals
        .monitors
        .get(globals.selmon)
        .map(|m| TagMask::from_bits(m.tagset[m.seltags as usize]))
        .unwrap_or(TagMask::EMPTY);
    let prev_tag = globals
        .monitors
        .get(globals.selmon)
        .map(|m| m.prev_tag)
        .unwrap_or(0);

    let mask = selection.to_mask(current_mask, prev_tag, num_tags);
    super::view(mask);
}

/// Shift the current view in a direction.
pub fn shift_view_direction(direction: super::shift::ShiftDirection) {
    use super::shift::ShiftDirection;
    use crate::types::Direction;

    let dir = match direction {
        ShiftDirection::Left => Direction::Left,
        ShiftDirection::Right => Direction::Right,
    };
    super::shift_view(dir);
}

/// Focus a monitor using type-safe direction.
pub fn focus_monitor(direction: MonitorDirection) {
    super::super::monitor::focus_mon(direction.value());
}

/// Move client to a monitor using type-safe direction.
pub fn tag_monitor(direction: MonitorDirection) {
    super::tag_mon(direction.value());
}

/// Follow client to a monitor using type-safe direction.
pub fn follow_monitor(direction: MonitorDirection) {
    super::super::monitor::follow_mon(direction.value());
}
