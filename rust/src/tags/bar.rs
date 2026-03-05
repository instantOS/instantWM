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
use crate::globals::Globals;
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
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> Vec<VisibleTag<'a>> {
    let horizontal_padding = core.g.cfg.horizontal_padding;
    let show_alt = core.g.tags.show_alt;
    let slot_count = monitor.tags.len().min(MAX_BAR_SLOTS);

    let mut out = Vec::with_capacity(slot_count);
    for slot in 0..slot_count {
        let tag_index = tag_index_for_slot(monitor, slot);
        if tag_index >= monitor.tags.len() {
            continue;
        }
        if should_skip(monitor, tag_index, occupied) {
            continue;
        }

        let tag = &monitor.tags[tag_index];
        let label = display_name(tag, show_alt);
        let width = painter.text_width(label) + horizontal_padding;

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
pub fn get_tag_width(core: &CoreCtx, painter: &mut dyn crate::bar::paint::BarPainter) -> i32 {
    let occupied = occupied_tags_on_selmon(core.g);

    let m = core.g.selected_monitor();
    if m.tags.is_empty() {
        return core.g.cfg.startmenusize;
    }

    let tags_width: i32 = visible_tags_ctx(core, m, occupied, painter)
        .iter()
        .map(|t| t.width)
        .sum();
    core.g.cfg.startmenusize + tags_width
}

/// Return the 0-based tag index at `click_x`, or `-1` if outside all tags.
///
/// `click_x` is relative to the left edge of the bar window.
pub fn get_tag_at_x(
    core: &CoreCtx,
    click_x: i32,
    painter: &mut dyn crate::bar::paint::BarPainter,
) -> i32 {
    let occupied = occupied_tags_on_selmon(core.g);

    let m = core.g.selected_monitor();
    if m.tags.is_empty() {
        return -1;
    }

    let mut acc = core.g.cfg.startmenusize;
    for t in visible_tags_ctx(core, m, occupied, painter) {
        acc += t.width;
        if acc > click_x {
            return t.tag_index as i32;
        }
    }

    -1
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Map a bar slot (0..8) to the actual tag index.
///
/// Slot 8 is remapped to `current_tag - 1` when the monitor has more than 9
/// tags active (the "overflow" slot).
fn tag_index_for_slot(monitor: &Monitor, slot: usize) -> usize {
    if slot == MAX_BAR_SLOTS - 1 && monitor.current_tag > MAX_BAR_SLOTS {
        monitor.current_tag - 1
    } else {
        slot
    }
}

/// Return `true` if the tag at `tag_index` should be hidden.
///
/// A tag is hidden when `showtags != 0` and it is neither occupied nor selected.
fn should_skip(monitor: &Monitor, tag_index: usize, occupied: u32) -> bool {
    if monitor.showtags == 0 {
        return false;
    }
    let bit = 1u32 << tag_index;
    (occupied & bit) == 0 && (monitor.selected_tags() & bit) == 0
}

/// Choose between the regular name and the alt-name for display.
fn display_name(tag: &crate::types::Tag, show_alt: bool) -> &str {
    if show_alt && !tag.alt_name.is_empty() {
        tag.alt_name.as_str()
    } else {
        tag.name.as_str()
    }
}

/// Compute a bitmask of tags that have at least one client on the selected
/// monitor (excluding the special scratchpad tag `255`).
fn occupied_tags_on_selmon(globals: &Globals) -> u32 {
    let mut occupied: u32 = 0;

    let m = globals.selected_monitor();
    for &win in &m.clients {
        if let Some(c) = globals.clients.get(&win) {
            if c.tags != 255 {
                occupied |= c.tags;
            }
        }
    }

    occupied
}
