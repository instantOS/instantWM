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

use crate::contexts::WmCtx;
use crate::focus::{focus, warp_into};
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use super::constants::{KEYCODE_ESCAPE, RESIZE_BORDER_ZONE};
use super::drag::move_mouse;
use super::grab::{grab_pointer_with_keys, ungrab_ctx, wait_event};
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
fn set_root_cursor(ctx: &WmCtx, cursor_index: usize) {
    let conn = ctx.x11.conn;
    if let Some(ref cur) = ctx.g.cfg.cursors[cursor_index] {
        let _ = change_window_attributes(
            conn,
            ctx.g.cfg.root,
            &ChangeWindowAttributesAux::new().cursor(cur.cursor),
        );
        let _ = conn.flush();
    }
}

/// Warp the pointer to the edge/corner of `win` described by `dir`.
fn warp_pointer_resize(ctx: &WmCtx, win: Window, dir: ResizeDirection) {
    let conn = ctx.x11.conn;
    let Some(c) = ctx.g.clients.get(&win) else {
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
pub fn find_floating_win_at_resize_border(ctx: &WmCtx) -> Option<Window> {
    let has_tiling = ctx
        .g
        .monitors
        .get(ctx.g.selmon)
        .map(|m| m.is_tiling_layout())
        .unwrap_or(true);

    let (px, py) = get_root_ptr(ctx)?;

    if let Some(mon) = ctx.g.monitors.get(ctx.g.selmon) {
        if mon.showbar && py < mon.monitor_rect.y + ctx.g.cfg.bh {
            return None;
        }
    }

    let mon = ctx.g.monitors.get(ctx.g.selmon)?;
    let selected = mon.selected_tags();
    for (w, c) in mon.iter_clients(&ctx.g.clients) {
        if !c.is_visible_on_tags(selected) {
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
pub fn is_in_resize_border(ctx: &WmCtx) -> bool {
    let Some(win) = ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.sel) else {
        return false;
    };
    let Some(c) = ctx.g.clients.get(&win) else {
        return false;
    };
    let has_tiling = ctx
        .g
        .monitors
        .get(ctx.g.selmon)
        .map(|m| m.is_tiling_layout())
        .unwrap_or(true);
    if !c.isfloating && has_tiling {
        return false;
    }

    let Some((px, py)) = get_root_ptr(ctx) else {
        return false;
    };
    if let Some(mon) = ctx.g.monitors.get(ctx.g.selmon) {
        if mon.showbar && py < mon.monitor_rect.y + ctx.g.cfg.bh {
            return false;
        }
    }
    is_point_in_resize_border(&c.geo, px, py)
}

/// Check whether any visible client on the current monitor is tiled.
fn has_visible_tiled_client(ctx: &WmCtx) -> bool {
    let has_tiling = ctx
        .g
        .monitors
        .get(ctx.g.selmon)
        .map(|m| m.is_tiling_layout())
        .unwrap_or(true);

    let Some(mon) = ctx.g.monitors.get(ctx.g.selmon) else {
        return false;
    };
    let selected = mon.selected_tags();

    for (_w, c) in mon.iter_clients(&ctx.g.clients) {
        if c.is_visible_on_tags(selected) && !(c.isfloating || !has_tiling) {
            return true;
        }
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
pub fn handle_floating_resize_hover(ctx: &mut WmCtx) -> bool {
    if has_visible_tiled_client(ctx) {
        return false;
    }

    if let Some(win) = find_floating_win_at_resize_border(ctx) {
        let (cursor_idx, dir, should_focus) = {
            let (cursor_idx, dir) = if let Some(c) = ctx.g.clients.get(&win) {
                let (px, py) = get_root_ptr(ctx).unwrap_or_default();
                let hit_x = px - c.geo.x;
                let hit_y = py - c.geo.y;
                let dir = get_resize_direction(c.geo.w, c.geo.h, hit_x, hit_y);
                (dir.cursor_index(), dir)
            } else {
                (1, ResizeDirection::BottomRight)
            };
            let should_focus = ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.sel) != Some(win);
            (cursor_idx, dir, should_focus)
        };

        set_root_cursor(ctx, cursor_idx);
        ctx.g.altcursor = AltCursor::Resize;
        ctx.g.resize_direction = Some(dir);

        if should_focus {
            focus(ctx, Some(win));
        }
        return true;
    }

    if ctx.g.altcursor == AltCursor::Resize {
        ctx.g.resize_direction = None;
        crate::mouse::reset_cursor(ctx);
    }
    false
}

pub fn handle_sidebar_hover(ctx: &mut WmCtx, root_x: i32, root_y: i32) -> bool {
    let Some(mon) = ctx.g.monitors.get(ctx.g.selmon) else {
        return false;
    };

    if root_x > mon.monitor_rect.x + mon.monitor_rect.w - SIDEBAR_WIDTH {
        if ctx.g.altcursor == AltCursor::None && root_y > ctx.g.cfg.bh + 60 {
            set_root_cursor(ctx, 8);
            ctx.g.altcursor = AltCursor::Sidebar;
        }
        return true;
    }

    if ctx.g.altcursor == AltCursor::Sidebar {
        ctx.g.altcursor = AltCursor::None;
        set_root_cursor(ctx, 0);
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
pub fn hover_resize_mouse(ctx: &mut WmCtx) -> bool {
    if !is_in_resize_border(ctx) {
        return false;
    }

    if !grab_pointer_with_keys(ctx, 1) {
        return false;
    }

    let mut action_started = false;

    loop {
        let Some(event) = wait_event(ctx) else {
            break;
        };

        match &event {
            x11rb::protocol::Event::ButtonRelease(_) => break,

            x11rb::protocol::Event::MotionNotify(_) => {
                if !is_in_resize_border(ctx) {
                    // Focus the window under the cursor when leaving.
                    let should_refocus = get_cursor_client_win(ctx).filter(|&hover_win| {
                        ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.sel) != Some(hover_win)
                    });
                    if let Some(hover_win) = should_refocus {
                        focus(ctx, Some(hover_win));
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
                ungrab_ctx(ctx);

                let Some(win) = ctx.g.monitors.get(ctx.g.selmon).and_then(|m| m.sel) else {
                    break;
                };
                let (geo, w, h) = {
                    let Some(c) = ctx.g.clients.get(&win) else {
                        break;
                    };
                    (c.geo, c.geo.w, c.geo.h)
                };

                // Query cursor position relative to the client window.
                let (root_x, root_y, win_x, win_y) =
                    query_pointer_on_win(ctx, win).unwrap_or((0, 0, 0, 0));

                match bp.detail {
                    // Right-click → move
                    3 => {
                        warp_into(ctx, win);
                        move_mouse(ctx);
                    }
                    // Left-click
                    1 => {
                        if is_at_top_middle_edge(&geo, root_x, root_y) {
                            warp_into(ctx, win);
                            move_mouse(ctx);
                        } else {
                            let dir = get_resize_direction(w, h, win_x, win_y);
                            warp_pointer_resize(ctx, win, dir);
                            resize_mouse_directional(ctx, Some(dir));
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
        ungrab_ctx(ctx);
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
pub fn get_cursor_client_win(ctx: &WmCtx) -> Option<Window> {
    let conn = ctx.x11.conn;

    // Query pointer on root to get the actual child window under cursor
    let reply = conn.query_pointer(ctx.g.cfg.root).ok()?.reply().ok()?;

    // child will be NONE if cursor is over root (no window)
    if reply.child == x11rb::NONE {
        return None;
    }

    // Convert the window under cursor to a client
    crate::client::win_to_client(reply.child)
}

/// Query the pointer position in both root and window-local coordinates.
fn query_pointer_on_win(ctx: &WmCtx, win: Window) -> Option<(i32, i32, i32, i32)> {
    let conn = ctx.x11.conn;
    let reply = conn.query_pointer(win).ok()?.reply().ok()?;
    Some((
        reply.root_x as i32,
        reply.root_y as i32,
        reply.win_x as i32,
        reply.win_y as i32,
    ))
}
