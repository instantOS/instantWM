use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::types::*;
use x11rb::protocol::xproto::Window;
use x11rb::protocol::xproto::*;
use x11rb::protocol::xproto::*;

pub fn animate_client(win: Window, x: i32, y: i32, w: i32, h: i32, frames: i32, reset_pos: i32) {
    if frames <= 0 {
        return;
    }

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if !globals.animated {
            if w > 0 && h > 0 {
                drop(globals);
                crate::client::resize_client(win, x, y, w, h);
            }
            return;
        }
    }

    let start_x: i32;
    let start_y: i32;
    let start_w: i32;
    let start_h: i32;

    {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            start_x = if reset_pos != 0 {
                client.x
            } else {
                client.oldx
            };
            start_y = if reset_pos != 0 {
                client.y
            } else {
                client.oldy
            };
            start_w = if reset_pos != 0 {
                client.w
            } else {
                client.oldw
            };
            start_h = if reset_pos != 0 {
                client.h
            } else {
                client.oldh
            };
        } else {
            return;
        }
    }

    let dx = (x - start_x) as f32 / frames as f32;
    let dy = (y - start_y) as f32 / frames as f32;
    let dw = (w - start_w) as f32 / frames as f32;
    let dh = (h - start_h) as f32 / frames as f32;

    for i in 1..=frames {
        let step_x = (start_x as f32 + dx * i as f32) as i32;
        let step_y = (start_y as f32 + dy * i as f32) as i32;
        let step_w = (start_w as f32 + dw * i as f32) as i32;
        let step_h = (start_h as f32 + dh * i as f32) as i32;

        if step_w > 0 && step_h > 0 {
            crate::client::resize_client(win, step_x, step_y, step_w, step_h);
        }

        let x11 = get_x11();
        if let Some(ref conn) = x11.conn {
            let _ = conn.flush();
        }
    }

    if w > 0 && h > 0 {
        crate::client::resize_client(win, x, y, w, h);
    }
}

pub fn check_animate(win: Window, x: i32, y: i32, w: i32, h: i32, frames: i32, reset_pos: i32) {
    let globals = get_globals();
    if globals.animated {
        drop(globals);
        animate_client(win, x, y, w, h, frames, reset_pos);
    } else {
        drop(globals);
        crate::client::resize_client(win, x, y, w, h);
    }
}

pub fn toggle_animated(_arg: &Arg) {
    let mut globals = get_globals_mut();
    globals.animated = !globals.animated;
}

pub fn up_scale_client(arg: &Arg) {
    let scale = arg.i.max(1);
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

    if let Some(win) = sel_win {
        crate::client::scale_client(win, scale);
    }
}

pub fn down_scale_client(arg: &Arg) {
    let scale = arg.i.max(1);
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

    if let Some(win) = sel_win {
        crate::client::scale_client(win, 100 / scale);
    }
}
