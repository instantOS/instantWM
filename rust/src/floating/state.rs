//! Floating state transitions and geometry persistence.

use crate::animation::animate_client;
use crate::client::{resize, restore_border_width};
use crate::globals::{get_globals, get_globals_mut, get_x11};
use crate::layouts::arrange;
use crate::types::*;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

pub fn set_floating_in_place(win: Window) {
    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.isfloating = true;
        }
    }

    restore_border_width(win);

    let x11 = get_x11();
    if let Some(ref conn) = x11.conn {
        let globals = get_globals();
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

pub fn apply_float_change(win: Window, floating: bool, animate: bool, update_borders: bool) {
    if floating {
        {
            let globals = get_globals_mut();
            if let Some(client) = globals.clients.get_mut(&win) {
                client.isfloating = true;
            }
        }

        if update_borders {
            restore_border_width(win);

            let x11 = get_x11();
            if let Some(ref conn) = x11.conn {
                let globals = get_globals();
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
            animate_client(
                win,
                &Rect {
                    x: saved_geo.x,
                    y: saved_geo.y,
                    w: saved_geo.w,
                    h: saved_geo.h,
                },
                7,
                0,
            );
        } else {
            resize(win, &saved_geo, false);
        }
    } else {
        let client_count = get_globals().clients.len();
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.isfloating = false;
            client.float_geo = client.geo;

            if update_borders && client_count <= 1 && client.snapstatus == SnapPosition::None {
                if client.border_width != 0 {
                    client.old_border_width = client.border_width;
                }
                client.border_width = 0;
            }
        }
    }
}

pub fn toggle_floating() {
    let sel_win = {
        let globals = get_globals();
        let mon = match globals.monitors.get(globals.selmon) {
            Some(m) => m,
            None => return,
        };
        match mon.sel {
            Some(sel) if Some(sel) != mon.overlay => {
                if let Some(c) = globals.clients.get(&sel) {
                    if c.is_fullscreen && !c.isfakefullscreen {
                        return;
                    }
                }
                Some(sel)
            }
            _ => None,
        }
    };

    let Some(win) = sel_win else { return };

    let (is_floating, is_fixed) = {
        let globals = get_globals();
        globals
            .clients
            .get(&win)
            .map(|c| (c.isfloating, c.isfixed))
            .unwrap_or((false, false))
    };

    let new_state = !is_floating || is_fixed;
    apply_float_change(win, new_state, true, true);
    arrange(Some(get_globals().selmon));
}

pub fn change_floating_win(win: Window) {
    let (is_fullscreen, is_fake_fullscreen, is_floating, is_fixed) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.is_fullscreen, c.isfakefullscreen, c.isfloating, c.isfixed),
            None => return,
        }
    };

    if is_fullscreen && !is_fake_fullscreen {
        return;
    }

    let new_state = !is_floating || is_fixed;
    apply_float_change(win, new_state, false, false);
    arrange(Some(get_globals().selmon));
}

pub fn set_floating(win: Window, should_arrange: bool) {
    let (is_fullscreen, is_fake_fullscreen, is_floating) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.is_fullscreen, c.isfakefullscreen, c.isfloating),
            None => return,
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
        arrange(Some(get_globals().selmon));
    }
}

pub fn set_tiled(win: Window, should_arrange: bool) {
    let (is_fullscreen, is_fake_fullscreen, is_floating, is_fixed) = {
        let globals = get_globals();
        match globals.clients.get(&win) {
            Some(c) => (c.is_fullscreen, c.isfakefullscreen, c.isfloating, c.isfixed),
            None => return,
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
        arrange(Some(get_globals().selmon));
    }
}

pub fn temp_fullscreen() {
    let (fullscreen_win, sel_win, animated) = {
        let globals = get_globals();
        let mon = match globals.monitors.get(globals.selmon) {
            Some(m) => m,
            None => return,
        };
        (mon.fullscreen, mon.sel, globals.animated)
    };

    if let Some(win) = fullscreen_win {
        let is_floating = {
            let globals = get_globals();
            globals
                .clients
                .get(&win)
                .map(|c| c.isfloating)
                .unwrap_or(false)
        };

        if is_floating || !super::helpers::has_tiling_layout() {
            restore_floating_win(win);
            super::helpers::apply_size(win);
        }

        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
            mon.fullscreen = None;
        }
    } else {
        let Some(win) = sel_win else { return };

        {
            let globals = get_globals_mut();
            if let Some(mon) = globals.monitors.get_mut(globals.selmon) {
                mon.fullscreen = Some(win);
            }
        }

        if super::helpers::check_floating(win) {
            save_floating_win(win);
        }
    }

    if animated {
        get_globals_mut().animated = false;
        arrange(Some(get_globals().selmon));
        get_globals_mut().animated = true;
    } else {
        arrange(Some(get_globals().selmon));
    }

    if let Some(win) = get_globals()
        .monitors
        .get(get_globals().selmon)
        .and_then(|m| m.fullscreen)
    {
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
