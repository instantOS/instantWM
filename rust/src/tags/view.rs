//! View (workspace) navigation.
//!
//! A "view" is the set of tags currently visible on a monitor.  This module
//! owns every operation that *changes which tags are visible*, as opposed to
//! operations that change which tags a *client* belongs to (see
//! `client_tags.rs`).
//!
//! # Vocabulary
//!
//! | Term | Meaning |
//! |---|---|
//! | **tagset** | The bitmask of currently visible tags on a monitor. |
//! | **current_tag** | The single active tag index (1-based, 0 = overview). |
//! | **prev_tag** | The tag index visited just before the current one. |
//! | **pertag** | Per-tag layout settings (`nmaster`, `mfact`) stored on [`Tag`]. |
//!
//! # Operation overview
//!
//! | Function | What it does |
//! |---|---|
//! | [`view`] | Switch to a specific tag (or overview with `!0`). |
//! | [`toggle_view`] | Add/remove a tag from the current view without switching away. |
//! | [`view_to_left`] / [`view_to_right`] | Scroll the view one tag to the left/right. |
//! | [`last_view`] | Jump back to the previously-visited tag. |
//! | [`win_view`] | Switch to the tag(s) of the currently focused window. |
//! | [`shift_view_direction`] | Jump to the next/previous tag that has visible clients. |
//! | [`follow_view`] | Move the selected client to the previous tag and switch to it. |
//! | [`swap_tags`] | Exchange the clients of two tags. |
//! | [`toggle_overview`] | Toggle the all-tags overview mode. |
//! | [`toggle_fullscreen_overview`] | Simpler overview toggle (no floating save/restore). |

use crate::floating::{restore_all_floating, save_all_floating};
use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::arrange;
use crate::types::{Arg, Direction, SCRATCHPAD_MASK};
use crate::util::get_sel_win;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::Window;

// ---------------------------------------------------------------------------
// Public API — primary view operations
// ---------------------------------------------------------------------------

/// Switch the selected monitor's view to `arg.ui`.
///
/// Passing `!0u32` activates the all-tags overview mode (sets `current_tag`
/// to 0).  Any other value is treated as a tag bitmask; the lowest set bit
/// determines `current_tag`.
///
/// Returns early (without changing the view) if `arg.ui` already equals the
/// current tagset — this prevents redundant redraws when a keybinding is held.
pub fn view(arg: &Arg) {
    let bits = crate::tags::compute_prefix(arg);
    let tagmask = get_globals().tags.mask();

    // Toggle the tagset slot so the previous view is preserved for last_view().
    {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.seltags ^= 1;
        }
    }

    // A zero-masked value means "no valid tag" — nothing to do.
    if bits & tagmask == 0 {
        return;
    }

    {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.tagset[mon.seltags as usize] = bits & tagmask;
        }
    }

    // Update current_tag / prev_tag.
    if bits == !0u32 {
        // Overview mode: current_tag = 0.
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.prev_tag = mon.current_tag;
            mon.current_tag = 0;
        }
    } else {
        // Find the lowest set bit to derive the 1-based tag index.
        let new_tag = lowest_set_bit(bits) + 1;

        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            // Already on this tag — undo the seltags toggle and bail.
            if new_tag == mon.current_tag {
                mon.seltags ^= 1;
                return;
            }
            mon.prev_tag = mon.current_tag;
            mon.current_tag = new_tag;
        }
    }

    let mut globals = get_globals_mut();
    apply_pertag_settings(globals);
    focus(None);
    arrange(Some(get_globals().selmon));
}

/// Toggle a tag's membership in the current view without switching to it.
///
/// Unlike [`view`] this does not replace the tagset — it XORs `arg.ui` into
/// it so the user can show or hide individual tags while keeping others
/// visible.  If the result would be an empty tagset the call is a no-op.
pub fn toggle_view(arg: &Arg) {
    let tagmask = get_globals().tags.mask();

    let new_tagset = {
        let globals = get_globals();
        let current = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.tagset[m.seltags as usize])
            .unwrap_or(0);
        current ^ (arg.ui & tagmask)
    };

    // Guard: do not produce an empty view.
    if new_tagset == 0 {
        return;
    }

    {
        let mut globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.tagset[mon.seltags as usize] = new_tagset;
        }

        // Keep current_tag / prev_tag consistent.
        if new_tagset == !0u32 {
            if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
                mon.prev_tag = mon.current_tag;
                mon.current_tag = 0;
            }
        } else {
            let new_tag = lowest_set_bit(new_tagset) + 1;
            if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
                // Only update current_tag if the previous one is no longer visible.
                let current_tag = mon.current_tag;
                if current_tag == 0 || (new_tagset & (1 << (current_tag - 1))) == 0 {
                    mon.prev_tag = current_tag;
                    mon.current_tag = new_tag;
                }
            }
        }

        apply_pertag_settings(globals);
    }

    focus(None);
    arrange(Some(get_globals().selmon));
}

// ---------------------------------------------------------------------------
// Public API — view scrolling
// ---------------------------------------------------------------------------

/// Scroll the view one tag to the left (lower-numbered tag).
///
/// Only works when a single tag is currently selected.  Does nothing if the
/// view is already at the leftmost tag.
pub fn view_to_left(_arg: &Arg) {
    scroll_view(Direction::Left);
}

/// Scroll the view one tag to the right (higher-numbered tag).
///
/// Only works when a single tag is currently selected.  Does nothing if the
/// view is already at the rightmost tag.
pub fn view_to_right(_arg: &Arg) {
    scroll_view(Direction::Right);
}

/// Shift the view to the next/previous tag that contains at least one visible
/// client, wrapping around if needed.
///
/// If no occupied tag is found within 10 steps the view is not changed.
pub fn shift_view_direction(forward: bool) {
    let direction: i32 = if forward { 1 } else { -1 };

    let (tagset, numtags) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (mon.tagset[mon.seltags as usize], globals.tags.count())
    };

    let mut next_tagset = tagset;
    let mut found = false;

    for step in 1..=10i32 {
        let shift = direction * step;
        next_tagset = if direction > 0 {
            (tagset << shift) | (tagset >> (numtags as i32 - 1 - shift))
        } else {
            let rshift = (-shift) as usize;
            let lshift = (numtags as i32 - 1 + shift) as usize;
            (tagset >> rshift) | (tagset << lshift)
        };

        // Check whether any visible client lives on next_tagset.
        let globals = get_globals();
        let mut cursor = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

        while let Some(win) = cursor {
            match globals.clients.get(&win) {
                Some(c) => {
                    if (next_tagset & c.tags) != 0 {
                        found = true;
                        break;
                    }
                    cursor = c.next;
                }
                None => break,
            }
        }

        if found {
            break;
        }
    }

    if !found {
        return;
    }

    // Strip the scratchpad pseudo-tag so it doesn't leak into the view.
    if (next_tagset & SCRATCHPAD_MASK) != 0 {
        next_tagset ^= SCRATCHPAD_MASK;
    }

    view(&Arg {
        ui: next_tagset,
        ..Default::default()
    });
}

/// Legacy `&Arg` wrapper for [`shift_view_direction`].
///
/// `arg.i > 0` → forward, otherwise backward.
pub fn shift_view(arg: &Arg) {
    shift_view_direction(arg.i > 0);
}

// ---------------------------------------------------------------------------
// Public API — jump / history
// ---------------------------------------------------------------------------

/// Jump back to the previously-visited tag.
///
/// If `current_tag == prev_tag` (i.e. there is no real history) this falls
/// back to [`focus_last_client`](crate::focus::focus_last_client) so the user
/// always gets a useful action.
pub fn last_view(_arg: &Arg) {
    let (current_tag, prev_tag) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (mon.current_tag, mon.prev_tag)
    };

    if current_tag == prev_tag {
        crate::focus::focus_last_client(_arg);
        return;
    }

    view(&Arg {
        ui: 1 << (prev_tag.saturating_sub(1)),
        ..Default::default()
    });
}

/// Switch to the tag(s) of the currently focused X window.
///
/// Queries the X server for the input-focus window, locates the corresponding
/// client, and calls [`view`] with that client's tag mask.  Scratchpad clients
/// are treated specially: their view is the current tag, not their stored tag.
pub fn win_view(_arg: &Arg) {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };

    let Ok(cookie) = conn.get_input_focus() else {
        return;
    };
    let reply = match cookie.reply() {
        Ok(r) => r,
        Err(_) => return,
    };
    let focused_win = reply.focus;

    let client_win = find_client_for_window(focused_win);

    let Some(win) = client_win else { return };

    let tags = {
        let globals = get_globals();
        globals.clients.get(&win).map(|c| c.tags).unwrap_or(1)
    };

    if tags == SCRATCHPAD_MASK {
        // Show the scratchpad on whatever tag is currently active.
        let current_tag = {
            let globals = get_globals();
            globals
                .monitors
                .get(globals.selmon)
                .map(|m| m.current_tag)
                .unwrap_or(1)
        };
        view(&Arg {
            ui: 1 << (current_tag.saturating_sub(1)),
            ..Default::default()
        });
    } else {
        view(&Arg {
            ui: tags,
            ..Default::default()
        });
    }

    focus(Some(win));
}

// ---------------------------------------------------------------------------
// Public API — tag swapping & follow
// ---------------------------------------------------------------------------

/// Exchange all clients between the current tag and `arg.ui`.
///
/// Every client on the current tag moves to the target tag and vice versa.
/// The view then switches to the target tag.
///
/// Returns early if:
/// - `arg.ui` already equals the current tagset (would be a no-op).
/// - The current tagset is empty or spans multiple tags (ambiguous swap).
pub fn swap_tags(arg: &Arg) {
    let bits = crate::tags::compute_prefix(arg);
    let tagmask = get_globals().tags.mask();
    let newtag = bits & tagmask;

    let (current_tag, current_tagset) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (mon.current_tag as u32, mon.tagset[mon.seltags as usize])
    };

    // Guard: must be on a single tag and the target must differ.
    if newtag == current_tagset
        || current_tagset == 0
        || (current_tagset & (current_tagset - 1)) != 0
    {
        return;
    }

    let target_idx = lowest_set_bit(bits);

    // Collect clients that live on either tag before mutating.
    let clients_to_swap: Vec<Window> = {
        let globals = get_globals();
        let mut result = Vec::new();
        let mut cursor = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

        while let Some(win) = cursor {
            match globals.clients.get(&win) {
                Some(c) => {
                    if (c.tags & newtag) != 0 || (c.tags & current_tagset) != 0 {
                        result.push(win);
                    }
                    cursor = c.next;
                }
                None => break,
            }
        }
        result
    };

    for win in clients_to_swap {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.tags ^= current_tagset ^ newtag;
            // A client that was on *both* tags ends up with 0 — put it on the target.
            if client.tags == 0 {
                client.tags = newtag;
            }
        }
    }

    {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.tagset[mon.seltags as usize] = newtag;
            // Keep prev_tag pointing at the old current tag.
            if mon.prev_tag == target_idx + 1 {
                mon.prev_tag = current_tag as usize;
            }
            mon.current_tag = target_idx + 1;
        }
    }

    focus(None);
    arrange(Some(get_globals().selmon));
}

/// Move the selected client to the previously-visited tag and switch there.
///
/// This lets the user "throw" a window back to where they came from without
/// manually re-selecting a tag.
pub fn follow_view(_arg: &Arg) {
    let sel_win = get_sel_win();
    let Some(win) = sel_win else { return };

    let prev_tag = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        mon.prev_tag
    };

    if prev_tag == 0 {
        return;
    }

    let target_bits = 1u32 << (prev_tag - 1);

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.tags = target_bits;
        }
    }

    view(&Arg {
        ui: target_bits,
        ..Default::default()
    });
    focus(Some(win));
    arrange(Some(get_globals().selmon));
}

// ---------------------------------------------------------------------------
// Public API — overview modes
// ---------------------------------------------------------------------------

/// Toggle the all-tags overview.
///
/// * If a monitor has no clients and is already in overview mode, falls back
///   to [`last_view`] to exit gracefully.
/// * Entering overview: saves floating positions, then calls [`view`] with
///   `!0` to show all tags.
/// * Leaving overview: restores floating positions, then calls [`win_view`] to
///   return to the tag of the focused window.
pub fn toggle_overview(_arg: &Arg) {
    let (has_clients, current_tag) = {
        let globals = get_globals();
        let has_clients = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.clients.is_some())
            .unwrap_or(false);
        let current_tag = globals.monitors.get(globals.selmon).map(|m| m.current_tag);
        (has_clients, current_tag)
    };

    if !has_clients {
        if current_tag == Some(0) {
            last_view(&Arg::default());
        }
        return;
    }

    match current_tag {
        Some(0) => {
            // Currently in overview — exit.
            let sel_mon_id = get_globals().selmon;
            restore_all_floating(Some(sel_mon_id));
            win_view(&Arg::default());
        }
        Some(_) => {
            // Not in overview — enter.
            let sel_mon_id = get_globals().selmon;
            save_all_floating(Some(sel_mon_id));
            view(&Arg {
                ui: !0u32,
                ..Default::default()
            });
        }
        None => {}
    }
}

/// Simpler overview toggle that does **not** save/restore floating positions.
///
/// Entering shows all tags; leaving calls [`win_view`] to return to the
/// focused window's tag.  Prefer [`toggle_overview`] for the full experience.
//
// TODO: cargo check reports this as unused — cross-check with C codebase to
//       determine whether it should be wired up or removed.
pub fn toggle_fullscreen_overview(_arg: &Arg) {
    let current_tag = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).map(|m| m.current_tag)
    };

    match current_tag {
        Some(0) => win_view(&Arg::default()),
        Some(_) => view(&Arg {
            ui: !0u32,
            ..Default::default()
        }),
        None => {}
    }
}

// ---------------------------------------------------------------------------
// pub(super) — shared with other tag sub-modules
// ---------------------------------------------------------------------------

/// Apply per-tag layout settings (`nmaster`, `mfact`) to the selected monitor.
///
/// Called after any view change so the monitor immediately reflects the layout
/// preferences stored for the newly active tag.
pub(super) fn apply_pertag_settings(globals: &mut crate::globals::Globals) {
    let sel_mon_id = globals.selmon;

    let (nmaster, mfact) = {
        let Some(mon) = globals.monitors.get(sel_mon_id) else {
            return;
        };
        let current_tag = mon.current_tag;
        if current_tag == 0 || current_tag > globals.tags.tags.len() {
            return;
        }
        let tag = &globals.tags.tags[current_tag - 1];
        (tag.nmaster, tag.mfact)
    };

    if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
        mon.nmaster = nmaster;
        mon.mfact = mfact;
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Scroll the view one step in `dir` (Left or Right).
///
/// Requires that exactly one tag is currently selected.
fn scroll_view(dir: Direction) {
    let (current_tag, tagset, tagmask) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (
            mon.current_tag as u32,
            mon.tagset[mon.seltags as usize],
            globals.tags.mask(),
        )
    };

    // Boundary guards.
    if dir == Direction::Left && current_tag <= 1 {
        return;
    }
    if dir == Direction::Right && current_tag >= 20 {
        return;
    }

    // Only scroll when a single tag is active.
    if (tagset & tagmask).count_ones() != 1 {
        return;
    }

    let new_tagset = match dir {
        Direction::Left => {
            if tagset <= 1 {
                return;
            }
            tagset >> 1
        }
        Direction::Right => {
            if (tagset & (tagmask >> 1)) == 0 {
                return;
            }
            tagset << 1
        }
        // Vertical directions map to the same logic for callers that pass them.
        Direction::Up => {
            if tagset <= 1 {
                return;
            }
            tagset >> 1
        }
        Direction::Down => {
            if (tagset & (tagmask >> 1)) == 0 {
                return;
            }
            tagset << 1
        }
    };

    let new_tag = lowest_set_bit(new_tagset) + 1;

    {
        let mut globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.seltags ^= 1;
            mon.tagset[mon.seltags as usize] = new_tagset;
            mon.prev_tag = mon.current_tag;
            mon.current_tag = new_tag;
        }
        apply_pertag_settings(globals);
    }

    focus(None);
    arrange(Some(get_globals().selmon));
}

/// Return the index of the lowest set bit in `bits` (0-based).
///
/// Panics if `bits == 0`.
#[inline]
fn lowest_set_bit(bits: u32) -> usize {
    bits.trailing_zeros() as usize
}

/// Locate the [`Client`](crate::types::Client) key for a raw X [`Window`].
///
/// First checks if the window itself is a known client key, then falls back
/// to a linear scan of the selected monitor's client list.
fn find_client_for_window(win: Window) -> Option<Window> {
    let globals = get_globals();

    if globals.clients.contains_key(&win) {
        return Some(win);
    }

    let mut cursor = globals.monitors.get(globals.selmon).and_then(|m| m.clients);

    while let Some(c_win) = cursor {
        match globals.clients.get(&c_win) {
            Some(c) => {
                if c.win == win {
                    return Some(c_win);
                }
                cursor = c.next;
            }
            None => break,
        }
    }

    None
}
