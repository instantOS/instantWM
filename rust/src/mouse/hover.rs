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

use crate::backend::x11::X11BackendRef;
use crate::contexts::{CoreCtx, WmCtx};
use crate::globals::X11RuntimeConfig;
// focus() is used via focus_soft() in this module
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use super::constants::{KEYCODE_ESCAPE, RESIZE_BORDER_ZONE};
use super::cursor::{set_cursor_default, set_cursor_resize};
use super::grab::{grab_pointer_with_keys_ctx, ungrab_ctx, wait_event_ctx};
use super::warp::{get_root_ptr, warp_into_ctx_x11};

use super::resize::resize_mouse_directional;

// ── Resize direction ─────────────────────────────────────────────────────────

// ResizeDirection and get_resize_direction are now defined in types.rs

/// Returns `true` when the cursor (in root coordinates) is on the top-middle
/// edge of a window — used to distinguish a *move* from a *resize*.
pub fn is_at_top_middle_edge(geo: &Rect, root_x: i32, root_y: i32) -> bool {
    let at_top = root_y >= geo.y - RESIZE_BORDER_ZONE && root_y < geo.y + RESIZE_BORDER_ZONE;
    let in_middle_third = root_x >= geo.x + geo.w / 3 && root_x <= geo.x + 2 * geo.w / 3;
    at_top && in_middle_third
}

// ── Cursor helpers ───────────────────────────────────────────────────────────

/// Warp the pointer to the edge/corner of `win` described by `dir`.
fn warp_pointer_resize(ctx: &mut WmCtx, win: WindowId, dir: ResizeDirection) {
    let (conn, clients) = match ctx {
        WmCtx::X11(x11) => (&x11.x11.conn, &mut x11.core.g.clients),
        WmCtx::Wayland(_) => return,
    };
    let Some(c) = clients.get(&win) else {
        return;
    };
    let (x_off, y_off) = dir.warp_offset(c.geo.w, c.geo.h, c.border_width);
    let x11_win: Window = win.into();
    let _ = conn.warp_pointer(x11rb::NONE, x11_win, 0, 0, 0, 0, x_off as i16, y_off as i16);
    let _ = conn.flush();
}

// ── Border detection ─────────────────────────────────────────────────────────

/// Check if point (x, y) is in the resize-border zone of a window with geometry geo.
/// The zone is a [`RESIZE_BORDER_ZONE`]-pixel band around the outside of the window.
fn is_point_in_resize_border(geo: &Rect, x: i32, y: i32) -> bool {
    if x > geo.x && x < geo.x + geo.w && y > geo.y && y < geo.y + geo.h {
        return false;
    }
    if y < geo.y - RESIZE_BORDER_ZONE
        || x < geo.x - RESIZE_BORDER_ZONE
        || y > geo.y + geo.h + RESIZE_BORDER_ZONE
        || x > geo.x + geo.w + RESIZE_BORDER_ZONE
    {
        return false;
    }
    true
}

/// Find a visible floating window whose resize border zone contains (`x`, `y`).
/// Returns `None` if the cursor is on the bar or no window matches.
pub fn find_floating_win_at_resize_border(ctx: &WmCtx, x: i32, y: i32) -> Option<WindowId> {
    let has_tiling = ctx.g().selected_monitor().is_tiling_layout();

    let mon = ctx.g().selected_monitor();
    if mon.showbar && y < mon.monitor_rect.y + ctx.g().cfg.bar_height {
        return None;
    }

    let selected = mon.selected_tags();
    for (w, c) in mon.iter_clients(&ctx.g().clients) {
        if !c.is_visible_on_tags(selected) {
            continue;
        }
        if !c.isfloating && has_tiling {
            continue;
        }
        if is_point_in_resize_border(&c.geo, x, y) {
            return Some(w);
        }
    }
    None
}

/// Return the floating window + direction currently targeted by hover-resize.
pub fn hover_resize_target_at(
    ctx: &WmCtx,
    root_x: i32,
    root_y: i32,
) -> Option<(WindowId, ResizeDirection)> {
    let win = find_floating_win_at_resize_border(ctx, root_x, root_y)?;
    let dir = ctx
        .g()
        .clients
        .get(&win)
        .map(|c| {
            let hit_x = root_x - c.geo.x;
            let hit_y = root_y - c.geo.y;
            get_resize_direction(c.geo.w, c.geo.h, hit_x, hit_y)
        })
        .unwrap_or(ResizeDirection::BottomRight);
    Some((win, dir))
}

fn clear_hover_resize_offer(ctx: &mut WmCtx) {
    ctx.g_mut().altcursor = AltCursor::None;
    ctx.g_mut().drag.resize_direction = None;
    set_cursor_default(ctx);
}

/// Find a visible tiled window at point (`x`, `y`), skipping `skip_win`.
///
/// Unlike [`get_cursor_client_win`] (which uses `query_pointer` and returns the
/// topmost X11 window), this walks the monitor's client list directly. This is
/// needed when a floating window is stacked on top: `query_pointer` would return
/// the floating window, but we want the tiled window *behind* it.
fn find_tiled_win_at_point(
    ctx: &WmCtx,
    x: i32,
    y: i32,
    skip_win: Option<WindowId>,
) -> Option<WindowId> {
    let mon = ctx.g().selected_monitor();
    let selected = mon.selected_tags();
    let has_tiling = mon.is_tiling_layout();
    if !has_tiling {
        return None;
    }

    for (w, c) in mon.iter_clients(&ctx.g().clients) {
        if Some(w) == skip_win {
            continue;
        }
        if !c.is_visible_on_tags(selected) || c.is_hidden || c.isfloating {
            continue;
        }
        // Check if the cursor is within the window's geometry (including border).
        let bw = c.border_width;
        if x >= c.geo.x - bw
            && x <= c.geo.x + c.geo.w + bw
            && y >= c.geo.y - bw
            && y <= c.geo.y + c.geo.h + bw
        {
            return Some(w);
        }
    }
    None
}

/// Return `true` when (`x`, `y`) is in the resize-border zone of the selected floating window.
pub fn is_in_resize_border(ctx: &WmCtx, x: i32, y: i32) -> bool {
    let Some(win) = ctx.selected_client() else {
        return false;
    };
    let Some(c) = ctx.g().clients.get(&win) else {
        return false;
    };
    let has_tiling = ctx.g().selected_monitor().is_tiling_layout();
    if !c.isfloating && has_tiling {
        return false;
    }

    let mon = ctx.g().selected_monitor();
    if mon.showbar && y < mon.monitor_rect.y + ctx.g().cfg.bar_height {
        return false;
    }
    is_point_in_resize_border(&c.geo, x, y)
}

/// Check whether any visible client on the current monitor is tiled.
fn has_visible_tiled_client(ctx: &WmCtx) -> bool {
    let has_tiling = ctx.g().selected_monitor().is_tiling_layout();

    let mon = ctx.g().selected_monitor();
    let selected = mon.selected_tags();

    for (_w, c) in mon.iter_clients(&ctx.g().clients) {
        if c.is_visible_on_tags(selected) && !(c.isfloating || !has_tiling) {
            return true;
        }
    }
    false
}

// ── Motion-notify hook ───────────────────────────────────────────────────────

/// Sets the resize cursor and `altcursor` when the pointer is in a floating
/// window's border zone; resets both when it leaves.  Returns `true` when the
/// event is consumed.
pub fn handle_floating_resize_hover(
    ctx: &mut WmCtx,
    root_x: i32,
    root_y: i32,
    do_focus: bool,
) -> bool {
    if let Some((win, dir)) = hover_resize_target_at(ctx, root_x, root_y) {
        set_cursor_resize(ctx, Some(dir));
        ctx.g_mut().altcursor = AltCursor::Resize;
        ctx.g_mut().drag.resize_direction = Some(dir);
        // Only focus when: do_focus requested AND no visible tiled clients.
        // When tiled clients exist, enter_notify handles focus transitions,
        // so motion_notify must not steal focus back to the floating window.
        let should_focus =
            do_focus && ctx.selected_client() != Some(win) && !has_visible_tiled_client(ctx);

        if should_focus {
            crate::focus::focus_soft(ctx, Some(win));
        }
        return true;
    }

    if ctx.g_mut().altcursor == AltCursor::Resize {
        clear_hover_resize_offer(ctx);
    }
    false
}

pub fn handle_sidebar_hover(ctx: &mut WmCtx, root_x: i32, root_y: i32) -> bool {
    let mon = ctx.g_mut().selected_monitor();

    if root_x > mon.monitor_rect.x + mon.monitor_rect.w - SIDEBAR_WIDTH {
        if ctx.g_mut().altcursor == AltCursor::None && root_y > ctx.g_mut().cfg.bar_height + 60 {
            set_cursor_resize(ctx, Some(ResizeDirection::TopLeft));
            ctx.g_mut().altcursor = AltCursor::Sidebar;
        }
        return true;
    }

    if ctx.g_mut().altcursor == AltCursor::Sidebar {
        ctx.g_mut().altcursor = AltCursor::None;
        set_cursor_default(ctx);
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
    require_x11_ret!(ctx, false);
    let Some((x, y)) = get_root_ptr(ctx) else {
        return false;
    };
    let _sel = ctx.selected_client();
    let in_border = is_in_resize_border(ctx, x, y);
    if !in_border {
        return false;
    }

    if !grab_pointer_with_keys_ctx(ctx, 1) {
        return false;
    }

    crate::mouse::handle_floating_resize_hover(ctx, x, y, false);

    let action_started = run_hover_resize_loop(ctx);

    if !action_started {
        ungrab_ctx(ctx);
        clear_hover_resize_offer(ctx);
    }

    true
}

/// Shared modal grab loop for hover-resize operations.
///
/// Waits for the user to either click (starting resize/move), move the cursor
/// outside the resize border (focusing the window under cursor), or press
/// Escape (aborting). Returns `true` if a resize/move action was started.
fn run_hover_resize_loop(ctx: &mut WmCtx) -> bool {
    let mut action_started = false;

    loop {
        let Some(event) = wait_event_ctx(ctx) else {
            break;
        };

        match &event {
            x11rb::protocol::Event::ButtonRelease(_) => break,

            x11rb::protocol::Event::MotionNotify(_) => {
                let in_border = get_root_ptr(ctx)
                    .map(|(x, y)| is_in_resize_border(ctx, x, y))
                    .unwrap_or(false);
                if !in_border {
                    // Focus the window under the cursor when leaving the
                    // resize border zone.  Normally get_cursor_client_win
                    // returns the correct window (since the cursor is outside
                    // the floating window).  Fall back to searching the
                    // client list if it returns the already-selected window.
                    let sel = ctx.selected_client();
                    let target = get_cursor_client_win(ctx)
                        .filter(|&w| Some(w) != sel)
                        .or_else(|| {
                            let (x, y) = get_root_ptr(ctx)?;
                            find_tiled_win_at_point(ctx, x, y, sel)
                        });
                    if let Some(win) = target {
                        crate::focus::focus_soft(ctx, Some(win));
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

                let Some(win) = ctx.selected_client() else {
                    break;
                };
                let (geo, w, h) = {
                    let Some(c) = ctx.g_mut().clients.get(&win) else {
                        break;
                    };
                    (c.geo, c.geo.w, c.geo.h)
                };

                // Query cursor position relative to the client window.
                let (root_x, root_y, win_x, win_y) =
                    query_pointer_on_win(ctx, win).unwrap_or((0, 0, 0, 0));

                let btn = MouseButton::from_u8(bp.detail).unwrap_or(MouseButton::Left);
                match bp.detail {
                    // Right-click → move
                    3 => {
                        if let WmCtx::X11(x11) = ctx {
                            warp_into_ctx_x11(x11, win);
                            crate::mouse::move_mouse(x11, btn);
                        }
                    }
                    // Left-click
                    1 => {
                        if is_at_top_middle_edge(&geo, root_x, root_y) {
                            if let WmCtx::X11(x11) = ctx {
                                warp_into_ctx_x11(x11, win);
                                crate::mouse::move_mouse(x11, btn);
                            }
                        } else {
                            let dir = get_resize_direction(w, h, win_x, win_y);
                            warp_pointer_resize(ctx, win, dir);
                            if let WmCtx::X11(x11) = ctx {
                                resize_mouse_directional(x11, Some(dir), btn);
                            }
                        }
                    }
                    _ => {}
                }
                break;
            }

            _ => {}
        }
    }

    action_started
}

/// Handle the transition from a floating window to a tiled window.
///
/// When the selected window is floating and the cursor enters a tiled window,
/// this activates the resize offer cursor.  If the cursor is in the floating
/// window's resize border zone, a modal grab loop waits for the user to either
/// click (resize/move) or move far enough away (deactivate + focus tiled).
/// If the cursor has already moved past the border zone, the tiled window is
/// focused immediately.
///
/// Returns `true` if the transition was handled.
pub fn floating_to_tiled_hover(ctx: &mut WmCtx) -> bool {
    // Selected window must be floating in a tiling layout
    let selected_window = match ctx.selected_client() {
        Some(w) => w,
        None => return false,
    };
    let is_tiling_layout = ctx.g().selected_monitor().is_tiling_layout();
    let sel_geo = match ctx.g().clients.get(&selected_window) {
        Some(c) if c.isfloating || !is_tiling_layout => c.geo,
        _ => return false,
    };

    // Must have a different, tiled window under the cursor
    let hovered_win = match get_cursor_client_win(ctx) {
        Some(w) if w != selected_window => w,
        _ => return false,
    };
    let has_tiling = ctx.g_mut().selected_monitor().is_tiling_layout();
    if !has_tiling {
        return false;
    }
    let hovered_is_tiled = ctx
        .g()
        .clients
        .get(&hovered_win)
        .map(|c| !c.isfloating)
        .unwrap_or(false);
    if !hovered_is_tiled {
        return false;
    }

    let Some((x, y)) = get_root_ptr(ctx) else {
        return false;
    };

    // If cursor is already outside the resize border, just focus the tiled window
    if !is_point_in_resize_border(&sel_geo, x, y) {
        crate::focus::focus_soft(ctx, Some(hovered_win));
        return true;
    }

    // Activate resize cursor and enter the grab loop
    handle_floating_resize_hover(ctx, x, y, false);

    if !grab_pointer_with_keys_ctx(ctx, 1) {
        return false;
    }

    let action_started = run_hover_resize_loop(ctx);

    if !action_started {
        ungrab_ctx(ctx);
        clear_hover_resize_offer(ctx);
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
pub fn get_cursor_client_win(ctx: &mut WmCtx) -> Option<WindowId> {
    let (conn, root, core) = match ctx {
        WmCtx::X11(x11) => (x11.x11.conn, x11.x11_runtime.root, &mut x11.core),
        WmCtx::Wayland(_) => return None,
    };
    get_cursor_client_win_with_conn(core, conn, root)
}

pub fn get_cursor_client_win_x11(
    core: &CoreCtx,
    x11: &X11BackendRef,
    x11_runtime: &X11RuntimeConfig,
) -> Option<WindowId> {
    get_cursor_client_win_with_conn(core, x11.conn, x11_runtime.root)
}

fn get_cursor_client_win_with_conn(
    core: &CoreCtx,
    conn: &x11rb::rust_connection::RustConnection,
    root: x11rb::protocol::xproto::Window,
) -> Option<WindowId> {
    // Query pointer on root to get the actual child window under cursor
    let reply = conn.query_pointer(root).ok()?.reply().ok()?;

    // child will be NONE if cursor is over root (no window)
    if reply.child == x11rb::NONE {
        return None;
    }

    let win = WindowId::from(reply.child);
    if core.g.clients.contains(&win) {
        Some(win)
    } else {
        None
    }
}

/// Query the pointer position in both root and window-local coordinates.
fn query_pointer_on_win(ctx: &mut WmCtx, win: WindowId) -> Option<(i32, i32, i32, i32)> {
    let conn = match ctx {
        WmCtx::X11(x11) => x11.x11.conn,
        WmCtx::Wayland(_) => return None,
    };
    let x11_win: Window = win.into();
    let reply = conn.query_pointer(x11_win).ok()?.reply().ok()?;
    Some((
        reply.root_x as i32,
        reply.root_y as i32,
        reply.win_x as i32,
        reply.win_y as i32,
    ))
}
