use crate::animation::animate_client_rect;
use crate::bar::draw_bar;
use crate::client::{resize, unfocus_win};
use crate::floating::{reset_snap, toggle_floating, SNAP_LEFT, SNAP_RIGHT, SNAP_TOP};
use crate::focus::{focus, warp_into};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::{rect_to_mon_rect, send_mon};
use crate::tags::{follow_tag, get_tag_at_x, get_tag_width, tag, tag_all, view};
use crate::types::*;
use crate::util::spawn;
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
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
    };

    let Some(win) = sel_win else { return };

    let (is_floating, c_x, c_y, c_w, c_h, border_width) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            (
                client.isfloating,
                client.geo.x,
                client.geo.y,
                client.geo.w,
                client.geo.h,
                client.border_width,
            )
        } else {
            return;
        }
    };

    let has_tiling = {
        let globals = get_globals();
        globals
            .monitors
            .get(globals.selmon)
            .map(|mon| crate::monitor::is_current_layout_tiling(mon, &globals.tags))
            .unwrap_or(true)
    };

    if has_tiling && !is_floating {
        return;
    }

    let (mon_mx, mon_my, mon_mw, mon_mh, mon_ww, mon_wh, bh) = {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(globals.selmon) {
            (
                mon.monitor_rect.x,
                mon.monitor_rect.y,
                mon.monitor_rect.w,
                mon.monitor_rect.h,
                mon.work_rect.w,
                mon.work_rect.h,
                globals.bh,
            )
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
    warp_cursor_to_client_impl(win);
}

pub fn get_cursor_client() -> Option<Client> {
    let globals = get_globals();

    let (x, y) = get_root_ptr()?;

    for mon in &globals.monitors {
        let mut current = mon.clients;
        while let Some(c_win) = current {
            if let Some(c) = globals.clients.get(&c_win) {
                if c.geo.contains_point(x, y) {
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

pub fn warp(c: &Client) {
    warp_cursor_to_client_impl(c.win);
}

pub fn force_warp(c: &Client) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.warp_pointer(
            x11rb::NONE,
            c.win,
            0i16,
            0i16,
            0u16,
            0u16,
            (c.geo.w / 2) as i16,
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
            let globals = get_globals();
            if !globals.monitors.is_empty() {
                if let Some(mon) = globals.monitors.get(globals.selmon) {
                    let _ = conn.warp_pointer(
                        CURRENT_TIME,
                        root,
                        0,
                        0,
                        0,
                        0,
                        (mon.work_rect.x + mon.work_rect.w / 2) as i16,
                        (mon.work_rect.y + mon.work_rect.h / 2) as i16,
                    );
                    let _ = conn.flush();
                }
            }
            return;
        }

        if let Some(c) = globals.clients.get(&win) {
            if let Some((x, y)) = get_root_ptr() {
                let in_window = c.geo.contains_point(x, y)
                    || (x > c.geo.x - c.border_width
                        && y > c.geo.y - c.border_width
                        && x < c.geo.x + c.geo.w + c.border_width * 2
                        && y < c.geo.y + c.geo.h + c.border_width * 2);

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
                    (c.geo.w / 2) as i16,
                    (c.geo.h / 2) as i16,
                );
                let _ = conn.flush();
            }
        }
    }
}

pub fn warp_cursor_to_client_win(c: &Client) {
    warp_cursor_to_client_impl(c.win);
}

pub fn warp_to_focus(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
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
        if let Some(mon) = globals.monitors.get(globals.selmon) {
            if let Some(sel) = mon.sel {
                if let Some(c) = globals.clients.get(&sel) {
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
                }
                Some(sel)
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
        let saved = (c.float_geo.x, c.float_geo.y, c.float_geo.w, c.float_geo.h);
        let has_tiling = globals
            .monitors
            .get(globals.selmon)
            .map(|m| crate::monitor::is_current_layout_tiling(m, &globals.tags))
            .unwrap_or(true);
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
            let mon = globals.monitors.get(globals.selmon).unwrap();
            let bh = globals.bh;
            if c.geo.x >= mon.monitor_rect.x - MAX_UNMAXIMIZE_OFFSET
                && c.geo.y >= mon.monitor_rect.y + bh - MAX_UNMAXIMIZE_OFFSET
                && c.geo.w >= mon.monitor_rect.w - MAX_UNMAXIMIZE_OFFSET
                && c.geo.h >= mon.monitor_rect.h - MAX_UNMAXIMIZE_OFFSET
            {
                resize(
                    win,
                    &Rect {
                        x: saved_x,
                        y: saved_y,
                        w: saved_w,
                        h: saved_h,
                    },
                    false,
                );
            }
            (
                c.geo.x,
                c.geo.y,
                c.geo.w,
                c.geo.h,
                mon.monitor_rect.w,
                mon.monitor_rect.h,
                mon.monitor_rect.x,
                mon.monitor_rect.y,
                bh,
            )
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
                .map(|c| (c.geo.x, c.geo.y))
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
                        let snap = globals.snap;
                        let c = globals.clients.get(&win);
                        if let Some(client) = c {
                            let width = client.geo.total_width(client.border_width);
                            let height = client.geo.total_height(client.border_width);

                            let mut adj_nx = nx;
                            let mut adj_ny = ny;

                            if let Some(mon) = globals.monitors.get(globals.selmon) {
                                if (mon.work_rect.x - nx).abs() < snap {
                                    adj_nx = mon.work_rect.x;
                                } else if (mon.work_rect.x + mon.work_rect.w - (nx + width)).abs()
                                    < snap
                                {
                                    adj_nx = mon.work_rect.x + mon.work_rect.w - width;
                                }
                                if (mon.work_rect.y - ny).abs() < snap {
                                    adj_ny = mon.work_rect.y;
                                } else if (mon.work_rect.y + mon.work_rect.h - (ny + height)).abs()
                                    < snap
                                {
                                    adj_ny = mon.work_rect.y + mon.work_rect.h - height;
                                }
                            }

                            let has_tiling = globals
                                .monitors
                                .get(globals.selmon)
                                .map(|m| crate::monitor::is_current_layout_tiling(m, &globals.tags))
                                .unwrap_or(true);

                            if !client.isfloating
                                && has_tiling
                                && ((nx - client.geo.x).abs() > snap
                                    || (ny - client.geo.y).abs() > snap)
                            {
                                toggle_floating(&Arg::default());
                            } else if !has_tiling || client.isfloating {
                                resize(
                                    win,
                                    &Rect {
                                        x: adj_nx,
                                        y: adj_ny,
                                        w: client.geo.w,
                                        h: client.geo.h,
                                    },
                                    true,
                                );
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
    if let Some(mon) = globals.monitors.get(globals.selmon) {
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
    }
    0
}

pub fn resize_mouse(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
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
                (c.geo.x, c.geo.y, c.geo.x + c.geo.w, c.geo.y + c.geo.h)
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
                        let snap = globals.snap;
                        if let Some(client) = globals.clients.get(&win) {
                            let has_tiling = globals
                                .monitors
                                .get(globals.selmon)
                                .map(|m| crate::monitor::is_current_layout_tiling(m, &globals.tags))
                                .unwrap_or(true);

                            if !client.isfloating
                                && has_tiling
                                && ((nw - client.geo.w).abs() > snap
                                    || (nh - client.geo.h).abs() > snap)
                            {
                                toggle_floating(&Arg::default());
                            } else if !has_tiling || client.isfloating {
                                resize(
                                    win,
                                    &Rect {
                                        x: client.geo.x,
                                        y: client.geo.y,
                                        w: nw,
                                        h: nh,
                                    },
                                    true,
                                );
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
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
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
                (c.geo.x, c.geo.y)
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

                            resize(
                                win,
                                &Rect {
                                    x: client.geo.x,
                                    y: client.geo.y,
                                    w: nw,
                                    h: nh,
                                },
                                true,
                            );
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
                            if let Some(mon) = globals.monitors.get(globals.selmon) {
                                let threshold = mon.monitor_rect.h / 30;
                                if (last_y - m.event_y as i32).abs() > threshold {
                                    spawn(&Arg::default());
                                    last_y = m.event_y as i32;
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
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
    };

    let Some(win) = sel_win else { return false };

    let (is_floating, has_tiling, c_x, c_y, c_w, c_h) = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            let has_tiling = globals
                .monitors
                .get(globals.selmon)
                .map(|m| crate::monitor::is_current_layout_tiling(m, &globals.tags))
                .unwrap_or(true);
            (c.isfloating, has_tiling, c.geo.x, c.geo.y, c.geo.w, c.geo.h)
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
    if let Some(mon) = globals.monitors.get(globals.selmon) {
        if mon.showbar && y < mon.monitor_rect.y + bh {
            return false;
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
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
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
        let was_focused = globals.monitors.get(globals.selmon).and_then(|m| m.sel) == Some(win);
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
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
    };

    let Some(win) = sel_win else { return };

    let output = std::process::Command::new("instantslop")
        .arg("-f")
        .arg("x%xx%yx%wx%hx")
        .output();

    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let dims = parse_slop_output(&stdout);

        if let Some(rect) = dims {
            if rect.w > MIN_WINDOW_SIZE && rect.h > MIN_WINDOW_SIZE {
                let globals = get_globals();
                if let Some(c) = globals.clients.get(&win) {
                    let is_different = (c.geo.w - rect.w).abs() > 20
                        || (c.geo.h - rect.h).abs() > 20
                        || (c.geo.x - rect.x).abs() > 20
                        || (c.geo.y - rect.y).abs() > 20;

                    if is_different {
                        handle_monitor_switch(win, &rect);

                        let is_floating = {
                            let globals = get_globals();
                            globals
                                .clients
                                .get(&win)
                                .map(|c| c.isfloating)
                                .unwrap_or(false)
                        };

                        if is_floating {
                            resize(win, &rect, true);
                        } else {
                            toggle_floating(&Arg::default());
                            resize(win, &rect, true);
                        }
                    }
                }
            }
        }
    }
}

/// Parse slop output and return a Rect.
pub fn parse_slop_output(output: &str) -> Option<Rect> {
    let parts: Vec<&str> = output.split('x').collect();
    if parts.len() < 5 {
        return None;
    }

    let x = parts.get(1)?.parse().ok()?;
    let y = parts.get(2)?.parse().ok()?;
    let w = parts.get(3)?.parse().ok()?;
    let h = parts.get(4)?.trim_end().parse().ok()?;

    Some(Rect { x, y, w, h })
}

pub fn is_valid_window_size(x: i32, y: i32, width: i32, height: i32, c_win: Window) -> bool {
    let globals = get_globals();
    if let Some(c) = globals.clients.get(&c_win) {
        width > MIN_WINDOW_SIZE
            && height > MIN_WINDOW_SIZE
            && x > -SLOP_MARGIN
            && y > -SLOP_MARGIN
            && ((c.geo.w - width).abs() > 20
                || (c.geo.h - height).abs() > 20
                || (c.geo.x - x).abs() > 20
                || (c.geo.y - y).abs() > 20)
    } else {
        false
    }
}

pub fn is_valid_window_size_rect(rect: &Rect, c_win: Window) -> bool {
    is_valid_window_size(rect.x, rect.y, rect.w, rect.h, c_win)
}

/// Handle monitor switch when a window is moved/resized to a different monitor.
pub fn handle_monitor_switch(c_win: Window, rect: &Rect) {
    let new_mon = rect_to_mon_rect(rect);
    let current_mon = get_globals().selmon;

    if new_mon != Some(current_mon) {
        if let Some(target) = new_mon {
            send_mon(c_win, target);
            {
                let globals = get_globals();
                if let Some(cur_sel) = globals.monitors.get(current_mon).and_then(|m| m.sel) {
                    unfocus_win(cur_sel, false);
                }
            }
            let globals = get_globals_mut();
            globals.selmon = target;
            focus(None);
        }
    }
}

/// Handle monitor switch for a client based on its current geometry.
pub fn handle_client_monitor_switch(c_win: Window) {
    let rect = {
        let globals = get_globals();
        match globals.clients.get(&c_win) {
            Some(c) => c.geo,
            None => return,
        }
    };

    handle_monitor_switch(c_win, &rect);
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

    let rect = Rect {
        x,
        y,
        w: width,
        h: height,
    };
    if is_floating {
        resize(c_win, &rect, true);
    } else {
        toggle_floating(&Arg::default());
        resize(c_win, &rect, true);
    }
}

pub fn apply_window_resize_rect(c_win: Window, rect: &Rect) {
    apply_window_resize(c_win, rect.x, rect.y, rect.w, rect.h);
}

pub fn drag_tag(arg: &Arg) {
    let globals = get_globals();
    let tagwidth = if globals.tags.width == 0 {
        get_tag_width()
    } else {
        globals.tags.width
    };

    let current_tagset = globals
        .monitors
        .get(globals.selmon)
        .map(|m| m.tagset[m.seltags as usize]);

    if (arg.ui & globals.tags.mask()) != current_tagset.unwrap_or(0) {
        view(arg);
        return;
    }

    let sel_win = globals.monitors.get(globals.selmon).and_then(|m| m.sel);

    let Some(_win) = sel_win else { return };

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let selmon_id = globals.selmon;
        let mon_mx = globals
            .monitors
            .get(selmon_id)
            .map(|m| m.monitor_rect.x)
            .unwrap_or(0);
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
                                globals
                                    .monitors
                                    .get(globals.selmon)
                                    .map(|m| m.by + globals.bh + 1)
                                    .unwrap_or(9999)
                            } {
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
                                    mon.gesture = Gesture::from_tag_index(tag_x as usize)
                                        .unwrap_or(Gesture::None);
                                    draw_bar(mon);
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
                        if (state as u32 & (ModMask::SHIFT.bits() as u32)) != 0 {
                            tag(&tag_arg);
                        } else if (state as u32 & (ModMask::CONTROL.bits() as u32)) != 0 {
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

fn snap_to_monitor_edges(c: &Client, nx: &mut i32, ny: &mut i32) {
    let globals = get_globals();
    let snap = globals.snap;

    if let Some(mon) = globals.monitors.get(globals.selmon) {
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
}
