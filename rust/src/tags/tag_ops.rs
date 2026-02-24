//! Type-safe tag operations.
//!
//! This module provides ergonomic wrappers around tag operations using
//! the new `TagMask` and `TagSelection` types.

use crate::globals::{get_globals, get_globals_mut};
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
    super::view(mask.bits());
}

/// Set the selected client's tags using a type-safe mask.
pub fn set_client_tag_mask(mask: TagMask) {
    super::set_client_tag(mask.bits());
}

/// Toggle tags on the selected client using a type-safe mask.
pub fn toggle_client_tag_mask(mask: TagMask) {
    super::toggle_tag(mask.bits());
}

/// Follow a tag (move client and view) using type-safe selection.
pub fn follow_tag_mask(mask: TagMask) {
    super::follow_tag(mask.bits());
}

/// Swap current tag with another using type-safe mask.
pub fn swap_tags_mask(mask: TagMask) {
    super::swap_tags(mask.bits());
}

/// Toggle view of tags using type-safe selection.
pub fn toggle_view_mask(mask: TagMask) {
    super::toggle_view(mask.bits());
}

/// View all tags (overview mode).
pub fn view_all() {
    let globals = get_globals();
    let num_tags = globals.tags.count();
    let mask = TagMask::all(num_tags);
    super::view(mask.bits());
}

/// Check if the current view is showing all tags.
pub fn is_viewing_all() -> bool {
    let globals = get_globals();
    globals
        .monitors
        .get(globals.selmon)
        .map(|m| m.current_tag == 0)
        .unwrap_or(false)
}

/// Get the currently selected tag mask.
pub fn current_tag_mask() -> TagMask {
    let globals = get_globals();
    globals
        .monitors
        .get(globals.selmon)
        .map(|m| TagMask::from_bits(m.tagset[m.seltags as usize]))
        .unwrap_or(TagMask::EMPTY)
}

/// Get the previous tag index.
pub fn previous_tag() -> usize {
    let globals = get_globals();
    globals
        .monitors
        .get(globals.selmon)
        .map(|m| m.prev_tag)
        .unwrap_or(0)
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

/// Get the tag mask for a specific tag index (1-indexed).
///
/// Returns `None` if the index is invalid.
pub fn mask_for_tag(tag_index: usize) -> Option<TagMask> {
    TagMask::single(tag_index)
}

/// Get the tag mask for tag keys 1-9.
///
/// This is a convenience function for keybinding generation.
/// Returns `None` for indices > 8 (0-indexed).
pub fn mask_for_key_index(idx: usize) -> Option<TagMask> {
    if idx < 9 {
        TagMask::single(idx + 1)
    } else {
        None
    }
}

/// Builder-style API for complex tag operations.
///
/// # Example
///
/// ```
/// use crate::tags::tag_ops::TagViewBuilder;
///
/// TagViewBuilder::new()
///     .tag(3)
///     .view();
/// ```
pub struct TagViewBuilder {
    selection: TagSelection,
}

impl TagViewBuilder {
    /// Create a new builder with no selection.
    pub fn new() -> Self {
        Self {
            selection: TagSelection::None,
        }
    }

    /// Set a single tag to view.
    pub fn tag(mut self, idx: usize) -> Self {
        self.selection = TagSelection::Single(idx);
        self
    }

    /// Set multiple tags using a mask.
    pub fn mask(mut self, mask: TagMask) -> Self {
        self.selection = TagSelection::Multi(mask);
        self
    }

    /// View all tags.
    pub fn all(mut self) -> Self {
        self.selection = TagSelection::All;
        self
    }

    /// Toggle the selection.
    pub fn toggle(mut self) -> Self {
        if let TagSelection::Multi(mask) = self.selection {
            self.selection = TagSelection::Toggle(mask);
        }
        self
    }

    /// Execute the view operation.
    pub fn view(self) {
        view_selection(self.selection);
    }
}

impl Default for TagViewBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Extension trait for tag operations on windows.
///
/// This trait provides ergonomic methods for manipulating client tags.
pub trait ClientTagExt {
    /// Set this client's tags.
    fn set_tags(&self, mask: TagMask);

    /// Add tags to this client.
    fn add_tags(&self, mask: TagMask);

    /// Remove tags from this client.
    fn remove_tags(&self, mask: TagMask);

    /// Toggle tags on this client.
    fn toggle_tags(&self, mask: TagMask);

    /// Check if this client has any of the given tags.
    fn has_any_tag(&self, mask: TagMask) -> bool;

    /// Check if this client has all of the given tags.
    fn has_all_tags(&self, mask: TagMask) -> bool;
}

impl ClientTagExt for x11rb::protocol::xproto::Window {
    fn set_tags(&self, mask: TagMask) {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(self) {
            client.tags = mask.bits();
        }
    }

    fn add_tags(&self, mask: TagMask) {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(self) {
            client.tags |= mask.bits();
        }
    }

    fn remove_tags(&self, mask: TagMask) {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(self) {
            client.tags &= !mask.bits();
        }
    }

    fn toggle_tags(&self, mask: TagMask) {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(self) {
            client.tags ^= mask.bits();
        }
    }

    fn has_any_tag(&self, mask: TagMask) -> bool {
        let globals = get_globals();
        globals
            .clients
            .get(self)
            .map(|c| TagMask::from_bits(c.tags).intersects(mask))
            .unwrap_or(false)
    }

    fn has_all_tags(&self, mask: TagMask) -> bool {
        let globals = get_globals();
        globals
            .clients
            .get(self)
            .map(|c| (TagMask::from_bits(c.tags) & mask) == mask)
            .unwrap_or(false)
    }
}
