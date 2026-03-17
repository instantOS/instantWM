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

use crate::contexts::{CoreCtx, WmCtx, WmCtxX11};
// focus() is used via focus_soft() in this module
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use super::constants::{KEYCODE_ESCAPE, RESIZE_BORDER_ZONE};
use super::cursor::{set_cursor_default, set_cursor_resize};
use super::warp::get_root_ptr;

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
    let Some(c) = ctx.client(win) else {
        return;
    };
    let (x_off, y_off) = dir.warp_offset(c.geo.w, c.geo.h, c.border_width);
    let target_x = c.geo.x + x_off;
    let target_y = c.geo.y + y_off;
    match ctx {
        WmCtx::X11(x11) => {
            let x11_win: Window = win.into();
            let _ = x11.x11.conn.warp_pointer(
                x11rb::NONE,
                x11_win,
                0,
                0,
                0,
                0,
                x_off as i16,
                y_off as i16,
            );
            let _ = x11.x11.conn.flush();
        }
        WmCtx::Wayland(wl) => {
            wl.wayland
                .backend
                .warp_pointer(target_x as f64, target_y as f64);
        }
    }
}

// ── Border detection ─────────────────────────────────────────────────────────

/// Find a visible floating window whose resize border zone contains (`x`, `y`).
/// Returns `None` if the cursor is on the bar or no window matches.
pub fn find_floating_win_at_resize_border(ctx: &WmCtx, x: i32, y: i32) -> Option<WindowId> {
    let has_tiling = ctx.g().selected_monitor().is_tiling_layout();

    let mon = ctx.g().selected_monitor();
    if mon.showbar && y < mon.monitor_rect.y + ctx.g().cfg.bar_height {
        return None;
    }

    let selected = mon.selected_tags();
    for (w, c) in mon.iter_clients(ctx.g().clients.map()) {
        if !c.is_visible_on_tags(selected) {
            continue;
        }
        if !c.is_floating && has_tiling {
            continue;
        }
        if geometry::is_point_in_resize_border(&c.geo, x, y, RESIZE_BORDER_ZONE) {
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

pub fn selected_hover_resize_target_at(
    ctx: &WmCtx,
    root_x: i32,
    root_y: i32,
) -> Option<(WindowId, ResizeDirection)> {
    let win = ctx.selected_client()?;
    let c = ctx.client(win)?;
    let mon = ctx.g().selected_monitor();
    let bar_h = ctx.g().cfg.bar_height.max(1);
    if mon.showbar && root_y >= mon.bar_y && root_y < mon.bar_y + bar_h {
        return None;
    }
    let selected_tags = mon.selected_tags();
    let has_tiling = mon.is_tiling_layout();
    if c.is_hidden || !c.is_visible_on_tags(selected_tags) {
        return None;
    }
    if !c.is_floating && has_tiling {
        return None;
    }
    if !geometry::is_point_in_resize_border(&c.geo, root_x, root_y, RESIZE_BORDER_ZONE) {
        return None;
    }
    let hit_x = root_x - c.geo.x;
    let hit_y = root_y - c.geo.y;
    let dir = get_resize_direction(c.geo.w, c.geo.h, hit_x, hit_y);
    Some((win, dir))
}

fn clear_hover_resize_offer(ctx: &mut WmCtx) {
    ctx.g_mut().behavior.cursor_icon = AltCursor::None;
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

    for (w, c) in mon.iter_clients(ctx.g().clients.map()) {
        if Some(w) == skip_win {
            continue;
        }
        if !c.is_visible_on_tags(selected) || c.is_hidden || c.is_floating {
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
    let Some(c) = ctx.client(win) else {
        return false;
    };
    let has_tiling = ctx.g().selected_monitor().is_tiling_layout();
    if !c.is_floating && has_tiling {
        return false;
    }

    let mon = ctx.g().selected_monitor();
    if mon.showbar && y < mon.monitor_rect.y + ctx.g().cfg.bar_height {
        return false;
    }
    geometry::is_point_in_resize_border(&c.geo, x, y, RESIZE_BORDER_ZONE)
}

/// Check whether any visible client on the current monitor is tiled.
fn has_visible_tiled_client(ctx: &WmCtx) -> bool {
    let has_tiling = ctx.g().selected_monitor().is_tiling_layout();

    let mon = ctx.g().selected_monitor();
    let selected = mon.selected_tags();

    for (_w, c) in mon.iter_clients(ctx.g().clients.map()) {
        if c.is_visible_on_tags(selected) && !(c.is_floating || !has_tiling) {
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
        ctx.g_mut().behavior.cursor_icon = AltCursor::Resize;
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

    if ctx.g_mut().behavior.cursor_icon == AltCursor::Resize {
        clear_hover_resize_offer(ctx);
    }
    false
}

pub fn handle_sidebar_hover(ctx: &mut WmCtx, root_x: i32, root_y: i32) -> bool {
    let mon = ctx.g_mut().selected_monitor();

    if root_x > mon.monitor_rect.x + mon.monitor_rect.w - SIDEBAR_WIDTH {
        if ctx.g_mut().behavior.cursor_icon == AltCursor::None
            && root_y > ctx.g_mut().cfg.bar_height + 60
        {
            set_cursor_resize(ctx, Some(ResizeDirection::TopLeft));
            ctx.g_mut().behavior.cursor_icon = AltCursor::Sidebar;
        }
        return true;
    }

    if ctx.g_mut().behavior.cursor_icon == AltCursor::Sidebar {
        ctx.g_mut().behavior.cursor_icon = AltCursor::None;
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
pub fn hover_resize_mouse(ctx: &mut WmCtxX11) -> bool {
    {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        let Some((x, y)) = get_root_ptr(&wm_ctx) else {
            return false;
        };
        let in_border = is_in_resize_border(&wm_ctx, x, y);
        if !in_border {
            return false;
        }

        crate::mouse::handle_floating_resize_hover(&mut wm_ctx, x, y, false);
    };

    let action_started = run_hover_resize_loop(ctx);

    if !action_started {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        clear_hover_resize_offer(&mut wm_ctx);
    }

    true
}

/// Shared modal grab loop for hover-resize operations.
///
/// Waits for the user to either click (starting resize/move), move the cursor
/// outside the resize border (focusing the window under cursor), or press
/// Escape (aborting). Returns `true` if a resize/move action was started.
fn run_hover_resize_loop(ctx: &mut WmCtxX11) -> bool {
    let mut action_started = false;

    super::grab::mouse_drag_loop(
        ctx,
        MouseButton::Left,
        crate::types::Cursor::Resize,
        true,
        |ctx, event| {
            match event {
                x11rb::protocol::Event::ButtonRelease(_) => false,

                x11rb::protocol::Event::MotionNotify(_) => {
                    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
                    let in_border = get_root_ptr(&wm_ctx)
                        .map(|(x, y)| is_in_resize_border(&wm_ctx, x, y))
                        .unwrap_or(false);
                    if !in_border {
                        let sel = wm_ctx.selected_client();
                        let target = get_cursor_client_win(&mut wm_ctx)
                            .filter(|&w| Some(w) != sel)
                            .or_else(|| {
                                let (x, y) = get_root_ptr(&wm_ctx)?;
                                find_tiled_win_at_point(&wm_ctx, x, y, sel)
                            });
                        if let Some(win) = target {
                            crate::focus::focus_soft(&mut wm_ctx, Some(win));
                        }
                        return false;
                    }
                    true
                }

                x11rb::protocol::Event::KeyPress(k) => {
                    if k.detail == KEYCODE_ESCAPE {
                        return false;
                    }
                    true
                }

                x11rb::protocol::Event::ButtonPress(bp) => {
                    action_started = true;
                    let mut wm_ctx = WmCtx::X11(ctx.reborrow());

                    let Some(win) = wm_ctx.selected_client() else {
                        return false;
                    };
                    let (geo, w, h) = {
                        let Some(c) = wm_ctx.client(win) else {
                            return false;
                        };
                        (c.geo, c.geo.w, c.geo.h)
                    };

                    // Query cursor position relative to the client window.
                    let (root_x, root_y, win_x, win_y) =
                        query_pointer_on_win(&mut wm_ctx, win).unwrap_or((0, 0, 0, 0));

                    let btn = MouseButton::from_u8(bp.detail).unwrap_or(MouseButton::Left);
                    wm_ctx.raise_interactive(win);
                    match bp.detail {
                        // Right-click → move
                        3 => {
                            let mut wm_ctx_x11 = ctx.reborrow();
                            let mut wmctx = WmCtx::X11(wm_ctx_x11.reborrow());
                            super::warp::warp_into(&mut wmctx, win);
                            crate::backend::x11::mouse::move_mouse_x11(&mut wm_ctx_x11, btn, None);
                        }
                        // Left-click
                        1 => {
                            if is_at_top_middle_edge(&geo, root_x, root_y) {
                                let mut wm_ctx_x11 = ctx.reborrow();
                                let mut wmctx = WmCtx::X11(wm_ctx_x11.reborrow());
                                super::warp::warp_into(&mut wmctx, win);
                                crate::backend::x11::mouse::move_mouse_x11(
                                    &mut wm_ctx_x11,
                                    btn,
                                    None,
                                );
                            } else {
                                let dir = get_resize_direction(w, h, win_x, win_y);
                                warp_pointer_resize(&mut wm_ctx, win, dir);
                                resize_mouse_directional(ctx, Some(dir), btn);
                            }
                        }
                        _ => {}
                    }
                    false
                }

                _ => true,
            }
        },
    );

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
pub fn floating_to_tiled_hover(ctx: &mut WmCtxX11) -> bool {
    // Pre-loop: do all checks and setup while we have wm_ctx
    {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());

        // Selected window must be floating in a tiling layout
        let selected_window = match wm_ctx.selected_client() {
            Some(w) => w,
            None => return false,
        };
        let is_tiling_layout = wm_ctx.g().selected_monitor().is_tiling_layout();
        let sel_geo = match wm_ctx.client(selected_window) {
            Some(c) if c.is_floating || !is_tiling_layout => c.geo,
            _ => return false,
        };

        // Must have a different, tiled window under the cursor
        let hovered_win = match get_cursor_client_win(&mut wm_ctx) {
            Some(w) if w != selected_window => w,
            _ => return false,
        };
        let has_tiling = wm_ctx.g().selected_monitor().is_tiling_layout();
        if !has_tiling {
            return false;
        }
        let hovered_is_tiled = wm_ctx
            .g()
            .clients
            .get(&hovered_win)
            .map(|c| !c.is_floating)
            .unwrap_or(false);
        if !hovered_is_tiled {
            return false;
        }

        let Some((x, y)) = get_root_ptr(&wm_ctx) else {
            return false;
        };

        // If cursor is already outside the resize border, just focus the tiled window
        if !geometry::is_point_in_resize_border(&sel_geo, x, y, RESIZE_BORDER_ZONE) {
            crate::focus::focus_soft(&mut wm_ctx, Some(hovered_win));
            return true;
        }

        // Activate resize cursor and enter the grab loop
        handle_floating_resize_hover(&mut wm_ctx, x, y, false);

        // Return the coordinates for the loop
        (x, y)
    };

    let action_started = run_hover_resize_loop(ctx);

    if !action_started {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        clear_hover_resize_offer(&mut wm_ctx);
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

pub fn get_cursor_client_win_with_conn(
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
    if core.g.clients.contains_key(&win) {
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
