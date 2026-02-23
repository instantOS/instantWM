//! Interactive mouse-drag operations.
//!
//! This module covers every grab loop that *moves* things rather than resizes
//! them:
//!
//! | Function                            | Description                                               |
//! |-------------------------------------|-----------------------------------------------------------|
//! | [`move_mouse`]                      | Drag the focused window to a new position                 |
//! | [`gesture_mouse`]                   | Vertical-swipe gesture recogniser on the root window      |
//! | [`drag_tag`]                        | Drag across the tag bar to switch/move tags               |
//! | [`window_title_mouse_handler`]      | Left-click/drag on a window title bar entry               |
//! | [`window_title_mouse_handler_right`]| Right-click/drag on a window title bar entry              |
//! | [`moveresize`]                      | Keyboard-driven nudge of a floating window (arrow keys)   |
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
//! post-loop cleanup (monitor switch, bar redraw, …)
//! ```

use crate::animation::animate_client_rect;
use crate::bar::draw_bar;
use crate::client::resize;
use crate::floating::{reset_snap, toggle_floating, SNAP_LEFT, SNAP_RIGHT, SNAP_TOP};
use crate::focus::{focus, warp_into};
use crate::globals::{get_globals, get_globals_mut};
use crate::monitor::is_current_layout_tiling;
use crate::tags::{follow_tag, get_tag_at_x, get_tag_width, tag, tag_all, view};
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

use super::constants::{
    DRAG_THRESHOLD, MAX_UNMAXIMIZE_OFFSET, OVERLAY_ZONE_WIDTH, REFRESH_RATE_HI, REFRESH_RATE_LO,
};
use super::grab::{grab_pointer, ungrab};
use super::monitor::handle_client_monitor_switch;
use super::warp::get_root_ptr;

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Decide the motion-event throttle based on `globals.doubledraw`.
fn refresh_rate() -> u32 {
    if get_globals().doubledraw {
        REFRESH_RATE_HI
    } else {
        REFRESH_RATE_LO
    }
}

/// Check whether the pointer is in a screen-edge zone that should trigger a
/// snap indicator during [`move_mouse`].
///
/// Returns one of `SNAP_LEFT`, `SNAP_RIGHT`, `SNAP_TOP`, or `0`.
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

/// Snap `nx`/`ny` to the work-area edges of the selected monitor when they are
/// within `globals.snap` pixels.
///
/// Operates in-place on the mutable references.
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

// ── moveresize ────────────────────────────────────────────────────────────────

/// Keyboard-driven nudge of a floating window.
///
/// `arg.i` selects the direction:
/// * `0` → down  (+y)
/// * `1` → up    (−y)
/// * `2` → right (+x)
/// * `3` → left  (−x)
///
/// The window is clamped to the monitor boundary after each step.
/// Does nothing when the layout is tiling and the window is not floating.
pub fn moveresize(arg: &Arg) {
    let direction = arg.i;

    let globals = get_globals();
    let Some(mon) = globals.monitors.get(globals.selmon) else {
        return;
    };
    let Some(win) = mon.sel else { return };
    let Some(client) = globals.clients.get(&win) else {
        return;
    };

    let has_tiling = is_current_layout_tiling(mon, &globals.tags);
    if has_tiling && !client.isfloating {
        return;
    }

    let (c_x, c_y, c_w, c_h, border_width) = (
        client.geo.x,
        client.geo.y,
        client.geo.w,
        client.geo.h,
        client.border_width,
    );
    let (mon_mx, mon_my, mon_mw, mon_mh) = (
        mon.monitor_rect.x,
        mon.monitor_rect.y,
        mon.monitor_rect.w,
        mon.monitor_rect.h,
    );
    let bh = globals.bh;

    // [dx, dy] per direction index (0=down, 1=up, 2=right, 3=left)
    const STEP: i32 = 40;
    let deltas: [[i32; 2]; 4] = [[0, STEP], [0, -STEP], [STEP, 0], [-STEP, 0]];
    let [dx, dy] = deltas[(direction as usize).min(3)];

    let nx = (c_x + dx)
        .max(mon_mx)
        .min(mon_mw + mon_mx - c_w - border_width * 2);
    let ny = (c_y + dy)
        .max(mon_my + bh)
        .min(mon_mh + mon_my - c_h - border_width * 2);

    animate_client_rect(
        win,
        &Rect {
            x: nx,
            y: ny,
            w: c_w,
            h: c_h,
        },
        5,
        0,
    );
    super::warp::warp_impl(win);
}

// ── move_mouse ────────────────────────────────────────────────────────────────

/// Interactive window move: grab the pointer and drag the focused window.
///
/// Special cases handled before the grab:
/// * True-fullscreen windows are skipped entirely.
/// * Overlay windows are skipped.
/// * Fullscreen (fake) windows are un-fullscreened and returned (the user must
///   trigger move again).
/// * Snapped windows are un-snapped and returned.
/// * Near-maximized windows in a non-tiling layout are restored from their
///   saved float geometry.
///
/// During the grab:
/// * Motion events are throttled to [`REFRESH_RATE_HI`] / [`REFRESH_RATE_LO`].
/// * The window snaps to work-area edges within `globals.snap` pixels.
/// * A tiled window is promoted to floating when dragged more than `snap`
///   pixels from its original position.
pub fn move_mouse(_arg: &Arg) {
    // ── Pre-flight checks ────────────────────────────────────────────────────

    let sel_win = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        let Some(sel) = mon.sel else { return };
        let Some(c) = globals.clients.get(&sel) else {
            return;
        };

        if c.is_fullscreen && !c.isfakefullscreen {
            return;
        }
        if Some(sel) == mon.overlay {
            return;
        }
        if Some(sel) == mon.fullscreen {
            crate::floating::temp_fullscreen(&Arg::default());
            return;
        }
        sel
    };

    // Un-snap and return – the user will trigger move again after un-snapping.
    let should_unsnap = {
        let globals = get_globals();
        globals
            .clients
            .get(&sel_win)
            .map(|c| c.snapstatus != SnapPosition::None)
            .unwrap_or(false)
    };
    if should_unsnap {
        reset_snap(sel_win);
        return;
    }

    // If the window is near-maximized in a non-tiling layout, restore saved
    // float geometry so the user is dragging the "real" window, not a maximized
    // one.
    {
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
                    let saved = Rect {
                        x: c.float_geo.x,
                        y: c.float_geo.y,
                        w: c.float_geo.w,
                        h: c.float_geo.h,
                    };
                    drop(c);
                    drop(mon);
                    resize(sel_win, &saved, false);
                }
            }
        }
    }

    // ── Grab ─────────────────────────────────────────────────────────────────

    let Some(conn) = grab_pointer(2) else { return };

    let Some((start_x, start_y)) = get_root_ptr() else {
        ungrab(conn);
        return;
    };

    let (ocx, ocy) = {
        let globals = get_globals();
        globals
            .clients
            .get(&sel_win)
            .map(|c| (c.geo.x, c.geo.y))
            .unwrap_or((0, 0))
    };

    let rate = refresh_rate();
    let mut last_time: u32 = 0;
    let mut edge_snap_indicator: i32 = 0;

    // ── Event loop ───────────────────────────────────────────────────────────

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

                let nx = ocx + (m.event_x as i32 - start_x);
                let ny = ocy + (m.event_y as i32 - start_y);

                // Update edge-snap indicator state (visual feedback only).
                let at_edge = check_edge_snap(m.event_x as i32, m.event_y as i32);
                if at_edge != 0 && edge_snap_indicator == 0 {
                    edge_snap_indicator = at_edge;
                } else if at_edge == 0 && edge_snap_indicator != 0 {
                    edge_snap_indicator = 0;
                }

                let globals = get_globals();
                let snap = globals.snap;

                if let Some(client) = globals.clients.get(&sel_win) {
                    let has_tiling = globals
                        .monitors
                        .get(globals.selmon)
                        .map(|m| is_current_layout_tiling(m, &globals.tags))
                        .unwrap_or(true);

                    // Promote tiled window to floating when dragged far enough.
                    if !client.isfloating
                        && has_tiling
                        && ((nx - client.geo.x).abs() > snap || (ny - client.geo.y).abs() > snap)
                    {
                        toggle_floating(&Arg::default());
                        continue;
                    }

                    if !has_tiling || client.isfloating {
                        let mut adj_nx = nx;
                        let mut adj_ny = ny;
                        snap_to_monitor_edges(client, &mut adj_nx, &mut adj_ny);
                        let new_rect = Rect {
                            x: adj_nx,
                            y: adj_ny,
                            w: client.geo.w,
                            h: client.geo.h,
                        };
                        drop(client);
                        resize(sel_win, &new_rect, true);
                    }
                }
            }

            _ => {}
        }
    }

    ungrab(conn);
    handle_client_monitor_switch(sel_win);
}

// ── gesture_mouse ─────────────────────────────────────────────────────────────

/// Root-window vertical-swipe gesture recogniser.
///
/// Grabs the pointer on the root window and watches for large vertical pointer
/// movements.  When the cursor travels more than `monitor_height / 30` pixels
/// vertically in one throttled interval, [`crate::util::spawn`] is called with
/// a default `Arg`.
///
/// This is used to trigger a gesture-activated launcher or similar action.
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

                let threshold = {
                    let globals = get_globals();
                    globals
                        .monitors
                        .get(globals.selmon)
                        .map(|m| m.monitor_rect.h / 30)
                        .unwrap_or(0)
                };
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
/// Behaviour:
/// * If `arg.ui` does not match the current tagset, calls [`view`] and returns
///   (a plain click, not a drag).
/// * If there is no focused window, returns early.
/// * While dragging over the bar a gesture indicator is drawn via
///   [`draw_bar`].
/// * On release, depending on modifier keys held at the moment the button was
///   released:
///   - `Shift`   → [`tag`]        (move window to tag, stay on current view)
///   - `Control` → [`tag_all`]    (move all windows to tag)
///   - neither   → [`follow_tag`] (move window and follow it)
///
/// If the pointer leaves the bar during the drag, the loop ends without taking
/// any tag action.
pub fn drag_tag(arg: &Arg) {
    let globals = get_globals();

    let tagwidth = if globals.tags.width == 0 {
        get_tag_width()
    } else {
        globals.tags.width
    };

    // Plain click on the current tag → switch view.
    let current_tagset = globals
        .monitors
        .get(globals.selmon)
        .map(|m| m.tagset[m.seltags as usize]);
    if (arg.ui & globals.tags.mask()) != current_tagset.unwrap_or(0) {
        drop(globals);
        view(arg);
        return;
    }

    // Require a focused window to drag.
    if globals
        .monitors
        .get(globals.selmon)
        .and_then(|m| m.sel)
        .is_none()
    {
        return;
    }

    let selmon_id = globals.selmon;
    let mon_mx = globals
        .monitors
        .get(selmon_id)
        .map(|m| m.monitor_rect.x)
        .unwrap_or(0);
    drop(globals);

    let Some(conn) = grab_pointer(2) else { return };

    // Signal to the bar renderer that we are in drag mode.
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
    // Captures the final pointer state for the post-loop action.
    let mut last_motion: Option<(i32, i32, u16)> = None;

    // ── Event loop ───────────────────────────────────────────────────────────

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

                // If the cursor left the bar, stop tracking.
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

                // Update gesture indicator when the hovered tag changes.
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

    // ── Post-drag action ─────────────────────────────────────────────────────

    if cursor_on_bar {
        if let Some((x, _, state)) = last_motion {
            let globals = get_globals();
            let mon_x = globals
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

                    if (state as u32 & ModMask::SHIFT.bits() as u32) != 0 {
                        tag(&tag_arg);
                    } else if (state as u32 & ModMask::CONTROL.bits() as u32) != 0 {
                        tag_all(&tag_arg);
                    } else {
                        follow_tag(&tag_arg);
                    }
                }
            }
        }
    }

    // Clear drag mode and redraw the bar.
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

/// Left-click / drag handler for window title bar entries.
///
/// `arg.v` must contain the target window's `Window` id.
///
/// Behaviour on **release without drag** (click):
/// * If the window was hidden → show and focus it.
/// * If the window was focused → hide it.
/// * Otherwise → focus it.
///
/// Behaviour on **drag** (cursor moves more than [`DRAG_THRESHOLD`] px):
/// * Shows and focuses the window, warps the cursor into it, then hands off
///   to [`move_mouse`].
pub fn window_title_mouse_handler(arg: &Arg) {
    let Some(win) = arg.v.map(|v| v as Window) else {
        return;
    };

    let was_focused = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.sel) == Some(win)
    };
    let was_hidden = crate::client::is_hidden(win);

    let Some(conn) = grab_pointer(0) else { return };

    let Some((start_x, start_y)) = get_root_ptr() else {
        ungrab(conn);
        return;
    };

    let mut drag_started = false;
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
                    drag_started = true;
                    ungrab(conn);
                    crate::client::show(win);
                    focus(Some(win));
                    warp_into(win);
                    move_mouse(&Arg::default());
                    break;
                }
            }

            _ => {}
        }
    }

    if !drag_started {
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
}

// ── window_title_mouse_handler_right ─────────────────────────────────────────

/// Right-click / drag handler for window title bar entries.
///
/// `arg.v` must contain the target window's `Window` id.
///
/// Behaviour on **release without drag** (click):
/// * Shows and focuses the window if it was hidden.
/// * Calls [`crate::client::zoom`] to promote the window to the master area.
///
/// Behaviour on **drag** (cursor moves more than [`DRAG_THRESHOLD`] px):
/// * Shows and focuses the window if it was hidden, then hands off to
///   [`crate::mouse::resize::resize_mouse`].
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

    let mut drag_started = false;
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
                    drag_started = true;
                    ungrab(conn);
                    if crate::client::is_hidden(win) {
                        crate::client::show(win);
                        focus(Some(win));
                    }
                    super::resize::resize_mouse(&Arg::default());
                    break;
                }
            }

            _ => {}
        }
    }

    if !drag_started {
        ungrab(conn);
        if crate::client::is_hidden(win) {
            crate::client::show(win);
            focus(Some(win));
        }
        crate::client::zoom(&Arg::default());
    }
}
