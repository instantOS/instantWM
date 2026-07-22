#![allow(clippy::too_many_arguments)]
//! Move and drop operations for window dragging.
//!
//! This module contains the core logic for moving windows with the mouse,
//! including bar hover handling, edge snapping, and drop completion.

use crate::contexts::WmCtx;
use crate::core_state::CoreState;
use crate::floating::{change_snap, reset_snap, set_window_mode};
use crate::geometry::MoveResizeOptions;
use crate::layouts::arrange;
use crate::tags::{move_client_follow_view, shift_tag};
use crate::types::*;

use crate::mouse::constants::{MAX_UNMAXIMIZE_OFFSET, OVERLAY_ZONE_WIDTH};

use crate::mouse::monitor::handle_client_monitor_switch;

/// Snap `pos` to the work-area edges of `selmon` when within `globals.config.window.snap_threshold` pixels.
pub fn snap_to_monitor_edges(ctx: &mut WmCtx, c: &Client, pos: &mut Point) {
    snap_window_to_monitor_edges(ctx.core().state(), c.win, c.geo.size(), pos);
}

pub fn snap_window_to_monitor_edges(
    state: &CoreState,
    window: WindowId,
    content_size: Size,
    position: &mut Point,
) {
    let snap = state.config.window.snap_threshold;
    let Some(view) = state.model.client_view(window) else {
        return;
    };
    let monitor = view.monitor;
    let border_width = view.client.border_width.max(0);
    let outer_size = Size::new(
        content_size.w + border_width * 2,
        content_size.h + border_width * 2,
    );
    let work_rect = monitor.work_rect();

    if (work_rect.x - position.x).abs() < snap {
        position.x = work_rect.x;
    } else if (work_rect.right() - (position.x + outer_size.w)).abs() < snap {
        position.x = work_rect.right() - outer_size.w;
    }

    if (work_rect.y - position.y).abs() < snap {
        position.y = work_rect.y;
    } else if (work_rect.bottom() - (position.y + outer_size.h)).abs() < snap {
        position.y = work_rect.bottom() - outer_size.h;
    }
}

/// Returns edge snap position based on cursor position.
pub fn check_edge_snap(model: &crate::model::WmModel, root: Point) -> Option<SnapPosition> {
    let mon = model.expect_selected_monitor();
    let mask = mon.selected_tags();

    if root.x < mon.monitor_rect.x + OVERLAY_ZONE_WIDTH && root.x > mon.monitor_rect.x - 1 {
        return Some(SnapPosition::Left);
    }
    if root.x > mon.monitor_rect.right() - OVERLAY_ZONE_WIDTH
        && root.x < mon.monitor_rect.right() + 1
    {
        return Some(SnapPosition::Right);
    }
    if root.y
        <= mon.monitor_rect.y
            + if mon.show_bar_for_mask(mask) {
                mon.bar_height
            } else {
                5
            }
    {
        return Some(SnapPosition::Top);
    }
    None
}

/// Project the semantic tiled-drop target into the shared preview overlay.
/// Both the synchronous X11 drag loop and Wayland's event-driven drag path
/// call this routine so bar and screen-edge precedence cannot diverge.
pub fn update_tiled_drag_preview(
    ctx: &mut WmCtx,
    win: WindowId,
    root: Point,
    on_bar: bool,
    edge: Option<SnapPosition>,
) {
    let preview = (!on_bar && edge.is_none())
        .then(|| crate::layouts::preview_tree_at_point(ctx, win, root))
        .flatten();
    ctx.update_layout_preview(preview);
}

/// Returns `true` when `root` (root-space) is inside the bar of `selmon`.
pub fn point_is_on_bar(model: &crate::model::WmModel, root: Point) -> bool {
    let mon = model.expect_selected_monitor();
    let mask = mon.selected_tags();
    mon.show_bar_for_mask(mask)
        && mon.y_in_bar(root.y)
        && root.x >= mon.monitor_rect.x
        && root.x < mon.monitor_rect.right()
}

// ── move_mouse helpers ────────────────────────────────────────────────────

/// State threaded through the move-mouse event loop.
pub struct MoveState {
    /// Drag origin in root coordinates.
    pub start_point: Point,
    /// Window geometry at drag start.
    pub grab_start_rect: Rect,
    /// Floating geometry to retain if the drag ends by re-tiling the client.
    /// This is deliberately separate from `grab_start_rect`: promoting a
    /// tiled client changes the geometry used for motion without discarding
    /// its saved floating restore geometry.
    pub drop_restore_rect: Rect,
    /// Whether the cursor was over the bar on the previous motion event.
    pub cursor_on_bar: bool,
    /// The last edge-snap zone the cursor was in.
    pub edge_snap_indicator: Option<SnapPosition>,
}

/// Perform the pre-flight checks for [`crate::backend::x11::mouse::move_mouse`].
///
/// Returns the window to drag, or `None` if the drag should be aborted.
/// As a side effect:
/// * exits fake-fullscreen and returns `None` so the caller re-enters after the transition
/// * calls `reset_snap` and returns `None` if the window is snapped (un-snap first)
/// * restores a near-maximized floating window to its saved geometry
pub fn prepare_drag_target(ctx: &mut WmCtx) -> Option<WindowId> {
    let sel = {
        let g = ctx.core_mut().state_mut();
        let mon = g.expect_selected_monitor();
        mon.selected?
    };
    let c = ctx.core().model().client(sel)?;
    let is_true_fullscreen = c.mode().is_true_fullscreen();
    let is_edge_scratchpad = c.is_edge_scratchpad();
    let is_maximized = c.mode().is_maximized();

    if is_true_fullscreen {
        return None;
    }
    if is_edge_scratchpad {
        return None;
    }
    if is_maximized {
        crate::floating::toggle_client_maximized(ctx);
        return None;
    }
    let selected_window = sel;

    let selmon_id = ctx.core_mut().model_mut().selected_monitor_id();
    crate::layouts::sync_monitor_z_order(ctx, selmon_id);

    // Un-snap: surface the real window first; the user re-drags after.
    let is_snapped = {
        let c = ctx.core().model().client(selected_window)?;
        c.snap_status != SnapPosition::None
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
            .state_mut()
            .expect_selected_monitor()
            .is_tiling_layout();

        if !has_tiling {
            let mon = ctx.core().model().expect_selected_monitor();
            let bar_height = mon.bar_height;
            if let Some(c) = ctx.core().model().client(selected_window) {
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
    let on_bar = point_is_on_bar(ctx.core().model(), root);

    if on_bar {
        let new_gesture = {
            let core = ctx.core();
            let mon = core.model().expect_selected_monitor();
            mon.bar_position_at_x(core, mon.local_work_point(root).x)
                .to_gesture()
        };

        let monitor_id = ctx.core().model().selected_monitor_id();
        let gesture_changed = ctx.core().bar.hover.gesture_on(monitor_id) != new_gesture;

        if !state.cursor_on_bar || gesture_changed {
            ctx.core_mut().bar.hover.set(monitor_id, new_gesture, true);
            ctx.request_bar_update();
        }
    } else if state.cursor_on_bar {
        ctx.core_mut().bar.hover.clear();
        ctx.request_bar_update();
    }

    on_bar
}

/// Simplified bar hover update for Wayland drag paths that don't use [`MoveState`].
///
/// Sets the drag hover and gesture highlight when the cursor enters the bar,
/// and clears them when it leaves.  Returns `true` while on the bar.
pub fn update_bar_hover_simple(ctx: &mut WmCtx, root: Point) -> bool {
    let on_bar = point_is_on_bar(ctx.core().model(), root);
    let was_on_bar = ctx.core().bar.hover.drag_active;

    if on_bar {
        let new_gesture = {
            let core = ctx.core();
            let mon = core.model().expect_selected_monitor();
            mon.bar_position_at_x(core, mon.local_work_point(root).x)
                .to_gesture()
        };
        let monitor_id = ctx.core().model().selected_monitor_id();
        let gesture_changed = ctx.core().bar.hover.gesture_on(monitor_id) != new_gesture;
        if !was_on_bar || gesture_changed {
            ctx.core_mut().bar.hover.set(monitor_id, new_gesture, true);
            ctx.request_bar_update();
        }
    } else if was_on_bar {
        ctx.core_mut().bar.hover.clear();
        ctx.request_bar_update();
    }

    on_bar
}

/// Process a single throttled `MotionNotify` event during [`crate::backend::x11::mouse::move_mouse`].
pub fn on_motion(ctx: &mut WmCtx, win: WindowId, event: Point, root: Point, state: &mut MoveState) {
    state.cursor_on_bar = update_bar_hover(ctx, root, state);
    state.edge_snap_indicator = check_edge_snap(ctx.core().model(), root);

    let mut new_pos = Point::new(
        state.grab_start_rect.x + (event.x - state.start_point.x),
        state.grab_start_rect.y + (event.y - state.start_point.y),
    );

    // While hovering over the bar, keep the window just below it.
    if state.cursor_on_bar {
        let bar_bottom = {
            let mon = ctx.core().model().expect_selected_monitor();
            mon.bar_y() + mon.bar_height
        };
        new_pos.y = bar_bottom;
    }

    if crate::layouts::manager::uses_manual_tree_pointer_interaction(ctx, win) {
        update_tiled_drag_preview(
            ctx,
            win,
            root,
            state.cursor_on_bar,
            state.edge_snap_indicator,
        );
        return;
    }

    // Thresholding is owned by the shared client/title drag state machine.
    // Once motion reaches this function, any tiled client that cannot perform
    // a meaningful tree edit becomes an ordinary floating move.
    let Some((drag_geo, _)) = promote_to_floating(ctx, win, None) else {
        return;
    };
    ctx.update_layout_preview(None);

    if let Some(client) = ctx.core().model().client(win).cloned() {
        snap_to_monitor_edges(ctx, &client, &mut new_pos);
    }
    ctx.move_resize(
        win,
        Rect {
            x: new_pos.x,
            y: new_pos.y,
            w: drag_geo.w,
            h: drag_geo.h,
        },
        MoveResizeOptions::hinted_immediate(true),
    );
}

/// Clears `bar_dragging` and redraws the bar unconditionally.
///
/// Called once the drag loop exits so that hover state is always cleaned up.
pub fn clear_bar_hover(ctx: &mut WmCtx) {
    ctx.core_mut().bar.hover.clear();
    ctx.request_bar_update();
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
    modifiers: u32,
) {
    let Some(root) = pointer_override.or_else(|| ctx.pointer_backend().pointer_location()) else {
        return;
    };
    if !point_is_on_bar(ctx.core().model(), root) {
        return;
    }

    let position = {
        let core = ctx.core();
        let mon = core.model().expect_selected_monitor();
        mon.bar_position_at_x(core, mon.local_work_point(root).x)
    };

    // Remember whether the window was floating *before* any state change so
    // we know whether to correct float_geo afterwards.
    let was_floating = match ctx.core().model().client(win) {
        Some(c) => c.mode().is_floating(),
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
            .state()
            .model
            .client(win)
            .is_some_and(|c| c.mode().is_true_fullscreen())
        {
            let _ = set_window_mode(ctx, win, BaseClientMode::Tiling);
        }
        crate::mouse::drag::tag::apply_window_tag_drop(
            ctx,
            win,
            TagMask::from_index(tag_idx).unwrap_or(TagMask::EMPTY),
            modifiers,
        );
    } else if was_floating {
        // Dropped on the bar but not on a tag button: tile the window.
        // Use set_window_mode directly instead of toggle_floating() which
        // operates on mon.sel — a value that could theoretically diverge from
        // the window we actually dragged.
        let _ = set_window_mode(ctx, win, BaseClientMode::Tiling);
        let selmon_id = ctx.core().model().selected_monitor_id();
        arrange(ctx, Some(selmon_id));
    } else {
        // Window is already tiled and not dropped on a tag — nothing to do.
        return;
    }

    // ── Correct float_geo using pre-drag dimensions ───────────────────────
    //
    // Keep the drop position (x/y from set_window_mode's saved client.geo), but
    // preserve the pre-drag floating size so un-tiling restores dimensions.
    if was_floating && let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
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

    let is_tiling = ctx
        .core()
        .model()
        .expect_selected_monitor()
        .is_tiling_layout();

    if is_tiling {
        let (mon_my, mon_mh) = (
            ctx.core().model().expect_selected_monitor().monitor_rect.y,
            ctx.core().model().expect_selected_monitor().monitor_rect.h,
        );

        // Upper 2/3 of the monitor → move view; lower 1/3 → send window.
        if root.y < mon_my + (2 * mon_mh) / 3 {
            if at_left {
                move_client_follow_view(ctx, HorizontalDirection::Left);
            } else {
                move_client_follow_view(ctx, HorizontalDirection::Right);
            }
        } else if at_left {
            shift_tag(ctx, HorizontalDirection::Left.into(), 1);
        } else {
            shift_tag(ctx, HorizontalDirection::Right.into(), 1);
        }

        if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
            finish_tiling_edge_drop(client);
        }
        let selmon_id = ctx.core().model().selected_monitor_id();
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

/// Tile after an edge drop without replacing the pre-drag floating restore rectangle.
fn finish_tiling_edge_drop(client: &mut Client) {
    // The drag has already moved `geo` to the edge. Unlike the ordinary
    // tiled-mode command, snapshotting it here would destroy the position
    // restored by a later float toggle.
    client.replace_mode_with_base(BaseClientMode::Tiling);
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
    modifiers: u32,
) {
    // Hide the speculative frame before applying the authoritative drop. This
    // also covers release outside every valid tree target.
    ctx.update_layout_preview(None);
    let pointer = pointer_override.or_else(|| ctx.pointer_backend().pointer_location());
    let edge =
        edge_hint.or_else(|| pointer.and_then(|root| check_edge_snap(ctx.core().model(), root)));
    let handled_edge = pointer
        .map(|root| apply_edge_drop(ctx, win, edge, root))
        .unwrap_or(false);
    if !handled_edge {
        let handled_tree = pointer.is_some_and(|root| {
            !point_is_on_bar(ctx.core().model(), root)
                && ctx
                    .core()
                    .model()
                    .client(win)
                    .is_some_and(|client| client.mode().is_tiling())
                && crate::layouts::place_tree_at_point(ctx, win, root)
        });
        if handled_tree {
            return;
        }
        handle_bar_drop(ctx, win, grab_start_rect, pointer, modifiers);
        handle_client_monitor_switch(ctx, win);
    }
}

/// Helper function for promoting a window to floating.
/// Used by both title drag and move operations.
pub fn promote_to_floating(
    ctx: &mut WmCtx,
    win: WindowId,
    center_under_ptr: Option<Point>,
) -> Option<(Rect, bool)> {
    let (is_floating, geo) = ctx
        .core()
        .state()
        .model
        .client(win)
        .map(|c| (c.mode().is_floating(), c.geo))?;

    if is_floating {
        return Some((geo, false));
    }

    let restored_geometry = match set_window_mode(ctx, win, BaseClientMode::Floating) {
        crate::floating::WindowModeChange::ChangedToFloating { restored_geometry } => {
            restored_geometry
        }
        crate::floating::WindowModeChange::MissingClient => return None,
        crate::floating::WindowModeChange::ChangedToTiling => {
            unreachable!("requesting floating mode produced a tiling transition")
        }
    };
    let selmon_id = ctx.core_mut().model_mut().selected_monitor_id();
    arrange(ctx, Some(selmon_id));

    let (target_w, target_h) = (restored_geometry.w, restored_geometry.h);

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
    Some((new_geo, true))
}

#[cfg(test)]
mod tests {
    use super::finish_tiling_edge_drop;
    use crate::types::{BaseClientMode, Client, ClientMode, Rect};

    #[test]
    fn edge_drop_keeps_the_pre_drag_floating_restore_rectangle() {
        let saved = Rect::new(300, 200, 700, 500);
        let mut client = Client {
            geo: Rect::new(0, 200, 700, 500),
            float_geo: saved,
            ..Client::default()
        };
        client.replace_mode_with_base(BaseClientMode::Floating);

        finish_tiling_edge_drop(&mut client);

        assert_eq!(client.mode(), ClientMode::Tiling);
        assert_eq!(client.float_geo, saved);
    }
}
