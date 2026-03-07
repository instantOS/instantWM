//! Tag bar rendering helpers.
//!
//! These functions answer three questions the bar needs on every redraw:
//!
//! * [`visible_tags_ctx`] – which tags should be drawn, and with what label/width?
//! * [`get_tag_width`] – how many pixels wide is the entire tag strip?
//! * [`get_tag_at_x`] – which tag (if any) lives at a given X coordinate?
//!
//! All three share a single iteration through [`visible_tags_ctx`], which resolves
//! tag-index remapping, skip logic, display names, and widths in one place.

use crate::contexts::CoreCtx;
use crate::types::Monitor;

/// Maximum number of tag slots rendered in the bar.
const MAX_BAR_SLOTS: usize = 9;

/// A tag that should be drawn in the bar, with all derived data pre-computed.
pub(crate) struct VisibleTag<'a> {
    /// Slot index (0..MAX_BAR_SLOTS-1). Used for hover/gesture matching.
    pub slot: usize,
    /// Actual tag index into `globals.tags.tags` / bitmask space.
    pub tag_index: usize,
    /// Display label (regular or alt name).
    pub label: &'a str,
    /// Total pixel width of this tag cell (text width + horizontal_padding).
    pub width: i32,
}

pub(crate) fn visible_tags_ctx<'a>(
    core: &CoreCtx,
    monitor: &'a Monitor,
    occupied: u32,
) -> Vec<VisibleTag<'a>> {
    let horizontal_padding = core.g.cfg.horizontal_padding;
    let show_alt = core.g.tags.show_alternative_names;
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
        let cached = core.bar.get_tag_width(slot);
        let width = if cached > 0 {
            cached
        } else {
            ((label.chars().count() as i32) * 8 + horizontal_padding).max(horizontal_padding)
        };

        out.push(VisibleTag {
            slot,
            tag_index,
            label,
            width,
        });
    }

    out
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Return the total pixel width of the tag strip (including the start-menu
/// button at the left edge).
pub fn get_tag_width(core: &CoreCtx) -> i32 {
    let m = core.g.selected_monitor();
    if m.tags.is_empty() {
        return core.g.cfg.startmenusize;
    }

    let occupied = m.occupied_tags(&core.g.clients);
    let tags_width: i32 = visible_tags_ctx(core, m, occupied)
        .iter()
        .map(|t| t.width)
        .sum();
    core.g.cfg.startmenusize + tags_width
}

/// Return the 0-based tag index at `click_x`, or `-1` if outside all tags.
///
/// `click_x` is relative to the left edge of the bar window.
pub fn get_tag_at_x(core: &CoreCtx, click_x: i32) -> i32 {
    let m = core.g.selected_monitor();
    if m.tags.is_empty() {
        return -1;
    }

    let occupied = m.occupied_tags(&core.g.clients);
    let mut acc = core.g.cfg.startmenusize;
    for t in visible_tags_ctx(core, m, occupied) {
        acc += t.width;
        if acc > click_x {
            return t.tag_index as i32;
        }
    }

    -1
}
