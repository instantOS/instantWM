use crate::client::resize_client_rect;
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::types::*;
use std::thread;
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::Window;

const DIR_LEFT: i32 = 0;
const DIR_RIGHT: i32 = 1;

pub fn ease_out_cubic(t: f64) -> f64 {
    let t = t - 1.0;
    1.0 + t * t * t
}

pub fn animate_client(win: Window, x: i32, y: i32, w: i32, h: i32, frames: i32, reset_pos: i32) {
    if frames <= 0 {
        return;
    }

    let (start_x, start_y, start_w, start_h) = {
        let globals = get_globals();
        if let Some(client) = globals.clients.get(&win) {
            let start_rect = if reset_pos != 0 {
                client.geo
            } else {
                client.old_geo
            };
            (start_rect.x, start_rect.y, start_rect.w, start_rect.h)
        } else {
            return;
        }
    };

    let (target_w, target_h) = if w != 0 { (w, h) } else { (start_w, start_h) };

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
        if !globals.animated {
            if target_w > 0 && target_h > 0 {
                drop(globals);
                resize_client_rect(
                    win,
                    &Rect {
                        x,
                        y,
                        w: target_w,
                        h: target_h,
                    },
                );
            }
            return;
        }

        let (mon_mw, mon_mh) = {
            if let Some(mon_id) = globals.clients.get(&win).and_then(|c| c.mon_id) {
                if let Some(mon) = globals.monitors.get(mon_id) {
                    (mon.monitor_rect.w, mon.monitor_rect.h)
                } else {
                    (0, 0)
                }
            } else {
                (0, 0)
            }
        };

        let actual_w = if target_w > mon_mw - 2 * 2 {
            mon_mw - 2 * 2
        } else {
            target_w
        };
        let actual_h = if target_h > mon_mh - 2 * 2 {
            mon_mh - 2 * 2
        } else {
            target_h
        };

        drop(globals);

        let dx = (x - start_x) as f64;
        let dy = (y - start_y) as f64;

        let dist_moved = (start_x - x).abs() > 10
            || (start_y - y).abs() > 10
            || (w - start_w).abs() > 10
            || (h - start_h).abs() > 10;

        if dist_moved {
            if x == start_x && y == start_y && start_w < mon_mw - 50 {
                let delta_w = actual_w - start_w;
                let delta_h = actual_h - start_h;
                animate_client(win, start_x + delta_w, start_y + delta_h, 0, 0, frames, 0);
            } else {
                for time in 1..=frames {
                    let progress = ease_out_cubic(time as f64 / frames as f64);
                    let step_x = (start_x as f64 + progress * dx) as i32;
                    let step_y = (start_y as f64 + progress * dy) as i32;

                    if actual_w > 0 && actual_h > 0 {
                        resize_client_rect(
                            win,
                            &Rect {
                                x: step_x,
                                y: step_y,
                                w: actual_w,
                                h: actual_h,
                            },
                        );
                    }

                    let _ = conn.flush();
                    thread::sleep(Duration::from_micros(15000));
                }
            }
        }

        if reset_pos != 0 {
            if actual_w > 0 && actual_h > 0 {
                resize_client_rect(
                    win,
                    &Rect {
                        x: start_x,
                        y: start_y,
                        w: actual_w,
                        h: actual_h,
                    },
                );
            }
        } else {
            if actual_w > 0 && actual_h > 0 {
                resize_client_rect(
                    win,
                    &Rect {
                        x,
                        y,
                        w: actual_w,
                        h: actual_h,
                    },
                );
            }
        }
    }
}

pub fn check_animate(win: Window, x: i32, y: i32, w: i32, h: i32, frames: i32, reset_pos: i32) {
    let globals = get_globals();
    if let Some(client) = globals.clients.get(&win) {
        let should_animate =
            client.geo.x != x || client.geo.y != y || client.geo.w != w || client.geo.h != h;
        drop(globals);
        if should_animate {
            animate_client(win, x, y, w, h, frames, reset_pos);
        }
    }
}

/// Animate a window using a Rect struct.
pub fn animate_client_rect(win: Window, rect: &Rect, frames: i32, reset_pos: i32) {
    animate_client(win, rect.x, rect.y, rect.w, rect.h, frames, reset_pos);
}

/// Check and animate a window using a Rect struct.
pub fn check_animate_rect(win: Window, rect: &Rect, frames: i32, reset_pos: i32) {
    check_animate(win, rect.x, rect.y, rect.w, rect.h, frames, reset_pos);
}

pub fn toggle_animated(_arg: &Arg) {
    let globals = get_globals_mut();
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

pub fn anim_left(arg: &Arg) {
    anim_scroll(arg, DIR_LEFT);
}

pub fn anim_right(arg: &Arg) {
    anim_scroll(arg, DIR_RIGHT);
}

fn anim_scroll(arg: &Arg, dir: i32) {
    let is_overview = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                crate::monitor::is_current_layout_tiling(mon, &globals.tags)
            } else {
                false
            }
        } else {
            false
        }
    };

    if is_overview {
        let focus_arg = Arg {
            ui: if dir == DIR_RIGHT {
                FOCUS_DIR_DOWN
            } else {
                FOCUS_DIR_UP
            },
            ..Default::default()
        };
        crate::focus::direction_focus(&focus_arg);
        return;
    }

    let (is_floating, has_tiling) = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                let is_floating = mon
                    .sel
                    .and_then(|w| globals.clients.get(&w).map(|c| c.isfloating))
                    .unwrap_or(false);
                let has_tiling = crate::monitor::is_current_layout_tiling(mon, &globals.tags);
                (is_floating, has_tiling)
            } else {
                (false, true)
            }
        } else {
            (false, true)
        }
    };

    if !has_tiling {
        if let Some(sel_win) = {
            let globals = get_globals();
            globals
                .selmon
                .and_then(|id| globals.monitors.get(id).and_then(|m| m.sel))
        } {
            let snap_dir = if dir == DIR_RIGHT { 1 } else { 3 };
            change_snap(sel_win, snap_dir);
        }
        return;
    }

    let current_tag = {
        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                mon.current_tag as u32
            } else {
                return;
            }
        } else {
            return;
        }
    };

    if current_tag == 0 {
        return;
    }

    if dir == DIR_LEFT && current_tag == 1 {
        return;
    }

    if dir == DIR_RIGHT && current_tag >= 20 {
        return;
    }

    let animated = {
        let globals = get_globals();
        globals.animated
    };

    if animated {
        let modifier: i32 = if dir == DIR_RIGHT { 1 } else { -1 };
        let target = current_tag + modifier as u32;

        let globals = get_globals();
        if let Some(sel_mon_id) = globals.selmon {
            if let Some(mon) = globals.monitors.get(sel_mon_id) {
                let mut current = mon.clients;
                while let Some(c_win) = current {
                    if let Some(c) = globals.clients.get(&c_win) {
                        if (c.tags & (1 << (target - 1))) != 0 && !c.isfloating {
                            let _ = std::mem::drop(());
                        }
                        current = c.next;
                    } else {
                        break;
                    }
                }
            }
        }
    }

    if dir == DIR_RIGHT {
        view_to_right(arg);
    } else {
        view_to_left(arg);
    }
}

fn change_snap(_win: Window, _dir: i32) {}

fn view_to_left(_arg: &Arg) {}
fn view_to_right(_arg: &Arg) {}

const FOCUS_DIR_UP: u32 = 0;
const FOCUS_DIR_DOWN: u32 = 2;
