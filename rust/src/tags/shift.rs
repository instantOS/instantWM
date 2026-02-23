//! Moving clients between tags.
//!
//! This module handles *shifting* a client from its current tag to an adjacent
//! one — the "send window left/right" family of operations.
//!
//! # Operations
//!
//! | Function | What it does |
//! |---|---|
//! | [`tag_to_left`] / [`tag_to_right`] | `&Arg` wrappers used by keybindings. |
//! | [`tag_to_left_by`] / [`tag_to_right_by`] | Typed versions that take an offset directly. |
//! | [`move_left`] / [`move_right`] | Shift the client **and** scroll the view together. |
//!
//! # Internals
//!
//! All of the above ultimately call [`shift_tag`], which:
//!
//! 1. Handles the special case of an overlay window (delegates to the overlay
//!    module instead).
//! 2. Optionally plays a slide animation before the tag change.
//! 3. Clears the sticky flag on the client if it was sticky.
//! 4. Shifts the client's tag bitmask left or right by `offset` bits.

use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::arrange;
use crate::types::{Arg, Direction, OverlayMode, Rect};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, StackMode};

// ---------------------------------------------------------------------------
// Public API — typed entry points
// ---------------------------------------------------------------------------

/// Move the selected client's tag one step to the left (lower-numbered tag).
///
/// `offset` is clamped to a minimum of 1 so callers can pass `arg.i` directly
/// without worrying about zero or negative values.
pub fn tag_to_left_by(offset: i32) {
    shift_tag(Direction::Left, offset.max(1));
}

/// Move the selected client's tag one step to the right (higher-numbered tag).
///
/// `offset` is clamped to a minimum of 1 so callers can pass `arg.i` directly
/// without worrying about zero or negative values.
pub fn tag_to_right_by(offset: i32) {
    shift_tag(Direction::Right, offset.max(1));
}

// ---------------------------------------------------------------------------
// Public API — &Arg wrappers for keybinding dispatch
// ---------------------------------------------------------------------------

/// `&Arg` wrapper for [`tag_to_left_by`].  `arg.i` is the shift offset.
pub fn tag_to_left(arg: &Arg) {
    tag_to_left_by(arg.i);
}

/// `&Arg` wrapper for [`tag_to_right_by`].  `arg.i` is the shift offset.
pub fn tag_to_right(arg: &Arg) {
    tag_to_right_by(arg.i);
}

/// Shift the selected client left **and** scroll the view left together.
///
/// The window stays visible as it moves — the viewport follows it.
pub fn move_left(arg: &Arg) {
    tag_to_left(arg);
    crate::tags::view::view_to_left(arg);
}

/// Shift the selected client right **and** scroll the view right together.
///
/// The window stays visible as it moves — the viewport follows it.
pub fn move_right(arg: &Arg) {
    tag_to_right(arg);
    crate::tags::view::view_to_right(arg);
}

// ---------------------------------------------------------------------------
// Private — core implementation
// ---------------------------------------------------------------------------

/// Core tag-shift logic.
///
/// Moves the currently selected client one or more tag positions in `dir`.
/// Does nothing if:
/// - No client is selected.
/// - The view currently spans multiple tags (ambiguous which single tag to
///   shift to).
/// - The client is already at the boundary in the requested direction.
fn shift_tag(dir: Direction, offset: i32) {
    let sel_win = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
    };
    let Some(win) = sel_win else { return };

    let (current_tag, overlay_win) = {
        let globals = get_globals();
        let current_tag = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.current_tag as u32);
        let overlay = globals.monitors.get(globals.selmon).and_then(|m| m.overlay);
        (current_tag, overlay)
    };

    let Some(current_tag) = current_tag else {
        return;
    };

    // -----------------------------------------------------------------------
    // Special case: overlay windows are re-positioned via the overlay module.
    // -----------------------------------------------------------------------
    if Some(win) == overlay_win {
        let mode = match dir {
            Direction::Left => OverlayMode::Left,
            Direction::Right => OverlayMode::Right,
            Direction::Up => OverlayMode::Top,
            Direction::Down => OverlayMode::Bottom,
        };
        crate::overlay::set_overlay_mode(mode);
        return;
    }

    // -----------------------------------------------------------------------
    // Boundary guards — don't shift past tag 1 or tag 20.
    // -----------------------------------------------------------------------
    if dir == Direction::Left && current_tag <= 1 {
        return;
    }
    if dir == Direction::Right && current_tag >= 20 {
        return;
    }

    // -----------------------------------------------------------------------
    // Only shift when the view shows exactly one tag.
    // -----------------------------------------------------------------------
    let (tagset, tagmask) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (mon.tagset[mon.seltags as usize], globals.tags.mask())
    };

    if (tagset & tagmask).count_ones() != 1 {
        return;
    }

    // -----------------------------------------------------------------------
    // Clear the sticky flag before moving (sticky clients are pinned to the
    // current tag; shifting them implicitly un-stickies them).
    // -----------------------------------------------------------------------
    clear_sticky(win);

    // -----------------------------------------------------------------------
    // Optional slide animation.
    // -----------------------------------------------------------------------
    if get_globals().animated {
        play_slide_animation(win, dir);
    }

    // -----------------------------------------------------------------------
    // Perform the bit-shift on the client's tag mask.
    // -----------------------------------------------------------------------
    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            match dir {
                Direction::Left if tagset > 1 => {
                    client.tags >>= offset;
                }
                Direction::Right if (tagset & (tagmask >> 1)) != 0 => {
                    client.tags <<= offset;
                }
                _ => return, // boundary hit after animation guard — bail cleanly
            }
        }
    }

    focus(None);
    arrange(Some(get_globals().selmon));
}

/// Clear the sticky flag on `win` and pin it to the monitor's current tag.
///
/// When a sticky client is shifted to a new tag it should lose its sticky
/// status; otherwise it would immediately reappear on every tag again.
fn clear_sticky(win: x11rb::protocol::xproto::Window) {
    let target_tags = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|mon| {
            if mon.current_tag > 0 {
                Some(1u32 << (mon.current_tag - 1))
            } else {
                None
            }
        })
    };

    let globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        if client.issticky {
            client.issticky = false;
            if let Some(tags) = target_tags {
                client.tags = tags;
            }
        }
    }
}

/// Play a short slide animation that moves `win` off-screen in `dir`.
///
/// The animation is purely visual — the client is raised to the top of the
/// stack first so it is visible during the transition, then animated to a
/// position 1/10th of the monitor width outside the current edge.
fn play_slide_animation(win: x11rb::protocol::xproto::Window, dir: Direction) {
    // Raise the window so it is visible during the animation.
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        let _ = conn.flush();
    }

    let (mon_w, c_x, c_y) = {
        let globals = get_globals();
        let mon_w = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.monitor_rect.w)
            .unwrap_or(0);
        let (cx, cy) = globals
            .clients
            .get(&win)
            .map(|c| (c.geo.x, c.geo.y))
            .unwrap_or((0, 0));
        (mon_w, cx, cy)
    };

    // Slide 10% of the monitor width in the direction of travel.
    let anim_dx = (mon_w / 10)
        * match dir {
            Direction::Left => -1,
            Direction::Right => 1,
            Direction::Up => -1,
            Direction::Down => 1,
        };

    crate::animation::animate_client_rect(
        win,
        &Rect {
            x: c_x + anim_dx,
            y: c_y,
            w: 0,
            h: 0,
        },
        7,
        0,
    );
}
