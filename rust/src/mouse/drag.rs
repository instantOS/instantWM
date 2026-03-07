//! Interactive mouse-drag operations.
//!
//! | Function                            | Description                                               |
//! |-------------------------------------|-----------------------------------------------------------|
//! | [`begin_keyboard_move`]             | Keyboard-initiated window drag (works on X11 and Wayland) |
//! | [`move_mouse`]                      | Drag the focused window to a new position (X11 only)      |
//! | [`gesture_mouse`]                   | Vertical-swipe gesture recogniser on the root window      |
//! | [`drag_tag`]                        | Drag across the tag bar to switch/move tags               |
//! | [`window_title_mouse_handler`]      | Left-click/drag on a window title bar entry               |
//! | [`window_title_mouse_handler_right`]| Right-click/drag on a window title bar entry              |
//!
//! All loops follow the same skeleton:
//!
//! ```text
//! grab_pointer(cursor)
//! loop {
//!     ButtonRelease → break
//!     MotionNotify  → throttle → update
//!     _             → ignore
//! }
//! ungrab(ctx)
//! post-loop cleanup (bar drop, monitor switch, bar redraw, …)
//! ```

use crate::bar::bar_position_at_x;
use crate::bar::bar_position_to_gesture;
use crate::client::resize;
use crate::contexts::{WmCtx, WmCtxX11};
use crate::floating::{change_snap, reset_snap, set_floating_in_place, set_tiled, SnapDir};
// focus() is used via focus_soft() in this module
use crate::layouts::{arrange, restack};
use crate::tags::{move_client, shift_tag};
use crate::types::geometry::Rect;
use crate::types::SnapPosition;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use super::constants::{
    DRAG_THRESHOLD, MAX_UNMAXIMIZE_OFFSET, OVERLAY_ZONE_WIDTH, REFRESH_RATE_HI, REFRESH_RATE_LO,
};
use super::cursor::{set_cursor_default, set_cursor_move};
use super::monitor::handle_client_monitor_switch;
use super::warp::{get_root_ptr, get_root_ptr_ctx_x11, warp_into_ctx_x11};

// ── Shared helpers ────────────────────────────────────────────────────────────

fn refresh_rate(ctx: &mut WmCtx) -> u32 {
    if ctx.g_mut().doubledraw {
        REFRESH_RATE_HI
    } else {
        REFRESH_RATE_LO
    }
}

/// Snap `new_x`/`new_y` to the work-area edges of `selmon` when within `globals.cfg.snap` pixels.
fn snap_to_monitor_edges(ctx: &mut WmCtx, c: &Client, new_x: &mut i32, new_y: &mut i32) {
    snap_window_to_monitor_edges(ctx, c.win, c.geo.w, c.geo.h, new_x, new_y);
}

pub fn snap_window_to_monitor_edges(
    ctx: &WmCtx,
    win: WindowId,
    w: i32,
    h: i32,
    new_x: &mut i32,
    new_y: &mut i32,
) {
    let snap = ctx.g().cfg.snap;
    let mon = ctx.g().selected_monitor();
    let bw = ctx
        .g()
        .clients
        .get(&win)
        .map(|client| client.border_width.max(0))
        .unwrap_or(0);
    let width = w + bw * 2;
    let height = h + bw * 2;

    if (mon.work_rect.x - *new_x).abs() < snap {
        *new_x = mon.work_rect.x;
    } else if (mon.work_rect.x + mon.work_rect.w - (*new_x + width)).abs() < snap {
        *new_x = mon.work_rect.x + mon.work_rect.w - width;
    }

    if (mon.work_rect.y - *new_y).abs() < snap {
        *new_y = mon.work_rect.y;
    } else if (mon.work_rect.y + mon.work_rect.h - (*new_y + height)).abs() < snap {
        *new_y = mon.work_rect.y + mon.work_rect.h - height;
    }
}

/// Returns edge snap position based on cursor position.
fn check_edge_snap(ctx: &WmCtx, x: i32, y: i32) -> Option<SnapPosition> {
    let mon = ctx.g().selected_monitor();

    if x < mon.monitor_rect.x + OVERLAY_ZONE_WIDTH && x > mon.monitor_rect.x - 1 {
        return Some(SnapPosition::Left);
    }
    if x > mon.monitor_rect.x + mon.monitor_rect.w - OVERLAY_ZONE_WIDTH
        && x < mon.monitor_rect.x + mon.monitor_rect.w + 1
    {
        return Some(SnapPosition::Right);
    }
    if y <= mon.monitor_rect.y
        + if mon.showbar {
            ctx.g().cfg.bar_height
        } else {
            5
        }
    {
        return Some(SnapPosition::Top);
    }
    None
}

/// Returns `true` when `(x, y)` (root-space) is inside the bar of `selmon`.
fn point_is_on_bar(ctx: &WmCtx, x: i32, y: i32) -> bool {
    let mon = ctx.g().selected_monitor();
    mon.showbar
        && y >= mon.bar_y
        && y < mon.bar_y + ctx.g().cfg.bar_height
        && x >= mon.monitor_rect.x
        && x < mon.monitor_rect.x + mon.monitor_rect.w
}

// ── move_mouse helpers ────────────────────────────────────────────────────────

/// State threaded through the move-mouse event loop.
struct MoveState {
    /// Drag origin in root coordinates.
    start_x: i32,
    start_y: i32,
    /// Window geometry at drag start.
    grab_start_rect: Rect,
    /// Whether the cursor was over the bar on the previous motion event.
    cursor_on_bar: bool,
    /// The last edge-snap zone the cursor was in.
    edge_snap_indicator: Option<SnapPosition>,
}

/// Perform the pre-flight checks for `move_mouse`.
///
/// Returns the window to drag, or `None` if the drag should be aborted.
/// As a side effect:
/// * exits fake-fullscreen and returns `None` so the caller re-enters after the transition
/// * calls `reset_snap` and returns `None` if the window is snapped (un-snap first)
/// * restores a near-maximized floating window to its saved geometry
fn prepare_drag_target(ctx: &mut WmCtx) -> Option<WindowId> {
    let (sel, is_true_fs, is_overlay, is_fullscreen) = {
        let g = ctx.g_mut();
        let mon = g.selected_monitor();
        let sel = mon.sel?;
        let overlay = mon.overlay;
        let fullscreen = mon.fullscreen;
        let c = g.clients.get(&sel)?;
        (
            sel,
            c.is_true_fullscreen(),
            Some(sel) == overlay,
            Some(sel) == fullscreen,
        )
    };

    if is_true_fs {
        return None;
    }
    if is_overlay {
        return None;
    }
    if is_fullscreen {
        crate::floating::toggle_maximized(ctx);
        return None;
    }
    let selected_window = sel;

    let selmon_id = ctx.g_mut().selected_monitor_id();
    crate::layouts::restack(ctx, selmon_id);

    // Un-snap: surface the real window first; the user re-drags after.
    let is_snapped = ctx
        .g()
        .clients
        .get(&selected_window)
        .map(|c| c.snap_status != SnapPosition::None)
        .unwrap_or(false);
    if is_snapped {
        reset_snap(ctx, selected_window);
        return None;
    }

    // In a floating layout, if the window fills (nearly) the whole monitor,
    // restore the saved float geometry so we drag the real size, not a maximized one.
    let restore_geo: Option<Rect> = {
        let g = ctx.g_mut();
        let has_tiling = g.selected_monitor().is_tiling_layout();

        if !has_tiling {
            let mon = g.selected_monitor();
            let bar_height = g.cfg.bar_height;
            if let Some(c) = g.clients.get(&selected_window) {
                let nearly_maximized = c.geo.x >= mon.monitor_rect.x - MAX_UNMAXIMIZE_OFFSET
                    && c.geo.y >= mon.monitor_rect.y + bar_height - MAX_UNMAXIMIZE_OFFSET
                    && c.geo.w >= mon.monitor_rect.w - MAX_UNMAXIMIZE_OFFSET
                    && c.geo.h >= mon.monitor_rect.h - MAX_UNMAXIMIZE_OFFSET;
                if nearly_maximized {
                    Some(c.float_geo)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some(geo) = restore_geo {
        resize(ctx, selected_window, &geo, false);
    }

    Some(selected_window)
}

/// Update `bar_dragging` and the gesture (tag hover highlight) while dragging.
///
/// Tracks enter/leave transitions via `state.cursor_on_bar` so the bar is only
/// redrawn when something changes.  Returns `true` while the cursor is on the bar.
fn update_bar_hover(ctx: &mut WmCtx, ptr_x: i32, ptr_y: i32, state: &mut MoveState) -> bool {
    let on_bar = point_is_on_bar(ctx, ptr_x, ptr_y);

    let selmon_id = ctx.g().selected_monitor_id();

    if on_bar {
        let new_gesture = {
            let core = ctx.core();
            let mon = core.g.selected_monitor();
            let local_x = ptr_x - mon.work_rect.x;
            bar_position_to_gesture(bar_position_at_x(mon, core, local_x))
        };

        let gesture_changed = ctx.g().selected_monitor().gesture != new_gesture;

        if !state.cursor_on_bar || gesture_changed {
            ctx.g_mut().drag.bar_active = true;
            ctx.g_mut().selected_monitor_mut().gesture = new_gesture;
            ctx.request_bar_update(Some(selmon_id));
        }
    } else if state.cursor_on_bar {
        ctx.g_mut().drag.bar_active = false;
        ctx.g_mut().selected_monitor_mut().gesture = Gesture::None;
        ctx.request_bar_update(Some(selmon_id));
    }

    on_bar
}

/// Process a single throttled `MotionNotify` event during `move_mouse`.
fn on_motion(
    ctx: &mut WmCtx,
    win: WindowId,
    event_x: i32,
    event_y: i32,
    root_x: i32,
    root_y: i32,
    state: &mut MoveState,
) {
    state.cursor_on_bar = update_bar_hover(ctx, root_x, root_y, state);
    state.edge_snap_indicator = check_edge_snap(ctx, root_x, root_y);

    let mut new_x = state.grab_start_rect.x + (event_x - state.start_x);
    let mut new_y = state.grab_start_rect.y + (event_y - state.start_y);

    // While hovering over the bar, keep the window just below it.
    if state.cursor_on_bar {
        let bar_bottom = ctx.g().selected_monitor().bar_y + ctx.g().cfg.bar_height;
        new_y = bar_bottom;
    }

    let has_tiling = ctx.g().selected_monitor().is_tiling_layout();

    let (mut is_floating, mut drag_geo) = ctx
        .g()
        .clients
        .get(&win)
        .map(|c| (c.isfloating, c.geo))
        .unwrap_or((false, Rect::default()));

    maybe_promote_tiled_drag_to_floating(
        ctx,
        win,
        event_x,
        event_y,
        &mut new_x,
        &mut new_y,
        state,
        has_tiling,
        &mut is_floating,
        &mut drag_geo,
    );

    if !has_tiling || is_floating {
        let client_clone = ctx.g().clients.get(&win).cloned();
        if let Some(ref client) = client_clone {
            snap_to_monitor_edges(ctx, client, &mut new_x, &mut new_y);
        }
        resize(
            ctx,
            win,
            &Rect {
                x: new_x,
                y: new_y,
                w: drag_geo.w,
                h: drag_geo.h,
            },
            true,
        );
    }
}

fn maybe_promote_tiled_drag_to_floating(
    ctx: &mut WmCtx,
    win: WindowId,
    event_x: i32,
    event_y: i32,
    new_x: &mut i32,
    new_y: &mut i32,
    state: &mut MoveState,
    has_tiling: bool,
    is_floating: &mut bool,
    drag_geo: &mut Rect,
) {
    let snap = ctx.g().cfg.snap;
    if *is_floating
        || !has_tiling
        || ((*new_x - drag_geo.x).abs() <= snap && (*new_y - drag_geo.y).abs() <= snap)
    {
        return;
    }

    // Resolve float dimensions before touching state.
    // If the window was never floating, float_geo will be zeroed; fall
    // back to the current tiled dimensions so the window doesn't collapse.
    let (float_w, float_h) = {
        ctx.g()
            .clients
            .get(&win)
            .map(|c| {
                if c.float_geo.w > 0 && c.float_geo.h > 0 {
                    (c.float_geo.w, c.float_geo.h)
                } else {
                    (c.geo.w, c.geo.h)
                }
            })
            .unwrap_or((drag_geo.w, drag_geo.h))
    };

    // Flip isfloating + restore border — zero configure_window calls.
    set_floating_in_place(ctx, win);

    // Re-tile the remaining windows (touches only the other clients).
    let selmon_id = ctx.g().selected_monitor_id();
    arrange(ctx, Some(selmon_id));

    // The window's width is changing (tiled → floating), so the old
    // `grab_start_x`-based `new_x` would leave the window at x≈0 while the cursor
    // is far to the right. Re-center under cursor and rebase drag anchors.
    *new_x = event_x - float_w / 2;
    state.grab_start_rect.x = *new_x;
    state.grab_start_rect.y = *new_y;
    state.start_x = event_x;
    state.start_y = event_y;

    *is_floating = true;
    *drag_geo = Rect {
        x: *new_x,
        y: *new_y,
        w: float_w,
        h: float_h,
    };
}

/// Clears `bar_dragging` and redraws the bar unconditionally.
///
/// Called once the drag loop exits so that hover state is always cleaned up.
fn clear_bar_hover(ctx: &mut WmCtx) {
    ctx.g_mut().drag.bar_active = false;
    let selmon_id = ctx.g().selected_monitor_id();
    ctx.g_mut().selected_monitor_mut().gesture = Gesture::None;
    ctx.request_bar_update(Some(selmon_id));
}

/// Handle a drop onto the bar: tile the window, optionally moving it to the
/// hovered tag first.
///
/// Mirrors the C `handle_bar_drop`:
/// * Dropped on a tag button → `set_tiled()` + `tag()`
/// * Dropped elsewhere on bar, window floating → `set_tiled()`
///
/// # `grab_start_rect`
///
/// The window geometry at the moment the drag started.  When the window was
/// floating, this is the true pre-drag origin; we save it into `float_geo`
/// so un-tiling later restores the original floating position.
fn handle_bar_drop(
    ctx: &mut WmCtx,
    win: WindowId,
    grab_start_rect: Rect,
    pointer_override: Option<(i32, i32)>,
) {
    let Some((ptr_x, ptr_y)) = pointer_override.or_else(|| get_root_ptr(ctx)) else {
        return;
    };
    if !point_is_on_bar(ctx, ptr_x, ptr_y) {
        return;
    }

    let position = {
        let core = ctx.core();
        let mon = core.g.selected_monitor();
        let local_x = ptr_x - mon.work_rect.x;
        bar_position_at_x(mon, core, local_x)
    };

    // Remember whether the window was floating *before* any state change so
    // we know whether to correct float_geo afterwards.
    let was_floating = ctx
        .g()
        .clients
        .get(&win)
        .map(|c| c.isfloating)
        .unwrap_or(false);

    if let BarPosition::Tag(tag_idx) = position {
        // Tile first (no arrange), then tag.
        //
        // Old order: tag() → arrange() [window still floating, layout skips
        // it] → set_tiled() → arrange() again.  That's two arrange passes.
        //
        // New order: set_tiled(should_arrange=false) saves float_geo from the
        // current floating geometry *before* tag() calls arrange().  Then
        // tag() calls arrange() exactly once with the window already marked
        // tiled, so the layout places it correctly in a single pass.
        //
        // tag() uses selmon->sel internally (via set_client_tag_impl), so win
        // must still be the selected window at this point — which it is because
        // set_tiled does not touch focus.
        set_tiled(ctx, win, false);
        crate::tags::client_tags::set_client_tag_ctx(
            ctx,
            win,
            TagMask::single(tag_idx + 1).unwrap_or(TagMask::EMPTY),
        );
    } else if was_floating {
        // Dropped on the bar but not on a tag button: tile the window.
        // Use set_tiled(win, …) directly instead of toggle_floating() which
        // operates on mon.sel — a value that could theoretically diverge from
        // the window we actually dragged.
        set_tiled(ctx, win, true);
    } else {
        // Window is already tiled and not dropped on a tag — nothing to do.
        return;
    }

    // ── Correct float_geo using pre-drag dimensions ───────────────────────
    //
    // Keep the drop position (x/y from set_tiled's saved client.geo), but
    // preserve the pre-drag floating size so un-tiling restores dimensions.
    if was_floating {
        if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
            client.float_geo.w = grab_start_rect.w;
            client.float_geo.h = grab_start_rect.h;
        }
    }
}

/// Apply post-release logic for left/right screen-edge drops.
///
/// In a tiling layout: navigate to the adjacent tag (or send the window there).
/// In a floating layout: apply a directional screen-edge snap.
///
/// Returns `true` if the drop was fully handled (the caller should skip
/// `handle_bar_drop` and `handle_client_monitor_switch`).
fn apply_edge_drop(
    ctx: &mut WmCtx,
    win: WindowId,
    edge: Option<SnapPosition>,
    root_y: i32,
) -> bool {
    let edge = match edge {
        Some(e) => e,
        None => return false,
    };

    let at_left = edge == SnapPosition::Left;
    let at_right = edge == SnapPosition::Right;

    if !at_left && !at_right {
        return false;
    }

    let is_tiling = ctx.g().selected_monitor().is_tiling_layout();

    if is_tiling {
        let (mon_my, mon_mh) = (
            ctx.g().selected_monitor().monitor_rect.y,
            ctx.g().selected_monitor().monitor_rect.h,
        );

        // Upper 2/3 of the monitor → move view; lower 1/3 → send window.
        if root_y < mon_my + (2 * mon_mh) / 3 {
            if at_left {
                move_client(ctx, Direction::Left);
            } else {
                move_client(ctx, Direction::Right);
            }
        } else if at_left {
            shift_tag(ctx, Direction::Left, 1);
        } else {
            shift_tag(ctx, Direction::Right, 1);
        }

        if let Some(c) = ctx.g_mut().clients.get_mut(&win) {
            c.isfloating = false;
        }
        let selmon_id = ctx.g().selected_monitor_id();
        arrange(ctx, Some(selmon_id));
    } else {
        let dir = if at_left {
            SnapDir::Left
        } else {
            SnapDir::Right
        };
        change_snap(ctx, win, dir);
    }

    true
}

/// Shared post-release drop handling for move-like drags.
///
/// This keeps bar-drop and edge-drop behavior identical for all move paths.
pub fn complete_move_drop(
    ctx: &mut WmCtx,
    win: WindowId,
    grab_start_rect: Rect,
    edge_hint: Option<SnapPosition>,
    pointer_override: Option<(i32, i32)>,
) {
    let pointer = pointer_override.or_else(|| get_root_ptr(ctx));
    let edge = edge_hint.or_else(|| pointer.and_then(|(x, y)| check_edge_snap(ctx, x, y)));
    let handled_edge = pointer
        .map(|(_x, y)| apply_edge_drop(ctx, win, edge, y))
        .unwrap_or(false);
    if !handled_edge {
        handle_bar_drop(ctx, win, grab_start_rect, pointer);
        handle_client_monitor_switch(ctx, win);
    }
}

// ── begin_keyboard_move / move_mouse ─────────────────────────────────────────

/// Keyboard-initiated window move — works on both X11 and Wayland.
///
/// On **X11** this is identical to calling `move_mouse` directly: the pointer
/// is grabbed and a synchronous event loop drives the drag until the button is
/// released.
///
/// On **Wayland** a synchronous grab loop is not possible (no `XGrabPointer`
/// equivalent in the protocol).  Instead we arm the existing
/// `HoverResizeDragState` machinery in move mode at the current pointer
/// position.  Subsequent `MotionNotify` events delivered through calloop then
/// drive the drag, and `wayland_hover_resize_drag_finish` (called on button
/// release inside `handle_pointer_button`) performs the drop logic via the
/// shared `complete_move_drop` helper.
///
/// The button used to end the drag defaults to `MouseButton::Left` on Wayland
/// (matching the most common keyboard-move UX on other compositors).
pub fn begin_keyboard_move(ctx: &mut WmCtx) {
    // Pre-flight checks are shared: exit maximized state, un-snap, etc.
    let Some(win) = prepare_drag_target(ctx) else {
        return;
    };

    match ctx {
        WmCtx::X11(x11) => {
            // X11: synchronous grab loop, unchanged behaviour.
            move_mouse(x11, MouseButton::Left);
        }
        WmCtx::Wayland(wl) => {
            // Wayland: arm the hover-resize state in move mode so that calloop
            // motion/release events drive the drag asynchronously.
            let Some((root_x, root_y)) = wl.wayland.backend.pointer_location() else {
                return;
            };
            let geo = wl
                .core
                .g
                .clients
                .get(&win)
                .map(|c| c.geo)
                .unwrap_or_default();

            // Ensure the window is floating so the move makes sense.
            if !wl
                .core
                .g
                .clients
                .get(&win)
                .map(|c| c.isfloating)
                .unwrap_or(false)
            {
                super::super::floating::set_floating_in_place(
                    &mut WmCtx::Wayland(wl.reborrow()),
                    win,
                );
                let selmon_id = wl.core.g.selected_monitor_id();
                crate::layouts::arrange(&mut WmCtx::Wayland(wl.reborrow()), Some(selmon_id));
            }

            wl.core.g.drag.hover_resize = crate::globals::HoverResizeDragState {
                active: true,
                win,
                button: MouseButton::Left,
                direction: crate::types::ResizeDirection::BottomRight,
                move_mode: true,
                start_x: root_x,
                start_y: root_y,
                win_start_geo: geo,
                last_root_x: root_x,
                last_root_y: root_y,
            };
            wl.core.g.altcursor = crate::types::AltCursor::None;
            super::set_cursor_move_wayland(wl);
            crate::contexts::WmCtx::Wayland(wl.reborrow()).raise_interactive(win);
        }
    }
}

/// Interactively drag the focused window with the mouse.
///
/// Grab → event loop → release handling. See helpers above for each phase.
///
/// This is the X11-only synchronous implementation.  For the backend-agnostic
/// keyboard shortcut, use [`begin_keyboard_move`] instead.
pub fn move_mouse(ctx: &mut WmCtxX11, btn: MouseButton) {
    let Some(win) = ({
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        prepare_drag_target(&mut wm_ctx)
    }) else {
        return;
    };

    let Some((start_x, start_y)) = get_root_ptr_ctx_x11(ctx) else {
        return;
    };

    let grab_start_rect = ctx
        .core
        .g
        .clients
        .get(&win)
        .map(|c| c.geo)
        .unwrap_or(Rect::default());

    let mut state = MoveState {
        start_x,
        start_y,
        grab_start_rect,
        cursor_on_bar: false,
        edge_snap_indicator: None,
    };

    super::grab::mouse_drag_loop(ctx, btn, 2, false, |ctx, event| {
        if let x11rb::protocol::Event::MotionNotify(m) = event {
            let mut wm_ctx = WmCtx::X11(ctx.reborrow());
            on_motion(
                &mut wm_ctx,
                win,
                m.event_x as i32,
                m.event_y as i32,
                m.root_x as i32,
                m.root_y as i32,
                &mut state,
            );
        }
        true
    });

    {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        clear_bar_hover(&mut wm_ctx);
    }

    {
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        complete_move_drop(
            &mut wm_ctx,
            win,
            state.grab_start_rect,
            state.edge_snap_indicator,
            None,
        );
    }
}

// ── gesture_mouse ─────────────────────────────────────────────────────────────

/// Root-window vertical-swipe gesture recogniser.
///
/// Watches for large vertical pointer movements; each time the cursor travels
/// more than `monitor_height / 30` pixels [`crate::util::spawn`] is called.
pub fn gesture_mouse(ctx: &mut WmCtx, btn: MouseButton) {
    if let WmCtx::X11(x11) = ctx {
        gesture_mouse_x11(x11, btn);
    }
}

pub fn gesture_mouse_x11(ctx: &mut WmCtxX11, btn: MouseButton) {
    let Some((_, start_y)) = get_root_ptr_ctx_x11(ctx) else {
        return;
    };

    let mut last_y = start_y;

    super::grab::mouse_drag_loop(ctx, btn, 2, false, |ctx, event| {
        if let x11rb::protocol::Event::MotionNotify(m) = event {
            let threshold = ctx.core.g.selected_monitor().monitor_rect.h / 30;
            if (last_y - m.event_y as i32).abs() > threshold {
                let event_y = m.event_y as i32;
                let cmd = if event_y < last_y {
                    &["/usr/share/instantassist/utils/p.sh", "+"]
                } else {
                    &["/usr/share/instantassist/utils/p.sh", "-"]
                };
                let wm_ctx = WmCtx::X11(ctx.reborrow());
                crate::util::spawn(&wm_ctx, cmd);
                last_y = event_y;
            }
        }
        true
    });
}

// ── drag_tag ──────────────────────────────────────────────────────────────────

/// Begin a tag-bar drag. Returns `true` if a drag was started (current tag
/// clicked with a selected window), `false` if the click was fully handled
/// (view switch or no selection).
///
/// On Wayland the caller should return after this — the calloop will drive
/// [`drag_tag_motion`] and [`drag_tag_finish`].  On X11 the caller enters a
/// grab loop that calls those two functions synchronously.
pub fn drag_tag_begin(ctx: &mut WmCtx, bar_pos: BarPosition, btn: MouseButton) -> bool {
    let selmon_id = ctx.g_mut().selected_monitor_id();
    let mon_mx = ctx.g_mut().selected_monitor().work_rect.x;

    let initial_tag = match bar_pos {
        BarPosition::Tag(idx) => 1u32 << (idx as u32),
        _ => {
            let ptr_x = get_root_ptr(ctx).map(|(x, _)| x).unwrap_or(0);
            let core = ctx.core();
            core.g
                .monitors
                .get(selmon_id)
                .and_then(|mon| {
                    let local_x = ptr_x - mon.work_rect.x;
                    match bar_position_at_x(mon, core, local_x) {
                        BarPosition::Tag(idx) => Some(1u32 << (idx as u32)),
                        _ => None,
                    }
                })
                .unwrap_or(0)
        }
    };

    let current_tagset = ctx.g_mut().selected_monitor().selected_tags();
    let is_current_tag = (initial_tag & ctx.g_mut().tags.mask()) == current_tagset;
    let has_sel = ctx.g_mut().selected_monitor().sel.is_some();

    // Click on a *different* tag → switch view, no drag.
    if !is_current_tag && initial_tag != 0 {
        crate::tags::view::view(ctx, TagMask::from_bits(initial_tag));
        return false;
    }
    // No selected window → nothing to drag.
    if !has_sel {
        return false;
    }

    // Initialise the drag state machine.
    ctx.g_mut().drag.tag = crate::globals::TagDragState {
        active: true,
        initial_tag,
        monitor_id: selmon_id,
        mon_mx,
        last_tag: -1,
        cursor_on_bar: true,
        last_motion: None,
        button: btn,
    };
    set_cursor_move(ctx);
    ctx.g_mut().drag.bar_active = true;
    ctx.request_bar_update(Some(selmon_id));
    true
}

/// Process a single motion event during an active tag drag.
///
/// Updates gesture highlighting and detects when the cursor leaves the bar.
/// Returns `false` if the cursor left the bar (caller should finish the drag).
pub fn drag_tag_motion(ctx: &mut WmCtx, root_x: i32, root_y: i32) -> bool {
    if !ctx.g_mut().drag.tag.active {
        return false;
    }

    let selmon_id = ctx.g_mut().drag.tag.monitor_id;
    let mon_mx = ctx.g_mut().drag.tag.mon_mx;

    let bar_bottom = ctx.g_mut().selected_monitor().bar_y + ctx.g_mut().cfg.bar_height + 1;

    if root_y > bar_bottom {
        ctx.g_mut().drag.tag.cursor_on_bar = false;
        return false;
    }

    // Store last motion for release handling.  Modifier state is not available
    // from root coords alone; the caller sets it via drag_tag_finish.
    ctx.g_mut().drag.tag.last_motion = Some((root_x, root_y, 0));

    let local_x = root_x - mon_mx;
    let new_gesture = {
        let core = ctx.core();
        core.g
            .monitors
            .get(selmon_id)
            .map(|mon| bar_position_to_gesture(bar_position_at_x(mon, core, local_x)))
            .unwrap_or(Gesture::None)
    };
    let gesture_key = match new_gesture {
        Gesture::Tag(idx) => idx as i32,
        _ => -1,
    };

    if ctx.g_mut().drag.tag.last_tag != gesture_key {
        ctx.g_mut().drag.tag.last_tag = gesture_key;
        if let Some(mon) = ctx.g_mut().monitors.get_mut(selmon_id) {
            mon.gesture = new_gesture;
        }
        ctx.request_bar_update(Some(selmon_id));
    }
    true
}

/// Finish a tag drag: apply the action based on the final position and
/// modifier keys held at release time.
///
/// `modifier_state` is the X11-style modifier bitmask at release time
/// (Shift, Control, …).
pub fn drag_tag_finish(ctx: &mut WmCtx, modifier_state: u32) {
    if !ctx.g_mut().drag.tag.active {
        return;
    }

    let selmon_id = ctx.g_mut().drag.tag.monitor_id;
    let cursor_on_bar = ctx.g_mut().drag.tag.cursor_on_bar;
    let last_motion = ctx.g_mut().drag.tag.last_motion;

    // Clear state first so re-entrant calls are safe.
    ctx.g_mut().drag.tag.active = false;

    if cursor_on_bar {
        if let Some((x, _, _)) = last_motion {
            let position = {
                let core = ctx.core();
                let mon = core.g.selected_monitor();
                let local_x = x - mon.work_rect.x;
                bar_position_at_x(mon, core, local_x)
            };

            if let BarPosition::Tag(tag_idx) = position {
                let tag_mask = TagMask::single(tag_idx + 1).unwrap_or(TagMask::EMPTY);
                if (modifier_state & ModMask::SHIFT.bits() as u32) != 0 {
                    if let Some(win) = ctx.g_mut().monitor(selmon_id).and_then(|m| m.sel) {
                        crate::tags::client_tags::set_client_tag_ctx(ctx, win, tag_mask);
                    }
                } else if (modifier_state & ModMask::CONTROL.bits() as u32) != 0 {
                    crate::tags::client_tags::tag_all_ctx(ctx, tag_mask);
                } else if let Some(win) = ctx.g_mut().monitor(selmon_id).and_then(|m| m.sel) {
                    crate::tags::client_tags::follow_tag_ctx(ctx, win, tag_mask);
                }
            }
        }
    }

    ctx.g_mut().drag.bar_active = false;
    if let Some(mon) = ctx.g_mut().monitor_mut(selmon_id) {
        mon.gesture = Gesture::None;
    }
    set_cursor_default(ctx);
    ctx.request_bar_update(Some(selmon_id));
}

/// Drag across the tag bar to switch the view or move/follow a window to a tag.
///
/// * Plain click on a different tag   → [`view`]
/// * Plain click on the current tag   → drag; release with `Shift` → [`set_client_tag_ctx`],
///   `Control` → [`tag_all_ctx`], no modifier → [`follow_tag_ctx`]
///
/// Exits without action if the pointer leaves the bar during the drag.
///
/// On X11, runs a synchronous grab loop.  On Wayland, starts the drag and
/// returns immediately — the calloop drives [`drag_tag_motion`] and
/// [`drag_tag_finish`].
pub fn drag_tag(ctx: &mut WmCtxX11, bar_pos: BarPosition, btn: MouseButton, _click_root_x: i32) {
    if !{
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        drag_tag_begin(&mut wm_ctx, bar_pos, btn)
    } {
        return;
    }

    // ── X11 synchronous grab loop ─────────────────────────────────────────
    super::grab::mouse_drag_loop(ctx, btn, 2, false, |ctx, event| {
        if let x11rb::protocol::Event::MotionNotify(m) = event {
            // Update stored modifier state from latest motion.
            let root_x = m.event_x as i32;
            let root_y = m.event_y as i32;
            let mod_state = u16::from(m.state) as u32;

            // Store motion with modifier state for release handling.
            ctx.core.g.drag.tag.last_motion = Some((root_x, root_y, mod_state));

            let mut wm_ctx = WmCtx::X11(ctx.reborrow());
            return drag_tag_motion(&mut wm_ctx, root_x, root_y);
        }
        true
    });

    let modifier_state = {
        ctx.core
            .g
            .drag
            .tag
            .last_motion
            .map(|(_, _, m)| m)
            .unwrap_or(0)
    };

    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    drag_tag_finish(&mut wm_ctx, modifier_state);
}

// ── window title drag state machine ──────────────────────────────────────────

/// Initialise a title-bar click/drag interaction.
///
/// Returns `true` if the state machine was started.  On X11 the caller
/// continues into the synchronous grab loop; on Wayland the calloop drives
/// [`title_drag_motion`] and [`title_drag_finish`].
pub fn title_drag_begin(
    ctx: &mut WmCtx,
    win: WindowId,
    btn: MouseButton,
    click_root_x: i32,
    click_root_y: i32,
    suppress_click_action: bool,
) -> bool {
    if btn == MouseButton::Right {
        if ctx
            .g
            .clients
            .get(&win)
            .map(|c| c.is_true_fullscreen())
            .unwrap_or(false)
        {
            return false;
        }
        crate::focus::focus_soft(ctx, Some(win));
    }

    let sel = ctx.selected_client();
    let (win_start_geo, drop_restore_geo) = ctx
        .g
        .clients
        .get(&win)
        .map(|c| {
            let restore = if c.isfloating && c.geo.w > 0 && c.geo.h > 0 {
                c.geo
            } else if c.float_geo.w > 0 && c.float_geo.h > 0 {
                c.float_geo
            } else {
                c.geo
            };
            (c.geo, restore)
        })
        .unwrap_or((Rect::default(), Rect::default()));
    ctx.g_mut().drag.title = crate::globals::TitleDragState {
        active: true,
        win,
        button: btn,
        was_focused: sel == Some(win),
        was_hidden: ctx.g_mut().clients.is_hidden(win),
        start_x: click_root_x,
        start_y: click_root_y,
        win_start_geo,
        drop_restore_geo,
        last_root_x: click_root_x,
        last_root_y: click_root_y,
        dragging: false,
        suppress_click_action,
    };
    true
}

/// Process a pointer motion event during an active title drag.
///
/// Returns `true` if the drag threshold was exceeded and the drag action
/// (move/resize) was initiated — the caller should consider the interaction
/// consumed.
pub fn title_drag_motion(ctx: &mut WmCtx, root_x: i32, root_y: i32) -> bool {
    if !ctx.g_mut().drag.title.active {
        return false;
    }
    ctx.g_mut().drag.title.last_root_x = root_x;
    ctx.g_mut().drag.title.last_root_y = root_y;

    if ctx.g_mut().drag.title.dragging {
        if ctx.is_x11() {
            return false;
        }
        // On Wayland a right-click title-drag hands off to HoverResizeDragState
        // at the threshold-exceeded moment (see below).  If we somehow arrive
        // here with a right-click still marked dragging it means HoverResizeDragState
        // is now driving the resize — just clear the title state and bail so we
        // don't double-process.
        if ctx.g_mut().drag.title.button == MouseButton::Right {
            ctx.g_mut().drag.title.active = false;
            ctx.g_mut().drag.title.dragging = false;
            return false;
        }

        let td = &ctx.g_mut().drag.title;
        let win = td.win;
        let td_win_start_geo = td.win_start_geo;
        let td_start_x = td.start_x;
        let td_start_y = td.start_y;
        let mut new_x = td_win_start_geo.x + (root_x - td_start_x);
        let mut new_y = td_win_start_geo.y + (root_y - td_start_y);

        let (is_floating, geo) = ctx
            .g()
            .clients
            .get(&win)
            .map(|c| (c.isfloating, c.geo))
            .unwrap_or((false, Rect::default()));
        if is_floating {
            let client_clone = ctx.g_mut().clients.get(&win).cloned();
            if let Some(ref c) = client_clone {
                snap_to_monitor_edges(ctx, c, &mut new_x, &mut new_y);
            }
            resize(
                ctx,
                win,
                &Rect {
                    x: new_x,
                    y: new_y,
                    w: geo.w,
                    h: geo.h,
                },
                true,
            );
            if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
                client.float_geo.x = new_x;
                client.float_geo.y = new_y;
            }
        }
        return true;
    }

    let td = &ctx.g_mut().drag.title;
    if (root_x - td.start_x).abs() <= DRAG_THRESHOLD
        && (root_y - td.start_y).abs() <= DRAG_THRESHOLD
    {
        return false;
    }

    // Threshold exceeded — start the drag action.
    let win = ctx.g_mut().drag.title.win;
    let btn = ctx.g_mut().drag.title.button;
    let was_hidden = ctx.g_mut().drag.title.was_hidden;
    let is_right_click = btn == MouseButton::Right;

    if was_hidden {
        crate::client::show(ctx, win);
    }
    crate::focus::focus_soft(ctx, Some(win));
    ctx.raise_interactive(win);

    if ctx.is_wayland() {
        if is_right_click {
            // Right-click title drag on Wayland: hand straight off to
            // HoverResizeDragState.  That machinery already handles
            // directional resize correctly — no warp, no anchor chaos.
            //
            // Promote tiled windows to floating first (same as the move path).
            let (is_floating, geo, float_geo) = ctx
                .g()
                .clients
                .get(&win)
                .map(|c| (c.isfloating, c.geo, c.float_geo))
                .unwrap_or((false, Rect::default(), Rect::default()));
            let current_geo = if !is_floating {
                set_floating_in_place(ctx, win);
                let selmon_id = ctx.g_mut().selected_monitor_id();
                arrange(ctx, Some(selmon_id));
                let target_w = if float_geo.w > 0 { float_geo.w } else { geo.w };
                let target_h = if float_geo.h > 0 { float_geo.h } else { geo.h };
                let new_geo = Rect {
                    x: geo.x,
                    y: geo.y,
                    w: target_w,
                    h: target_h,
                };
                resize(ctx, win, &new_geo, true);
                new_geo
            } else {
                geo
            };

            // Compute direction from the original click position relative to
            // the (possibly freshly promoted) window geometry.
            let start_x = ctx.g().drag.title.start_x;
            let start_y = ctx.g().drag.title.start_y;
            let hit_x = start_x - current_geo.x;
            let hit_y = start_y - current_geo.y;
            let dir = crate::types::input::get_resize_direction(
                current_geo.w,
                current_geo.h,
                hit_x,
                hit_y,
            );

            // Arm HoverResizeDragState so calloop motion/release events drive
            // the resize from here on.  The title drag is deactivated so
            // title_drag_finish won't also fire.
            //
            // Warp the cursor to the nearest edge/corner for this direction so
            // the visual position matches what is being dragged.  The resize
            // math uses root_x/root_y directly against the window edges, so
            // correctness doesn't depend on start_x/start_y — but placing the
            // cursor at the right handle gives immediate visual feedback.
            let bw = ctx
                .g()
                .clients
                .get(&win)
                .map(|c| c.border_width)
                .unwrap_or(0);
            let (x_off, y_off) = dir.warp_offset(current_geo.w, current_geo.h, bw);
            let warp_x = current_geo.x + x_off;
            let warp_y = current_geo.y + y_off;

            ctx.g_mut().drag.title.active = false;
            ctx.g_mut().drag.title.dragging = false;
            if let WmCtx::Wayland(wl) = ctx {
                wl.wayland
                    .backend
                    .warp_pointer(warp_x as f64, warp_y as f64);
                wl.core.g.drag.hover_resize = crate::globals::HoverResizeDragState {
                    active: true,
                    win,
                    button: btn,
                    direction: dir,
                    move_mode: false,
                    start_x: warp_x,
                    start_y: warp_y,
                    win_start_geo: current_geo,
                    last_root_x: warp_x,
                    last_root_y: warp_y,
                };
                wl.core.g.altcursor = crate::types::AltCursor::Resize;
                wl.core.g.drag.resize_direction = Some(dir);
                super::cursor::set_cursor_resize_wayland(wl, Some(dir));
            }
            return true;
        }

        // Left-click move path — keep title drag active so calloop drives it.
        let (is_floating, geo, float_geo) = ctx
            .g()
            .clients
            .get(&win)
            .map(|c| (c.isfloating, c.geo, c.border_width, c.float_geo))
            .map(|(f, g, _bw, fg)| (f, g, fg))
            .unwrap_or((false, Rect::default(), Rect::default()));

        let mut anchor_rebased = false;
        if !is_floating {
            set_floating_in_place(ctx, win);
            let selmon_id = ctx.g_mut().selected_monitor_id();
            arrange(ctx, Some(selmon_id));
            let target_w = if float_geo.w > 0 { float_geo.w } else { geo.w };
            let target_h = if float_geo.h > 0 { float_geo.h } else { geo.h };
            // Place the restored floating window so its top-middle sits under
            // the cursor (matches the title-bar drag warp semantics).
            let target_x = root_x - target_w / 2;
            let target_y = root_y;
            ctx.g_mut().drag.title.win_start_geo = Rect {
                x: target_x,
                y: target_y,
                w: target_w,
                h: target_h,
            };
            ctx.g_mut().drag.title.start_x = root_x;
            ctx.g_mut().drag.title.start_y = root_y;
            anchor_rebased = true;
            resize(
                ctx,
                win,
                &Rect {
                    x: target_x,
                    y: target_y,
                    w: target_w,
                    h: target_h,
                },
                true,
            );
        }

        if !anchor_rebased {
            // Clamp the drag anchor inside the window bounds and rebase
            // start_x/y to the clamped position so deltas are correct even
            // before the deferred warp takes effect.
            let current_geo = ctx.g().clients.get(&win).map(|c| c.geo).unwrap_or(geo);
            super::warp::warp_into(ctx, win);
            let ptr = super::warp::get_root_ptr(ctx).unwrap_or((root_x, root_y));
            let pad = super::warp::WARP_INTO_PADDING;
            let clamped_x = ptr
                .0
                .clamp(current_geo.x + pad, current_geo.x + current_geo.w - pad);
            let clamped_y = ptr
                .1
                .clamp(current_geo.y + pad, current_geo.y + current_geo.h - pad);
            ctx.g_mut().drag.title.start_x = clamped_x;
            ctx.g_mut().drag.title.start_y = clamped_y;
        }

        set_cursor_move(ctx);
        ctx.g_mut().drag.title.dragging = true;
        return title_drag_motion(ctx, root_x, root_y);
    }

    ctx.g_mut().drag.title.dragging = true;
    ctx.g_mut().drag.title.active = false;

    if btn == MouseButton::Right {
        if let Some(c) = ctx.g_mut().clients.get(&win) {
            let (x_off, y_off) =
                ResizeDirection::BottomRight.warp_offset(c.geo.w, c.geo.h, c.border_width);
            if let WmCtx::X11(x11) = ctx {
                let x11_win: Window = win.into();
                let _ = x11.x11.conn.warp_pointer(
                    x11rb::NONE,
                    x11_win,
                    0i16,
                    0i16,
                    0u16,
                    0u16,
                    x_off as i16,
                    y_off as i16,
                );
                let _ = x11.x11.conn.flush();
            }
        }
        if let WmCtx::X11(x11) = ctx {
            super::resize::resize_mouse_directional(x11, Some(ResizeDirection::BottomRight), btn);
        }
    } else {
        if let WmCtx::X11(x11) = ctx {
            warp_into_ctx_x11(x11, win);
            move_mouse(x11, btn);
        }
    }
    true
}

/// Finish a title drag interaction (button release without exceeding the
/// drag threshold).  Performs the click action.
pub fn title_drag_finish(ctx: &mut WmCtx) {
    if !ctx.g_mut().drag.title.active {
        return;
    }

    let is_right_click = ctx.g_mut().drag.title.button == MouseButton::Right;

    if ctx.g_mut().drag.title.dragging {
        let win = ctx.g_mut().drag.title.win;
        let grab_start_rect = ctx.g_mut().drag.title.drop_restore_geo;
        let last = (
            ctx.g_mut().drag.title.last_root_x,
            ctx.g_mut().drag.title.last_root_y,
        );
        ctx.g_mut().drag.title.active = false;
        ctx.g_mut().drag.title.dragging = false;
        set_cursor_default(ctx);
        if !is_right_click {
            complete_move_drop(ctx, win, grab_start_rect, None, Some(last));
        } else {
            handle_client_monitor_switch(ctx, win);
        }
        return;
    }

    let win = ctx.g_mut().drag.title.win;
    let was_focused = ctx.g_mut().drag.title.was_focused;
    let was_hidden = ctx.g_mut().drag.title.was_hidden;
    let suppress_click_action = ctx.g_mut().drag.title.suppress_click_action;

    ctx.g_mut().drag.title.active = false;
    if suppress_click_action {
        return;
    }

    if is_right_click {
        if was_hidden {
            crate::client::show(ctx, win);
            crate::focus::focus_soft(ctx, Some(win));
        }
        crate::client::zoom(ctx);
    } else if was_hidden {
        crate::client::show(ctx, win);
        crate::focus::focus_soft(ctx, Some(win));
        let selmon_id = ctx.g_mut().selected_monitor_id();
        restack(ctx, selmon_id);
    } else if was_focused {
        crate::client::hide(ctx, win);
    } else {
        crate::focus::focus_soft(ctx, Some(win));
        let selmon_id = ctx.g_mut().selected_monitor_id();
        restack(ctx, selmon_id);
    }
}

// ── window_title_mouse_handler ────────────────────────────────────────────────

/// Left-click / drag handler for a window title bar entry.
///
/// Click: hidden → show+focus; focused → hide; otherwise → focus.
/// Drag > [`DRAG_THRESHOLD`]: show, focus, warp, hand off to [`move_mouse`].
///
/// On Wayland, starts the async state machine and returns immediately.
/// On X11, runs a synchronous grab loop.
pub fn window_title_mouse_handler(
    ctx: &mut WmCtxX11,
    win: WindowId,
    btn: MouseButton,
    click_root_x: i32,
    click_root_y: i32,
) {
    if !{
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        title_drag_begin(&mut wm_ctx, win, btn, click_root_x, click_root_y, false)
    } {
        return;
    }

    // ── X11 synchronous grab loop ─────────────────────────────────────
    super::grab::mouse_drag_loop(ctx, btn, 0, false, |ctx, event| {
        if let x11rb::protocol::Event::MotionNotify(m) = event {
            let mut wm_ctx = WmCtx::X11(ctx.reborrow());
            if title_drag_motion(&mut wm_ctx, m.event_x as i32, m.event_y as i32) {
                return false;
            }
        }
        true
    });

    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    title_drag_finish(&mut wm_ctx);
}

// ── window_title_mouse_handler_right ─────────────────────────────────────────

/// Right-click / drag handler for a window title bar entry.
///
/// Click: show+focus if hidden, otherwise zoom to master.
/// Drag > [`DRAG_THRESHOLD`]: show+focus if hidden, resize from bottom-right.
/// No-op when the window is in true fullscreen.
///
pub fn window_title_mouse_handler_right(
    ctx: &mut WmCtxX11,
    win: WindowId,
    btn: MouseButton,
    click_root_x: i32,
    click_root_y: i32,
) {
    if !{
        let mut wm_ctx = WmCtx::X11(ctx.reborrow());
        title_drag_begin(&mut wm_ctx, win, btn, click_root_x, click_root_y, false)
    } {
        return;
    }

    // ── X11 synchronous grab loop ─────────────────────────────────────
    super::grab::mouse_drag_loop(ctx, btn, 2, false, |ctx, event| {
        if let x11rb::protocol::Event::MotionNotify(m) = event {
            let mut wm_ctx = WmCtx::X11(ctx.reborrow());
            if title_drag_motion(&mut wm_ctx, m.event_x as i32, m.event_y as i32) {
                return false;
            }
        }
        true
    });

    let mut wm_ctx = WmCtx::X11(ctx.reborrow());
    title_drag_finish(&mut wm_ctx);
}
