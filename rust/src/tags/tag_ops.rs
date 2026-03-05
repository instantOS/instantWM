//! Type-safe tag operations.
//!
//! This module provides ergonomic wrappers around tag operations using
//! the new `TagMask` and `TagSelection` types.

use crate::contexts::{CoreCtx, X11Ctx};
use crate::types::{TagMask, TagSelection};

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
pub fn view_selection(core: &mut CoreCtx, x11: &X11Ctx, selection: TagSelection) {
    let num_tags = core.g.tags.count();
    let current_mask = TagMask::from_bits(core.g.selected_monitor().selected_tags());
    let prev_tag = core.g.selected_monitor().prev_tag;

    let mask = selection.to_mask(current_mask, prev_tag, num_tags);
    super::view(core, x11, mask);
}
