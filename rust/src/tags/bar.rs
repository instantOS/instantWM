//! Tag bar rendering helpers.
//!
//! These functions answer two questions the bar needs answered on every redraw:
//!
//! * [`get_tag_width`] – how many pixels wide should the entire tag strip be?
//! * [`get_tag_at_x`] – which tag (if any) lives at a given X coordinate?
//!
//! Both functions share the same iteration logic: walk the monitor's client
//! list to find which tags are *occupied*, then iterate over every tag and
//! skip hidden ones when `showtags` is active.

use crate::bar::text_width;
use crate::globals::get_globals;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Return the total pixel width of the tag strip (including the start-menu
/// button at the left edge).
///
/// Tags that are neither occupied nor selected are omitted when the monitor's
/// `showtags` flag is set, so the width can shrink dynamically.
pub fn get_tag_width() -> i32 {
    let globals = get_globals();

    let occupied = occupied_tags_on_selmon(globals);
    let lrpad = globals.lrpad;
    let show_alt = globals.tags.show_alt;
    let start_menu_size = globals.startmenusize;

    let mut x = 0i32;

    for (i, tag) in globals.tags.tags.iter().enumerate() {
        if i >= 9 {
            break;
        }

        if should_skip_tag(globals, i, occupied) {
            continue;
        }

        let name = display_name(tag, show_alt);
        x += text_width(name) + lrpad;
    }

    x + start_menu_size
}

/// Return the 0-based index of the tag whose button contains `click_x`, or
/// `-1` if `click_x` falls outside all tag buttons.
///
/// `click_x` is relative to the left edge of the bar window (i.e. it already
/// includes the monitor's X offset).
pub fn get_tag_at_x(click_x: i32) -> i32 {
    let globals = get_globals();

    let occupied = occupied_tags_on_selmon(globals);
    let lrpad = globals.lrpad;
    let show_alt = globals.tags.show_alt;

    // The tag strip starts immediately after the start-menu button.
    let mut accumulated = globals.startmenusize;

    for (i, tag) in globals.tags.tags.iter().enumerate() {
        if i >= 9 {
            break;
        }

        if should_skip_tag(globals, i, occupied) {
            continue;
        }

        let name = display_name(tag, show_alt);
        accumulated += text_width(name) + lrpad;

        if accumulated >= click_x {
            return i as i32;
        }
    }

    -1
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Compute a bitmask of tags that have at least one visible client on the
/// selected monitor (excluding the special scratchpad tag `255`).
fn occupied_tags_on_selmon(globals: &crate::globals::Globals) -> u32 {
    let mut occupied: u32 = 0;

    let mut current = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

    while let Some(win) = current {
        match globals.clients.get(&win) {
            Some(c) => {
                if c.tags != 255 {
                    occupied |= c.tags;
                }
                current = c.next;
            }
            None => break,
        }
    }

    occupied
}

/// Return `true` if tag `i` should be hidden from the bar.
///
/// A tag is hidden when `showtags != 0` *and* it has neither any occupied
/// clients nor is currently selected.
fn should_skip_tag(globals: &crate::globals::Globals, i: usize, occupied: u32) -> bool {
    let Some(mon) = globals.monitors.get(globals.selmon) else {
        return false;
    };

    if mon.showtags == 0 {
        return false;
    }

    let bit = 1u32 << i;
    let is_occupied = (occupied & bit) != 0;
    let is_selected = (mon.tagset[mon.seltags as usize] & bit) != 0;

    !is_occupied && !is_selected
}

/// Choose between the regular name and the alt-name for display.
fn display_name<'a>(tag: &'a crate::types::Tag, show_alt: bool) -> &'a str {
    if show_alt && !tag.alt_name.is_empty() {
        tag.alt_name
    } else {
        tag.name.as_str()
    }
}
