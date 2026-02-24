use crate::bar::draw_bars;
use crate::client::{set_focus, set_urgent, unfocus_win};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::tags::view;
use crate::types::*;
use crate::util::{self, get_sel_win};
use std::sync::atomic::Ordering;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;
use x11rb::CURRENT_TIME;

pub const FOCUS_DIR_UP: u32 = 0;
pub const FOCUS_DIR_RIGHT: u32 = 1;
pub const FOCUS_DIR_DOWN: u32 = 2;
pub const FOCUS_DIR_LEFT: u32 = 3;

pub fn focus(win: Option<Window>) {
    let (sel_mon_id, current_sel, mut target, root, net_active_window) = {
        let globals = get_globals();
        if globals.monitors.is_empty() {
            return;
        }
        let sel_mon_id = globals.selmon;
        let Some(mon) = globals.monitors.get(sel_mon_id) else {
            return;
        };

        let mut target = win.filter(|w| {
            globals
                .clients
                .get(w)
                .map(|c| c.is_visible() && !c.is_hidden)
                .unwrap_or(false)
        });

        if target.is_none() {
            let mut stack = mon.stack;
            while let Some(c_win) = stack {
                let Some(c) = globals.clients.get(&c_win) else {
                    break;
                };
                if c.is_visible() && !c.is_hidden {
                    target = Some(c_win);
                    break;
                }
                stack = c.snext;
            }
        }

        (
            sel_mon_id,
            mon.sel,
            target,
            globals.root,
            globals.netatom.active_window,
        )
    };

    if current_sel == target {
        if let Some(w) = target {
            set_focus(w);
        } else {
            let x11 = get_x11();
            if let Some(ref conn) = x11.conn {
                let _ = conn.set_input_focus(InputFocus::POINTER_ROOT, root, CURRENT_TIME);
                let _ = conn.delete_property(root, net_active_window);
                let _ = conn.flush();
            }
        }
        return;
    }

    if let Some(cur_win) = current_sel {
        unfocus_win(cur_win, false);
    }

    {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
            mon.sel = target;
            if !matches!(mon.gesture, Gesture::None | Gesture::Overlay) {
                mon.gesture = Gesture::None;
            }
        }
    }

    draw_bars();

    if let Some(w) = target.take() {
        let is_urgent = {
            let globals = get_globals();
            globals.clients.get(&w).map(|c| c.isurgent).unwrap_or(false)
        };
        if is_urgent {
            set_urgent(w, false);
        }
        set_focus(w);
    } else {
        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = conn.set_input_focus(InputFocus::POINTER_ROOT, root, CURRENT_TIME);
            let _ = conn.delete_property(root, net_active_window);
            let _ = conn.flush();
        }
    }
}

pub fn set_focus_win(win: Window) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            if !c.neverfocus {
                let _ = conn.set_input_focus(InputFocus::POINTER_ROOT, win, CURRENT_TIME);
                let _ = conn.change_property32(
                    PropMode::REPLACE,
                    globals.root,
                    globals.netatom.active_window,
                    AtomEnum::WINDOW,
                    &[win],
                );
            }
            let _ = conn.flush();
        }
    }
}

pub fn focus_direction(direction: Direction) {
    let Some(sel_mon_id) = util::get_sel_mon() else {
        return;
    };
    let Some(source_win) = get_sel_win() else {
        return;
    };

    let globals = get_globals();
    let Some(source_client) = globals.clients.get(&source_win) else {
        return;
    };

    let (source_center_x, source_center_y) = source_client.geo.center();

    let Some(mon) = globals.monitors.get(sel_mon_id) else {
        return;
    };

    let candidates = get_directional_candidates(
        mon.clients,
        &globals,
        source_win,
        source_center_x,
        source_center_y,
        direction,
    );

    if let Some(target) = candidates {
        focus(Some(target));
    }
}

fn get_directional_candidates(
    clients: Option<Window>,
    globals: &crate::globals::Globals,
    source_win: Window,
    source_center_x: i32,
    source_center_y: i32,
    direction: Direction,
) -> Option<Window> {
    let mut out_client: Option<Window> = None;
    let mut min_score: i32 = 0;

    let mut current = clients;
    while let Some(c_win) = current {
        let Some(c) = globals.clients.get(&c_win) else {
            break;
        };

        if !c.is_visible() {
            current = c.next;
            continue;
        }

        let center_x = c.geo.x + c.geo.w / 2;
        let center_y = c.geo.y + c.geo.h / 2;

        if is_client_in_direction(
            c_win,
            source_win,
            center_x,
            center_y,
            source_center_x,
            source_center_y,
            direction,
        ) {
            let score = calculate_direction_score(
                center_x,
                center_y,
                source_center_x,
                source_center_y,
                direction,
            );
            if score < min_score || min_score == 0 {
                out_client = Some(c_win);
                min_score = score;
            }
        }

        current = c.next;
    }

    out_client
}

fn is_client_in_direction(
    c_win: Window,
    source_win: Window,
    center_x: i32,
    center_y: i32,
    source_center_x: i32,
    source_center_y: i32,
    direction: Direction,
) -> bool {
    if c_win == source_win {
        return false;
    }

    match direction {
        Direction::Up => center_y < source_center_y,
        Direction::Down => center_y > source_center_y,
        Direction::Left => center_x < source_center_x,
        Direction::Right => center_x > source_center_x,
    }
}

fn calculate_direction_score(
    center_x: i32,
    center_y: i32,
    source_center_x: i32,
    source_center_y: i32,
    direction: Direction,
) -> i32 {
    let dist_x = (source_center_x - center_x).abs();
    let dist_y = (source_center_y - center_y).abs();

    match direction {
        Direction::Up | Direction::Down => {
            if dist_x > dist_y {
                return i32::MAX;
            }
            dist_x + dist_y / 4
        }
        Direction::Left | Direction::Right => {
            if dist_y > dist_x {
                return i32::MAX;
            }
            dist_y + dist_x / 4
        }
    }
}

pub fn direction_focus(dir_index: u32) {
    if let Some(dir) = Direction::from_index(dir_index) {
        focus_direction(dir);
    }
}

pub fn focus_last_client() {
    let last_client_win = crate::client::LAST_CLIENT.load(Ordering::Relaxed);
    if last_client_win == 0 {
        return;
    }
    let last_win = last_client_win;

    let globals = get_globals();
    let last_client = match globals.clients.get(&last_win) {
        Some(c) => c.clone(),
        None => return,
    };

    if last_client.is_scratchpad() {
        crate::scratchpad::scratchpad_show_name(&last_client.scratchpad_name);
        return;
    }

    let tags = last_client.tags;
    let last_mon_id = last_client.mon_id;

    if let Some(last_mid) = last_mon_id {
        let globals = get_globals();
        let sel_mon_id = globals.selmon;
        if !globals.monitors.is_empty() && sel_mon_id != last_mid {
            if let Some(sel) = globals.monitors.get(sel_mon_id).and_then(|m| m.sel) {
                unfocus_win(sel, false);
                let globals = get_globals_mut();
                globals.selmon = last_mid;
            }
        }
    }

    if let Some(cur) = get_sel_win() {
        crate::client::LAST_CLIENT.store(cur, Ordering::Relaxed);
    }

    view(tags);
    focus(Some(last_win));

    let mon_id = {
        let globals = get_globals();
        globals.selmon
    };
    crate::monitor::arrange(Some(mon_id));
}

pub fn warp(c_win: Window) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&c_win) {
            if let Some(_cursor_x) = get_root_ptr() {
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

pub fn force_warp(c_win: Window) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&c_win) {
            let _ = conn.warp_pointer(
                CURRENT_TIME,
                c.win,
                0,
                0,
                0,
                0,
                (c.geo.w / 2) as i16,
                10_i16,
            );
            let _ = conn.flush();
        }
    }
}

pub fn warp_cursor_to_client(c_win: Window) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let root = globals.root;
        let bh = globals.bh;

        if c_win == 0 {
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

        if let Some(c) = globals.clients.get(&c_win) {
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

pub fn warp_into(c_win: Window) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let root = globals.root;

        if let Some(c) = globals.clients.get(&c_win) {
            if let Some((mut x, mut y)) = get_root_ptr() {
                if x < c.geo.x {
                    x = c.geo.x + 10;
                } else if x > c.geo.x + c.geo.w {
                    x = c.geo.x + c.geo.w - 10;
                }

                if y < c.geo.y {
                    y = c.geo.y + 10;
                } else if y > c.geo.y + c.geo.h {
                    y = c.geo.y + c.geo.h - 10;
                }

                let _ = conn.warp_pointer(CURRENT_TIME, root, 0, 0, 0, 0, x as i16, y as i16);
                let _ = conn.flush();
            }
        }
    }
}

pub fn warp_to_focus() {
    if let Some(win) = get_sel_win() {
        warp_cursor_to_client(win);
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

pub fn focus_stack_direction(forward: bool) {
    let Some(sel_mon_id) = util::get_sel_mon() else {
        return;
    };
    let sel_win = get_sel_win();

    let globals = get_globals();
    let stack = get_visible_stack(sel_mon_id, &globals);

    if stack.is_empty() {
        return;
    }

    let current_idx = match sel_win {
        Some(w) => stack.iter().position(|&win| win == w).unwrap_or(0),
        None => 0,
    };

    let next_idx = if forward {
        (current_idx + 1) % stack.len()
    } else if current_idx == 0 {
        stack.len() - 1
    } else {
        current_idx - 1
    };

    focus(Some(stack[next_idx]));
}

fn get_visible_stack(sel_mon_id: MonitorId, globals: &crate::globals::Globals) -> Vec<Window> {
    let mut stack = Vec::new();

    let Some(mon) = globals.monitors.get(sel_mon_id) else {
        return stack;
    };

    let mut current = mon.stack;
    while let Some(c_win) = current {
        let Some(c) = globals.clients.get(&c_win) else {
            break;
        };
        if c.is_visible() {
            stack.push(c_win);
        }
        current = c.snext;
    }

    stack
}

pub fn focus_stack(direction: i32) {
    focus_stack_direction(direction > 0);
}
