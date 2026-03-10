//! Type-safe tag operations.
//!
//! This module provides ergonomic wrappers around tag operations using
//! the new `TagMask` and `TagSelection` types.

use crate::contexts::WmCtx;
use crate::types::{TagMask, TagSelection};

/// View tags using a type-safe selection.
///
/// This is the preferred way to switch tag views as it provides
/// semantic meaning and type safety.
///

pub fn view_selection(ctx: &mut WmCtx, selection: TagSelection) {
    let num_tags = ctx.g().tags.count();
    let current_mask = TagMask::from_bits(ctx.g().selected_monitor().selected_tags());
    let prev_tag = ctx.g().selected_monitor().prev_tag;

    let mask = selection.to_mask(current_mask, prev_tag, num_tags);
    super::view::view(ctx, mask);
}
