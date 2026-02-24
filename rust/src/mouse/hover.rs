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
use super::resize::resize_mouse;
use super::warp::get_root_ptr;

// ── Resize direction ─────────────────────────────────────────────────────────

/// Which edge or corner of a window the cursor is nearest to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeDirection {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl ResizeDirection {
    /// Index into `globals.cursors` for the appropriate resize cursor shape.
    pub fn cursor_index(self) -> usize {
        match self {
            Self::TopLeft => 8,     // XC_TOP_LEFT_CORNER
            Self::Top => 4,         // XC_SB_V_DOUBLE_ARROW
            Self::TopRight => 9,    // XC_TOP_RIGHT_CORNER
            Self::Right => 5,       // XC_SB_H_DOUBLE_ARROW
            Self::BottomRight => 7, // XC_BOTTOM_RIGHT_CORNER
            Self::Bottom => 4,      // XC_SB_V_DOUBLE_ARROW
            Self::BottomLeft => 6,  // XC_BOTTOM_LEFT_CORNER
            Self::Left => 5,        // XC_SB_H_DOUBLE_ARROW
        }
    }

    /// Warp offset (relative to the client window) to place the pointer on the
    /// edge/corner corresponding to this direction.
    pub fn warp_offset(self, w: i32, h: i32, bw: i32) -> (i32, i32) {
        match self {
            Self::TopLeft => (-bw, -bw),
            Self::Top => ((w + bw - 1) / 2, -bw),
            Self::TopRight => (w + bw - 1, -bw),
            Self::Right => (w + bw - 1, (h + bw - 1) / 2),
            Self::BottomRight => (w + bw - 1, h + bw - 1),
            Self::Bottom => ((w + bw - 1) / 2, h + bw - 1),
            Self::BottomLeft => (-bw, h + bw - 1),
            Self::Left => (-bw, (h + bw - 1) / 2),
        }
    }
}

/// Determine which edge or corner the cursor is closest to, using the cursor's
/// position *relative to the window* (win-local coordinates).
pub fn get_resize_direction(w: i32, h: i32, hit_x: i32, hit_y: i32) -> ResizeDirection {
    if hit_y > h / 2 {
        // bottom half
        if hit_x < w / 3 {
            if hit_y < 2 * h / 3 {
                ResizeDirection::Left
            } else {
                ResizeDirection::BottomLeft
            }
        } else if hit_x > 2 * w / 3 {
            if hit_y < 2 * h / 3 {
                ResizeDirection::Right
            } else {
                ResizeDirection::BottomRight
            }
        } else {
            ResizeDirection::Bottom
        }
    } else {
        // top half
        if hit_x < w / 3 {
            if hit_y > h / 3 {
                ResizeDirection::Left
            } else {
                ResizeDirection::TopLeft
            }
        } else if hit_x > 2 * w / 3 {
            if hit_y > h / 3 {
                ResizeDirection::Right
            } else {
                ResizeDirection::TopRight
            }
        } else {
            ResizeDirection::Top
        }
    }
}

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

/// Return `true` when the pointer is in the resize-border zone of the
/// currently selected floating window.
///
/// The zone is a [`RESIZE_BORDER_ZONE`]-pixel band around the outside of the
/// window.  Returns `false` when:
/// * No window is selected, or it is tiled in a tiling layout.
/// * The cursor is on the bar.
/// * The cursor is inside the window content.
/// * The cursor is further than `RESIZE_BORDER_ZONE` from any edge.
pub fn is_in_resize_border() -> bool {
    let globals = get_globals();

    let Some(win) = get_sel_win() else {
        return false;
    };
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
    let geo = c.geo;

    // Release the globals borrow before calling get_root_ptr (may re-borrow).
    let (px, py) = {
        let _ = globals;
        let Some((px, py)) = get_root_ptr() else {
            return false;
        };
        (px, py)
    };

    let globals = get_globals();
    if let Some(mon) = globals.monitors.get(globals.selmon) {
        if mon.showbar && py < mon.monitor_rect.y + globals.bh {
            return false;
        }
    }

    // Inside the window content area → not a border.
    if px > geo.x && px < geo.x + geo.w && py > geo.y && py < geo.y + geo.h {
        return false;
    }

    // Too far from any edge → not in border zone.
    if py < geo.y - RESIZE_BORDER_ZONE
        || px < geo.x - RESIZE_BORDER_ZONE
        || py > geo.y + geo.h + RESIZE_BORDER_ZONE
        || px > geo.x + geo.w + RESIZE_BORDER_ZONE
    {
        return false;
    }

    true
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
/// Also focuses the floating window under the cursor if it differs from `sel`.
///
/// Returns `true` when the hover consumed the event and the caller should
/// skip further motion handling.
pub fn handle_floating_resize_hover() -> bool {
    // Only activate when sel is floating (or non-tiling layout).
    {
        let globals = get_globals();
        let Some(win) = get_sel_win() else {
            return false;
        };
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
    }

    // Don't activate when there are visible tiled windows (mixed layout).
    if has_visible_tiled_client() {
        return false;
    }

    if is_in_resize_border() {
        if get_globals().altcursor != AltCursor::Resize {
            set_root_cursor(1); // crosshair / generic resize
            get_globals_mut().altcursor = AltCursor::Resize;
        }

        // Focus the floating window under the cursor if it differs from sel.
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

        return true;
    }

    // Left the resize zone — reset if we were in resize mode.
    if get_globals().altcursor == AltCursor::Resize {
        crate::mouse::reset_cursor();
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
                        move_mouse(&Arg::default());
                    }
                    // Left-click
                    1 => {
                        if is_at_top_middle_edge(&geo, root_x, root_y) {
                            warp_into(win);
                            move_mouse(&Arg::default());
                        } else {
                            let dir = get_resize_direction(w, h, win_x, win_y);
                            warp_pointer_resize(win, dir);
                            resize_mouse(&Arg::default());
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
fn get_cursor_client_win() -> Option<Window> {
    let (ptr_x, ptr_y) = get_root_ptr()?;
    let globals = get_globals();
    for mon in &globals.monitors {
        let mut current = mon.clients;
        while let Some(win) = current {
            match globals.clients.get(&win) {
                Some(c) if c.is_visible() && c.geo.contains_point(ptr_x, ptr_y) => {
                    return Some(win);
                }
                Some(c) => current = c.next,
                None => break,
            }
        }
    }
    None
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
