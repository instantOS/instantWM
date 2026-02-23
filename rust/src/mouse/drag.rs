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

use crate::bar::draw_bar;
use crate::client::resize;
use crate::floating::{
    change_snap, reset_snap, set_tiled, toggle_floating, SnapDir, SNAP_LEFT, SNAP_RIGHT, SNAP_TOP,
};
use crate::focus::{focus, warp_into};
use crate::globals::{get_globals, get_globals_mut};
use crate::monitor::{arrange, is_current_layout_tiling};
use crate::tags::{
    follow_tag, get_tag_at_x, get_tag_width, move_left, move_right, tag, tag_all, tag_to_left,
    tag_to_right, view,
};
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

/// Snap `nx`/`ny` to the work-area edges of `selmon` when within `globals.snap` pixels.
fn snap_to_monitor_edges(c: &Client, nx: &mut i32, ny: &mut i32) {
    let globals = get_globals();
    let snap = globals.snap;
    let Some(mon) = globals.monitors.get(globals.selmon) else {
        return;
    };

    let width = c.geo.total_width(c.border_width);
    let height = c.geo.total_height(c.border_width);

    if (mon.work_rect.x - *nx).abs() < snap {
        *nx = mon.work_rect.x;
    } else if (mon.work_rect.x + mon.work_rect.w - (*nx + width)).abs() < snap {
        *nx = mon.work_rect.x + mon.work_rect.w - width;
    }

    if (mon.work_rect.y - *ny).abs() < snap {
        *ny = mon.work_rect.y;
    } else if (mon.work_rect.y + mon.work_rect.h - (*ny + height)).abs() < snap {
        *ny = mon.work_rect.y + mon.work_rect.h - height;
    }
}

/// Returns `SNAP_LEFT`, `SNAP_RIGHT`, `SNAP_TOP`, or `0` based on cursor position.
fn check_edge_snap(x: i32, y: i32) -> i32 {
    let globals = get_globals();
    let Some(mon) = globals.monitors.get(globals.selmon) else {
        return 0;
    };

    if x < mon.monitor_rect.x + OVERLAY_ZONE_WIDTH && x > mon.monitor_rect.x - 1 {
        return SNAP_LEFT;
    }
    if x > mon.monitor_rect.x + mon.monitor_rect.w - OVERLAY_ZONE_WIDTH
        && x < mon.monitor_rect.x + mon.monitor_rect.w + 1
    {
        return SNAP_RIGHT;
    }
    if y <= mon.monitor_rect.y + if mon.showbar { globals.bh } else { 5 } {
        return SNAP_TOP;
    }
    0
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
    ocx: i32,
    ocy: i32,
    /// Whether the cursor was over the bar on the previous motion event.
    cursor_on_bar: bool,
    /// The last edge-snap zone the cursor was in (`SNAP_*` constant or `0`).
    edge_snap_indicator: i32,
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
            crate::floating::temp_fullscreen(&Arg::default());
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
        let (selmon_id, mon_x, tagwidth) = {
            let globals = get_globals();
            let selmon_id = globals.selmon;
            let mon_x = globals
                .monitors
                .get(selmon_id)
                .map(|m| m.monitor_rect.x)
                .unwrap_or(0);
            let cached = globals.tags.width;
            let tagwidth = if cached == 0 { get_tag_width() } else { cached };
            (selmon_id, mon_x, tagwidth)
        };

        let local_x = ptr_x - mon_x;
        let tag_idx = if local_x >= 0 && local_x < tagwidth {
            get_tag_at_x(local_x)
        } else {
            -1
        };
        let new_gesture = Gesture::from_tag_index(tag_idx as usize).unwrap_or(Gesture::None);

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

    let mut nx = state.ocx + (event_x - state.start_x);
    let mut ny = state.ocy + (event_y - state.start_y);

    // While hovering over the bar, keep the window just below it.
    if state.cursor_on_bar {
        let globals = get_globals();
        let bar_bottom = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.by + globals.bh)
            .unwrap_or(ny);
        ny = bar_bottom;
    }

    let (snap, has_tiling, is_floating, client_geo) = {
        let globals = get_globals();
        let snap = globals.snap;
        let has_tiling = globals
            .monitors
            .get(globals.selmon)
            .map(|m| is_current_layout_tiling(m, &globals.tags))
            .unwrap_or(true);
        let (is_floating, client_geo) = globals
            .clients
            .get(&win)
            .map(|c| (c.isfloating, c.geo))
            .unwrap_or((false, Rect::default()));
        (snap, has_tiling, is_floating, client_geo)
    };

    // Promote a tiled window to floating once the user drags it far enough.
    if !is_floating
        && has_tiling
        && ((nx - client_geo.x).abs() > snap || (ny - client_geo.y).abs() > snap)
    {
        toggle_floating(&Arg::default());
        return;
    }

    if !has_tiling || is_floating {
        {
            let globals = get_globals();
            if let Some(client) = globals.clients.get(&win) {
                snap_to_monitor_edges(client, &mut nx, &mut ny);
            }
        }
        resize(
            win,
            &Rect {
                x: nx,
                y: ny,
                w: client_geo.w,
                h: client_geo.h,
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
/// * Dropped on a tag button → `tag()` + `set_tiled()`
/// * Dropped elsewhere on bar, window floating → `toggle_floating()`
fn handle_bar_drop(win: Window) {
    let Some((ptr_x, ptr_y)) = get_root_ptr() else {
        return;
    };
    if !point_is_on_bar(ptr_x, ptr_y) {
        return;
    }

    let (mon_x, tagwidth) = {
        let globals = get_globals();
        let mon_x = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.monitor_rect.x)
            .unwrap_or(0);
        let cached = globals.tags.width;
        let tagwidth = if cached == 0 { get_tag_width() } else { cached };
        (mon_x, tagwidth)
    };

    let local_x = ptr_x - mon_x;
    let tag_idx = get_tag_at_x(local_x);

    if tag_idx >= 0 && local_x < tagwidth {
        // tag() changes selmon->sel via focus(None), so we address the window
        // by its id with set_tiled rather than using toggle_floating.
        tag(&Arg {
            ui: 1u32 << (tag_idx as u32),
            ..Default::default()
        });
        set_tiled(win, true);
    } else if get_globals()
        .clients
        .get(&win)
        .map(|c| c.isfloating)
        .unwrap_or(false)
    {
        toggle_floating(&Arg::default());
    }
}

/// Apply post-release logic for left/right screen-edge drops.
///
/// In a tiling layout: navigate to the adjacent tag (or send the window there).
/// In a floating layout: apply a directional screen-edge snap.
///
/// Returns `true` if the drop was fully handled (the caller should skip
/// `handle_bar_drop` and `handle_client_monitor_switch`).
fn apply_edge_drop(win: Window, edge: i32) -> bool {
    if edge == 0 {
        return false;
    }

    let Some((root_x, root_y)) = get_root_ptr() else {
        return false;
    };

    let snap_dir = check_edge_snap(root_x, root_y);
    let at_left = snap_dir == SNAP_LEFT;
    let at_right = snap_dir == SNAP_RIGHT;

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
                move_left(&Arg::default());
            } else {
                move_right(&Arg::default());
            }
        } else {
            if at_left {
                tag_to_left(&Arg::default());
            } else {
                tag_to_right(&Arg::default());
            }
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
pub fn move_mouse(_arg: &Arg) {
    let Some(win) = prepare_drag_target() else {
        return;
    };

    let Some(conn) = grab_pointer(2) else { return };
    let Some((start_x, start_y)) = get_root_ptr() else {
        ungrab(conn);
        return;
    };

    let (ocx, ocy) = get_globals()
        .clients
        .get(&win)
        .map(|c| (c.geo.x, c.geo.y))
        .unwrap_or((0, 0));

    let mut state = MoveState {
        start_x,
        start_y,
        ocx,
        ocy,
        cursor_on_bar: false,
        edge_snap_indicator: 0,
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
        handle_bar_drop(win);
        handle_client_monitor_switch(win);
    }
}

// ── gesture_mouse ─────────────────────────────────────────────────────────────

/// Root-window vertical-swipe gesture recogniser.
///
/// Watches for large vertical pointer movements; each time the cursor travels
/// more than `monitor_height / 30` pixels [`crate::util::spawn`] is called.
pub fn gesture_mouse(_arg: &Arg) {
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
                    crate::util::spawn(&Arg::default());
                    last_y = m.event_y as i32;
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
pub fn drag_tag(arg: &Arg) {
    let (is_current_tag, has_sel, selmon_id, mon_mx, tagwidth) = {
        let globals = get_globals();
        let cached = globals.tags.width;
        let tagwidth = if cached == 0 { get_tag_width() } else { cached };
        let current_tagset = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.tagset[m.seltags as usize]);
        let is_current_tag = (arg.ui & globals.tags.mask()) == current_tagset.unwrap_or(0);
        let has_sel = globals
            .monitors
            .get(globals.selmon)
            .and_then(|m| m.sel)
            .is_some();
        let selmon_id = globals.selmon;
        let mon_mx = globals
            .monitors
            .get(selmon_id)
            .map(|m| m.monitor_rect.x)
            .unwrap_or(0);
        (is_current_tag, has_sel, selmon_id, mon_mx, tagwidth)
    };

    if !is_current_tag {
        view(arg);
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
                let tag_x = if local_x >= 0 {
                    get_tag_at_x(local_x)
                } else {
                    -1
                };

                if last_tag != tag_x {
                    last_tag = tag_x;
                    let gm = get_globals_mut();
                    if let Some(mon) = gm.monitors.get_mut(selmon_id) {
                        mon.gesture =
                            Gesture::from_tag_index(tag_x as usize).unwrap_or(Gesture::None);
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
            let mon_x = get_globals()
                .monitors
                .get(selmon_id)
                .map(|m| m.monitor_rect.x)
                .unwrap_or(0);
            let local_x = x - mon_x;

            if local_x >= 0 && local_x < tagwidth {
                let tag_idx = get_tag_at_x(local_x);
                if tag_idx >= 0 {
                    let tag_arg = Arg {
                        ui: 1u32 << (tag_idx as u32),
                        ..Default::default()
                    };
                    let state = state as u32;
                    if (state & ModMask::SHIFT.bits() as u32) != 0 {
                        tag(&tag_arg);
                    } else if (state & ModMask::CONTROL.bits() as u32) != 0 {
                        tag_all(&tag_arg);
                    } else {
                        follow_tag(&tag_arg);
                    }
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
pub fn window_title_mouse_handler(arg: &Arg) {
    let Some(win) = arg.v.map(|v| v as Window) else {
        return;
    };

    let was_focused = get_globals()
        .monitors
        .get(get_globals().selmon)
        .and_then(|m| m.sel)
        == Some(win);
    let was_hidden = crate::client::is_hidden(win);

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
                    crate::client::show(win);
                    focus(Some(win));
                    warp_into(win);
                    move_mouse(&Arg::default());
                    return;
                }
            }
            _ => {}
        }
    }

    // Only reached on ButtonRelease (click, not drag).
    ungrab(conn);
    if was_hidden {
        crate::client::show(win);
        focus(Some(win));
    } else if was_focused {
        crate::client::hide(win);
    } else {
        focus(Some(win));
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
pub fn window_title_mouse_handler_right(arg: &Arg) {
    let Some(win) = arg.v.map(|v| v as Window) else {
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
                    super::resize::resize_mouse(&Arg::default());
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
    crate::client::zoom(&Arg::default());
}
