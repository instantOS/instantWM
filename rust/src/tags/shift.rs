//! Moving clients between tags.

use crate::focus::focus;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::layouts::arrange;
use crate::types::{Direction, OverlayMode, Rect};
use crate::util::get_sel_win;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConfigureWindowAux, ConnectionExt, StackMode};

pub fn tag_to_left_by(offset: i32) {
    shift_tag(Direction::Left, offset.max(1));
}

pub fn tag_to_right_by(offset: i32) {
    shift_tag(Direction::Right, offset.max(1));
}

pub fn tag_to_left() {
    tag_to_left_by(1);
}

pub fn tag_to_right() {
    tag_to_right_by(1);
}

pub fn move_left() {
    tag_to_left();
    crate::tags::view::view_to_left();
}

pub fn move_right() {
    tag_to_right();
    crate::tags::view::view_to_right();
}

fn shift_tag(dir: Direction, offset: i32) {
    let sel_win = get_sel_win();
    let Some(win) = sel_win else { return };

    let (current_tag, overlay_win) = {
        let globals = get_globals();
        let current_tag = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.current_tag as u32);
        let overlay = globals.monitors.get(globals.selmon).and_then(|m| m.overlay);
        (current_tag, overlay)
    };

    let Some(current_tag) = current_tag else {
        return;
    };

    if Some(win) == overlay_win {
        let mode = match dir {
            Direction::Left => OverlayMode::Left,
            Direction::Right => OverlayMode::Right,
            Direction::Up => OverlayMode::Top,
            Direction::Down => OverlayMode::Bottom,
        };
        crate::overlay::set_overlay_mode(mode);
        return;
    }

    if dir == Direction::Left && current_tag <= 1 {
        return;
    }
    if dir == Direction::Right && current_tag >= 20 {
        return;
    }

    let (tagset, tagmask) = {
        let globals = get_globals();
        let Some(mon) = globals.monitors.get(globals.selmon) else {
            return;
        };
        (mon.tagset[mon.seltags as usize], globals.tags.mask())
    };

    if (tagset & tagmask).count_ones() != 1 {
        return;
    }

    clear_sticky(win);

    if get_globals().animated {
        play_slide_animation(win, dir);
    }

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            match dir {
                Direction::Left if tagset > 1 => {
                    client.tags >>= offset;
                }
                Direction::Right if (tagset & (tagmask >> 1)) != 0 => {
                    client.tags <<= offset;
                }
                _ => return,
            }
        }
    }

    focus(None);
    arrange(Some(get_globals().selmon));
}

fn clear_sticky(win: x11rb::protocol::xproto::Window) {
    let target_tags = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|mon| {
            if mon.current_tag > 0 {
                Some(1u32 << (mon.current_tag - 1))
            } else {
                None
            }
        })
    };

    let globals = get_globals_mut();
    if let Some(client) = globals.clients.get_mut(&win) {
        if client.issticky {
            client.issticky = false;
            if let Some(tags) = target_tags {
                client.tags = tags;
            }
        }
    }
}

fn play_slide_animation(win: x11rb::protocol::xproto::Window, dir: Direction) {
    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let _ = conn.configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        let _ = conn.flush();
    }

    let (mon_w, client_x, client_y) = {
        let globals = get_globals();
        let mon_w = globals
            .monitors
            .get(globals.selmon)
            .map(|m| m.monitor_rect.w)
            .unwrap_or(0);
        let (client_x, client_y) = globals
            .clients
            .get(&win)
            .map(|c| (c.geo.x, c.geo.y))
            .unwrap_or((0, 0));
        (mon_w, client_x, client_y)
    };

    let anim_dx = (mon_w / 10)
        * match dir {
            Direction::Left => -1,
            Direction::Right => 1,
            Direction::Up => -1,
            Direction::Down => 1,
        };

    crate::animation::animate_client(
        win,
        &Rect {
            x: client_x + anim_dx,
            y: client_y,
            w: 0,
            h: 0,
        },
        7,
        0,
    );
}
