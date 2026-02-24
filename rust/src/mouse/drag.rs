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
//! ungrab(conn)
//! post-loop cleanup (bar drop, monitor switch, bar redraw, …)
//! ```

use crate::bar::bar_position_at_x;
use crate::bar::draw_bar;
use crate::bar::BarPosition;
use crate::client::resize;
use crate::config::commands::Cmd;
use crate::floating::{change_snap, reset_snap, set_floating_in_place, set_tiled, SnapDir};
use crate::focus::{focus, warp_into};
use crate::globals::{get_globals, get_globals_mut};
use crate::layouts::{arrange, restack};
use crate::monitor::is_current_layout_tiling;
use crate::tags::{
    follow_tag, move_left, move_right, set_client_tag, tag_all, tag_to_left, tag_to_right, view,
};
use crate::types::SnapPosition;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use super::constants::{
    DRAG_THRESHOLD, MAX_UNMAXIMIZE_OFFSET, OVERLAY_ZONE_WIDTH, REFRESH_RATE_HI, REFRESH_RATE_LO,
};
use super::grab::{grab_pointer, ungrab};
use super::monitor::handle_client_monitor_switch;
use super::warp::get_root_ptr;

// ── Shared helpers ────────────────────────────────────────────────────────────

fn refresh_rate() -> u32 {
    if get_globals().doubledraw {
        REFRESH_RATE_HI
    } else {
        REFRESH_RATE_LO
    }
}

/// Snap `new_x`/`new_y` to the work-area edges of `selmon` when within `globals.snap` pixels.
fn snap_to_monitor_edges(c: &Client, new_x: &mut i32, new_y: &mut i32) {
    let globals = get_globals();
    let snap = globals.snap;
    let Some(mon) = globals.monitors.get(globals.selmon) else {
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
fn check_edge_snap(x: i32, y: i32) -> Option<SnapPosition> {
    let globals = get_globals();
    let Some(mon) = globals.monitors.get(globals.selmon) else {
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
    if y <= mon.monitor_rect.y + if mon.showbar { globals.bh } else { 5 } {
        return Some(SnapPosition::Top);
    }
    None
}

/// Returns `true` when `(x, y)` (root-space) is inside the bar of `selmon`.
fn point_is_on_bar(x: i32, y: i32) -> bool {
    let globals = get_globals();
    let Some(mon) = globals.monitors.get(globals.selmon) else {
        return false;
    };
    mon.showbar
        && y >= mon.by
        && y < mon.by + globals.bh
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
fn prepare_drag_target() -> Option<Window> {
    let sel_win = {
        let globals = get_globals();
        let mon = globals.monitors.get(globals.selmon)?;
        let sel = mon.sel?;
        let c = globals.clients.get(&sel)?;

        if c.is_fullscreen && !c.isfakefullscreen {
            return None;
        }
        if Some(sel) == mon.overlay {
            return None;
        }
        if Some(sel) == mon.fullscreen {
            crate::floating::temp_fullscreen();
            return None;
        }
        sel
    };

    // Un-snap: surface the real window first; the user re-drags after.
    let is_snapped = get_globals()
        .clients
        .get(&sel_win)
        .map(|c| c.snapstatus != SnapPosition::None)
        .unwrap_or(false);
    if is_snapped {
        reset_snap(sel_win);
        return None;
    }

    // In a floating layout, if the window fills (nearly) the whole monitor,
    // restore the saved float geometry so we drag the real size, not a maximized one.
    let restore_geo: Option<Rect> = {
        let globals = get_globals();
        let has_tiling = globals
            .monitors
            .get(globals.selmon)
            .map(|m| is_current_layout_tiling(m, &globals.tags))
            .unwrap_or(true);

        if !has_tiling {
            if let (Some(c), Some(mon)) = (
                globals.clients.get(&sel_win),
                globals.monitors.get(globals.selmon),
            ) {
                let bh = globals.bh;
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
        resize(sel_win, &geo, false);
    }

    Some(sel_win)
}

/// Update `bar_dragging` and the gesture (tag hover highlight) while dragging.
///
/// Tracks enter/leave transitions via `state.cursor_on_bar` so the bar is only
/// redrawn when something changes.  Returns `true` while the cursor is on the bar.
fn update_bar_hover(ptr_x: i32, ptr_y: i32, state: &mut MoveState) -> bool {
    let on_bar = point_is_on_bar(ptr_x, ptr_y);

    if on_bar {
        // Use the canonical bar hit-test so that tag hover highlighting during
        // a window-drag uses exactly the same geometry as click dispatch and
        // motion_notify gesture detection.
        let (selmon_id, new_gesture) = {
            let globals = get_globals();
            let selmon_id = globals.selmon;
            let new_gesture = globals
                .monitors
                .get(selmon_id)
                .map(|mon| {
                    let local_x = ptr_x - mon.monitor_rect.x;
                    bar_position_at_x(mon, globals, local_x).to_gesture()
                })
                .unwrap_or(Gesture::None);
            (selmon_id, new_gesture)
        };

        let gm = get_globals_mut();
        let gesture_changed = gm
            .monitors
            .get(selmon_id)
            .map(|m| m.gesture != new_gesture)
            .unwrap_or(false);

        if !state.cursor_on_bar || gesture_changed {
            gm.bar_dragging = true;
            if let Some(mon) = gm.monitors.get_mut(selmon_id) {
                mon.gesture = new_gesture;
                draw_bar(mon);
            }
        }
    } else if state.cursor_on_bar {
        let gm = get_globals_mut();
        gm.bar_dragging = false;
        let selmon_id = gm.selmon;
        if let Some(mon) = gm.monitors.get_mut(selmon_id) {
            mon.gesture = Gesture::None;
            draw_bar(mon);
        }
    }

    on_bar
}

/// Process a single throttled `MotionNotify` event during `move_mouse`.
fn on_motion(
    win: Window,
    event_x: i32,
    event_y: i32,
    root_x: i32,
    root_y: i32,
    state: &mut MoveState,
) {
    state.cursor_on_bar = update_bar_hover(root_x, root_y, state);
    state.edge_snap_indicator = check_edge_snap(root_x, root_y);

    let mut new_x = state.grab_start_x + (event_x - state.start_x);
    let mut new_y = state.grab_start_y + (event_y - state.start_y);

    // While hovering over the bar, keep the window just below it.
    if state.cursor_on_bar {
        let globals = get_globals();
        let bar_bottom = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.by + globals.bh)
            .unwrap_or(new_y);
        new_y = bar_bottom;
    }

    let (snap, has_tiling) = {
        let globals = get_globals();
        let snap = globals.snap;
        let has_tiling = globals
            .monitors
            .get(globals.selmon)
            .map(|m| is_current_layout_tiling(m, &globals.tags))
            .unwrap_or(true);
        (snap, has_tiling)
    };

    let (mut is_floating, mut drag_geo) = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| (c.isfloating, c.geo))
            .unwrap_or((false, Rect::default()))
    };

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
            let globals = get_globals();
            globals
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
        set_floating_in_place(win);

        // Re-tile the remaining windows (touches only the other clients).
        arrange(Some(get_globals().selmon));

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
        {
            let globals = get_globals();
            if let Some(client) = globals.clients.get(&win) {
                snap_to_monitor_edges(client, &mut new_x, &mut new_y);
            }
        }
        resize(
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
fn clear_bar_hover() {
    let gm = get_globals_mut();
    gm.bar_dragging = false;
    let selmon_id = gm.selmon;
    if let Some(mon) = gm.monitors.get_mut(selmon_id) {
        mon.gesture = Gesture::None;
        draw_bar(mon);
    }
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
fn handle_bar_drop(win: Window, grab_start_x: i32) {
    let Some((ptr_x, ptr_y)) = get_root_ptr() else {
        return;
    };
    if !point_is_on_bar(ptr_x, ptr_y) {
        return;
    }

    let position = {
        let globals = get_globals();
        let selmon_id = globals.selmon;
        globals
            .monitors
            .get(selmon_id)
            .map(|mon| {
                let local_x = ptr_x - mon.monitor_rect.x;
                bar_position_at_x(mon, globals, local_x)
            })
            .unwrap_or(BarPosition::Root)
    };

    // Remember whether the window was floating *before* any state change so
    // we know whether to correct float_geo afterwards.
    let was_floating = get_globals()
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
        set_tiled(win, false);
        set_client_tag(TagMask::single(tag_idx as usize + 1).unwrap_or(TagMask::EMPTY));
    } else if was_floating {
        // Dropped on the bar but not on a tag button: tile the window.
        // Use set_tiled(win, …) directly instead of toggle_floating() which
        // operates on mon.sel — a value that could theoretically diverge from
        // the window we actually dragged.
        set_tiled(win, true);
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
            let globals = get_globals();
            globals
                .monitors
                .get(globals.selmon)
                .map(|m| m.by + globals.bh)
                .unwrap_or(0)
        };
        if let Some(client) = get_globals_mut().clients.get_mut(&win) {
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
fn apply_edge_drop(win: Window, edge: Option<SnapPosition>) -> bool {
    let edge = match edge {
        Some(e) => e,
        None => return false,
    };

    let Some((_root_x, root_y)) = get_root_ptr() else {
        return false;
    };

    let at_left = edge == SnapPosition::Left;
    let at_right = edge == SnapPosition::Right;

    if !at_left && !at_right {
        return false;
    }

    let is_tiling = {
        let globals = get_globals();
        globals
            .monitors
            .get(globals.selmon)
            .map(|m| is_current_layout_tiling(m, &globals.tags))
            .unwrap_or(true)
    };

    if is_tiling {
        let (mon_my, mon_mh) = {
            let globals = get_globals();
            globals
                .monitors
                .get(globals.selmon)
                .map(|m| (m.monitor_rect.y, m.monitor_rect.h))
                .unwrap_or((0, 1))
        };

        // Upper 2/3 of the monitor → move view; lower 1/3 → send window.
        if root_y < mon_my + (2 * mon_mh) / 3 {
            if at_left {
                move_left();
            } else {
                move_right();
            }
        } else if at_left {
            tag_to_left();
        } else {
            tag_to_right();
        }

        {
            let globals = get_globals_mut();
            if let Some(c) = globals.clients.get_mut(&win) {
                c.isfloating = false;
            }
            let selmon_id = globals.selmon;
            arrange(Some(selmon_id));
        }
    } else {
        let dir = if at_left {
            SnapDir::Left
        } else {
            SnapDir::Right
        };
        change_snap(win, dir);
    }

    true
}

// ── move_mouse ────────────────────────────────────────────────────────────────

/// Interactively drag the focused window with the mouse.
///
/// Grab → event loop → release handling. See helpers above for each phase.
pub fn move_mouse() {
    let Some(win) = prepare_drag_target() else {
        return;
    };

    let Some(conn) = grab_pointer(2) else { return };
    let Some((start_x, start_y)) = get_root_ptr() else {
        ungrab(conn);
        return;
    };

    let (grab_start_x, grab_start_y) = get_globals()
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
    let rate = refresh_rate();
    let mut last_time: u32 = 0;

    loop {
        let Ok(event) = conn.wait_for_event() else {
            break;
        };
        match &event {
            x11rb::protocol::Event::ButtonRelease(_) => break,
            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= 1000 / rate {
                    continue;
                }
                last_time = m.time;
                on_motion(
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

    ungrab(conn);
    clear_bar_hover();

    if !apply_edge_drop(win, state.edge_snap_indicator) {
        handle_bar_drop(win, state.grab_start_x);
        handle_client_monitor_switch(win);
    }
}

// ── gesture_mouse ─────────────────────────────────────────────────────────────

/// Root-window vertical-swipe gesture recogniser.
///
/// Watches for large vertical pointer movements; each time the cursor travels
/// more than `monitor_height / 30` pixels [`crate::util::spawn`] is called.
pub fn gesture_mouse() {
    let Some(conn) = grab_pointer(2) else { return };
    let Some((_, start_y)) = get_root_ptr() else {
        ungrab(conn);
        return;
    };

    let mut last_y = start_y;
    let mut last_time: u32 = 0;

    loop {
        let Ok(event) = conn.wait_for_event() else {
            break;
        };
        match &event {
            x11rb::protocol::Event::ButtonRelease(_) => break,
            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= 1000 / REFRESH_RATE_LO {
                    continue;
                }
                last_time = m.time;

                let threshold = get_globals()
                    .monitors
                    .get(get_globals().selmon)
                    .map(|m| m.monitor_rect.h / 30)
                    .unwrap_or(0);
                if (last_y - m.event_y as i32).abs() > threshold {
                    let event_y = m.event_y as i32;
                    let cmd = if event_y < last_y {
                        Cmd::UpVol
                    } else {
                        Cmd::DownVol
                    };
                    crate::util::spawn(cmd);
                    last_y = event_y;
                }
            }
            _ => {}
        }
    }

    ungrab(conn);
}

// ── drag_tag ──────────────────────────────────────────────────────────────────

/// Drag across the tag bar to switch the view or move/follow a window to a tag.
///
/// * Plain click on the current tag → [`view`]
/// * Drag then release with `Shift`   → [`tag`]   (move to tag, stay on view)
/// * Drag then release with `Control` → [`tag_all`]
/// * Drag then release (no modifier)  → [`follow_tag`]
///
/// If the pointer leaves the bar during the drag the loop exits without action.
pub fn drag_tag() {
    let (initial_tag, is_current_tag, has_sel, selmon_id, mon_mx) = {
        let globals = get_globals();
        let selmon_id = globals.selmon;
        let mon_mx = globals
            .monitors
            .get(selmon_id)
            .map(|m| m.monitor_rect.x)
            .unwrap_or(0);

        let Some((ptr_x, _)) = get_root_ptr() else {
            return;
        };

        let initial_tag = globals
            .monitors
            .get(selmon_id)
            .and_then(|mon| {
                let local_x = ptr_x - mon.monitor_rect.x;
                match bar_position_at_x(mon, globals, local_x) {
                    BarPosition::Tag(idx) => Some(1u32 << (idx as u32)),
                    _ => None,
                }
            })
            .unwrap_or(0);

        let current_tagset = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.tagset[m.seltags as usize]);
        let is_current_tag = (initial_tag & globals.tags.mask()) == current_tagset.unwrap_or(0);
        let has_sel = globals
            .monitors
            .get(globals.selmon)
            .and_then(|m| m.sel)
            .is_some();
        (initial_tag, is_current_tag, has_sel, selmon_id, mon_mx)
    };

    if !is_current_tag && initial_tag != 0 {
        view(TagMask::from_bits(initial_tag));
        return;
    }
    if !has_sel {
        return;
    }

    let Some(conn) = grab_pointer(2) else { return };

    {
        let gm = get_globals_mut();
        gm.bar_dragging = true;
        if let Some(mon) = gm.monitors.get_mut(selmon_id) {
            draw_bar(mon);
        }
    }

    let mut cursor_on_bar = true;
    let mut last_tag: i32 = -1;
    let mut last_time: u32 = 0;
    let mut last_motion: Option<(i32, i32, u16)> = None;

    loop {
        let Ok(event) = conn.wait_for_event() else {
            break;
        };
        match &event {
            x11rb::protocol::Event::ButtonRelease(_) => break,
            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= 1000 / REFRESH_RATE_LO {
                    continue;
                }
                last_time = m.time;

                last_motion = Some((m.event_x as i32, m.event_y as i32, u16::from(m.state)));

                let bar_bottom = {
                    let globals = get_globals();
                    globals
                        .monitors
                        .get(globals.selmon)
                        .map(|m| m.by + globals.bh + 1)
                        .unwrap_or(9999)
                };
                if m.event_y as i32 > bar_bottom {
                    cursor_on_bar = false;
                    break;
                }

                let local_x = m.event_x as i32 - mon_mx;
                // Use the canonical hit-test to get the hovered bar position,
                // then convert to a gesture for tag hover highlighting.
                let new_gesture = {
                    let globals = get_globals();
                    globals
                        .monitors
                        .get(selmon_id)
                        .map(|mon| bar_position_at_x(mon, globals, local_x).to_gesture())
                        .unwrap_or(Gesture::None)
                };
                // Encode gesture as i32 for change-detection (reuse last_tag slot).
                let gesture_key = match new_gesture {
                    Gesture::Tag(idx) => idx as i32,
                    _ => -1,
                };

                if last_tag != gesture_key {
                    last_tag = gesture_key;
                    let gm = get_globals_mut();
                    if let Some(mon) = gm.monitors.get_mut(selmon_id) {
                        mon.gesture = new_gesture;
                        draw_bar(mon);
                    }
                }
            }
            _ => {}
        }
    }

    ungrab(conn);

    if cursor_on_bar {
        if let Some((x, _, state)) = last_motion {
            let position = {
                let globals = get_globals();
                let selmon_id = globals.selmon;
                globals
                    .monitors
                    .get(selmon_id)
                    .map(|mon| {
                        let local_x = x - mon.monitor_rect.x;
                        bar_position_at_x(mon, globals, local_x)
                    })
                    .unwrap_or(BarPosition::Root)
            };

            if let BarPosition::Tag(tag_idx) = position {
                let tag_mask = TagMask::single(tag_idx as usize + 1).unwrap_or(TagMask::EMPTY);
                let state = state as u32;
                if (state & ModMask::SHIFT.bits() as u32) != 0 {
                    set_client_tag(tag_mask);
                } else if (state & ModMask::CONTROL.bits() as u32) != 0 {
                    tag_all(tag_mask);
                } else {
                    follow_tag(tag_mask);
                }
            }
        }
    }

    {
        let gm = get_globals_mut();
        gm.bar_dragging = false;
        if let Some(mon) = gm.monitors.get_mut(selmon_id) {
            mon.gesture = Gesture::None;
            draw_bar(mon);
        }
    }
}

// ── window_title_mouse_handler ────────────────────────────────────────────────

/// Left-click / drag handler for a window title bar entry.
///
/// Click (no drag):
/// * Hidden window → show and focus it.
/// * Focused window → hide it.
/// * Otherwise → focus it.
///
/// Drag (cursor moves > [`DRAG_THRESHOLD`] px): show, focus, warp, then
/// hand off to [`move_mouse`].
pub fn window_title_mouse_handler() {
    let Some(win) = crate::util::get_sel_win() else {
        return;
    };

    // Snapshot was_focused and was_hidden before grabbing the pointer so that
    // the state reflects the moment the user clicked, not after any side-effects.
    let (was_focused, was_hidden) = {
        let g = get_globals();
        let sel = g.monitors.get(g.selmon).and_then(|m| m.sel);
        (sel == Some(win), crate::client::is_hidden(win))
    };

    let Some(conn) = grab_pointer(0) else { return };
    let Some((start_x, start_y)) = get_root_ptr() else {
        ungrab(conn);
        return;
    };

    let mut last_time: u32 = 0;

    loop {
        let Ok(event) = conn.wait_for_event() else {
            break;
        };
        match &event {
            x11rb::protocol::Event::ButtonRelease(_) => break,
            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= 1000 / REFRESH_RATE_LO {
                    continue;
                }
                last_time = m.time;
                if (m.event_x as i32 - start_x).abs() > DRAG_THRESHOLD
                    || (m.event_y as i32 - start_y).abs() > DRAG_THRESHOLD
                {
                    ungrab(conn);
                    if was_hidden {
                        crate::client::show(win);
                    }
                    focus(Some(win));
                    warp_into(win);
                    move_mouse();
                    return;
                }
            }
            _ => {}
        }
    }

    // Only reached on ButtonRelease (click, not drag).
    ungrab(conn);
    if was_hidden {
        // Unminimize: show the window, focus it, and restack.
        crate::client::show(win);
        focus(Some(win));
        let g = get_globals_mut();
        if let Some(mon) = g.monitors.get_mut(g.selmon) {
            restack(mon);
        }
    } else if was_focused {
        // Already focused: minimize it.
        crate::client::hide(win);
    } else {
        // Unfocused: focus it and restack.
        focus(Some(win));
        let g = get_globals_mut();
        if let Some(mon) = g.monitors.get_mut(g.selmon) {
            restack(mon);
        }
    }
}

// ── window_title_mouse_handler_right ─────────────────────────────────────────

/// Right-click / drag handler for a window title bar entry.
///
/// Click (no drag):
/// * Shows and focuses the window if hidden.
/// * Calls [`crate::client::zoom`] to promote it to the master area.
///
/// Drag (cursor moves > [`DRAG_THRESHOLD`] px): shows/focuses if hidden, then
/// hands off to [`crate::mouse::resize::resize_mouse`].
///
/// Does nothing when the window is in true fullscreen.
pub fn window_title_mouse_handler_right() {
    let Some(win) = crate::util::get_sel_win() else {
        return;
    };

    {
        let globals = get_globals();
        if globals
            .clients
            .get(&win)
            .map(|c| c.is_fullscreen && !c.isfakefullscreen)
            .unwrap_or(false)
        {
            return;
        }
    }

    focus(Some(win));

    let Some(conn) = grab_pointer(2) else { return };
    let Some((start_x, start_y)) = get_root_ptr() else {
        ungrab(conn);
        return;
    };

    let mut last_time: u32 = 0;

    loop {
        let Ok(event) = conn.wait_for_event() else {
            break;
        };
        match &event {
            x11rb::protocol::Event::ButtonRelease(_) => break,
            x11rb::protocol::Event::MotionNotify(m) => {
                if m.time - last_time <= 1000 / REFRESH_RATE_LO {
                    continue;
                }
                last_time = m.time;
                if (m.event_x as i32 - start_x).abs() > DRAG_THRESHOLD
                    || (m.event_y as i32 - start_y).abs() > DRAG_THRESHOLD
                {
                    ungrab(conn);
                    if crate::client::is_hidden(win) {
                        crate::client::show(win);
                        focus(Some(win));
                    }
                    super::resize::resize_mouse_from_cursor();
                    return;
                }
            }
            _ => {}
        }
    }

    // Only reached on ButtonRelease (click, not drag).
    ungrab(conn);
    if crate::client::is_hidden(win) {
        crate::client::show(win);
        focus(Some(win));
    }
    crate::client::zoom();
}
