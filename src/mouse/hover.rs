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
//! | Function                                      | Called from          | Purpose                                    |
//! |-----------------------------------------------|----------------------|--------------------------------------------|
//! | [`update_floating_resize_offer_at`]           | `motion_notify`      | Update resize offer and cursor feedback    |
//! | [`update_selected_resize_offer_at`]           | Wayland motion       | Update selected-window resize offer        |
//! | [`commit_x11_hover_offer`]                    | X11 button press     | Commit current offer to move/resize        |
//! | [`run_x11_hover_resize_offer_loop`]           | `enter_notify`, etc. | Modal loop: wait for click near border     |

use crate::contexts::{WmCtx, WmCtxX11};
use crate::globals::{Globals, HoverOffer};
use crate::types::{
    AltCursor, MouseButton, Point, Rect, ResizeDirection, WindowId, get_resize_direction,
};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, Window};

use super::constants::{KEYCODE_ESCAPE, RESIZE_BORDER_ZONE};
use super::cursor::set_cursor_style;

use super::resize::resize_mouse_directional;

// ── Hover offer helpers ──────────────────────────────────────────────────────
//
// Pure hover-offer state lives on [`crate::globals::HoverOffer`] /
// [`crate::globals::DragState`]; these functions apply the matching cursor.

/// Window and direction selected by the resize-border hit test.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HoverResizeTarget {
    pub win: WindowId,
    pub dir: ResizeDirection,
    pub geo: Rect,
}

/// Activate a resize hover offer and apply the matching cursor.
fn offer_hover_resize(ctx: &mut WmCtx, target: HoverResizeTarget) {
    ctx.core_mut()
        .globals_mut()
        .drag
        .set_hover_offer(HoverOffer::Resize {
            win: target.win,
            dir: target.dir,
        });
    set_cursor_style(ctx, AltCursor::Resize(target.dir));
}

/// Clear any active hover offer and reset the cursor if the state changed.
pub fn clear_hover_offer(ctx: &mut WmCtx) {
    if ctx.core_mut().globals_mut().drag.clear_hover_offer() {
        set_cursor_style(ctx, AltCursor::Default);
    }
}

/// Commit the current X11 resize offer to a move or resize operation.
///
/// Returns `false` when there is no resize offer or the mouse button is not a
/// valid commit button for hover resize.
pub fn commit_x11_hover_offer(ctx: &mut WmCtxX11, btn: MouseButton) -> bool {
    if btn != MouseButton::Left && btn != MouseButton::Right {
        return false;
    }

    let Some((win, dir)) = ctx.core.globals().drag.hover_offer.resize_target() else {
        return false;
    };

    let move_from_top_middle = {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        clear_hover_offer(&mut wm_ctx);
        if wm_ctx.core().selected_client() != Some(win) {
            crate::focus::focus_soft(&mut wm_ctx, Some(win));
        }

        let Some(c) = wm_ctx.core().client(win) else {
            return false;
        };
        wm_ctx
            .pointer_location()
            .map(|p| c.geo.is_at_top_middle_edge(p, RESIZE_BORDER_ZONE))
            .unwrap_or(dir == ResizeDirection::Top)
    };

    if btn == MouseButton::Right || move_from_top_middle {
        crate::backend::x11::mouse::move_mouse_x11(ctx, btn, None);
    } else {
        resize_mouse_directional(ctx, Some(dir), btn);
    }

    true
}

fn resize_target_for_window(
    globals: &Globals,
    win: WindowId,
    root_x: i32,
    root_y: i32,
) -> Option<HoverResizeTarget> {
    let c = globals.clients.get(&win)?;
    let mon = globals.selected_monitor();
    let selected_tags = mon.selected_tags();
    let has_tiling = mon.is_tiling_layout();

    if !c.is_visible(selected_tags) {
        return None;
    }
    if !c.mode.is_floating() && has_tiling {
        return None;
    }
    if !c
        .geo
        .contains_resize_border_point(Point::new(root_x, root_y), RESIZE_BORDER_ZONE)
    {
        return None;
    }

    let hit_x = root_x - c.geo.x;
    let hit_y = root_y - c.geo.y;
    Some(HoverResizeTarget {
        win,
        dir: get_resize_direction(c.geo.w, c.geo.h, hit_x, hit_y),
        geo: c.geo,
    })
}

fn pointer_in_bar(globals: &Globals, root_y: i32) -> bool {
    let mon = globals.selected_monitor();
    crate::bar::monitor_bar_visible(globals, mon)
        && root_y >= mon.bar_y
        && root_y < mon.bar_y + mon.bar_height.max(1)
}

// ── Cursor helpers ───────────────────────────────────────────────────────────

/// Warp the pointer to the edge/corner of `win` described by `dir`.
fn warp_pointer_resize(ctx: &mut WmCtx, win: WindowId, dir: ResizeDirection) {
    let Some(c) = ctx.core().client(win) else {
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

/// Return the floating window + direction currently targeted by hover-resize.
fn hover_resize_target_at(
    globals: &Globals,
    root_x: i32,
    root_y: i32,
) -> Option<HoverResizeTarget> {
    if pointer_in_bar(globals, root_y) {
        return None;
    }
    let mon = globals.selected_monitor();
    mon.iter_clients(globals.clients.map())
        .find_map(|(win, _)| resize_target_for_window(globals, win, root_x, root_y))
}

pub fn selected_hover_resize_target_at(
    globals: &Globals,
    position: Point,
) -> Option<HoverResizeTarget> {
    let win = globals.selected_win()?;
    if pointer_in_bar(globals, position.y) {
        return None;
    }
    resize_target_for_window(globals, win, position.x, position.y)
}

/// Find a visible tiled window at point (`x`, `y`), skipping `skip_win`.
///
/// Unlike [`get_cursor_client_win`] (which uses `query_pointer` and returns the
/// topmost X11 window), this walks the monitor's client list directly. This is
/// needed when a floating window is stacked on top: `query_pointer` would return
/// the floating window, but we want the tiled window *behind* it.
fn find_tiled_win_at_point(
    globals: &Globals,
    x: i32,
    y: i32,
    skip_win: Option<WindowId>,
) -> Option<WindowId> {
    let mon = globals.selected_monitor();
    let selected = mon.selected_tags();
    let has_tiling = mon.is_tiling_layout();
    if !has_tiling {
        return None;
    }

    for (w, c) in mon.iter_clients(globals.clients.map()) {
        if Some(w) == skip_win {
            continue;
        }
        if !c.is_visible(selected) || c.mode.is_floating() {
            continue;
        }
        // Check if the cursor is within the window's geometry (including border).
        let border_width = c.border_width;
        if x >= c.geo.x - border_width
            && x <= c.geo.x + c.geo.w + border_width
            && y >= c.geo.y - border_width
            && y <= c.geo.y + c.geo.h + border_width
        {
            return Some(w);
        }
    }
    None
}

/// Check whether any visible client on the current monitor is tiled.
fn has_visible_tiled_client(globals: &Globals) -> bool {
    let has_tiling = globals.selected_monitor().is_tiling_layout();
    let mon = globals.selected_monitor();
    let selected = mon.selected_tags();
    has_tiling
        && mon
            .iter_clients(globals.clients.map())
            .any(|(_, c)| c.is_visible(selected) && !c.mode.is_floating())
}

// ── Motion-notify hook ───────────────────────────────────────────────────────

/// Updates the resize offer when the pointer is in a floating window border.
///
/// Returns `true` when the pointer is over a resize offer zone and the caller
/// should stop processing the motion event.
pub fn update_floating_resize_offer_at(
    ctx: &mut WmCtx,
    root_x: i32,
    root_y: i32,
    do_focus: bool,
) -> bool {
    if let Some(target) = hover_resize_target_at(ctx.core().globals(), root_x, root_y) {
        offer_hover_resize(ctx, target);
        // Only focus when: do_focus requested AND no visible tiled clients.
        // When tiled clients exist, enter_notify handles focus transitions,
        // so motion_notify must not steal focus back to the floating window.
        let should_focus = do_focus
            && ctx.core().selected_client() != Some(target.win)
            && !has_visible_tiled_client(ctx.core().globals());

        if should_focus {
            crate::focus::focus_soft(ctx, Some(target.win));
        }
        return true;
    }

    clear_hover_offer(ctx);
    false
}

/// Update the resize offer for the currently selected window.
///
/// This is the backend-neutral hover-resize path used by Wayland motion events.
pub fn update_selected_resize_offer_at(ctx: &mut WmCtx, position: Point) -> Option<WindowId> {
    let Some(target) = selected_hover_resize_target_at(ctx.core().globals(), position) else {
        clear_hover_offer(ctx);
        return None;
    };
    offer_hover_resize(ctx, target);
    Some(target.win)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum SidebarOfferUpdate {
    None,
    Active,
    Cleared,
}

impl SidebarOfferUpdate {
    pub fn affects_pointer_handling(self) -> bool {
        !matches!(self, SidebarOfferUpdate::None)
    }
}

pub fn update_sidebar_offer_at(ctx: &mut WmCtx, root: crate::types::Point) -> SidebarOfferUpdate {
    if let Some(target) = crate::mouse::pointer::sidebar_target_at(ctx.core(), root) {
        if ctx.core().globals().drag.hover_offer != HoverOffer::Sidebar(target) {
            ctx.core_mut()
                .globals_mut()
                .drag
                .set_hover_offer(HoverOffer::Sidebar(target));
            set_cursor_style(ctx, AltCursor::Resize(ResizeDirection::Left));
        }
        return SidebarOfferUpdate::Active;
    }

    if ctx.core().globals().drag.hover_offer.is_sidebar() {
        clear_hover_offer(ctx);
        return SidebarOfferUpdate::Cleared;
    }

    SidebarOfferUpdate::None
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum X11HoverResizeOfferResult {
    NotOffered,
    OfferedWithoutAction,
    StartedAction,
}

impl X11HoverResizeOfferResult {
    pub fn consumed_event(self) -> bool {
        !matches!(self, X11HoverResizeOfferResult::NotOffered)
    }
}

pub fn run_x11_hover_resize_offer_loop(ctx: &mut WmCtxX11) -> X11HoverResizeOfferResult {
    {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        let Some(ptr) = wm_ctx.pointer_location() else {
            return X11HoverResizeOfferResult::NotOffered;
        };
        let Some(target) = selected_hover_resize_target_at(wm_ctx.core().globals(), ptr) else {
            return X11HoverResizeOfferResult::NotOffered;
        };

        offer_hover_resize(&mut wm_ctx, target);
    };

    let action_started = run_x11_hover_offer_grab_loop(ctx);

    if !action_started {
        clear_hover_offer(&mut WmCtx::X11(ctx.reborrow()));
        return X11HoverResizeOfferResult::OfferedWithoutAction;
    }

    X11HoverResizeOfferResult::StartedAction
}

/// Shared modal grab loop for hover-resize operations.
///
/// Waits for the user to either click (starting resize/move), move the cursor
/// outside the resize border (focusing the window under cursor), or press
/// Escape (aborting). Returns `true` if a resize/move action was started.
fn run_x11_hover_offer_grab_loop(ctx: &mut WmCtxX11) -> bool {
    let mut action_started = false;

    crate::backend::x11::grab::mouse_drag_loop(
        ctx,
        MouseButton::Left,
        AltCursor::Resize(ResizeDirection::BottomRight),
        true,
        |ctx, event| {
            match event {
                x11rb::protocol::Event::ButtonRelease(_) => false,

                x11rb::protocol::Event::MotionNotify(_) => {
                    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
                    let in_resize_border = wm_ctx
                        .pointer_location()
                        .map(|p| {
                            selected_hover_resize_target_at(wm_ctx.core().globals(), p).is_some()
                        })
                        .unwrap_or(false);
                    if !in_resize_border {
                        let sel = wm_ctx.core().selected_client();
                        let target = get_cursor_client_win(&mut wm_ctx)
                            .filter(|&w| Some(w) != sel)
                            .or_else(|| {
                                let p = wm_ctx.pointer_location()?;
                                find_tiled_win_at_point(wm_ctx.core().globals(), p.x, p.y, sel)
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

                    let Some(win) = wm_ctx.core().selected_client() else {
                        return false;
                    };
                    let (geo, w, h) = {
                        let Some(c) = wm_ctx.core().client(win) else {
                            return false;
                        };
                        (c.geo, c.geo.w, c.geo.h)
                    };

                    // Query cursor position relative to the client window.
                    let (root_x, root_y, win_x, win_y) =
                        query_pointer_on_win(&mut wm_ctx, win).unwrap_or((0, 0, 0, 0));

                    let Some(btn) = MouseButton::from_x11_detail(bp.detail) else {
                        return false;
                    };
                    wm_ctx.raise_client(win);
                    match btn {
                        MouseButton::Right => {
                            let mut wm_ctx_x11 = ctx.reborrow();
                            let mut wmctx = WmCtx::X11(wm_ctx_x11.reborrow());
                            super::warp::warp_into(&mut wmctx, win);
                            crate::backend::x11::mouse::move_mouse_x11(&mut wm_ctx_x11, btn, None);
                        }
                        MouseButton::Left => {
                            if geo.is_at_top_middle_edge(
                                Point::new(root_x, root_y),
                                RESIZE_BORDER_ZONE,
                            ) {
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
pub fn handle_x11_floating_to_tiled_hover_offer(ctx: &mut WmCtxX11) -> bool {
    // Pre-loop: do all checks and setup while we have wm_ctx
    {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());

        // Selected window must be floating in a tiling layout
        let selected_window = match wm_ctx.core().selected_client() {
            Some(w) => w,
            None => return false,
        };
        let is_tiling_layout = wm_ctx
            .core()
            .globals()
            .selected_monitor()
            .is_tiling_layout();
        let sel_geo = match wm_ctx.core().client(selected_window) {
            Some(c) if c.mode.is_floating() || !is_tiling_layout => c.geo,
            _ => return false,
        };

        // Must have a different, tiled window under the cursor
        let hovered_win = match get_cursor_client_win(&mut wm_ctx) {
            Some(w) if w != selected_window => w,
            _ => return false,
        };
        let has_tiling = wm_ctx
            .core()
            .globals()
            .selected_monitor()
            .is_tiling_layout();
        if !has_tiling {
            return false;
        }
        let hovered_is_tiled = wm_ctx
            .core()
            .globals()
            .clients
            .get(&hovered_win)
            .map(|c| !c.mode.is_floating())
            .unwrap_or(false);
        if !hovered_is_tiled {
            return false;
        }

        let Some(ptr) = wm_ctx.pointer_location() else {
            return false;
        };

        // If cursor is already outside the resize border, just focus the tiled window
        if !sel_geo.contains_resize_border_point(ptr, RESIZE_BORDER_ZONE) {
            crate::focus::focus_soft(&mut wm_ctx, Some(hovered_win));
            return true;
        }

        // Activate resize cursor and enter the grab loop
        update_floating_resize_offer_at(&mut wm_ctx, ptr.x, ptr.y, false);

        // Return the coordinates for the loop
        (ptr.x, ptr.y)
    };

    let action_started = run_x11_hover_offer_grab_loop(ctx);

    if !action_started {
        clear_hover_offer(&mut WmCtx::X11(ctx.reborrow()));
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
fn get_cursor_client_win(ctx: &mut WmCtx) -> Option<WindowId> {
    let (conn, root, core) = match ctx {
        WmCtx::X11(x11) => (x11.x11.conn, x11.x11_runtime.root, &mut x11.core),
        WmCtx::Wayland(_) => return None,
    };
    crate::backend::x11::mouse::get_cursor_client_win_with_conn(core, conn, root)
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
