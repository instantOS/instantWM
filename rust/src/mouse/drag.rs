//! Interactive mouse-drag operations.
//!
//! | Function                            | Description                                               |
//! |-------------------------------------|-----------------------------------------------------------|
//! | [`move_mouse`]                      | Drag the focused window to a new position                 |
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
//! ungrab_ctx(ctx)
//! post-loop cleanup (bar drop, monitor switch, bar redraw, …)
//! ```

use crate::backend::BackendKind;
use crate::bar::bar_position_at_x;
use crate::bar::bar_position_to_gesture;
use crate::bar::draw_bar;
use crate::client::resize;
use crate::config::commands::Cmd;
use crate::contexts::WmCtx;
use crate::floating::{change_snap, reset_snap, set_floating_in_place, set_tiled, SnapDir};
// focus() is used via focus_soft() in this module
use crate::layouts::{arrange, restack};
use crate::mouse::warp::warp_into;
use crate::tags::{follow_tag, move_client, set_client_tag, shift_tag_by, tag_all, view};
use crate::types::SnapPosition;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use super::constants::{
    DRAG_THRESHOLD, MAX_UNMAXIMIZE_OFFSET, OVERLAY_ZONE_WIDTH, REFRESH_RATE_HI, REFRESH_RATE_LO,
};
use super::cursor::{set_cursor_default, set_cursor_move, set_cursor_resize};
use super::grab::{grab_pointer, ungrab_ctx, wait_event};
use super::monitor::handle_client_monitor_switch;
use super::warp::get_root_ptr;

// ── Shared helpers ────────────────────────────────────────────────────────────

fn refresh_rate(ctx: &WmCtx) -> u32 {
    if ctx.g.doubledraw {
        REFRESH_RATE_HI
    } else {
        REFRESH_RATE_LO
    }
}

/// Snap `new_x`/`new_y` to the work-area edges of `selmon` when within `globals.cfg.snap` pixels.
fn snap_to_monitor_edges(ctx: &WmCtx, c: &Client, new_x: &mut i32, new_y: &mut i32) {
    let snap = ctx.g.cfg.snap;
    let Some(mon) = ctx.g.selmon() else {
        return;
    };

    let width = c.geo.total_width(c.border_width);
    let height = c.geo.total_height(c.border_width);

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
    let Some(mon) = ctx.g.selmon() else {
        return None;
    };

    if x < mon.monitor_rect.x + OVERLAY_ZONE_WIDTH && x > mon.monitor_rect.x - 1 {
        return Some(SnapPosition::Left);
    }
    if x > mon.monitor_rect.x + mon.monitor_rect.w - OVERLAY_ZONE_WIDTH
        && x < mon.monitor_rect.x + mon.monitor_rect.w + 1
    {
        return Some(SnapPosition::Right);
    }
    if y <= mon.monitor_rect.y + if mon.showbar { ctx.g.cfg.bar_height } else { 5 } {
        return Some(SnapPosition::Top);
    }
    None
}

/// Returns `true` when `(x, y)` (root-space) is inside the bar of `selmon`.
fn point_is_on_bar(ctx: &WmCtx, x: i32, y: i32) -> bool {
    let Some(mon) = ctx.g.selmon() else {
        return false;
    };
    mon.showbar
        && y >= mon.by
        && y < mon.by + ctx.g.cfg.bar_height
        && x >= mon.monitor_rect.x
        && x < mon.monitor_rect.x + mon.monitor_rect.w
}

// ── move_mouse helpers ────────────────────────────────────────────────────────

/// State threaded through the move-mouse event loop.
struct MoveState {
    /// Drag origin in root coordinates.
    start_x: i32,
    start_y: i32,
    /// Window position at drag start.
    grab_start_x: i32,
    grab_start_y: i32,
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
    let sel_win = {
        let mon = ctx.g.selmon()?;
        let sel = mon.sel?;
        let c = ctx.g.clients.get(&sel)?;

        if c.is_true_fullscreen() {
            return None;
        }
        if Some(sel) == mon.overlay {
            return None;
        }
        if Some(sel) == mon.fullscreen {
            crate::floating::temp_fullscreen(ctx);
            return None;
        }
        sel
    };

    crate::layouts::restack(ctx, ctx.g.selmon_id());

    // Un-snap: surface the real window first; the user re-drags after.
    let is_snapped = ctx
        .g
        .clients
        .get(&sel_win)
        .map(|c| c.snapstatus != SnapPosition::None)
        .unwrap_or(false);
    if is_snapped {
        reset_snap(ctx, sel_win);
        return None;
    }

    // In a floating layout, if the window fills (nearly) the whole monitor,
    // restore the saved float geometry so we drag the real size, not a maximized one.
    let restore_geo: Option<Rect> = {
        let has_tiling = ctx.g.selmon().map(|m| m.is_tiling_layout()).unwrap_or(true);

        if !has_tiling {
            if let (Some(c), Some(mon)) = (ctx.g.clients.get(&sel_win), ctx.g.selmon()) {
                let bh = ctx.g.cfg.bar_height;
                let nearly_maximized = c.geo.x >= mon.monitor_rect.x - MAX_UNMAXIMIZE_OFFSET
                    && c.geo.y >= mon.monitor_rect.y + bh - MAX_UNMAXIMIZE_OFFSET
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
        resize(ctx, sel_win, &geo, false);
    }

    Some(sel_win)
}

/// Update `bar_dragging` and the gesture (tag hover highlight) while dragging.
///
/// Tracks enter/leave transitions via `state.cursor_on_bar` so the bar is only
/// redrawn when something changes.  Returns `true` while the cursor is on the bar.
fn update_bar_hover(ctx: &mut WmCtx, ptr_x: i32, ptr_y: i32, state: &mut MoveState) -> bool {
    let on_bar = point_is_on_bar(ctx, ptr_x, ptr_y);

    let selmon_id = ctx.g.selmon_id();

    if on_bar {
        // Use the canonical bar hit-test so that tag hover highlighting during
        // a window-drag uses exactly the same geometry as click dispatch and
        // motion_notify gesture detection.
        let new_gesture = ctx
            .g
            .selmon()
            .map(|mon| {
                let local_x = ptr_x - mon.work_rect.x;
                bar_position_to_gesture(bar_position_at_x(mon, ctx, local_x))
            })
            .unwrap_or(Gesture::None);

        let gesture_changed = ctx
            .g
            .selmon()
            .map(|m| m.gesture != new_gesture)
            .unwrap_or(false);

        if !state.cursor_on_bar || gesture_changed {
            ctx.g.bar_dragging = true;
            if let Some(mon) = ctx.g.selmon_mut() {
                mon.gesture = new_gesture;
            }
            draw_bar(ctx, selmon_id);
        }
    } else if state.cursor_on_bar {
        ctx.g.bar_dragging = false;
        if let Some(mon) = ctx.g.selmon_mut() {
            mon.gesture = Gesture::None;
        }
        draw_bar(ctx, selmon_id);
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

    let mut new_x = state.grab_start_x + (event_x - state.start_x);
    let mut new_y = state.grab_start_y + (event_y - state.start_y);

    // While hovering over the bar, keep the window just below it.
    if state.cursor_on_bar {
        let bar_bottom = ctx
            .g
            .selmon()
            .map(|m| m.by + ctx.g.cfg.bar_height)
            .unwrap_or(new_y);
        new_y = bar_bottom;
    }

    let snap = ctx.g.cfg.snap;
    let has_tiling = ctx.g.selmon().map(|m| m.is_tiling_layout()).unwrap_or(true);

    let (mut is_floating, mut drag_geo) = ctx
        .g
        .clients
        .get(&win)
        .map(|c| (c.isfloating, c.geo))
        .unwrap_or((false, Rect::default()));

    // Promote a tiled window to floating once the user drags it far enough.
    //
    // The critical constraint: we must issue exactly ONE configure_window for
    // `win` during this promotion so the compositor never sees an intermediate
    // position.  toggle_floating() is wrong here because it:
    //   a) resizes to float_geo  (configure #1  → compositor paints "right" pos)
    //   b) calls arrange()       (flushes for other windows)
    //   c) caller then resizes to drag position  (configure #2  → "jumps left")
    //
    // set_floating_in_place() only flips isfloating + restores border width
    // without issuing any configure_window, leaving the single resize below
    // as the only geometry change the compositor ever sees.
    if !is_floating
        && has_tiling
        && ((new_x - drag_geo.x).abs() > snap || (new_y - drag_geo.y).abs() > snap)
    {
        // Resolve float dimensions before touching state.
        // If the window was never floating, float_geo will be zeroed; fall
        // back to the current tiled dimensions so the window doesn't collapse.
        let (float_w, float_h) = {
            ctx.g
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
        arrange(ctx, Some(ctx.g.selmon_id()));

        // The window's width is changing (tiled → floating), so the old
        // `grab_start_x`-based `new_x` would leave the window at x≈0 while the cursor
        // is far to the right.  Re-center the floating window under the
        // cursor and rebase the drag anchors so subsequent motion events
        // track correctly from the new position.
        new_x = event_x - float_w / 2;
        state.grab_start_x = new_x;
        state.grab_start_y = new_y;
        state.start_x = event_x;
        state.start_y = event_y;

        is_floating = true;
        drag_geo = Rect {
            x: new_x,
            y: new_y,
            w: float_w,
            h: float_h,
        };
    }

    if !has_tiling || is_floating {
        if let Some(client) = ctx.g.clients.get(&win) {
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

/// Clears `bar_dragging` and redraws the bar unconditionally.
///
/// Called once the drag loop exits so that hover state is always cleaned up.
fn clear_bar_hover(ctx: &mut WmCtx) {
    ctx.g.bar_dragging = false;
    let selmon_id = ctx.g.selmon_id();
    if let Some(mon) = ctx.g.selmon_mut() {
        mon.gesture = Gesture::None;
    }
    draw_bar(ctx, selmon_id);
}

/// Handle a drop onto the bar: tile the window, optionally moving it to the
/// hovered tag first.
///
/// Mirrors the C `handle_bar_drop`:
/// * Dropped on a tag button → `set_tiled()` + `tag()`
/// * Dropped elsewhere on bar, window floating → `set_tiled()`
///
/// # `grab_start_x`
///
/// The window's x position at the moment the drag started.  When the window
/// was floating, this is the true pre-drag origin; we save it into
/// `float_geo.x` so that un-tiling later restores the window to where it was
/// before the user dragged it onto the bar.
fn handle_bar_drop(
    ctx: &mut WmCtx,
    win: WindowId,
    grab_start_x: i32,
    pointer_override: Option<(i32, i32)>,
) {
    let Some((ptr_x, ptr_y)) = pointer_override.or_else(|| get_root_ptr(ctx)) else {
        return;
    };
    if !point_is_on_bar(ctx, ptr_x, ptr_y) {
        return;
    }

    let position = {
        ctx.g
            .selmon()
            .map(|mon| {
                let local_x = ptr_x - mon.work_rect.x;
                bar_position_at_x(mon, ctx, local_x)
            })
            .unwrap_or(BarPosition::Root)
    };

    // Remember whether the window was floating *before* any state change so
    // we know whether to correct float_geo afterwards.
    let was_floating = ctx
        .g
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
        set_client_tag(
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

    // ── Correct float_geo using the pre-drag position ─────────────────────
    //
    // set_tiled saved `client.geo` (the drag position near
    // the bar) into `float_geo`.  That's wrong — we want the position the
    // window occupied *before* the drag started:
    //   x = grab_start_x  (original window x at grab time)
    //   y = just below the bar
    if was_floating {
        let bar_bottom = {
            ctx.g
                .selmon()
                .map(|m| m.by + ctx.g.cfg.bar_height)
                .unwrap_or(0)
        };
        if let Some(client) = ctx.g.clients.get_mut(&win) {
            client.float_geo.x = grab_start_x;
            client.float_geo.y = bar_bottom;
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
fn apply_edge_drop(ctx: &mut WmCtx, win: WindowId, edge: Option<SnapPosition>, root_y: i32) -> bool {
    let edge = match edge {
        Some(e) => e,
        None => return false,
    };

    let at_left = edge == SnapPosition::Left;
    let at_right = edge == SnapPosition::Right;

    if !at_left && !at_right {
        return false;
    }

    let is_tiling = ctx.g.selmon().map(|m| m.is_tiling_layout()).unwrap_or(true);

    if is_tiling {
        let (mon_my, mon_mh) = ctx
            .g
            .selmon()
            .map(|m| (m.monitor_rect.y, m.monitor_rect.h))
            .unwrap_or((0, 1));

        // Upper 2/3 of the monitor → move view; lower 1/3 → send window.
        if root_y < mon_my + (2 * mon_mh) / 3 {
            if at_left {
                move_client(ctx, Direction::Left);
            } else {
                move_client(ctx, Direction::Right);
            }
        } else if at_left {
            shift_tag_by(ctx, Direction::Left, 1);
        } else {
            shift_tag_by(ctx, Direction::Right, 1);
        }

        if let Some(c) = ctx.g.clients.get_mut(&win) {
            c.isfloating = false;
        }
        arrange(ctx, Some(ctx.g.selmon_id()));
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
    grab_start_x: i32,
    edge_hint: Option<SnapPosition>,
    pointer_override: Option<(i32, i32)>,
) {
    let pointer = pointer_override.or_else(|| get_root_ptr(ctx));
    let edge = edge_hint.or_else(|| pointer.and_then(|(x, y)| check_edge_snap(ctx, x, y)));
    let handled_edge = pointer
        .map(|(_x, y)| apply_edge_drop(ctx, win, edge, y))
        .unwrap_or(false);
    if !handled_edge {
        handle_bar_drop(ctx, win, grab_start_x, pointer);
        handle_client_monitor_switch(ctx, win);
    }
}

// ── move_mouse ────────────────────────────────────────────────────────────────

/// Interactively drag the focused window with the mouse.
///
/// Grab → event loop → release handling. See helpers above for each phase.
pub fn move_mouse(ctx: &mut WmCtx, btn: MouseButton) {
    require_x11!(ctx);
    let Some(win) = prepare_drag_target(ctx) else {
        return;
    };

    if !grab_pointer(ctx, 2) {
        return;
    }
    let Some((start_x, start_y)) = get_root_ptr(ctx) else {
        ungrab_ctx(ctx);
        return;
    };

    let (grab_start_x, grab_start_y) = ctx
        .g
        .clients
        .get(&win)
        .map(|c| (c.geo.x, c.geo.y))
        .unwrap_or((0, 0));

    let mut state = MoveState {
        start_x,
        start_y,
        grab_start_x,
        grab_start_y,
        cursor_on_bar: false,
        edge_snap_indicator: None,
    };
    let rate = refresh_rate(ctx);
    let mut last_time: u32 = 0;

    loop {
        let Some(event) = wait_event(ctx) else {
            break;
        };
        match &event {
            x11rb::protocol::Event::ButtonRelease(br) => {
                if br.detail == btn.as_u8() {
                    break;
                }
            }
            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= crate::constants::animation::MOUSE_EVENT_RATE {
                    continue;
                }
                last_time = m.time;
                on_motion(
                    ctx,
                    win,
                    m.event_x as i32,
                    m.event_y as i32,
                    m.root_x as i32,
                    m.root_y as i32,
                    &mut state,
                );
            }
            _ => {}
        }
    }

    ungrab_ctx(ctx);
    clear_bar_hover(ctx);

    complete_move_drop(ctx, win, state.grab_start_x, state.edge_snap_indicator, None);
}

// ── gesture_mouse ─────────────────────────────────────────────────────────────

/// Root-window vertical-swipe gesture recogniser.
///
/// Watches for large vertical pointer movements; each time the cursor travels
/// more than `monitor_height / 30` pixels [`crate::util::spawn`] is called.
pub fn gesture_mouse(ctx: &mut WmCtx, btn: MouseButton) {
    require_x11!(ctx);
    if !grab_pointer(ctx, 2) {
        return;
    }
    let Some((_, start_y)) = get_root_ptr(ctx) else {
        ungrab_ctx(ctx);
        return;
    };

    let mut last_y = start_y;
    let mut last_time: u32 = 0;

    loop {
        let Some(event) = wait_event(ctx) else {
            break;
        };
        match &event {
            x11rb::protocol::Event::ButtonRelease(br) => {
                if br.detail == btn.as_u8() {
                    break;
                }
            }
            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= crate::constants::animation::MOUSE_EVENT_RATE {
                    continue;
                }
                last_time = m.time;

                let threshold = ctx.g.selmon().map(|m| m.monitor_rect.h / 30).unwrap_or(0);
                if (last_y - m.event_y as i32).abs() > threshold {
                    let event_y = m.event_y as i32;
                    let cmd = if event_y < last_y {
                        Cmd::UpVol
                    } else {
                        Cmd::DownVol
                    };
                    crate::util::spawn(ctx, cmd);
                    last_y = event_y;
                }
            }
            _ => {}
        }
    }

    ungrab_ctx(ctx);
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
    let selmon_id = ctx.g.selmon_id();
    let mon_mx = ctx.g.selmon().map(|m| m.work_rect.x).unwrap_or(0);

    let initial_tag = match bar_pos {
        BarPosition::Tag(idx) => 1u32 << (idx as u32),
        _ => {
            let ptr_x = get_root_ptr(ctx).map(|(x, _)| x).unwrap_or(0);
            ctx.g
                .monitors
                .get(selmon_id)
                .and_then(|mon| {
                    let local_x = ptr_x - mon.work_rect.x;
                    match bar_position_at_x(mon, ctx, local_x) {
                        BarPosition::Tag(idx) => Some(1u32 << (idx as u32)),
                        _ => None,
                    }
                })
                .unwrap_or(0)
        }
    };

    let current_tagset = ctx.g.selmon().map(|m| m.tagset[m.seltags as usize]);
    let is_current_tag = (initial_tag & ctx.g.tags.mask()) == current_tagset.unwrap_or(0);
    let has_sel = ctx.g.selmon().and_then(|m| m.sel).is_some();

    // Click on a *different* tag → switch view, no drag.
    if !is_current_tag && initial_tag != 0 {
        view(ctx, TagMask::from_bits(initial_tag));
        return false;
    }
    // No selected window → nothing to drag.
    if !has_sel {
        return false;
    }

    // Initialise the drag state machine.
    ctx.g.tag_drag = crate::globals::TagDragState {
        active: true,
        initial_tag,
        mon_id: selmon_id,
        mon_mx,
        last_tag: -1,
        cursor_on_bar: true,
        last_motion: None,
        button: btn,
    };
    set_cursor_move(ctx);
    ctx.g.bar_dragging = true;
    draw_bar(ctx, selmon_id);
    true
}

/// Process a single motion event during an active tag drag.
///
/// Updates gesture highlighting and detects when the cursor leaves the bar.
/// Returns `false` if the cursor left the bar (caller should finish the drag).
pub fn drag_tag_motion(ctx: &mut WmCtx, root_x: i32, root_y: i32) -> bool {
    if !ctx.g.tag_drag.active {
        return false;
    }

    let selmon_id = ctx.g.tag_drag.mon_id;
    let mon_mx = ctx.g.tag_drag.mon_mx;

    let bar_bottom = ctx
        .g
        .selmon()
        .map(|m| m.by + ctx.g.cfg.bar_height + 1)
        .unwrap_or(9999);

    if root_y > bar_bottom {
        ctx.g.tag_drag.cursor_on_bar = false;
        return false;
    }

    // Store last motion for release handling.  Modifier state is not available
    // from root coords alone; the caller sets it via drag_tag_finish.
    ctx.g.tag_drag.last_motion = Some((root_x, root_y, 0));

    let local_x = root_x - mon_mx;
    let new_gesture = ctx
        .g
        .monitors
        .get(selmon_id)
        .map(|mon| bar_position_to_gesture(bar_position_at_x(mon, ctx, local_x)))
        .unwrap_or(Gesture::None);
    let gesture_key = match new_gesture {
        Gesture::Tag(idx) => idx as i32,
        _ => -1,
    };

    if ctx.g.tag_drag.last_tag != gesture_key {
        ctx.g.tag_drag.last_tag = gesture_key;
        if let Some(mon) = ctx.g.monitors.get_mut(selmon_id) {
            mon.gesture = new_gesture;
        }
        draw_bar(ctx, selmon_id);
    }
    true
}

/// Finish a tag drag: apply the action based on the final position and
/// modifier keys held at release time.
///
/// `modifier_state` is the X11-style modifier bitmask at release time
/// (Shift, Control, …).
pub fn drag_tag_finish(ctx: &mut WmCtx, modifier_state: u32) {
    if !ctx.g.tag_drag.active {
        return;
    }

    let selmon_id = ctx.g.tag_drag.mon_id;
    let cursor_on_bar = ctx.g.tag_drag.cursor_on_bar;
    let last_motion = ctx.g.tag_drag.last_motion;

    // Clear state first so re-entrant calls are safe.
    ctx.g.tag_drag.active = false;

    if cursor_on_bar {
        if let Some((x, _, _)) = last_motion {
            let position = ctx
                .g
                .selmon()
                .map(|mon| {
                    let local_x = x - mon.work_rect.x;
                    bar_position_at_x(mon, ctx, local_x)
                })
                .unwrap_or(BarPosition::Root);

            if let BarPosition::Tag(tag_idx) = position {
                let tag_mask = TagMask::single(tag_idx + 1).unwrap_or(TagMask::EMPTY);
                if (modifier_state & ModMask::SHIFT.bits() as u32) != 0 {
                    if let Some(win) = ctx.g.monitor(selmon_id).and_then(|m| m.sel) {
                        set_client_tag(ctx, win, tag_mask);
                    }
                } else if (modifier_state & ModMask::CONTROL.bits() as u32) != 0 {
                    tag_all(ctx, tag_mask);
                } else if let Some(win) = ctx.g.monitor(selmon_id).and_then(|m| m.sel) {
                    follow_tag(ctx, win, tag_mask);
                }
            }
        }
    }

    ctx.g.bar_dragging = false;
    if let Some(mon) = ctx.g.monitor_mut(selmon_id) {
        mon.gesture = Gesture::None;
    }
    set_cursor_default(ctx);
    draw_bar(ctx, selmon_id);
}

/// Drag across the tag bar to switch the view or move/follow a window to a tag.
///
/// * Plain click on a different tag   → [`view`]
/// * Plain click on the current tag   → drag; release with `Shift` → [`set_client_tag`],
///   `Control` → [`tag_all`], no modifier → [`follow_tag`]
///
/// Exits without action if the pointer leaves the bar during the drag.
///
/// On X11, runs a synchronous grab loop.  On Wayland, starts the drag and
/// returns immediately — the calloop drives [`drag_tag_motion`] and
/// [`drag_tag_finish`].
pub fn drag_tag(ctx: &mut WmCtx, bar_pos: BarPosition, btn: MouseButton, _click_root_x: i32) {
    if !drag_tag_begin(ctx, bar_pos, btn) {
        return;
    }

    // On Wayland the calloop drives motion/finish asynchronously.
    require_x11!(ctx);

    // ── X11 synchronous grab loop ─────────────────────────────────────────
    if !grab_pointer(ctx, 2) {
        drag_tag_finish(ctx, 0);
        return;
    }

    let mut last_time: u32 = 0;

    loop {
        let Some(event) = wait_event(ctx) else {
            break;
        };
        match &event {
            x11rb::protocol::Event::ButtonRelease(br) => {
                if br.detail == btn.as_u8() {
                    // Capture modifier state at release and finish.
                    let modifier_state = u16::from(br.state) as u32;
                    ungrab_ctx(ctx);
                    drag_tag_finish(ctx, modifier_state);
                    return;
                }
            }
            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= crate::constants::animation::MOUSE_EVENT_RATE {
                    continue;
                }
                last_time = m.time;

                // Update stored modifier state from latest motion.
                let root_x = m.event_x as i32;
                let root_y = m.event_y as i32;
                let mod_state = u16::from(m.state) as u32;

                // Store motion with modifier state for release handling.
                ctx.g.tag_drag.last_motion = Some((root_x, root_y, mod_state));

                if !drag_tag_motion(ctx, root_x, root_y) {
                    // Cursor left the bar — abort.
                    break;
                }
            }
            _ => {}
        }
    }

    ungrab_ctx(ctx);
    drag_tag_finish(ctx, 0);
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
    right_click: bool,
    click_root_x: i32,
    click_root_y: i32,
) -> bool {
    if right_click {
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

    let sel = ctx.g.selected_win();
    let (win_start_x, win_start_y) = ctx
        .g
        .clients
        .get(&win)
        .map(|c| (c.geo.x, c.geo.y))
        .unwrap_or((0, 0));
    let (win_start_w, win_start_h) = ctx
        .g
        .clients
        .get(&win)
        .map(|c| (c.geo.w, c.geo.h))
        .unwrap_or((0, 0));
    ctx.g.title_drag = crate::globals::TitleDragState {
        active: true,
        win,
        button: btn,
        right_click,
        was_focused: sel == Some(win),
        was_hidden: crate::client::is_hidden(win),
        start_x: click_root_x,
        start_y: click_root_y,
        win_start_x,
        win_start_y,
        win_start_w,
        win_start_h,
        last_root_x: click_root_x,
        last_root_y: click_root_y,
        dragging: false,
    };
    true
}

/// Process a pointer motion event during an active title drag.
///
/// Returns `true` if the drag threshold was exceeded and the drag action
/// (move/resize) was initiated — the caller should consider the interaction
/// consumed.
pub fn title_drag_motion(ctx: &mut WmCtx, root_x: i32, root_y: i32) -> bool {
    if !ctx.g.title_drag.active {
        return false;
    }
    ctx.g.title_drag.last_root_x = root_x;
    ctx.g.title_drag.last_root_y = root_y;

    if ctx.g.title_drag.dragging {
        if ctx.backend_kind() != BackendKind::Wayland {
            return false;
        }
        let td = &ctx.g.title_drag;
        let win = td.win;
        if td.right_click {
            let (new_w, new_h, x, y, is_floating) = ctx
                .g
                .clients
                .get(&win)
                .map(|c| {
                    (
                        (td.win_start_w + (root_x - td.start_x)).max(1),
                        (td.win_start_h + (root_y - td.start_y)).max(1),
                        c.geo.x,
                        c.geo.y,
                        c.isfloating,
                    )
                })
                .unwrap_or((1, 1, 0, 0, false));
            if is_floating {
                resize(
                    ctx,
                    win,
                    &Rect {
                        x,
                        y,
                        w: new_w,
                        h: new_h,
                    },
                    true,
                );
            }
            return true;
        }
        let mut new_x = td.win_start_x + (root_x - td.start_x);
        let mut new_y = td.win_start_y + (root_y - td.start_y);

        let (is_floating, geo) = ctx
            .g
            .clients
            .get(&win)
            .map(|c| (c.isfloating, c.geo))
            .unwrap_or((false, Rect::default()));
        if is_floating {
            if let Some(c) = ctx.g.clients.get(&win) {
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
            if let Some(client) = ctx.g.clients.get_mut(&win) {
                client.float_geo.x = new_x;
                client.float_geo.y = new_y;
            }
        }
        return true;
    }

    let td = &ctx.g.title_drag;
    if (root_x - td.start_x).abs() <= DRAG_THRESHOLD
        && (root_y - td.start_y).abs() <= DRAG_THRESHOLD
    {
        return false;
    }

    // Threshold exceeded — start the drag action.
    let win = ctx.g.title_drag.win;
    let btn = ctx.g.title_drag.button;
    let right_click = ctx.g.title_drag.right_click;
    let was_hidden = ctx.g.title_drag.was_hidden;
    if ctx.backend_kind() == BackendKind::Wayland {
        // Keep the title drag active so Wayland motion/release can keep driving it.
        if was_hidden {
            crate::client::show(ctx, win);
        }
        crate::focus::focus_soft(ctx, Some(win));
        if let Some((is_floating, geo, border_width)) = ctx
            .g
            .clients
            .get(&win)
            .map(|c| (c.isfloating, c.geo, c.border_width))
        {
            if !is_floating {
                set_floating_in_place(ctx, win);
                arrange(ctx, Some(ctx.g.selmon_id()));
            }
            if right_click {
                let (x_off, y_off) =
                    ResizeDirection::BottomRight.warp_offset(geo.w, geo.h, border_width);
                ctx.g.title_drag.start_x = geo.x + x_off;
                ctx.g.title_drag.start_y = geo.y + y_off;
            } else {
                // Wayland can't reliably warp the hardware pointer like X11, so
                // emulate warp_into by rebasing the drag anchor into window bounds.
                let pad = 10;
                let max_x = (geo.w - pad).max(pad);
                let max_y = (geo.h - pad).max(pad);
                let offset_x = (root_x - geo.x).clamp(pad, max_x);
                let offset_y = (root_y - geo.y).clamp(pad, max_y);
                ctx.g.title_drag.start_x = geo.x + offset_x;
                ctx.g.title_drag.start_y = geo.y + offset_y;
            }
        }
        if right_click {
            set_cursor_resize(ctx, Some(ResizeDirection::BottomRight));
        } else {
            set_cursor_move(ctx);
        }
        ctx.g.title_drag.dragging = true;
        return title_drag_motion(ctx, root_x, root_y);
    }

    ctx.g.title_drag.dragging = true;
    ctx.g.title_drag.active = false;

    if was_hidden {
        crate::client::show(ctx, win);
    }
    crate::focus::focus_soft(ctx, Some(win));

    if right_click {
        if let Some(c) = ctx.g.clients.get(&win) {
            let (x_off, y_off) =
                ResizeDirection::BottomRight.warp_offset(c.geo.w, c.geo.h, c.border_width);
            if let Some(conn) = ctx.x11_conn().map(|x11| x11.conn) {
                let x11_win: Window = win.into();
                let _ = conn.warp_pointer(
                    x11rb::NONE,
                    x11_win,
                    0i16,
                    0i16,
                    0u16,
                    0u16,
                    x_off as i16,
                    y_off as i16,
                );
                let _ = conn.flush();
            }
        }
        super::resize::resize_mouse_directional(ctx, Some(ResizeDirection::BottomRight), btn);
    } else {
        warp_into(ctx, win);
        move_mouse(ctx, btn);
    }
    true
}

/// Finish a title drag interaction (button release without exceeding the
/// drag threshold).  Performs the click action.
pub fn title_drag_finish(ctx: &mut WmCtx) {
    if !ctx.g.title_drag.active {
        return;
    }

    if ctx.g.title_drag.dragging {
        let win = ctx.g.title_drag.win;
        let right_click = ctx.g.title_drag.right_click;
        let grab_start_x = ctx.g.title_drag.win_start_x;
        let last = (ctx.g.title_drag.last_root_x, ctx.g.title_drag.last_root_y);
        ctx.g.title_drag.active = false;
        ctx.g.title_drag.dragging = false;
        set_cursor_default(ctx);
        if !right_click {
            complete_move_drop(ctx, win, grab_start_x, None, Some(last));
        } else {
            handle_client_monitor_switch(ctx, win);
        }
        return;
    }

    let win = ctx.g.title_drag.win;
    let right_click = ctx.g.title_drag.right_click;
    let was_focused = ctx.g.title_drag.was_focused;
    let was_hidden = ctx.g.title_drag.was_hidden;

    ctx.g.title_drag.active = false;

    if right_click {
        if was_hidden {
            crate::client::show(ctx, win);
            crate::focus::focus_soft(ctx, Some(win));
        }
        crate::client::zoom(ctx);
    } else if was_hidden {
        crate::client::show(ctx, win);
        crate::focus::focus_soft(ctx, Some(win));
        restack(ctx, ctx.g.selmon_id());
    } else if was_focused {
        crate::client::hide(ctx, win);
    } else {
        crate::focus::focus_soft(ctx, Some(win));
        restack(ctx, ctx.g.selmon_id());
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
    ctx: &mut WmCtx,
    win: WindowId,
    btn: MouseButton,
    click_root_x: i32,
    click_root_y: i32,
) {
    if !title_drag_begin(ctx, win, btn, false, click_root_x, click_root_y) {
        return;
    }

    // On Wayland the calloop drives motion/finish asynchronously.
    require_x11!(ctx);

    // ── X11 synchronous grab loop ─────────────────────────────────────
    if !grab_pointer(ctx, 0) {
        title_drag_finish(ctx);
        return;
    }

    let mut last_time: u32 = 0;

    loop {
        let Some(event) = wait_event(ctx) else {
            break;
        };
        match &event {
            x11rb::protocol::Event::ButtonRelease(br) => {
                if br.detail == btn.as_u8() {
                    break;
                }
            }
            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= crate::constants::animation::MOUSE_EVENT_RATE {
                    continue;
                }
                last_time = m.time;
                if title_drag_motion(ctx, m.event_x as i32, m.event_y as i32) {
                    return;
                }
            }
            _ => {}
        }
    }

    ungrab_ctx(ctx);
    title_drag_finish(ctx);
}

// ── window_title_mouse_handler_right ─────────────────────────────────────────

/// Right-click / drag handler for a window title bar entry.
///
/// Click: show+focus if hidden, otherwise zoom to master.
/// Drag > [`DRAG_THRESHOLD`]: show+focus if hidden, resize from bottom-right.
/// No-op when the window is in true fullscreen.
///
/// On Wayland and X11, this shares the same title-drag state machine.
pub fn window_title_mouse_handler_right(
    ctx: &mut WmCtx,
    win: WindowId,
    btn: MouseButton,
    click_root_x: i32,
    click_root_y: i32,
) {
    if !title_drag_begin(ctx, win, btn, true, click_root_x, click_root_y) {
        return;
    }

    // On Wayland the calloop drives motion/finish asynchronously.
    require_x11!(ctx);

    // ── X11 synchronous grab loop ─────────────────────────────────────
    if !grab_pointer(ctx, 2) {
        title_drag_finish(ctx);
        return;
    }

    let mut last_time: u32 = 0;

    loop {
        let Some(event) = wait_event(ctx) else {
            break;
        };
        match &event {
            x11rb::protocol::Event::ButtonRelease(br) => {
                if br.detail == btn.as_u8() {
                    break;
                }
            }
            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= crate::constants::animation::MOUSE_EVENT_RATE {
                    continue;
                }
                last_time = m.time;
                if title_drag_motion(ctx, m.event_x as i32, m.event_y as i32) {
                    return;
                }
            }
            _ => {}
        }
    }

    ungrab_ctx(ctx);
    title_drag_finish(ctx);
}
