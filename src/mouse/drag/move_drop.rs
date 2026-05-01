#![allow(clippy::too_many_arguments)]
//! Move and drop operations for window dragging.
//!
//! This module contains the core logic for moving windows with the mouse,
//! including bar hover handling, edge snapping, and drop completion.

use crate::bar::bar_position_to_gesture;
use crate::contexts::WmCtx;
use crate::floating::{change_snap, reset_snap, set_window_mode};
use crate::geometry::MoveResizeOptions;
use crate::globals::Globals;
use crate::layouts::arrange;
use crate::tags::{move_client, shift_tag};
use crate::types::*;

use crate::mouse::constants::{MAX_UNMAXIMIZE_OFFSET, OVERLAY_ZONE_WIDTH};

use crate::mouse::monitor::handle_client_monitor_switch;

/// Snap `new_x`/`new_y` to the work-area edges of `selmon` when within `globals.cfg.snap` pixels.
pub fn snap_to_monitor_edges(ctx: &mut WmCtx, c: &Client, new_x: &mut i32, new_y: &mut i32) {
    snap_window_to_monitor_edges(ctx.core().globals(), c.win, c.geo.w, c.geo.h, new_x, new_y);
}

pub fn snap_window_to_monitor_edges(
    g: &Globals,
    win: WindowId,
    w: i32,
    h: i32,
    new_x: &mut i32,
    new_y: &mut i32,
) {
    let snap = g.cfg.snap;
    let mon = g.selected_monitor();
    let bw = g
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
pub fn check_edge_snap(g: &Globals, root: Point) -> Option<SnapPosition> {
    let mon = g.selected_monitor();
    let mask = mon.selected_tags();

    if root.x < mon.monitor_rect.x + OVERLAY_ZONE_WIDTH && root.x > mon.monitor_rect.x - 1 {
        return Some(SnapPosition::Left);
    }
    if root.x > mon.monitor_rect.x + mon.monitor_rect.w - OVERLAY_ZONE_WIDTH
        && root.x < mon.monitor_rect.x + mon.monitor_rect.w + 1
    {
        return Some(SnapPosition::Right);
    }
    if root.y
        <= mon.monitor_rect.y
            + if mon.showbar_for_mask(mask) {
                mon.bar_height
            } else {
                5
            }
    {
        return Some(SnapPosition::Top);
    }
    None
}

/// Returns `true` when `root` (root-space) is inside the bar of `selmon`.
pub fn point_is_on_bar(g: &Globals, root: Point) -> bool {
    let mon = g.selected_monitor();
    let mask = mon.selected_tags();
    mon.showbar_for_mask(mask)
        && root.y >= mon.bar_y
        && root.y < mon.bar_y + mon.bar_height
        && root.x >= mon.monitor_rect.x
        && root.x < mon.monitor_rect.x + mon.monitor_rect.w
}

// ── move_mouse_x11 helpers ────────────────────────────────────────────────────

/// State threaded through the move-mouse event loop.
pub struct MoveState {
    /// Drag origin in root coordinates.
    pub start_point: Point,
    /// Window geometry at drag start.
    pub grab_start_rect: Rect,
    /// Whether the cursor was over the bar on the previous motion event.
    pub cursor_on_bar: bool,
    /// The last edge-snap zone the cursor was in.
    pub edge_snap_indicator: Option<SnapPosition>,
}

/// Perform the pre-flight checks for [`crate::backend::x11::mouse::move_mouse_x11`].
///
/// Returns the window to drag, or `None` if the drag should be aborted.
/// As a side effect:
/// * exits fake-fullscreen and returns `None` so the caller re-enters after the transition
/// * calls `reset_snap` and returns `None` if the window is snapped (un-snap first)
/// * restores a near-maximized floating window to its saved geometry
pub fn prepare_drag_target(ctx: &mut WmCtx) -> Option<WindowId> {
    let sel = {
        let g = ctx.core_mut().globals_mut();
        let mon = g.selected_monitor();
        mon.sel?
    };
    let c = ctx.core().client(sel)?;
    let is_true_fullscreen = c.mode.is_true_fullscreen();
    let is_edge_scratchpad = c.is_edge_scratchpad();
    let is_maximized = c.mode.is_maximized();

    if is_true_fullscreen {
        return None;
    }
    if is_edge_scratchpad {
        return None;
    }
    if is_maximized {
        crate::floating::toggle_maximized(ctx);
        return None;
    }
    let selected_window = sel;

    let selmon_id = ctx.core_mut().globals_mut().selected_monitor_id();
    crate::layouts::sync_monitor_z_order(ctx, selmon_id);

    // Un-snap: surface the real window first; the user re-drags after.
    let is_snapped = match ctx.core().client(selected_window) {
        Some(c) => c.snap_status != SnapPosition::None,
        None => return None,
    };
    if is_snapped {
        reset_snap(ctx, selected_window);
        return None;
    }

    // In a floating layout, if the window fills (nearly) the whole monitor,
    // restore the saved float geometry so we drag the real size, not a maximized one.
    let restore_geo: Option<Rect> = {
        let has_tiling = ctx
            .core_mut()
            .globals_mut()
            .selected_monitor()
            .is_tiling_layout();

        if !has_tiling {
            let mon = ctx.core().globals().selected_monitor();
            let bar_height = mon.bar_height;
            if let Some(c) = ctx.core().client(selected_window) {
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
        ctx.move_resize(
            selected_window,
            geo,
            MoveResizeOptions::hinted_immediate(false),
        );
    }

    Some(selected_window)
}

/// Update `bar_dragging` and the gesture (tag hover highlight) while dragging.
///
/// Tracks enter/leave transitions via `state.cursor_on_bar` so the bar is only
/// redrawn when something changes.  Returns `true` while the cursor is on the bar.
pub fn update_bar_hover(ctx: &mut WmCtx, root: Point, state: &mut MoveState) -> bool {
    let on_bar = point_is_on_bar(ctx.core().globals(), root);

    let selmon_id = ctx.core().globals().selected_monitor_id();

    if on_bar {
        let new_gesture = {
            let core = ctx.core();
            let mon = core.globals().selected_monitor();
            let local_x = root.x - mon.work_rect.x;
            bar_position_to_gesture(mon.bar_position_at_x(core, local_x))
        };

        let gesture_changed = ctx.core().globals().selected_monitor().gesture != new_gesture;

        if !state.cursor_on_bar || gesture_changed {
            ctx.core_mut().globals_mut().drag.bar_active = true;
            ctx.core_mut().globals_mut().selected_monitor_mut().gesture = new_gesture;
            ctx.request_bar_update(Some(selmon_id));
        }
    } else if state.cursor_on_bar {
        ctx.core_mut().globals_mut().drag.bar_active = false;
        ctx.core_mut().globals_mut().selected_monitor_mut().gesture = Gesture::None;
        ctx.request_bar_update(Some(selmon_id));
    }

    on_bar
}

/// Simplified bar hover update for Wayland drag paths that don't use [`MoveState`].
///
/// Sets `bar_active` and the gesture highlight when the cursor enters the bar,
/// and clears them when it leaves.  Returns `true` while on the bar.
pub fn update_bar_hover_simple(ctx: &mut WmCtx, root: Point) -> bool {
    let on_bar = point_is_on_bar(ctx.core().globals(), root);
    let selmon_id = ctx.core().globals().selected_monitor_id();
    let was_on_bar = ctx.core().globals().drag.bar_active;

    if on_bar {
        let new_gesture = {
            let core = ctx.core();
            let mon = core.globals().selected_monitor();
            let local_x = root.x - mon.work_rect.x;
            bar_position_to_gesture(mon.bar_position_at_x(core, local_x))
        };
        let gesture_changed = ctx.core().globals().selected_monitor().gesture != new_gesture;
        if !was_on_bar || gesture_changed {
            ctx.core_mut().globals_mut().drag.bar_active = true;
            ctx.core_mut().globals_mut().selected_monitor_mut().gesture = new_gesture;
            ctx.request_bar_update(Some(selmon_id));
        }
    } else if was_on_bar {
        ctx.core_mut().globals_mut().drag.bar_active = false;
        ctx.core_mut().globals_mut().selected_monitor_mut().gesture = Gesture::None;
        ctx.request_bar_update(Some(selmon_id));
    }

    on_bar
}

/// Process a single throttled `MotionNotify` event during [`crate::backend::x11::mouse::move_mouse_x11`].
pub fn on_motion(ctx: &mut WmCtx, win: WindowId, event: Point, root: Point, state: &mut MoveState) {
    state.cursor_on_bar = update_bar_hover(ctx, root, state);
    state.edge_snap_indicator = check_edge_snap(ctx.core().globals(), root);

    let mut new_x = state.grab_start_rect.x + (event.x - state.start_point.x);
    let mut new_y = state.grab_start_rect.y + (event.y - state.start_point.y);

    // While hovering over the bar, keep the window just below it.
    if state.cursor_on_bar {
        let bar_bottom = {
            let mon = ctx.core().globals().selected_monitor();
            mon.bar_y + mon.bar_height
        };
        new_y = bar_bottom;
    }

    let has_tiling = ctx.core().globals().selected_monitor().is_tiling_layout();

    let (mut is_floating, mut drag_geo) = match ctx.core().client(win) {
        Some(c) => (c.mode.is_floating(), c.geo),
        None => return,
    };

    maybe_promote_tiled_drag_to_floating(
        ctx,
        win,
        event,
        &mut new_x,
        &mut new_y,
        state,
        has_tiling,
        &mut is_floating,
        &mut drag_geo,
    );

    if !has_tiling || is_floating {
        if let Some(client) = ctx.core().client(win).cloned() {
            snap_to_monitor_edges(ctx, &client, &mut new_x, &mut new_y);
        }
        ctx.move_resize(
            win,
            Rect {
                x: new_x,
                y: new_y,
                w: drag_geo.w,
                h: drag_geo.h,
            },
            MoveResizeOptions::hinted_immediate(true),
        );
    }
}

fn maybe_promote_tiled_drag_to_floating(
    ctx: &mut WmCtx,
    win: WindowId,
    event: Point,
    new_x: &mut i32,
    new_y: &mut i32,
    state: &mut MoveState,
    has_tiling: bool,
    is_floating: &mut bool,
    drag_geo: &mut Rect,
) {
    let snap = ctx.core().globals().cfg.snap;
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
        ctx.core()
            .globals()
            .clients
            .get(&win)
            .map(|c| {
                let eff = c.effective_float_geo();
                (eff.w, eff.h)
            })
            .unwrap_or((drag_geo.w, drag_geo.h))
    };

    // Flip isfloating + restore border — zero configure_window calls.
    let _ = set_window_mode(ctx, win, BaseClientMode::Floating);

    // Re-tile the remaining windows (touches only the other clients).
    let selmon_id = ctx.core().globals().selected_monitor_id();
    arrange(ctx, Some(selmon_id));

    // The window's width is changing (tiled → floating), so the old
    // `grab_start_x`-based `new_x` would leave the window at x≈0 while the cursor
    // is far to the right. Re-center under cursor and rebase drag anchors.
    *new_x = event.x - float_w / 2;
    state.grab_start_rect.x = *new_x;
    state.grab_start_rect.y = *new_y;
    state.start_point = event;

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
pub fn clear_bar_hover(ctx: &mut WmCtx) {
    ctx.core_mut().globals_mut().drag.bar_active = false;
    let selmon_id = ctx.core().globals().selected_monitor_id();
    ctx.core_mut().globals_mut().selected_monitor_mut().gesture = Gesture::None;
    ctx.request_bar_update(Some(selmon_id));
}

/// Handle a drop onto the bar: tile the window, optionally moving it to the
/// hovered tag first.
///
/// Mirrors the C `handle_bar_drop`:
/// * Dropped on a tag button → `set_window_mode(Tiled)` + `tag()`
/// * Dropped elsewhere on bar, window floating → `set_window_mode(Tiled)`
///
/// # `grab_start_rect`
///
/// The window geometry at the moment the drag started.  When the window was
/// floating, this is the true pre-drag origin; we save it into `float_geo`
/// so un-tiling later restores the original floating position.
pub fn handle_bar_drop(
    ctx: &mut WmCtx,
    win: WindowId,
    grab_start_rect: Rect,
    pointer_override: Option<Point>,
) {
    let Some(root) = pointer_override.or_else(|| ctx.pointer_location()) else {
        return;
    };
    if !point_is_on_bar(ctx.core().globals(), root) {
        return;
    }

    let position = {
        let core = ctx.core();
        let mon = core.globals().selected_monitor();
        let local_x = root.x - mon.work_rect.x;
        mon.bar_position_at_x(core, local_x)
    };

    // Remember whether the window was floating *before* any state change so
    // we know whether to correct float_geo afterwards.
    let was_floating = match ctx.core().client(win) {
        Some(c) => c.mode.is_floating(),
        None => return,
    };

    if let BarPosition::Tag(tag_idx) = position {
        // Tile first (no arrange), then tag.
        //
        // Old order: tag() → arrange() [window still floating, layout skips
        // it] → set_window_mode() → arrange() again.  That's two arrange passes.
        //
        // New order: set_window_mode(should_arrange=false) saves float_geo from the
        // current floating geometry *before* tag() calls arrange().  Then
        // tag() calls arrange() exactly once with the window already marked
        // tiled, so the layout places it correctly in a single pass.
        //
        // tag() uses selmon->sel internally (via set_client_tag_impl), so win
        // must still be the selected window at this point — which it is because
        // set_window_mode does not touch focus.

        // Don't tile fullscreen windows
        if !ctx
            .core()
            .client(win)
            .is_some_and(|c| c.mode.is_true_fullscreen())
        {
            let _ = set_window_mode(ctx, win, BaseClientMode::Tiling);
        }
        crate::tags::client_tags::set_client_tag(
            ctx,
            win,
            TagMask::single(tag_idx + 1).unwrap_or(TagMask::EMPTY),
        );
    } else if was_floating {
        // Dropped on the bar but not on a tag button: tile the window.
        // Use set_window_mode directly instead of toggle_floating() which
        // operates on mon.sel — a value that could theoretically diverge from
        // the window we actually dragged.
        let _ = set_window_mode(ctx, win, BaseClientMode::Tiling);
        let selmon_id = ctx.core().globals().selected_monitor_id();
        arrange(ctx, Some(selmon_id));
    } else {
        // Window is already tiled and not dropped on a tag — nothing to do.
        return;
    }

    // ── Correct float_geo using pre-drag dimensions ───────────────────────
    //
    // Keep the drop position (x/y from set_window_mode's saved client.geo), but
    // preserve the pre-drag floating size so un-tiling restores dimensions.
    if was_floating && let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
        client.float_geo.w = grab_start_rect.w;
        client.float_geo.h = grab_start_rect.h;
    }
}

/// Apply post-release logic for left/right screen-edge drops.
///
/// In a tiling layout: navigate to the adjacent tag (or send the window there).
/// In a floating layout: apply a directional screen-edge snap.
///
/// Returns `true` if the drop was fully handled (the caller should skip
/// `handle_bar_drop` and `handle_client_monitor_switch`).
pub fn apply_edge_drop(
    ctx: &mut WmCtx,
    win: WindowId,
    edge: Option<SnapPosition>,
    root: Point,
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

    let is_tiling = ctx.core().globals().selected_monitor().is_tiling_layout();

    if is_tiling {
        let (mon_my, mon_mh) = (
            ctx.core().globals().selected_monitor().monitor_rect.y,
            ctx.core().globals().selected_monitor().monitor_rect.h,
        );

        // Upper 2/3 of the monitor → move view; lower 1/3 → send window.
        if root.y < mon_my + (2 * mon_mh) / 3 {
            if at_left {
                move_client(ctx, HorizontalDirection::Left);
            } else {
                move_client(ctx, HorizontalDirection::Right);
            }
        } else if at_left {
            shift_tag(ctx, HorizontalDirection::Left.into(), 1);
        } else {
            shift_tag(ctx, HorizontalDirection::Right.into(), 1);
        }

        if let Some(c) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
            c.mode = ClientMode::Tiling;
        }
        let selmon_id = ctx.core().globals().selected_monitor_id();
        arrange(ctx, Some(selmon_id));
    } else {
        let dir = if at_left {
            Direction::Left
        } else {
            Direction::Right
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
    pointer_override: Option<Point>,
) {
    let pointer = pointer_override.or_else(|| ctx.pointer_location());
    let edge =
        edge_hint.or_else(|| pointer.and_then(|root| check_edge_snap(ctx.core().globals(), root)));
    let handled_edge = pointer
        .map(|root| apply_edge_drop(ctx, win, edge, root))
        .unwrap_or(false);
    if !handled_edge {
        handle_bar_drop(ctx, win, grab_start_rect, pointer);
        handle_client_monitor_switch(ctx, win);
    }
}

/// Helper function for promoting a window to floating.
/// Used by both title drag and move operations.
pub fn promote_to_floating(
    ctx: &mut WmCtx,
    win: WindowId,
    center_under_ptr: Option<Point>,
) -> (Rect, bool) {
    let (is_floating, geo) = ctx
        .core()
        .globals()
        .clients
        .get(&win)
        .map(|c| (c.mode.is_floating(), c.geo))
        .unwrap_or((false, Rect::default()));

    if is_floating {
        return (geo, false);
    }

    let _ = set_window_mode(ctx, win, BaseClientMode::Floating);
    let selmon_id = ctx.core_mut().globals_mut().selected_monitor_id();
    arrange(ctx, Some(selmon_id));

    let (target_w, target_h) = ctx
        .core()
        .globals()
        .clients
        .get(&win)
        .map(|c| {
            let eff = c.effective_float_geo();
            (eff.w, eff.h)
        })
        .unwrap_or((geo.w, geo.h));

    let (target_x, target_y) = if let Some(root) = center_under_ptr {
        (root.x - target_w / 2, root.y)
    } else {
        (geo.x, geo.y)
    };

    let new_geo = Rect {
        x: target_x,
        y: target_y,
        w: target_w,
        h: target_h,
    };
    ctx.move_resize(win, new_geo, MoveResizeOptions::hinted_immediate(true));
    (new_geo, true)
}
