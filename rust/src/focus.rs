use crate::client::{is_visible, set_focus, unfocus_win};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::CURRENT_TIME;

pub const FOCUS_DIR_UP: u32 = 0;
pub const FOCUS_DIR_RIGHT: u32 = 1;
pub const FOCUS_DIR_DOWN: u32 = 2;
pub const FOCUS_DIR_LEFT: u32 = 3;

pub fn focus(win: Option<Window>) {
    let globals = get_globals();

    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get(sel_mon_id) {
            let current_sel = mon.sel;

            drop(globals);

            if win == current_sel {
                return;
            }

            if let Some(cur_win) = current_sel {
                unfocus_win(cur_win, true);
            }
        }
    }

    let mut globals = get_globals_mut();

    if let Some(sel_mon_id) = globals.selmon {
        if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
            mon.sel = win;
        }
    }

    if let Some(w) = win {
        if let Some(_client) = globals.clients.get(&w) {
            drop(globals);
            set_focus(w);
        }
    }
}

pub fn unfocus(_win: Window, _set_focus: bool) {}

pub fn set_focus_win(_win: Window) {}

pub fn direction_focus(arg: &Arg) {
    let direction = arg.ui;

    let (sel_mon_id, source_win) = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                if let Some(sel) = mon.sel {
                    (sel_mon_id, sel)
                } else {
                    return;
                }
            } else {
                return;
            }
        } else {
            return;
        }
    };

    let (sx, sy) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&source_win) {
            (client.x + client.w / 2, client.y + client.h / 2)
        } else {
            return;
        }
    };

    let mut out_client: Option<Window> = None;
    let mut min_score: i32 = 0;
    let mut found_one = false;

    let globals = get_globals();

    let current_mon = match globals.monitors.get(sel_mon_id) {
        Some(m) => m,
        None => return,
    };

    let mut current = current_mon.clients;
    while let Some(c_win) = current {
        if let Some(c) = globals.clients.get(&c_win) {
            if !is_visible(c) {
                current = c.next;
                continue;
            }

            let cx = c.x + c.w / 2;
            let cy = c.y + c.h / 2;

            let skip = c_win == source_win
                || (direction == FOCUS_DIR_UP && cy > sy)
                || (direction == FOCUS_DIR_RIGHT && cx < sx)
                || (direction == FOCUS_DIR_DOWN && cy < sy)
                || (direction == FOCUS_DIR_LEFT && cx > sx);

            if skip {
                current = c.next;
                continue;
            }

            let score = if direction % 2 == 0 {
                let dist_x = (sx - cx).abs();
                let dist_y = (sy - cy).abs();
                if dist_x > dist_y {
                    current = c.next;
                    continue;
                }
                dist_x + dist_y / 4
            } else {
                let dist_x = (sx - cx).abs();
                let dist_y = (sy - cy).abs();
                if dist_y > dist_x {
                    current = c.next;
                    continue;
                }
                dist_y + dist_x / 4
            };

            if score < min_score || min_score == 0 {
                out_client = Some(c_win);
                found_one = true;
                min_score = score;
            }

            current = c.next;
        } else {
            break;
        }
    }

    drop(globals);

    if let Some(c) = out_client {
        if found_one {
            focus(Some(c));
        }
    }
}

pub fn focus_last_client(_arg: &Arg) {
    let last_client_win = unsafe { crate::client::LAST_CLIENT };

    let last_win = match last_client_win {
        Some(w) => w,
        None => return,
    };

    let globals = get_globals();
    let last_client = match globals.clients.get(&last_win) {
        Some(c) => c.clone(),
        None => return,
    };

    if last_client.is_scratchpad() {
        drop(globals);
        let name = last_client.scratchpad_name;
        let arg = Arg {
            v: Some(unsafe { std::mem::transmute::<*const u8, usize>(name.as_ptr()) }),
            ..Default::default()
        };
        crate::scratchpad::scratchpad_show(&arg);
        return;
    }

    let tags = last_client.tags;
    let last_mon_id = last_client.mon_id;

    drop(globals);

    if let Some(last_mid) = last_mon_id {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if sel_mon_id != last_mid {
                if let Some(sel) = globals.monitors.get(sel_mon_id).and_then(|m| m.sel) {
                    drop(globals);
                    unfocus_win(sel, false);
                    let mut globals = get_globals_mut();
                    globals.selmon = Some(last_mid);
                }
            }
        }
    }

    let current_sel = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            globals.monitors.get(sel_mon_id).and_then(|m| m.sel)
        } else {
            None
        }
    };

    if let Some(cur) = current_sel {
        unsafe {
            crate::client::LAST_CLIENT = Some(cur);
        }
    }

    let arg = Arg {
        ui: tags,
        ..Default::default()
    };
    view(&arg);
    focus(Some(last_win));

    let mon_id = {
        let globals = get_globals();
        globals.selmon
    };
    if let Some(mid) = mon_id {
        crate::monitor::arrange(Some(mid));
    }
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
                    (c.w / 2) as i16,
                    (c.h / 2) as i16,
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
            let _ = conn.warp_pointer(CURRENT_TIME, c.win, 0, 0, 0, 0, (c.w / 2) as i16, 10 as i16);
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

        if let Some(c) = globals.clients.get(&c_win) {
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

pub fn warp_into(c_win: Window) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        let root = globals.root;

        if let Some(c) = globals.clients.get(&c_win) {
            if let Some((mut x, mut y)) = get_root_ptr() {
                if x < c.x {
                    x = c.x + 10;
                } else if x > c.x + c.w {
                    x = c.x + c.w - 10;
                }

                if y < c.y {
                    y = c.y + 10;
                } else if y > c.y + c.h {
                    y = c.y + c.h - 10;
                }

                let _ = conn.warp_pointer(CURRENT_TIME, root, 0, 0, 0, 0, x as i16, y as i16);
                let _ = conn.flush();
            }
        }
    }
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

fn view(_arg: &Arg) {}

pub fn focus_stack(arg: &Arg) {
    let direction = arg.i;

    let sel_win = {
        let globals = get_globals();
        let selmon_id = match globals.selmon {
            Some(id) => id,
            None => return,
        };

        let mon = match globals.monitors.get(selmon_id) {
            Some(m) => m,
            None => return,
        };

        mon.sel
    };

    let selmon_id = {
        let globals = get_globals();
        globals.selmon.unwrap_or(0)
    };

    let mut stack: Vec<Window> = Vec::new();
    {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(selmon_id) {
            let mut current = mon.stack;
            while let Some(c_win) = current {
                if let Some(c) = globals.clients.get(&c_win) {
                    if is_visible(c) {
                        stack.push(c_win);
                    }
                    current = c.snext;
                } else {
                    break;
                }
            }
        }
    }

    if stack.is_empty() {
        return;
    }

    let current_idx = match sel_win {
        Some(w) => stack.iter().position(|&win| win == w).unwrap_or(0),
        None => 0,
    };

    let next_idx = if direction > 0 {
        (current_idx + 1) % stack.len()
    } else {
        if current_idx == 0 {
            stack.len() - 1
        } else {
            current_idx - 1
        }
    };

    focus(Some(stack[next_idx]));
}
