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

use crate::bar::bar_position_at_x;
use crate::bar::bar_position_to_gesture;
use crate::bar::draw_bar;
use crate::client::resize;
use crate::config::commands::Cmd;
use crate::contexts::WmCtx;
use crate::floating::{change_snap, reset_snap, set_floating_in_place, set_tiled, SnapDir};
use crate::focus::{focus, warp_into};
use crate::layouts::{arrange, restack};
use crate::tags::{follow_tag, move_client, set_client_tag, shift_tag_by, tag_all, view};
use crate::types::SnapPosition;
use crate::types::*;
use x11rb::protocol::xproto::*;

use super::constants::{
    DRAG_THRESHOLD, MAX_UNMAXIMIZE_OFFSET, OVERLAY_ZONE_WIDTH, REFRESH_RATE_HI, REFRESH_RATE_LO,
};
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
fn prepare_drag_target(ctx: &mut WmCtx) -> Option<Window> {
    let sel_win = {
        let mon = ctx.g.selmon()?;
        let sel = mon.sel?;
        let c = ctx.g.clients.get(&sel)?;

        if c.is_fullscreen && !c.isfakefullscreen {
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
                let local_x = ptr_x - mon.monitor_rect.x;
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
    win: Window,
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
fn handle_bar_drop(ctx: &mut WmCtx, win: Window, grab_start_x: i32) {
    let Some((ptr_x, ptr_y)) = get_root_ptr(ctx) else {
        return;
    };
    if !point_is_on_bar(ctx, ptr_x, ptr_y) {
        return;
    }

    let position = {
        ctx.g
            .selmon()
            .map(|mon| {
                let local_x = ptr_x - mon.monitor_rect.x;
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
fn apply_edge_drop(ctx: &mut WmCtx, win: Window, edge: Option<SnapPosition>) -> bool {
    let edge = match edge {
        Some(e) => e,
        None => return false,
    };

    let Some((_root_x, root_y)) = get_root_ptr(ctx) else {
        return false;
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

// ── move_mouse ────────────────────────────────────────────────────────────────

/// Interactively drag the focused window with the mouse.
///
/// Grab → event loop → release handling. See helpers above for each phase.
pub fn move_mouse(ctx: &mut WmCtx, btn: MouseButton) {
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
                if m.time - last_time <= 1000 / rate {
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

    if !apply_edge_drop(ctx, win, state.edge_snap_indicator) {
        handle_bar_drop(ctx, win, state.grab_start_x);
        handle_client_monitor_switch(ctx, win);
    }
}

// ── gesture_mouse ─────────────────────────────────────────────────────────────

/// Root-window vertical-swipe gesture recogniser.
///
/// Watches for large vertical pointer movements; each time the cursor travels
/// more than `monitor_height / 30` pixels [`crate::util::spawn`] is called.
pub fn gesture_mouse(ctx: &mut WmCtx, btn: MouseButton) {
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
                if m.time - last_time <= 1000 / REFRESH_RATE_LO {
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

/// Drag across the tag bar to switch the view or move/follow a window to a tag.
///
/// * Plain click on a different tag   → [`view`]
/// * Plain click on the current tag   → drag; release with `Shift` → [`set_client_tag`],
///   `Control` → [`tag_all`], no modifier → [`follow_tag`]
///
/// Exits without action if the pointer leaves the bar during the drag.
pub fn drag_tag(ctx: &mut WmCtx, bar_pos: BarPosition, btn: MouseButton, _click_root_x: i32) {
    let (initial_tag, is_current_tag, has_sel, selmon_id, mon_mx) = {
        let selmon_id = ctx.g.selmon_id();
        let mon_mx = ctx.g.selmon().map(|m| m.monitor_rect.x).unwrap_or(0);

        let initial_tag = match bar_pos {
            BarPosition::Tag(idx) => 1u32 << (idx as u32),
            _ => {
                let Some((ptr_x, _)) = get_root_ptr(ctx) else {
                    return;
                };
                ctx.g
                    .monitors
                    .get(selmon_id)
                    .and_then(|mon| {
                        let local_x = ptr_x - mon.monitor_rect.x;
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
        (initial_tag, is_current_tag, has_sel, selmon_id, mon_mx)
    };

    if !is_current_tag && initial_tag != 0 {
        view(ctx, TagMask::from_bits(initial_tag));
        return;
    }
    if !has_sel {
        return;
    }

    if !grab_pointer(ctx, 2) {
        return;
    }

    {
        ctx.g.bar_dragging = true;
        draw_bar(ctx, selmon_id);
    }

    let mut cursor_on_bar = true;
    let mut last_tag: i32 = -1;
    let mut last_time: u32 = 0;
    let mut last_motion: Option<(i32, i32, u16)> = None;

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
                if m.time - last_time <= 1000 / REFRESH_RATE_LO {
                    continue;
                }
                last_time = m.time;

                last_motion = Some((m.event_x as i32, m.event_y as i32, u16::from(m.state)));

                let bar_bottom = {
                    ctx.g
                        .selmon()
                        .map(|m| m.by + ctx.g.cfg.bar_height + 1)
                        .unwrap_or(9999)
                };
                if m.event_y as i32 > bar_bottom {
                    cursor_on_bar = false;
                    break;
                }

                let local_x = m.event_x as i32 - mon_mx;
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

                if last_tag != gesture_key {
                    last_tag = gesture_key;
                    if let Some(mon) = ctx.g.monitors.get_mut(selmon_id) {
                        mon.gesture = new_gesture;
                    }
                    draw_bar(ctx, selmon_id);
                }
            }
            _ => {}
        }
    }

    ungrab_ctx(ctx);

    if cursor_on_bar {
        if let Some((x, _, state)) = last_motion {
            let position = {
                ctx.g
                    .selmon()
                    .map(|mon| {
                        let local_x = x - mon.monitor_rect.x;
                        bar_position_at_x(mon, ctx, local_x)
                    })
                    .unwrap_or(BarPosition::Root)
            };

            if let BarPosition::Tag(tag_idx) = position {
                let tag_mask = TagMask::single(tag_idx + 1).unwrap_or(TagMask::EMPTY);
                let state = state as u32;
                if (state & ModMask::SHIFT.bits() as u32) != 0 {
                    if let Some(win) = ctx.g.monitor(selmon_id).and_then(|m| m.sel) {
                        set_client_tag(ctx, win, tag_mask);
                    }
                } else if (state & ModMask::CONTROL.bits() as u32) != 0 {
                    tag_all(ctx, tag_mask);
                } else if let Some(win) = ctx.g.monitor(selmon_id).and_then(|m| m.sel) {
                    follow_tag(ctx, win, tag_mask);
                }
            }
        }
    }

    {
        ctx.g.bar_dragging = false;
        if let Some(mon) = ctx.g.monitor_mut(selmon_id) {
            mon.gesture = Gesture::None;
        }
        draw_bar(ctx, selmon_id);
    }
}

// ── window_title_mouse_handler ────────────────────────────────────────────────

/// Left-click / drag handler for a window title bar entry.
///
/// Click: hidden → show+focus; focused → hide; otherwise → focus.
/// Drag > [`DRAG_THRESHOLD`]: show, focus, warp, hand off to [`move_mouse`].
pub fn window_title_mouse_handler(
    ctx: &mut WmCtx,
    win: Window,
    btn: MouseButton,
    click_root_x: i32,
    click_root_y: i32,
) {
    let sel = ctx.g.selected_win();
    let was_focused = sel == Some(win);
    let was_hidden = crate::client::is_hidden(win);

    if !grab_pointer(ctx, 0) {
        return;
    }
    let start_x = click_root_x;
    let start_y = click_root_y;

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
                if m.time - last_time <= 1000 / REFRESH_RATE_LO {
                    continue;
                }
                last_time = m.time;
                if (m.event_x as i32 - start_x).abs() > DRAG_THRESHOLD
                    || (m.event_y as i32 - start_y).abs() > DRAG_THRESHOLD
                {
                    ungrab_ctx(ctx);
                    if was_hidden {
                        crate::client::show(ctx, win);
                    }
                    focus(ctx, Some(win));
                    warp_into(ctx, win);
                    move_mouse(ctx, btn);
                    return;
                }
            }
            _ => {}
        }
    }

    ungrab_ctx(ctx);
    if was_hidden {
        crate::client::show(ctx, win);
        focus(ctx, Some(win));
        restack(ctx, ctx.g.selmon_id());
    } else if was_focused {
        crate::client::hide(ctx, win);
    } else {
        focus(ctx, Some(win));
        restack(ctx, ctx.g.selmon_id());
    }
}

// ── window_title_mouse_handler_right ─────────────────────────────────────────

/// Right-click / drag handler for a window title bar entry.
///
/// Click: show+focus if hidden, otherwise zoom to master.
/// Drag > [`DRAG_THRESHOLD`]: show+focus if hidden, hand off to resize.
/// No-op when the window is in true fullscreen.
pub fn window_title_mouse_handler_right(
    ctx: &mut WmCtx,
    win: Window,
    btn: MouseButton,
    click_root_x: i32,
    click_root_y: i32,
) {
    {
        if ctx
            .g
            .clients
            .get(&win)
            .map(|c| c.is_fullscreen && !c.isfakefullscreen)
            .unwrap_or(false)
        {
            return;
        }
    }

    focus(ctx, Some(win));

    if !grab_pointer(ctx, 2) {
        return;
    }
    let start_x = click_root_x;
    let start_y = click_root_y;

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
                if m.time - last_time <= 1000 / REFRESH_RATE_LO {
                    continue;
                }
                last_time = m.time;
                if (m.event_x as i32 - start_x).abs() > DRAG_THRESHOLD
                    || (m.event_y as i32 - start_y).abs() > DRAG_THRESHOLD
                {
                    ungrab_ctx(ctx);
                    if crate::client::is_hidden(win) {
                        crate::client::show(ctx, win);
                        focus(ctx, Some(win));
                    }
                    super::resize::resize_mouse_from_cursor(ctx, btn);
                    return;
                }
            }
            _ => {}
        }
    }

    // Only reached on ButtonRelease (click, not drag).
    ungrab_ctx(ctx);
    if crate::client::is_hidden(win) {
        crate::client::show(ctx, win);
        focus(ctx, Some(win));
    }
    crate::client::zoom(ctx);
}
