use crate::animation::{animate_client_rect, check_animate_rect};
use crate::client::resize;
use crate::focus::warp_cursor_to_client;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::monitor::arrange;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

pub const SNAP_NONE: i32 = 0;
pub const SNAP_TOP: i32 = 1;
pub const SNAP_TOP_RIGHT: i32 = 2;
pub const SNAP_RIGHT: i32 = 3;
pub const SNAP_BOTTOM_RIGHT: i32 = 4;
pub const SNAP_BOTTOM: i32 = 5;
pub const SNAP_BOTTOM_LEFT: i32 = 6;
pub const SNAP_LEFT: i32 = 7;
pub const SNAP_TOP_LEFT: i32 = 8;
pub const SNAP_MAXIMIZED: i32 = 9;

static SNAP_MATRIX: [[i32; 4]; 10] = [
    [SNAP_MAXIMIZED, SNAP_RIGHT, SNAP_BOTTOM, SNAP_LEFT],
    [SNAP_MAXIMIZED, SNAP_TOP_RIGHT, SNAP_NONE, SNAP_TOP_LEFT],
    [SNAP_TOP_RIGHT, SNAP_TOP_RIGHT, SNAP_RIGHT, SNAP_TOP],
    [SNAP_TOP_RIGHT, SNAP_RIGHT, SNAP_BOTTOM_RIGHT, SNAP_NONE],
    [
        SNAP_RIGHT,
        SNAP_BOTTOM_RIGHT,
        SNAP_BOTTOM_RIGHT,
        SNAP_BOTTOM,
    ],
    [SNAP_NONE, SNAP_BOTTOM_RIGHT, SNAP_BOTTOM, SNAP_BOTTOM_LEFT],
    [SNAP_LEFT, SNAP_BOTTOM, SNAP_BOTTOM_LEFT, SNAP_BOTTOM_LEFT],
    [SNAP_TOP_LEFT, SNAP_NONE, SNAP_BOTTOM_LEFT, SNAP_LEFT],
    [SNAP_TOP_LEFT, SNAP_TOP, SNAP_LEFT, SNAP_TOP],
    [SNAP_TOP, SNAP_RIGHT, SNAP_NONE, SNAP_LEFT],
];

pub fn save_floating_win(win: Window) {
    let globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        client.float_geo = client.geo;
    }
}

pub fn restore_floating_win(win: Window) {
    let float_geo = {
        let globals = get_globals();
        globals.clients.get(&win).map(|c| c.float_geo)
    };
    if let Some(rect) = float_geo {
        resize(win, &rect, false);
    }
}

pub fn save_bw_win(win: Window) {
    let globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        if client.border_width != 0 {
            client.old_border_width = client.border_width;
        }
    }
}

pub fn restore_border_width_win(win: Window) {
    let globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        if client.old_border_width != 0 {
            client.border_width = client.old_border_width;
        }
    }
}

pub fn apply_size(win: Window) {
    let geo = {
        let globals = get_globals();
        globals.clients.get(&win).map(|c| c.geo)
    };
    if let Some(mut rect) = geo {
        rect.x += 1;
        resize(win, &rect, false);
    }
}

pub fn check_floating(win: Window) -> bool {
    let globals = get_globals();
    if let Some(client) = globals.clients.get(&win) {
        if client.isfloating {
            return true;
        }
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                if !crate::monitor::is_current_layout_tiling(mon, &globals.tags) {
                    return true;
                }
            }
        }
    }
    false
}

pub fn visible_client(win: Window) -> bool {
    let globals = get_globals();
    if let Some(client) = globals.clients.get(&win) {
        for mon in &globals.monitors {
            if (client.tags & mon.tagset[mon.seltags as usize]) != 0
                && client.mon_id == Some(globals.monitors.iter().position(|m| m == mon).unwrap())
            {
                return true;
            }
        }
    }
    false
}

pub fn save_all_floating(mon_id: Option<usize>) {
    let (numtags, _tagmask) = {
        let globals = get_globals();
        (globals.tags.count(), globals.tags.mask())
    };

    if let Some(mid) = mon_id {
        let mut to_save = Vec::new();
        {
            let globals = get_globals();
            if let Some(mon) = globals.monitors.get(mid) {
                for i in 1..=numtags {
                    if i > globals.tags.tags.len() {
                        break;
                    }
                    let tag_idx = i - 1;

                    let has_arrange = if tag_idx < globals.tags.tags.len() {
                        let tag = &globals.tags.tags[tag_idx];
                        if tag.sellt < 2 {
                            tag.ltidxs[tag.sellt as usize].is_some()
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if has_arrange {
                        continue;
                    }

                    let mut current = mon.clients;
                    while let Some(c_win) = current {
                        if let Some(c) = globals.clients.get(&c_win) {
                            if (c.tags & (1 << tag_idx)) != 0 && c.snapstatus == SnapPosition::None
                            {
                                to_save.push(c_win);
                            }
                            current = c.next;
                        } else {
                            break;
                        }
                    }
                }
            }
        }
        for win in to_save {
            save_floating_win(win);
        }
    }
}

pub fn restore_all_floating(mon_id: Option<usize>) {
    let numtags = {
        let globals = get_globals();
        globals.tags.count()
    };

    if let Some(mid) = mon_id {
        let mut to_restore = Vec::new();
        {
            let globals = get_globals();
            if let Some(mon) = globals.monitors.get(mid) {
                for i in 1..=numtags {
                    if i > globals.tags.tags.len() {
                        break;
                    }
                    let tag_idx = i - 1;

                    let has_arrange = if tag_idx < globals.tags.tags.len() {
                        let tag = &globals.tags.tags[tag_idx];
                        if tag.sellt < 2 {
                            tag.ltidxs[tag.sellt as usize].is_some()
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if has_arrange {
                        continue;
                    }

                    let mut current = mon.clients;
                    while let Some(c_win) = current {
                        if let Some(c) = globals.clients.get(&c_win) {
                            if (c.tags & (1 << tag_idx)) != 0 && c.snapstatus == SnapPosition::None
                            {
                                to_restore.push(c_win);
                            }
                            current = c.next;
                        } else {
                            break;
                        }
                    }
                }
            }
        }
        for win in to_restore {
            restore_floating_win(win);
        }
    }
}

pub fn reset_snap(win: Window) {
    let (is_floating, snapstatus, has_tiling) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            let has_tiling = if let Some(sel_mon_id) = globals.selmon {
                if let Some(mon) = globals.monitors.get(sel_mon_id) {
                    crate::monitor::is_current_layout_tiling(mon, &globals.tags)
                } else {
                    true
                }
            } else {
                true
            };
            (client.isfloating, client.snapstatus, has_tiling)
        } else {
            return;
        }
    };

    if snapstatus == SnapPosition::None {
        return;
    }

    if is_floating || !has_tiling {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.snapstatus = SnapPosition::None;
        }
        drop(globals);
        restore_border_width_win(win);
        restore_floating_win(win);
        apply_size(win);
    }
}

pub fn apply_snap(win: Window, mon_id: Option<usize>) {
    let (snapstatus, saved_geo, border_width) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            (client.snapstatus, client.float_geo, client.border_width)
        } else {
            return;
        }
    };

    if let Some(mid) = mon_id {
        let (m_mx, _m_my, m_mw, m_mh, m_wh, mony) = {
            let globals = get_globals();
            if let Some(m) = globals.monitors.get(mid) {
                let mony = m.monitor_rect.y + if m.showbar { globals.bh } else { 0 };
                (
                    m.monitor_rect.x,
                    m.monitor_rect.y,
                    m.monitor_rect.w,
                    m.monitor_rect.h,
                    m.work_rect.h,
                    mony,
                )
            } else {
                return;
            }
        };

        if snapstatus != SnapPosition::Maximized {
            restore_border_width_win(win);
        }

        match snapstatus {
            SnapPosition::None => {
                check_animate_rect(win, &saved_geo, 7, 0);
            }
            SnapPosition::Top => {
                check_animate_rect(
                    win,
                    &Rect {
                        x: m_mx,
                        y: mony,
                        w: m_mw,
                        h: m_mh / 2,
                    },
                    7,
                    0,
                );
            }
            SnapPosition::TopRight => {
                check_animate_rect(
                    win,
                    &Rect {
                        x: m_mx + m_mw / 2,
                        y: mony,
                        w: m_mw / 2,
                        h: m_mh / 2,
                    },
                    7,
                    0,
                );
            }
            SnapPosition::Right => {
                check_animate_rect(
                    win,
                    &Rect {
                        x: m_mx + m_mw / 2,
                        y: mony,
                        w: m_mw / 2 - border_width * 2,
                        h: m_wh - border_width * 2,
                    },
                    7,
                    0,
                );
            }
            SnapPosition::BottomRight => {
                check_animate_rect(
                    win,
                    &Rect {
                        x: m_mx + m_mw / 2,
                        y: mony + m_mh / 2,
                        w: m_mw / 2,
                        h: m_wh / 2,
                    },
                    7,
                    0,
                );
            }
            SnapPosition::Bottom => {
                check_animate_rect(
                    win,
                    &Rect {
                        x: m_mx,
                        y: mony + m_mh / 2,
                        w: m_mw,
                        h: m_mh / 2,
                    },
                    7,
                    0,
                );
            }
            SnapPosition::BottomLeft => {
                check_animate_rect(
                    win,
                    &Rect {
                        x: m_mx,
                        y: mony + m_mh / 2,
                        w: m_mw / 2,
                        h: m_wh / 2,
                    },
                    7,
                    0,
                );
            }
            SnapPosition::Left => {
                check_animate_rect(
                    win,
                    &Rect {
                        x: m_mx,
                        y: mony,
                        w: m_mw / 2,
                        h: m_wh,
                    },
                    7,
                    0,
                );
            }
            SnapPosition::TopLeft => {
                check_animate_rect(
                    win,
                    &Rect {
                        x: m_mx,
                        y: mony,
                        w: m_mw / 2,
                        h: m_mh / 2,
                    },
                    7,
                    0,
                );
            }
            SnapPosition::Maximized => {
                save_bw_win(win);
                let globals = get_globals_mut();
                if let Some(client) = globals.clients.get_mut(&win) {
                    client.border_width = 0;
                }
                drop(globals);
                check_animate_rect(
                    win,
                    &Rect {
                        x: m_mx,
                        y: mony,
                        w: m_mw - border_width * 2,
                        h: m_mh + border_width * 2,
                    },
                    7,
                    0,
                );

                let is_sel = {
                    let globals = get_globals();
                    if let Some(sel_mon_id) = globals.selmon {
                        globals.monitors.get(sel_mon_id).and_then(|mon| mon.sel) == Some(win)
                    } else {
                        false
                    }
                };

                if is_sel {
                    let x11 = get_x11();
                    if let Some(ref conn) = x11.conn {
                        let _ = configure_window(
                            conn,
                            win,
                            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
                        );
                        let _ = conn.flush();
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn change_snap(win: Window, snap_mode: i32) {
    let snapstatus = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            client.snapstatus
        } else {
            return;
        }
    };

    let tempsnap = match snapstatus {
        SnapPosition::None => 0,
        SnapPosition::Top => 1,
        SnapPosition::TopRight => 2,
        SnapPosition::Right => 3,
        SnapPosition::BottomRight => 4,
        SnapPosition::Bottom => 5,
        SnapPosition::BottomLeft => 6,
        SnapPosition::Left => 7,
        SnapPosition::TopLeft => 8,
        SnapPosition::Maximized => 9,
    };

    let new_snap = SNAP_MATRIX[tempsnap as usize][snap_mode as usize];

    let new_snap_pos = match new_snap {
        SNAP_NONE => SnapPosition::None,
        SNAP_TOP => SnapPosition::Top,
        SNAP_TOP_RIGHT => SnapPosition::TopRight,
        SNAP_RIGHT => SnapPosition::Right,
        SNAP_BOTTOM_RIGHT => SnapPosition::BottomRight,
        SNAP_BOTTOM => SnapPosition::Bottom,
        SNAP_BOTTOM_LEFT => SnapPosition::BottomLeft,
        SNAP_LEFT => SnapPosition::Left,
        SNAP_TOP_LEFT => SnapPosition::TopLeft,
        SNAP_MAXIMIZED => SnapPosition::Maximized,
        _ => SnapPosition::None,
    };

    if snapstatus == SnapPosition::None && check_floating(win) {
        save_floating_win(win);
    }

    let mon_id = {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.snapstatus = new_snap_pos;
            client.mon_id
        } else {
            return;
        }
    };

    apply_snap(win, mon_id);
    warp_cursor_to_client(win);
    crate::focus::focus(Some(win));
}

pub fn temp_fullscreen(_arg: &Arg) {
    let (fullscreen_win, sel_win, animated) = {
        let globals = get_globals();
        (
            globals
                .selmon
                .and_then(|id| globals.monitors.get(id).and_then(|m| m.fullscreen)),
            globals
                .selmon
                .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel)),
            globals.animated,
        )
    };

    if fullscreen_win.is_some() {
        let win = fullscreen_win.unwrap();
        let is_floating = {
            let globals = get_globals();
            globals
                .clients
                .get(&win)
                .map(|c| c.isfloating)
                .unwrap_or(false)
        };

        if is_floating || !has_tiling_layout() {
            restore_floating_win(win);
            apply_size(win);
        }

        let globals = get_globals_mut();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
                mon.fullscreen = None;
            }
        }
    } else {
        let Some(win) = sel_win else { return };

        let globals = get_globals_mut();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
                mon.fullscreen = Some(win);
            }
        }

        if check_floating(win) {
            save_floating_win(win);
        }
    }

    if animated {
        let globals = get_globals_mut();
        globals.animated = false;
        drop(globals);

        if let Some(sel_mon_id) = get_globals().selmon {
            arrange(Some(sel_mon_id));
        }

        let globals = get_globals_mut();
        globals.animated = true;
    } else {
        if let Some(sel_mon_id) = get_globals().selmon {
            arrange(Some(sel_mon_id));
        }
    }

    let fullscreen = {
        let globals = get_globals();
        globals
            .selmon
            .and_then(|id| globals.monitors.get(id).and_then(|m| m.fullscreen))
    };

    if let Some(win) = fullscreen {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = configure_window(
                conn,
                win,
                &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
            );
            let _ = conn.flush();
        }
    }
}

fn has_tiling_layout() -> bool {
    let globals = get_globals();
    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get(sel_mon_id) {
            return crate::monitor::is_current_layout_tiling(mon, &globals.tags);
        }
    }
    true
}

pub fn toggle_floating(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            let mon = match globals.monitors.get(sel_mon_id) {
                Some(m) => m,
                None => return,
            };

            if let Some(sel) = mon.sel {
                if Some(sel) == mon.overlay {
                    return;
                }
                if let Some(c) = globals.clients.get(&sel) {
                    if c.is_fullscreen && !c.isfakefullscreen {
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

    let is_fixed = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.isfixed)
            .unwrap_or(false)
    };

    let is_floating = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.isfloating)
            .unwrap_or(false)
    };

    let new_state = !is_floating || is_fixed;
    apply_float_change(win, new_state, true, true);

    if let Some(sel_mon_id) = get_globals().selmon {
        arrange(Some(sel_mon_id));
    }
}

fn apply_float_change(win: Window, floating: bool, animate: bool, update_borders: bool) {
    let x11 = get_x11();

    if floating {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.isfloating = true;
        }

        if update_borders {
            restore_border_width_win(win);
            if let Some(ref conn) = x11.conn {
                if let Some(ref scheme) = globals.borderscheme {
                    let pixel = scheme.float_focus.bg.color.pixel;
                    let _ = change_window_attributes(
                        conn,
                        win,
                        &ChangeWindowAttributesAux::new().border_pixel(Some(pixel as u32)),
                    );
                    let _ = conn.flush();
                }
            }
        }

        let saved_geo = {
            let globals = get_globals();
            globals.clients.get(&win).map(|c| c.float_geo)
        };

        let Some(saved_geo) = saved_geo else { return };

        if animate {
            animate_client_rect(win, &saved_geo, 7, 0);
        } else {
            resize(win, &saved_geo, false);
        }
    } else {
        let client_count = get_globals().clients.len();
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.isfloating = false;

            if update_borders {
                if client_count <= 1 && client.snapstatus == SnapPosition::None {
                    client.old_border_width = client.border_width;
                    client.border_width = 0;
                }
            }

            client.float_geo = client.geo;
        }
    }
}

pub fn set_floating(win: Window, should_arrange: bool) {
    let (is_fullscreen, is_fake_fullscreen, is_floating) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            (
                client.is_fullscreen,
                client.isfakefullscreen,
                client.isfloating,
            )
        } else {
            return;
        }
    };

    if is_fullscreen && !is_fake_fullscreen {
        return;
    }

    if is_floating {
        return;
    }

    apply_float_change(win, true, false, false);

    if should_arrange {
        if let Some(sel_mon_id) = get_globals().selmon {
            arrange(Some(sel_mon_id));
        }
    }
}

pub fn set_tiled(win: Window, should_arrange: bool) {
    let (is_fullscreen, is_fake_fullscreen, is_floating, is_fixed) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            (
                client.is_fullscreen,
                client.isfakefullscreen,
                client.isfloating,
                client.isfixed,
            )
        } else {
            return;
        }
    };

    if is_fullscreen && !is_fake_fullscreen {
        return;
    }

    if !is_floating && !is_fixed {
        return;
    }

    apply_float_change(win, false, false, false);

    if should_arrange {
        if let Some(sel_mon_id) = get_globals().selmon {
            arrange(Some(sel_mon_id));
        }
    }
}

pub fn change_floating_win(win: Window) {
    let (is_fullscreen, is_fake_fullscreen, is_floating, is_fixed) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            (
                client.is_fullscreen,
                client.isfakefullscreen,
                client.isfloating,
                client.isfixed,
            )
        } else {
            return;
        }
    };

    if is_fullscreen && !is_fake_fullscreen {
        return;
    }

    let new_state = !is_floating || is_fixed;
    apply_float_change(win, new_state, false, false);

    if let Some(sel_mon_id) = get_globals().selmon {
        arrange(Some(sel_mon_id));
    }
}

pub fn center_window(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            let mon = match globals.monitors.get(sel_mon_id) {
                Some(m) => m,
                None => return,
            };

            if let Some(sel) = mon.sel {
                if Some(sel) == mon.overlay {
                    return;
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

    let (w, h, is_floating) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            (client.geo.w, client.geo.h, client.isfloating)
        } else {
            return;
        }
    };

    if has_tiling_layout() && !is_floating {
        return;
    }

    let (mw, mh, showbar, mx, my) = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                (
                    mon.work_rect.w,
                    mon.work_rect.h,
                    mon.showbar,
                    mon.monitor_rect.x,
                    mon.monitor_rect.y,
                )
            } else {
                return;
            }
        } else {
            return;
        }
    };

    if w > mw || h > mh {
        return;
    }

    let bh = get_globals().bh;
    let y_offset = if showbar { bh } else { -bh };

    resize(
        win,
        &Rect {
            x: mx + (mw / 2) - (w / 2),
            y: my + (mh / 2) - (h / 2) + y_offset,
            w,
            h,
        },
        true,
    );
}

pub fn moveresize(arg: &Arg) {
    let direction = arg.i;

    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            globals.monitors.get(sel_mon_id).and_then(|m| m.sel)
        } else {
            None
        }
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

    if has_tiling_layout() && !is_floating {
        return;
    }

    let move_step = 40;
    let move_deltas: [[i32; 2]; 4] = [
        [0, move_step],  // Down
        [0, -move_step], // Up
        [move_step, 0],  // Right
        [-move_step, 0], // Left
    ];

    let dir_idx = direction.max(0).min(3) as usize;
    let mut nx = c_x + move_deltas[dir_idx][0];
    let mut ny = c_y + move_deltas[dir_idx][1];

    let (mon_mx, mon_my, mon_mw, mon_mh) = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                (
                    mon.monitor_rect.x,
                    mon.monitor_rect.y,
                    mon.monitor_rect.w,
                    mon.monitor_rect.h,
                )
            } else {
                return;
            }
        } else {
            return;
        }
    };

    if nx < mon_mx {
        nx = mon_mx;
    }
    if ny < mon_my {
        ny = mon_my;
    }
    if ny + c_h > mon_my + mon_mh {
        ny = (mon_mh + mon_my) - c_h - border_width * 2;
    }
    if nx + c_w > mon_mx + mon_mw {
        nx = (mon_mw + mon_mx) - c_w - border_width * 2;
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
    warp_cursor_to_client(win);
}

pub fn key_resize(arg: &Arg) {
    let direction = arg.i;

    let sel_win = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            globals.monitors.get(sel_mon_id).and_then(|m| m.sel)
        } else {
            None
        }
    };

    let Some(win) = sel_win else { return };

    let (is_floating, c_x, c_y, c_w, c_h) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            (
                client.isfloating,
                client.geo.x,
                client.geo.y,
                client.geo.w,
                client.geo.h,
            )
        } else {
            return;
        }
    };

    reset_snap(win);

    if has_tiling_layout() && !is_floating {
        return;
    }

    warp_cursor_to_client(win);

    let resize_step = 40;
    let resize_deltas: [[i32; 2]; 4] = [
        [0, resize_step],  // TallerDown
        [0, -resize_step], // ShorterUp
        [resize_step, 0],  // WiderRight
        [-resize_step, 0], // NarrowerLeft
    ];

    let dir_idx = direction.max(0).min(3) as usize;
    let nw = c_w + resize_deltas[dir_idx][0];
    let nh = c_h + resize_deltas[dir_idx][1];

    resize(
        win,
        &Rect {
            x: c_x,
            y: c_y,
            w: nw,
            h: nh,
        },
        true,
    );
}

pub fn upscale_client(arg: &Arg) {
    let sel_win = if arg.v.is_none() {
        let globals = get_globals();
        globals
            .selmon
            .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel))
    } else {
        arg.v.map(|v| v as Window)
    };

    if let Some(win) = sel_win {
        scale_client_win(win, 30);
    }
}

pub fn downscale_client(arg: &Arg) {
    let sel_win = if arg.v.is_none() {
        let globals = get_globals();
        globals
            .selmon
            .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel))
    } else {
        arg.v.map(|v| v as Window)
    };

    let Some(win) = sel_win else { return };

    let is_floating = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| c.isfloating)
            .unwrap_or(false)
    };

    if !is_floating {
        crate::focus::focus(Some(win));
        toggle_floating(&Arg::default());
    }

    scale_client_win(win, -30);
}

pub fn scale_client_win(win: Window, scale: i32) {
    let (is_floating, c_x, c_y, c_w, c_h) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            (
                client.isfloating,
                client.geo.x,
                client.geo.y,
                client.geo.w,
                client.geo.h,
            )
        } else {
            return;
        }
    };

    if !is_floating {
        return;
    }

    let (mon_mx, mon_my, mon_mw, mon_mh, bh) = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                (
                    mon.monitor_rect.x,
                    mon.monitor_rect.y,
                    mon.monitor_rect.w,
                    mon.monitor_rect.h,
                    globals.bh,
                )
            } else {
                return;
            }
        } else {
            return;
        }
    };

    let mut w = c_w + scale;
    let mut h = c_h + scale;
    let mut x = c_x - scale / 2;
    let mut y = c_y - scale / 2;

    if x < mon_mx {
        x = mon_mx;
    }
    if w > mon_mw {
        w = mon_mw;
    }
    if h > mon_mh {
        h = mon_mh;
    }
    if h + y > mon_my + mon_mh {
        y = mon_mh - h;
    }
    if y < bh {
        y = bh;
    }

    animate_client_rect(win, &Rect { x, y, w, h }, 3, 0);
}

pub fn apply_snap_mut(c: &mut Client, m: &MonitorInner) {
    let _mony = m.monitor_rect.y + if m.showbar { 0 } else { 0 };

    match c.snapstatus {
        SnapPosition::None => {}
        SnapPosition::Maximized => {
            c.border_width = 0;
        }
        _ => {}
    }
}

pub fn reset_sticky(_c: &mut Client) {}

pub fn set_border_width(_arg: &Arg) {}

pub fn distribute_clients(_arg: &Arg) {}

pub fn toggle_fullscreen_overview(_arg: &Arg) {}

pub fn toggle_overview(_arg: &Arg) {}

pub fn up_press(_arg: &Arg) {}

pub fn down_press(_arg: &Arg) {}
