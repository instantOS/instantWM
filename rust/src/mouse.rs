use crate::animation::animate_client;
use crate::bar::draw_bar;
use crate::client::{is_visible, next_tiled, resize, resize_client, unfocus_win};
use crate::floating::{
    change_snap, reset_snap, save_floating_win, toggle_floating, SNAP_BOTTOM, SNAP_BOTTOM_LEFT,
    SNAP_BOTTOM_RIGHT, SNAP_LEFT, SNAP_RIGHT, SNAP_TOP, SNAP_TOP_LEFT, SNAP_TOP_RIGHT,
};
use crate::focus::{focus, warp_into};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::{arrange, rect_to_mon, send_mon};
use crate::overlay::{create_overlay, set_overlay, set_overlay_mode};
use crate::tags::{
    follow_tag, get_tag_at_x, get_tag_width, move_left, move_right, tag, tag_all, tag_to_left,
    tag_to_right, view,
};
use crate::types::*;
use crate::util::spawn;
use std::ffi::CStr;
use std::io::Read;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

const MIN_WINDOW_SIZE: i32 = 50;
const RESIZE_BORDER_ZONE: i32 = 30;
const DRAG_THRESHOLD: i32 = 5;
const MAX_UNMAXIMIZE_OFFSET: i32 = 100;
const OVERLAY_ZONE_WIDTH: i32 = 50;
const SLOP_MARGIN: i32 = 40;
const REFRESH_RATE_HI: u32 = 240;
const REFRESH_RATE_LO: u32 = 120;
const KEYCODE_ESCAPE: u8 = 9;

const RESIZE_DIR_TOP_LEFT: i32 = 0;
const RESIZE_DIR_TOP: i32 = 1;
const RESIZE_DIR_TOP_RIGHT: i32 = 2;
const RESIZE_DIR_RIGHT: i32 = 3;
const RESIZE_DIR_BOTTOM_RIGHT: i32 = 4;
const RESIZE_DIR_BOTTOM: i32 = 5;
const RESIZE_DIR_BOTTOM_LEFT: i32 = 6;
const RESIZE_DIR_LEFT: i32 = 7;

pub fn motion_notify(_e: &MotionNotifyEvent) {}

pub fn button_press(_e: &ButtonPressEvent) {}

pub fn move_resize(_arg: &Arg) {}

pub fn moveresize(_arg: &Arg) {
    let direction = _arg.i;

    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.sel
            } else {
                return;
            }
        } else {
            return;
        }
    };

    let Some(win) = sel_win else { return };

    let (is_floating, c_x, c_y, c_w, c_h, border_width) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            (
                client.isfloating,
                client.x,
                client.y,
                client.w,
                client.h,
                client.border_width,
            )
        } else {
            return;
        }
    };

    let has_tiling = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.sellt == 0
            } else {
                true
            }
        } else {
            true
        }
    };

    if has_tiling && !is_floating {
        return;
    }

    let (mon_mx, mon_my, mon_mw, mon_mh, mon_ww, mon_wh, bh) = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                (mon.mx, mon.my, mon.mw, mon.mh, mon.ww, mon.wh, globals.bh)
            } else {
                return;
            }
        } else {
            return;
        }
    };

    let move_step = 40;
    let move_deltas: [[i32; 2]; 4] = [
        [0, move_step],
        [0, -move_step],
        [move_step, 0],
        [-move_step, 0],
    ];

    let dir_idx = (direction as usize).min(3);
    let mut nx = c_x + move_deltas[dir_idx][0];
    let mut ny = c_y + move_deltas[dir_idx][1];

    nx = nx.max(mon_mx);
    ny = ny.max(mon_my);

    if ny + c_h > mon_my + mon_mh {
        ny = mon_mh + mon_my - c_h - border_width * 2;
    }
    if nx + c_w > mon_mx + mon_mw {
        nx = mon_mw + mon_mx - c_w - border_width * 2;
    }

    animate_client(win, nx, ny, c_w, c_h, 5, 0);
    warp_cursor_to_client_impl(win);
}

pub fn get_cursor_client() -> Option<ClientInner> {
    let globals = get_globals();

    let (x, y) = get_root_ptr()?;

    for mon in &globals.monitors {
        let mut current = mon.clients;
        while let Some(c_win) = current {
            if let Some(c) = globals.clients.get(&c_win) {
                if x >= c.x && x <= c.x + c.w && y >= c.y && y <= c.y + c.h {
                    return Some(c.clone());
                }
                current = c.next;
            } else {
                break;
            }
        }
    }
    None
}

pub fn warp(c: &ClientInner) {
    warp_cursor_to_client_impl(c.win);
}

pub fn force_warp(c: &ClientInner) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.warp_pointer(
            x11rb::NONE,
            c.win,
            0i16,
            0i16,
            0u16,
            0u16,
            (c.w / 2) as i16,
            10i16,
        );
        let _ = conn.flush();
    }
}

fn warp_cursor_to_client_impl(win: Window) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let root = globals.root;
        let bh = globals.bh;

        if win == 0 {
            if let Some(sel_mon_id) = globals.selmon {
                if let Some(mon) = globals.monitors.get(sel_mon_id) {
                    let _ = conn.warp_pointer(
                        CURRENT_TIME,
                        root,
                        0,
                        0,
                        0,
                        0,
                        (mon.wx + mon.ww / 2) as i16,
                        (mon.wy + mon.wh / 2) as i16,
                    );
                    let _ = conn.flush();
                }
            }
            return;
        }

        if let Some(c) = globals.clients.get(&win) {
            if let Some((x, y)) = get_root_ptr() {
                let in_window = x > c.x - c.border_width
                    && y > c.y - c.border_width
                    && x < c.x + c.w + c.border_width * 2
                    && y < c.y + c.h + c.border_width * 2;

                let on_bar = if let Some(mon_id) = c.mon_id {
                    if let Some(mon) = globals.monitors.get(mon_id) {
                        (y > mon.by && y < mon.by + bh) || (mon.topbar && y == 0)
                    } else {
                        false
                    }
                } else {
                    false
                };

                if in_window || on_bar {
                    return;
                }

                let _ = conn.warp_pointer(
                    CURRENT_TIME,
                    c.win,
                    0,
                    0,
                    0,
                    0,
                    (c.w / 2) as i16,
                    (c.h / 2) as i16,
                );
                let _ = conn.flush();
            }
        }
    }
}

pub fn warp_cursor_to_client_win(c: &ClientInner) {
    warp_cursor_to_client_impl(c.win);
}

pub fn warp_to_focus(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            globals.monitors.get(sel_mon_id).and_then(|m| m.sel)
        } else {
            None
        }
    };

    if let Some(win) = sel_win {
        warp_cursor_to_client_impl(win);
    }
}

pub fn reset_cursor() {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if let Some(ref cursor) = globals.cursors[0] {
            let _ = change_window_attributes(
                conn,
                globals.root,
                &ChangeWindowAttributesAux::new().cursor(cursor.cursor),
            );
            let _ = conn.flush();
        }
    }
}

pub fn grab_buttons(c_win: Window, focused: bool) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.ungrab_button(0u8.into(), c_win, ModMask::from(0u16));

        if !focused {
            let globals = get_globals();
            let numlockmask = globals.numlockmask;

            let modifiers: [u16; 4] = [
                0,
                numlockmask as u16,
                ModMask::LOCK.bits(),
                (numlockmask as u16) | ModMask::LOCK.bits(),
            ];

            for &modifiers in &modifiers {
                let _ = conn.grab_button(
                    false,
                    c_win,
                    EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                    GrabMode::SYNC,
                    GrabMode::SYNC,
                    x11rb::NONE,
                    x11rb::NONE,
                    1u8.into(),
                    ModMask::from(modifiers),
                );
                let _ = conn.grab_button(
                    false,
                    c_win,
                    EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE,
                    GrabMode::SYNC,
                    GrabMode::SYNC,
                    x11rb::NONE,
                    x11rb::NONE,
                    3u8.into(),
                    ModMask::from(modifiers),
                );
            }
        }
        let _ = conn.flush();
    }
}

pub fn move_mouse(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                if let Some(sel) = mon.sel {
                    if let Some(c) = globals.clients.get(&sel) {
                        if c.is_fullscreen && !c.isfakefullscreen {
                            return;
                        }
                        if Some(sel) == mon.overlay {
                            return;
                        }
                        if Some(sel) == mon.fullscreen {
                            drop(globals);
                            crate::floating::temp_fullscreen(&Arg::default());
                            return;
                        }
                    }
                    Some(sel)
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

    let Some(win) = sel_win else { return };

    let (snapstatus, saved_x, saved_y, saved_w, saved_h, has_tiling) = {
        let globals = get_globals();
        let c = match globals.clients.get(&win) {
            Some(c) => c,
            None => return,
        };
        let snapstatus = c.snapstatus;
        let saved = (
            c.saved_float_x,
            c.saved_float_y,
            c.saved_float_width,
            c.saved_float_height,
        );
        let has_tiling = if let Some(sel_mon_id) = globals.selmon {
            globals
                .monitors
                .get(sel_mon_id)
                .map(|m| m.sellt == 0)
                .unwrap_or(true)
        } else {
            true
        };
        (snapstatus, saved.0, saved.1, saved.2, saved.3, has_tiling)
    };

    if snapstatus != SnapPosition::None {
        reset_snap(win);
        return;
    }

    if !has_tiling {
        let (c_x, c_y, c_w, c_h, mon_mw, mon_mh, mon_mx, mon_my, bh) = {
            let globals = get_globals();
            let c = globals.clients.get(&win).unwrap();
            let mon = globals.monitors.get(globals.selmon.unwrap()).unwrap();
            let bh = globals.bh;
            if c.x >= mon.mx - MAX_UNMAXIMIZE_OFFSET
                && c.y >= mon.my + bh - MAX_UNMAXIMIZE_OFFSET
                && c.w >= mon.mw - MAX_UNMAXIMIZE_OFFSET
                && c.h >= mon.mh - MAX_UNMAXIMIZE_OFFSET
            {
                resize(win, saved_x, saved_y, saved_w, saved_h, false);
            }
            (c.x, c.y, c.w, c.h, mon.mw, mon.mh, mon.mx, mon.my, bh)
        };
        let _ = (c_x, c_y, c_w, c_h, mon_mw, mon_mh, mon_mx, mon_my, bh);
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let root = globals.root;
        let cursor = globals.cursors[2].as_ref().map(|c| c.cursor).unwrap_or(0);

        if conn
            .grab_pointer(
                false,
                root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                cursor,
                CURRENT_TIME,
            )
            .is_err()
        {
            return;
        }

        let Some((start_x, start_y)) = get_root_ptr() else {
            return;
        };

        let (ocx, ocy) = {
            let globals = get_globals();
            globals
                .clients
                .get(&win)
                .map(|c| (c.x, c.y))
                .unwrap_or((0, 0))
        };

        let mut last_time: u32 = 0;
        let mut edge_snap_indicator = 0;
        let rate = if globals.doubledraw {
            REFRESH_RATE_HI
        } else {
            REFRESH_RATE_LO
        };

        loop {
            let event = conn.wait_for_event();
            if let Ok(e) = event {
                match &e {
                    x11rb::protocol::Event::ButtonRelease(_) => break,
                    x11rb::protocol::Event::MotionNotify(m) => {
                        if m.time - last_time <= 1000 / rate {
                            continue;
                        }
                        last_time = m.time;

                        let nx = ocx + (m.event_x as i32 - start_x);
                        let ny = ocy + (m.event_y as i32 - start_y);

                        let at_edge = check_edge_snap(m.event_x as i32, m.event_y as i32);

                        if at_edge != 0 && edge_snap_indicator == 0 {
                            edge_snap_indicator = at_edge;
                        } else if at_edge == 0 && edge_snap_indicator != 0 {
                            edge_snap_indicator = 0;
                        }

                        let globals = get_globals();
                        let snap = globals.snap as i32;
                        let c = globals.clients.get(&win);
                        if let Some(client) = c {
                            let width = client.w + 2 * client.border_width;
                            let height = client.h + 2 * client.border_width;

                            let mut adj_nx = nx;
                            let mut adj_ny = ny;

                            if let Some(sel_mon_id) = globals.selmon {
                                if let Some(mon) = globals.monitors.get(sel_mon_id) {
                                    if (mon.wx - nx).abs() < snap {
                                        adj_nx = mon.wx;
                                    } else if (mon.wx + mon.ww - (nx + width)).abs() < snap {
                                        adj_nx = mon.wx + mon.ww - width;
                                    }
                                    if (mon.wy - ny).abs() < snap {
                                        adj_ny = mon.wy;
                                    } else if (mon.wy + mon.wh - (ny + height)).abs() < snap {
                                        adj_ny = mon.wy + mon.wh - height;
                                    }
                                }
                            }

                            let has_tiling = if let Some(sel_mon_id) = globals.selmon {
                                globals
                                    .monitors
                                    .get(sel_mon_id)
                                    .map(|m| m.sellt == 0)
                                    .unwrap_or(true)
                            } else {
                                true
                            };

                            if !client.isfloating
                                && has_tiling
                                && ((nx - client.x).abs() > snap || (ny - client.y).abs() > snap)
                            {
                                drop(globals);
                                toggle_floating(&Arg::default());
                            } else if !has_tiling || client.isfloating {
                                resize(win, adj_nx, adj_ny, client.w, client.h, true);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let _ = ungrab_pointer(conn, CURRENT_TIME);
        let _ = conn.flush();
    }

    handle_client_monitor_switch(win);
}

fn check_edge_snap(x: i32, y: i32) -> i32 {
    let globals = get_globals();
    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get(sel_mon_id) {
            if x < mon.mx + OVERLAY_ZONE_WIDTH && x > mon.mx - 1 {
                return SNAP_LEFT;
            }
            if x > mon.mx + mon.mw - OVERLAY_ZONE_WIDTH && x < mon.mx + mon.mw + 1 {
                return SNAP_RIGHT;
            }
            if y <= mon.my + if mon.showbar { globals.bh } else { 5 } {
                return SNAP_TOP;
            }
        }
    }
    0
}

pub fn resize_mouse(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.sel
            } else {
                None
            }
        } else {
            None
        }
    };

    let Some(win) = sel_win else { return };

    let (is_fullscreen, is_fake_fullscreen, mon_id) = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            (c.is_fullscreen, c.isfakefullscreen, c.mon_id)
        } else {
            return;
        }
    };

    if is_fullscreen && !is_fake_fullscreen {
        return;
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let root = globals.root;
        let cursor = globals.cursors[1].as_ref().map(|c| c.cursor).unwrap_or(0);

        if conn
            .grab_pointer(
                false,
                root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                cursor,
                CURRENT_TIME,
            )
            .is_err()
        {
            return;
        }

        let (orig_left, orig_top, orig_right, orig_bottom) = {
            let globals = get_globals();
            if let Some(c) = globals.clients.get(&win) {
                (c.x, c.y, c.x + c.w, c.y + c.h)
            } else {
                return;
            }
        };

        let mut last_time: u32 = 0;
        let rate = if globals.doubledraw {
            REFRESH_RATE_HI
        } else {
            REFRESH_RATE_LO
        };
        let corner = RESIZE_DIR_BOTTOM_RIGHT;

        loop {
            let event = conn.wait_for_event();
            if let Ok(e) = event {
                match &e {
                    x11rb::protocol::Event::ButtonRelease(_) => break,
                    x11rb::protocol::Event::MotionNotify(m) => {
                        if m.time - last_time <= 1000 / rate {
                            continue;
                        }
                        last_time = m.time;

                        let nw = (m.event_x as i32 - orig_left + 1).max(1);
                        let nh = (m.event_y as i32 - orig_top + 1).max(1);

                        let globals = get_globals();
                        let snap = globals.snap as i32;
                        if let Some(client) = globals.clients.get(&win) {
                            let has_tiling = if let Some(sel_mon_id) = globals.selmon {
                                globals
                                    .monitors
                                    .get(sel_mon_id)
                                    .map(|m| m.sellt == 0)
                                    .unwrap_or(true)
                            } else {
                                true
                            };

                            if !client.isfloating
                                && has_tiling
                                && ((nw - client.w).abs() > snap || (nh - client.h).abs() > snap)
                            {
                                drop(globals);
                                toggle_floating(&Arg::default());
                            } else if !has_tiling || client.isfloating {
                                resize(win, client.x, client.y, nw, nh, true);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let _ = ungrab_pointer(conn, CURRENT_TIME);
        let _ = conn.flush();
    }

    handle_client_monitor_switch(win);
}

pub fn force_resize_mouse(arg: &Arg) {
    resize_mouse(arg);
}

pub fn resize_aspect_mouse(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.sel
            } else {
                None
            }
        } else {
            None
        }
    };

    let Some(win) = sel_win else { return };

    let is_fullscreen = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.is_fullscreen)
            .unwrap_or(false)
    };

    if is_fullscreen {
        return;
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let root = globals.root;
        let cursor = globals.cursors[1].as_ref().map(|c| c.cursor).unwrap_or(0);

        if conn
            .grab_pointer(
                false,
                root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                cursor,
                CURRENT_TIME,
            )
            .is_err()
        {
            return;
        }

        let (orig_left, orig_top) = {
            let globals = get_globals();
            if let Some(c) = globals.clients.get(&win) {
                (c.x, c.y)
            } else {
                return;
            }
        };

        let mut last_time: u32 = 0;
        let rate = if globals.doubledraw {
            REFRESH_RATE_HI
        } else {
            REFRESH_RATE_LO
        };

        loop {
            let event = conn.wait_for_event();
            if let Ok(e) = event {
                match &e {
                    x11rb::protocol::Event::ButtonRelease(_) => break,
                    x11rb::protocol::Event::MotionNotify(m) => {
                        if m.time - last_time <= 1000 / rate {
                            continue;
                        }
                        last_time = m.time;

                        let mut nw = (m.event_x as i32 - orig_left + 1).max(1);
                        let mut nh = (m.event_y as i32 - orig_top + 1).max(1);

                        let globals = get_globals();
                        if let Some(client) = globals.clients.get(&win) {
                            let (minw, minh, maxw, maxh, mina, maxa) = (
                                client.minw,
                                client.minh,
                                client.maxw,
                                client.maxh,
                                client.mina,
                                client.maxa,
                            );
                            let border_width = client.border_width;

                            if minw > 0 && nw < minw {
                                nw = minw;
                            }
                            if minh > 0 && nh < minh {
                                nh = minh;
                            }
                            if maxw > 0 && nw > maxw {
                                nw = maxw;
                            }
                            if maxh > 0 && nh > maxh {
                                nh = maxh;
                            }

                            if mina > 0.0 && maxa > 0.0 {
                                if maxa < nw as f32 / nh as f32 {
                                    nw = (nh as f32 * maxa) as i32;
                                } else if mina < nh as f32 / nw as f32 {
                                    nh = (nw as f32 * mina) as i32;
                                }
                            }

                            resize(win, client.x, client.y, nw, nh, true);
                        }
                    }
                    _ => {}
                }
            }
        }

        let _ = ungrab_pointer(conn, CURRENT_TIME);
        let _ = conn.flush();
    }

    handle_client_monitor_switch(win);
}

pub fn gesture_mouse(_arg: &Arg) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let root = globals.root;
        let cursor = globals.cursors[2].as_ref().map(|c| c.cursor).unwrap_or(0);

        if conn
            .grab_pointer(
                false,
                root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                cursor,
                CURRENT_TIME,
            )
            .is_err()
        {
            return;
        }

        let Some((_, start_y)) = get_root_ptr() else {
            return;
        };
        let mut last_y = start_y;
        let mut last_time: u32 = 0;

        loop {
            let event = conn.wait_for_event();
            if let Ok(e) = event {
                match &e {
                    x11rb::protocol::Event::ButtonRelease(_) => break,
                    x11rb::protocol::Event::MotionNotify(m) => {
                        let m = m;
                        {
                            if m.time - last_time <= 1000 / REFRESH_RATE_LO {
                                continue;
                            }
                            last_time = m.time;

                            let globals = get_globals();
                            if let Some(sel_mon_id) = globals.selmon {
                                if let Some(mon) = globals.monitors.get(sel_mon_id) {
                                    let threshold = mon.mh / 30;
                                    if (last_y - m.event_y as i32).abs() > threshold {
                                        drop(globals);
                                        spawn(&Arg::default());
                                        last_y = m.event_y as i32;
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let _ = ungrab_pointer(conn, CURRENT_TIME);
        let _ = conn.flush();
    }
}

pub fn is_in_resize_border() -> bool {
    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            globals.monitors.get(sel_mon_id).and_then(|m| m.sel)
        } else {
            None
        }
    };

    let Some(win) = sel_win else { return false };

    let (is_floating, has_tiling, c_x, c_y, c_w, c_h) = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            let has_tiling = if let Some(sel_mon_id) = globals.selmon {
                globals
                    .monitors
                    .get(sel_mon_id)
                    .map(|m| m.sellt == 0)
                    .unwrap_or(true)
            } else {
                true
            };
            (c.isfloating, has_tiling, c.x, c.y, c.w, c.h)
        } else {
            return false;
        }
    };

    if !is_floating && has_tiling {
        return false;
    }

    let Some((x, y)) = get_root_ptr() else {
        return false;
    };

    let globals = get_globals();
    let bh = globals.bh;
    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get(sel_mon_id) {
            if mon.showbar && y < mon.my + bh {
                return false;
            }
        }
    }

    if y > c_y && y < c_y + c_h && x > c_x && x < c_x + c_w {
        return false;
    }

    if y < c_y - RESIZE_BORDER_ZONE
        || x < c_x - RESIZE_BORDER_ZONE
        || y > c_y + c_h + RESIZE_BORDER_ZONE
        || x > c_x + c_w + RESIZE_BORDER_ZONE
    {
        return false;
    }

    true
}

pub fn hover_resize_mouse(_arg: &Arg) -> i32 {
    if !is_in_resize_border() {
        return 0;
    }

    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            globals.monitors.get(sel_mon_id).and_then(|m| m.sel)
        } else {
            return 0;
        }
    };

    let Some(win) = sel_win else { return 0 };

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let cursor = globals.cursors[1].as_ref().map(|c| c.cursor).unwrap_or(0);

        if conn
            .grab_pointer(
                false,
                globals.root,
                EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::POINTER_MOTION
                    | EventMask::KEY_PRESS,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                cursor,
                CURRENT_TIME,
            )
            .is_err()
        {
            return 0;
        }

        let mut resize_started = false;

        loop {
            let event = conn.wait_for_event();
            if let Ok(e) = event {
                match &e {
                    x11rb::protocol::Event::ButtonRelease(_) => break,
                    x11rb::protocol::Event::MotionNotify(_) => {
                        if !is_in_resize_border() {
                            break;
                        }
                    }
                    x11rb::protocol::Event::KeyPress(k) => {
                        let k = k;
                        if k.detail == KEYCODE_ESCAPE {
                            break;
                        }
                    }
                    x11rb::protocol::Event::ButtonPress(_) => {
                        resize_started = true;
                        let _ = ungrab_pointer(conn, CURRENT_TIME);
                        resize_mouse(&Arg::default());
                        break;
                    }
                    _ => {}
                }
            }
        }

        if !resize_started {
            let _ = ungrab_pointer(conn, CURRENT_TIME);
        }
        let _ = conn.flush();
    }

    1
}

pub fn window_title_mouse_handler(arg: &Arg) {
    let win = arg.v.map(|v| v as Window);
    let Some(win) = win else { return };

    let (was_focused, was_hidden) = {
        let globals = get_globals();
        let was_focused = globals
            .selmon
            .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel))
            == Some(win);
        let was_hidden = crate::client::is_hidden(win);
        (was_focused, was_hidden)
    };

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let cursor = globals.cursors[0].as_ref().map(|c| c.cursor).unwrap_or(0);

        if conn
            .grab_pointer(
                false,
                globals.root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                cursor,
                CURRENT_TIME,
            )
            .is_err()
        {
            return;
        }

        let Some((start_x, start_y)) = get_root_ptr() else {
            return;
        };

        let mut drag_started = false;
        let mut last_time: u32 = 0;

        loop {
            let event = conn.wait_for_event();
            if let Ok(e) = event {
                match &e {
                    x11rb::protocol::Event::ButtonRelease(_) => break,
                    x11rb::protocol::Event::MotionNotify(m) => {
                        let m = m;
                        {
                            if m.time - last_time <= 1000 / REFRESH_RATE_LO {
                                continue;
                            }
                            last_time = m.time;

                            if (m.event_x as i32 - start_x).abs() > DRAG_THRESHOLD
                                || (m.event_y as i32 - start_y).abs() > DRAG_THRESHOLD
                            {
                                drag_started = true;
                                let _ = ungrab_pointer(conn, CURRENT_TIME);
                                crate::client::show(win);
                                focus(Some(win));
                                warp_into(win);
                                move_mouse(&Arg::default());
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if !drag_started {
            let _ = ungrab_pointer(conn, CURRENT_TIME);
            if was_hidden {
                crate::client::show(win);
                focus(Some(win));
            } else {
                if was_focused {
                    crate::client::hide(win);
                } else {
                    focus(Some(win));
                }
            }
        }
        let _ = conn.flush();
    }
}

pub fn window_title_mouse_handler_right(arg: &Arg) {
    let win = arg.v.map(|v| v as Window);
    let Some(win) = win else { return };

    let is_fullscreen = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.is_fullscreen && !c.isfakefullscreen)
            .unwrap_or(false)
    };

    if is_fullscreen {
        return;
    }

    focus(Some(win));

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let cursor = globals.cursors[2].as_ref().map(|c| c.cursor).unwrap_or(0);

        if conn
            .grab_pointer(
                false,
                globals.root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                cursor,
                CURRENT_TIME,
            )
            .is_err()
        {
            return;
        }

        let Some((start_x, start_y)) = get_root_ptr() else {
            return;
        };

        let mut drag_started = false;
        let mut last_time: u32 = 0;

        loop {
            let event = conn.wait_for_event();
            if let Ok(e) = event {
                match &e {
                    x11rb::protocol::Event::ButtonRelease(_) => break,
                    x11rb::protocol::Event::MotionNotify(m) => {
                        let m = m;
                        {
                            if m.time - last_time <= 1000 / REFRESH_RATE_LO {
                                continue;
                            }
                            last_time = m.time;

                            if (m.event_x as i32 - start_x).abs() > DRAG_THRESHOLD
                                || (m.event_y as i32 - start_y).abs() > DRAG_THRESHOLD
                            {
                                drag_started = true;
                                let _ = ungrab_pointer(conn, CURRENT_TIME);
                                if crate::client::is_hidden(win) {
                                    crate::client::show(win);
                                    focus(Some(win));
                                }
                                resize_mouse(&Arg::default());
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if !drag_started {
            let _ = ungrab_pointer(conn, CURRENT_TIME);
            if crate::client::is_hidden(win) {
                crate::client::show(win);
                focus(Some(win));
            }
            crate::client::zoom(&Arg::default());
        }
        let _ = conn.flush();
    }
}

pub fn draw_window(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            globals.monitors.get(sel_mon_id).and_then(|m| m.sel)
        } else {
            None
        }
    };

    let Some(win) = sel_win else { return };

    let output = std::process::Command::new("instantslop")
        .arg("-f")
        .arg("x%xx%yx%wx%hx")
        .output();

    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let dims = parse_slop_output(&stdout);

        if let Some((x, y, w, h)) = dims {
            if w > MIN_WINDOW_SIZE && h > MIN_WINDOW_SIZE {
                let globals = get_globals();
                if let Some(c) = globals.clients.get(&win) {
                    let is_different = (c.w - w).abs() > 20
                        || (c.h - h).abs() > 20
                        || (c.x - x).abs() > 20
                        || (c.y - y).abs() > 20;

                    if is_different {
                        drop(globals);
                        handle_monitor_switch(win, x, y, w, h);

                        let is_floating = {
                            let globals = get_globals();
                            globals
                                .clients
                                .get(&win)
                                .map(|c| c.isfloating)
                                .unwrap_or(false)
                        };

                        if is_floating {
                            resize(win, x, y, w, h, true);
                        } else {
                            toggle_floating(&Arg::default());
                            resize(win, x, y, w, h, true);
                        }
                    }
                }
            }
        }
    }
}

pub fn parse_slop_output(output: &str) -> Option<(i32, i32, i32, i32)> {
    let parts: Vec<&str> = output.split('x').collect();
    if parts.len() < 5 {
        return None;
    }

    let x = parts.get(1)?.parse().ok()?;
    let y = parts.get(2)?.parse().ok()?;
    let w = parts.get(3)?.parse().ok()?;
    let h = parts.get(4)?.trim_end().parse().ok()?;

    Some((x, y, w, h))
}

pub fn is_valid_window_size(x: i32, y: i32, width: i32, height: i32, c_win: Window) -> bool {
    let globals = get_globals();
    if let Some(c) = globals.clients.get(&c_win) {
        width > MIN_WINDOW_SIZE
            && height > MIN_WINDOW_SIZE
            && x > -SLOP_MARGIN
            && y > -SLOP_MARGIN
            && ((c.w - width).abs() > 20
                || (c.h - height).abs() > 20
                || (c.x - x).abs() > 20
                || (c.y - y).abs() > 20)
    } else {
        false
    }
}

pub fn handle_monitor_switch(c_win: Window, x: i32, y: i32, width: i32, height: i32) {
    let new_mon = rect_to_mon(x, y, width, height);
    let current_mon = get_globals().selmon;

    if new_mon != current_mon {
        if let Some(target) = new_mon {
            send_mon(c_win, target);
            if let Some(cur) = current_mon {
                if let Some(cur_sel) = get_globals().monitors.get(cur).and_then(|m| m.sel) {
                    unfocus_win(cur_sel, false);
                }
            }
            let mut globals = get_globals_mut();
            globals.selmon = Some(target);
            drop(globals);
            focus(None);
        }
    }
}

pub fn handle_client_monitor_switch(c_win: Window) {
    let (c_x, c_y, c_w, c_h) = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&c_win) {
            (c.x, c.y, c.w, c.h)
        } else {
            return;
        }
    };

    let new_mon = rect_to_mon(c_x, c_y, c_w, c_h);
    let current_mon = get_globals().selmon;

    if new_mon != current_mon {
        if let Some(target) = new_mon {
            send_mon(c_win, target);
            if let Some(cur) = current_mon {
                if let Some(cur_sel) = get_globals().monitors.get(cur).and_then(|m| m.sel) {
                    unfocus_win(cur_sel, false);
                }
            }
            let mut globals = get_globals_mut();
            globals.selmon = Some(target);
            drop(globals);
            focus(None);
        }
    }
}

pub fn apply_window_resize(c_win: Window, x: i32, y: i32, width: i32, height: i32) {
    let is_floating = {
        let globals = get_globals();
        globals
            .clients
            .get(&c_win)
            .map(|c| c.isfloating)
            .unwrap_or(false)
    };

    if is_floating {
        resize(c_win, x, y, width, height, true);
    } else {
        toggle_floating(&Arg::default());
        resize(c_win, x, y, width, height, true);
    }
}

pub fn drag_tag(arg: &Arg) {
    let globals = get_globals();
    let tagwidth = if globals.tagwidth == 0 {
        get_tag_width()
    } else {
        globals.tagwidth
    };

    let current_tagset = if let Some(sel_mon_id) = globals.selmon {
        globals
            .monitors
            .get(sel_mon_id)
            .map(|m| m.tagset[m.seltags as usize])
    } else {
        None
    };

    if (arg.ui & globals.tagmask) != current_tagset.unwrap_or(0) {
        drop(globals);
        view(arg);
        return;
    }

    let sel_win = if let Some(sel_mon_id) = globals.selmon {
        globals.monitors.get(sel_mon_id).and_then(|m| m.sel)
    } else {
        None
    };

    let Some(_win) = sel_win else { return };

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let cursor = globals.cursors[2].as_ref().map(|c| c.cursor).unwrap_or(0);

        if conn
            .grab_pointer(
                false,
                globals.root,
                EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                x11rb::NONE,
                cursor,
                CURRENT_TIME,
            )
            .is_err()
        {
            return;
        }

        let mut cursor_on_bar = true;
        let mut last_tag: i32 = -1;
        let mut last_time: u32 = 0;
        let mut last_motion: Option<(i32, i32, u16)> = None;

        loop {
            let event = conn.wait_for_event();
            if let Ok(e) = event {
                match &e {
                    x11rb::protocol::Event::ButtonRelease(_) => break,
                    x11rb::protocol::Event::MotionNotify(m) => {
                        let m = m;
                        {
                            if m.time - last_time <= 1000 / REFRESH_RATE_LO {
                                continue;
                            }
                            last_time = m.time;

                            last_motion =
                                Some((m.event_x as i32, m.event_y as i32, u16::from(m.state)));

                            if m.event_y as i32 > {
                                let globals = get_globals();
                                if let Some(sel_mon_id) = globals.selmon {
                                    globals
                                        .monitors
                                        .get(sel_mon_id)
                                        .map(|m| m.by + globals.bh + 1)
                                        .unwrap_or(9999)
                                } else {
                                    9999
                                }
                            } {
                                cursor_on_bar = false;
                                break;
                            }

                            let tag_x = get_tag_at_x(m.event_x as i32);
                            if last_tag != tag_x {
                                last_tag = tag_x;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        let _ = ungrab_pointer(conn, CURRENT_TIME);
        let _ = conn.flush();

        if cursor_on_bar {
            if let Some((x, _, state)) = last_motion {
                let globals = get_globals();
                if x < globals
                    .monitors
                    .get(globals.selmon.unwrap())
                    .map(|m| m.mx)
                    .unwrap_or(0)
                    + tagwidth
                {
                    let tag_idx = get_tag_at_x(x);
                    if tag_idx >= 0 {
                        let tag_arg = Arg {
                            ui: 1 << tag_idx,
                            ..Default::default()
                        };
                        if (state as u32 & (ModMask::SHIFT.bits() as u32)) != 0 {
                            follow_tag(&tag_arg);
                        } else if (state as u32 & (ModMask::CONTROL.bits() as u32)) != 0 {
                            tag_all(&tag_arg);
                        } else {
                            tag(&tag_arg);
                        }
                    }
                }
            }
        }
    }
}

fn get_root_ptr() -> Option<(i32, i32)> {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if let Ok(cookie) = query_pointer(conn, globals.root) {
            if let Ok(reply) = cookie.reply() {
                return Some((reply.root_x as i32, reply.root_y as i32));
            }
        }
    }
    None
}

fn snap_to_monitor_edges(c: &ClientInner, nx: &mut i32, ny: &mut i32) {
    let globals = get_globals();
    let snap = globals.snap as i32;

    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get(sel_mon_id) {
            let width = c.w + 2 * c.border_width;
            let height = c.h + 2 * c.border_width;

            if (mon.wx - *nx).abs() < snap {
                *nx = mon.wx;
            } else if (mon.wx + mon.ww - (*nx + width)).abs() < snap {
                *nx = mon.wx + mon.ww - width;
            }

            if (mon.wy - *ny).abs() < snap {
                *ny = mon.wy;
            } else if (mon.wy + mon.wh - (*ny + height)).abs() < snap {
                *ny = mon.wy + mon.wh - height;
            }
        }
    }
}
