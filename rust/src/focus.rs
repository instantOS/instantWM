use crate::bar::draw_bars;
use crate::client::{set_focus, set_urgent, unfocus_win};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::tags::view;
use crate::types::*;
use crate::util::get_sel_win;
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
        // `mon.sel` can already be set before this call (e.g. manage path),
        // but X input focus may still point to PointerRoot.
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
        // Match dwm behavior: don't force root focus before selecting the new client.
        unfocus_win(cur_win, false);
    }

    {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(sel_mon_id) {
            mon.sel = target;
            // Reset transient hover gestures on focus change (matching C behavior).
            // Overlay is persistent state, not a hover; leave it alone.
            if !matches!(mon.gesture, Gesture::None | Gesture::Overlay) {
                mon.gesture = Gesture::None;
            }
        }
    }

    draw_bars();

    if let Some(w) = target.take() {
        // Clear urgent flag when focusing a window (matches C behavior)
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

/// Set focus to a specific window.
/// This is a low-level focus operation that sets the X input focus
/// and updates the active window property.
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

/// Focus the client in the given direction.
///
/// # Arguments
/// * `direction` - The direction to focus (Up, Down, Left, Right)
pub fn focus_direction(direction: Direction) {
    let (sel_mon_id, source_win) = {
        let globals = get_globals();
        if globals.monitors.is_empty() {
            return;
        }
        let sel_mon_id = globals.selmon;
        if let Some(sel) = get_sel_win() {
            (sel_mon_id, sel)
        } else {
            return;
        }
    };

    let (sx, sy) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&source_win) {
            (
                client.geo.x + client.geo.w / 2,
                client.geo.y + client.geo.h / 2,
            )
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
            if !c.is_visible() {
                current = c.next;
                continue;
            }

            let cx = c.geo.x + c.geo.w / 2;
            let cy = c.geo.y + c.geo.h / 2;

            let skip = c_win == source_win
                || (direction == Direction::Up && cy > sy)
                || (direction == Direction::Right && cx < sx)
                || (direction == Direction::Down && cy < sy)
                || (direction == Direction::Left && cx > sx);

            if skip {
                current = c.next;
                continue;
            }

            let score = match direction {
                Direction::Up | Direction::Down => {
                    let dist_x = (sx - cx).abs();
                    let dist_y = (sy - cy).abs();
                    if dist_x > dist_y {
                        current = c.next;
                        continue;
                    }
                    dist_x + dist_y / 4
                }
                Direction::Left | Direction::Right => {
                    let dist_x = (sx - cx).abs();
                    let dist_y = (sy - cy).abs();
                    if dist_y > dist_x {
                        current = c.next;
                        continue;
                    }
                    dist_y + dist_x / 4
                }
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

    if let Some(c) = out_client {
        if found_one {
            focus(Some(c));
        }
    }
}

/// Legacy wrapper for key bindings. Use `focus_direction` for new code.
pub fn direction_focus(arg: &Arg) {
    if let Some(dir) = Direction::from_index(arg.ui) {
        focus_direction(dir);
    }
}

pub fn focus_last_client(_arg: &Arg) {
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

pub fn warp_to_focus(_arg: &Arg) {
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

/// Focus the next/previous client in the stack.
///
/// # Arguments
/// * `forward` - If true, focus the next client; if false, focus the previous.
pub fn focus_stack_direction(forward: bool) {
    let globals = get_globals();
    if globals.monitors.is_empty() {
        return;
    }

    let (selmon_id, sel_win) = {
        let globals = get_globals();
        let selmon_id = globals.selmon;
        let sel_win = get_sel_win();
        (selmon_id, sel_win)
    };

    let mut stack: Vec<Window> = Vec::new();
    {
        let globals = get_globals();
        if let Some(mon) = globals.monitors.get(selmon_id) {
            let mut current = mon.stack;
            while let Some(c_win) = current {
                if let Some(c) = globals.clients.get(&c_win) {
                    if c.is_visible() {
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

    let next_idx = if forward {
        (current_idx + 1) % stack.len()
    } else if current_idx == 0 {
        stack.len() - 1
    } else {
        current_idx - 1
    };

    focus(Some(stack[next_idx]));
}

/// Legacy wrapper for key bindings. Use `focus_stack_direction` for new code.
pub fn focus_stack(arg: &Arg) {
    focus_stack_direction(arg.i > 0);
}
