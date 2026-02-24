//! Hover-resize: cursor feedback and click-to-resize/move near floating windows.
//!
//! When the pointer hovers just outside a floating window's border, the root
//! cursor changes to a resize shape.  A left-click then starts an interactive
//! resize (or move, when the cursor is at the window's top-middle edge);
//! a right-click always starts a move.  Moving further away deactivates the
//! mode.
//!
//! ## Entry points
//!
//! | Function                          | Called from          | Purpose                                     |
//! |-----------------------------------|----------------------|---------------------------------------------|
//! | [`handle_floating_resize_hover`]  | `motion_notify`      | Set/reset resize cursor and `altcursor`      |
//! | [`hover_resize_mouse`]            | `enter_notify`, etc. | Modal grab loop: wait for click near border  |

use crate::focus::{focus, warp_into};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::is_current_layout_tiling;
use crate::types::*;
use crate::util::get_sel_win;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use super::constants::{KEYCODE_ESCAPE, RESIZE_BORDER_ZONE};
use super::drag::move_mouse;
use super::grab::{grab_pointer_with_keys, ungrab};
use super::warp::get_root_ptr;

use super::resize::resize_mouse_directional;

// ── Resize direction ─────────────────────────────────────────────────────────

// ResizeDirection and get_resize_direction are now defined in types.rs

/// Returns `true` when the cursor (in root coordinates) is on the top-middle
/// edge of a window — used to distinguish a *move* from a *resize*.
fn is_at_top_middle_edge(geo: &Rect, root_x: i32, root_y: i32) -> bool {
    let at_top = root_y >= geo.y - RESIZE_BORDER_ZONE && root_y < geo.y + RESIZE_BORDER_ZONE;
    let in_middle_third = root_x >= geo.x + geo.w / 3 && root_x <= geo.x + 2 * geo.w / 3;
    at_top && in_middle_third
}

// ── Cursor helpers ───────────────────────────────────────────────────────────

/// Change the root window cursor to the shape at `cursor_index`.
fn set_root_cursor(cursor_index: usize) {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };
    let globals = get_globals();
    if let Some(ref cur) = globals.cursors[cursor_index] {
        let _ = change_window_attributes(
            conn,
            globals.root,
            &ChangeWindowAttributesAux::new().cursor(cur.cursor),
        );
        let _ = conn.flush();
    }
}

/// Warp the pointer to the edge/corner of `win` described by `dir`.
fn warp_pointer_resize(win: Window, dir: ResizeDirection) {
    let x11 = get_x11();
    let Some(ref conn) = x11.conn else { return };
    let globals = get_globals();
    let Some(c) = globals.clients.get(&win) else {
        return;
    };
    let (x_off, y_off) = dir.warp_offset(c.geo.w, c.geo.h, c.border_width);
    let _ = conn.warp_pointer(x11rb::NONE, win, 0, 0, 0, 0, x_off as i16, y_off as i16);
    let _ = conn.flush();
}

// ── Border detection ─────────────────────────────────────────────────────────

/// Check if point (px, py) is in the resize-border zone of a window with geometry geo.
/// The zone is a [`RESIZE_BORDER_ZONE`]-pixel band around the outside of the window.
fn is_point_in_resize_border(geo: &Rect, px: i32, py: i32) -> bool {
    if px > geo.x && px < geo.x + geo.w && py > geo.y && py < geo.y + geo.h {
        return false;
    }
    if py < geo.y - RESIZE_BORDER_ZONE
        || px < geo.x - RESIZE_BORDER_ZONE
        || py > geo.y + geo.h + RESIZE_BORDER_ZONE
        || px > geo.x + geo.w + RESIZE_BORDER_ZONE
    {
        return false;
    }
    true
}

/// Find a visible floating window whose resize border zone contains the cursor.
/// Returns the window ID if found, or None.
///
/// Also checks that the cursor is not on the bar.
pub fn find_floating_win_at_resize_border() -> Option<Window> {
    let globals = get_globals();

    let has_tiling = globals
        .monitors
        .get(globals.selmon)
        .map(|m| is_current_layout_tiling(m, &globals.tags))
        .unwrap_or(true);

    let (px, py) = {
        let _ = globals;
        get_root_ptr()?
    };

    let globals = get_globals();
    if let Some(mon) = globals.monitors.get(globals.selmon) {
        if mon.showbar && py < mon.monitor_rect.y + globals.bh {
            return None;
        }
    }

    let mon = globals.monitors.get(globals.selmon)?;
    let mut win = mon.clients;
    while let Some(w) = win {
        let Some(c) = globals.clients.get(&w) else {
            break;
        };
        win = c.next;
        if !c.is_visible() {
            continue;
        }
        if !c.isfloating && has_tiling {
            continue;
        }
        if is_point_in_resize_border(&c.geo, px, py) {
            return Some(w);
        }
    }
    None
}

/// Return `true` when the pointer is in the resize-border zone of the
/// currently selected floating window.
pub fn is_in_resize_border() -> bool {
    let Some(win) = get_sel_win() else {
        return false;
    };
    let globals = get_globals();
    let Some(c) = globals.clients.get(&win) else {
        return false;
    };
    let has_tiling = globals
        .monitors
        .get(globals.selmon)
        .map(|m| is_current_layout_tiling(m, &globals.tags))
        .unwrap_or(true);
    if !c.isfloating && has_tiling {
        return false;
    }

    let Some((px, py)) = get_root_ptr() else {
        return false;
    };
    let globals = get_globals();
    if let Some(mon) = globals.monitors.get(globals.selmon) {
        if mon.showbar && py < mon.monitor_rect.y + globals.bh {
            return false;
        }
    }
    is_point_in_resize_border(&c.geo, px, py)
}

/// Check whether any visible client on the current monitor is tiled.
fn has_visible_tiled_client() -> bool {
    let globals = get_globals();
    let has_tiling = globals
        .monitors
        .get(globals.selmon)
        .map(|m| is_current_layout_tiling(m, &globals.tags))
        .unwrap_or(true);

    let Some(mon) = globals.monitors.get(globals.selmon) else {
        return false;
    };

    let mut win = mon.clients;
    while let Some(w) = win {
        let Some(c) = globals.clients.get(&w) else {
            break;
        };
        if c.is_visible() && !(c.isfloating || !has_tiling) {
            return true;
        }
        win = c.next;
    }
    false
}

// ── Motion-notify hook ───────────────────────────────────────────────────────

/// Called from [`crate::events::motion_notify`] when the cursor is below the
/// bar (i.e. in the desktop area).
///
/// Sets the root cursor to resize and updates `globals.altcursor` when the
/// pointer is in the border zone of a floating window.  Resets both when the
/// pointer leaves.
///
/// Returns `true` when the hover consumed the event and the caller should
/// skip further motion handling.
pub fn handle_floating_resize_hover() -> bool {
    if has_visible_tiled_client() {
        return false;
    }

    if let Some(win) = find_floating_win_at_resize_border() {
        let (cursor_idx, dir, should_focus) = {
            let globals = get_globals();
            let (cursor_idx, dir) = if let Some(c) = globals.clients.get(&win) {
                let (px, py) = get_root_ptr().unwrap_or_default();
                let hit_x = px - c.geo.x;
                let hit_y = py - c.geo.y;
                let dir = get_resize_direction(c.geo.w, c.geo.h, hit_x, hit_y);
                (dir.cursor_index(), dir)
            } else {
                (1, ResizeDirection::BottomRight)
            };
            let should_focus =
                globals.monitors.get(globals.selmon).and_then(|m| m.sel) != Some(win);
            (cursor_idx, dir, should_focus)
        };

        set_root_cursor(cursor_idx);
        get_globals_mut().altcursor = AltCursor::Resize;
        get_globals_mut().resize_direction = Some(dir);

        if should_focus {
            focus(Some(win));
        }
        return true;
    }

    if get_globals().altcursor == AltCursor::Resize {
        get_globals_mut().resize_direction = None;
        crate::mouse::reset_cursor();
    }
    false
}

pub fn handle_sidebar_hover(root_x: i32, root_y: i32) -> bool {
    let globals = get_globals();
    let Some(mon) = globals.monitors.get(globals.selmon) else {
        return false;
    };

    if root_x > mon.monitor_rect.x + mon.monitor_rect.w - SIDEBAR_WIDTH {
        if get_globals().altcursor == AltCursor::None && root_y > get_globals().bh + 60 {
            set_root_cursor(8);
            get_globals_mut().altcursor = AltCursor::Sidebar;
        }
        return true;
    }

    if get_globals().altcursor == AltCursor::Sidebar {
        get_globals_mut().altcursor = AltCursor::None;
        set_root_cursor(0);
        return true;
    }

    false
}

// ── Modal hover-resize loop ──────────────────────────────────────────────────

/// Enter a modal grab loop that waits for a click while the cursor is in the
/// resize border zone.
///
/// | Input            | Action                                         |
/// |------------------|------------------------------------------------|
/// | Left click       | Resize (directional) — or move if top-middle   |
/// | Right click      | Move                                           |
/// | Escape           | Abort                                          |
/// | Cursor leaves    | Abort                                          |
/// | Button release   | Abort (spurious release from prior click)      |
///
/// Returns `true` if the function entered its loop (caller should skip normal
/// focus/event handling), `false` if the cursor was not in a resize border.
pub fn hover_resize_mouse() -> bool {
    if !is_in_resize_border() {
        return false;
    }

    let Some(conn) = grab_pointer_with_keys(1) else {
        return false;
    };

    let mut action_started = false;

    loop {
        let Ok(event) = conn.wait_for_event() else {
            break;
        };

        match &event {
            x11rb::protocol::Event::ButtonRelease(_) => break,

            x11rb::protocol::Event::MotionNotify(_) => {
                if !is_in_resize_border() {
                    // Focus the window under the cursor when leaving.
                    let should_refocus = get_cursor_client_win().filter(|&hover_win| {
                        get_globals()
                            .monitors
                            .get(get_globals().selmon)
                            .and_then(|m| m.sel)
                            != Some(hover_win)
                    });
                    if let Some(hover_win) = should_refocus {
                        focus(Some(hover_win));
                    }
                    break;
                }
            }

            x11rb::protocol::Event::KeyPress(k) => {
                if k.detail == KEYCODE_ESCAPE {
                    break;
                }
            }

            x11rb::protocol::Event::ButtonPress(bp) => {
                action_started = true;
                ungrab(conn);

                let Some(win) = get_sel_win() else { break };
                let (geo, w, h) = {
                    let globals = get_globals();
                    let Some(c) = globals.clients.get(&win) else {
                        break;
                    };
                    (c.geo, c.geo.w, c.geo.h)
                };

                // Query cursor position relative to the client window.
                let (root_x, root_y, win_x, win_y) =
                    query_pointer_on_win(win).unwrap_or((0, 0, 0, 0));

                match bp.detail {
                    // Right-click → move
                    3 => {
                        warp_into(win);
                        move_mouse();
                    }
                    // Left-click
                    1 => {
                        if is_at_top_middle_edge(&geo, root_x, root_y) {
                            warp_into(win);
                            move_mouse();
                        } else {
                            let dir = get_resize_direction(w, h, win_x, win_y);
                            warp_pointer_resize(win, dir);
                            resize_mouse_directional(Some(dir));
                        }
                    }
                    _ => {}
                }
                break;
            }

            _ => {}
        }
    }

    if !action_started {
        ungrab(conn);
    }

    true
}

// ── Utilities ────────────────────────────────────────────────────────────────

/// Return the window ID of the client currently under the mouse pointer.
///
/// Uses `query_pointer` on the root window to get the actual window under the
/// cursor, respecting stacking order. This ensures that if multiple windows
/// overlap, the topmost (visible) one is returned, not just any window whose
/// geometry contains the cursor.
pub fn get_cursor_client_win() -> Option<Window> {
    let x11 = get_x11();
    let conn = x11.conn.as_ref()?;
    let globals = get_globals();

    // Query pointer on root to get the actual child window under cursor
    let reply = conn.query_pointer(globals.root).ok()?.reply().ok()?;

    // child will be NONE if cursor is over root (no window)
    if reply.child == x11rb::NONE {
        return None;
    }

    // Convert the window under cursor to a client
    crate::client::win_to_client(reply.child)
}

/// Query the pointer position in both root and window-local coordinates.
fn query_pointer_on_win(win: Window) -> Option<(i32, i32, i32, i32)> {
    let x11 = get_x11();
    let conn = x11.conn.as_ref()?;
    let reply = conn.query_pointer(win).ok()?.reply().ok()?;
    Some((
        reply.root_x as i32,
        reply.root_y as i32,
        reply.win_x as i32,
        reply.win_y as i32,
    ))
}
