use crate::bar::draw_bar;
use crate::client::resize;
use crate::globals::{get_globals, get_globals_mut};
use crate::keyboard::grab_keys;
use crate::tags::get_tag_width;
use crate::types::*;

pub fn ctrl_toggle(value: &mut bool, arg: u32) {
    if arg == 0 || arg == 2 {
        *value = !*value;
    } else {
        *value = arg != 1;
    }
}

pub fn toggle_alt_tag(arg: &Arg) {
    let new_value = {
        let globals = get_globals();
        let mut showalttag = globals.tags.show_alt;
        ctrl_toggle(&mut showalttag, arg.ui);
        showalttag
    };

    {
        let globals = get_globals_mut();
        globals.tags.show_alt = new_value;
    }

    let monitors: Vec<usize> = {
        let globals = get_globals();
        globals
            .monitors
            .iter()
            .enumerate()
            .map(|(i, _)| i)
            .collect()
    };

    for i in monitors {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(i) {
            draw_bar(mon);
        }
    }

    let tagwidth = get_tag_width();
    let globals = get_globals_mut();
    globals.tags.width = tagwidth;
}

pub fn alt_tab_free(arg: &Arg) {
    ctrl_toggle(&mut get_globals_mut().tags.prefix, arg.ui);
    grab_keys();
}

pub fn toggle_sticky(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        let selmon_id = globals.selmon;
        globals.monitors.get(selmon_id).and_then(|m| m.sel)
    };

    let Some(win) = sel_win else { return };

    let mon_id = {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.issticky = !client.issticky;
            client.mon_id
        } else {
            return;
        }
    };

    if let Some(mid) = mon_id {
        crate::monitor::arrange(Some(mid));
    }
}

pub fn toggle_prefix(_arg: &Arg) {
    let globals = get_globals_mut();
    globals.tags.prefix = !globals.tags.prefix;

    let selmon_id = get_globals().selmon;
    let globals = get_globals_mut();
    if let Some(mon) = globals.monitors.get_mut(selmon_id) {
        draw_bar(mon);
    }
}

pub fn toggle_animated(arg: &Arg) {
    let globals = get_globals_mut();
    ctrl_toggle(&mut globals.animated, arg.ui);
}

pub fn set_border_width(arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        let selmon_id = globals.selmon;
        globals.monitors.get(selmon_id).and_then(|m| m.sel)
    };

    let Some(win) = sel_win else { return };

    let (old_bw, _mon_id) = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            (c.border_width, c.mon_id)
        } else {
            return;
        }
    };

    let new_bw = arg.i;
    let d = old_bw - new_bw;

    {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.border_width = new_bw;
        }
    }

    let geo = {
        let globals = get_globals();
        if let Some(c) = globals.clients.get(&win) {
            Rect {
                x: c.geo.x,
                y: c.geo.y,
                w: c.geo.w + 2 * d,
                h: c.geo.h + 2 * d,
            }
        } else {
            return;
        }
    };

    resize(win, &geo, false);
}

pub fn toggle_focus_follows_mouse(arg: &Arg) {
    ctrl_toggle(&mut get_globals_mut().focusfollowsmouse, arg.ui);
}

pub fn toggle_focus_follows_float_mouse(arg: &Arg) {
    ctrl_toggle(&mut get_globals_mut().focusfollowsfloatmouse, arg.ui);
}

pub fn toggle_double_draw(_arg: &Arg) {
    let globals = get_globals_mut();
    globals.doubledraw = !globals.doubledraw;
}

pub fn toggle_locked(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        let selmon_id = globals.selmon;
        globals.monitors.get(selmon_id).and_then(|m| m.sel)
    };

    let Some(win) = sel_win else { return };

    let _mon_id = {
        let globals = get_globals_mut();
        if let Some(client) = globals.clients.get_mut(&win) {
            client.islocked = !client.islocked;
            client.mon_id
        } else {
            return;
        }
    };

    {
        let selmon_id = get_globals().selmon;
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(selmon_id) {
            draw_bar(mon);
        }
    }
}

pub fn toggle_show_tags(arg: &Arg) {
    let (selmon_id, new_showtags) = {
        let globals = get_globals();
        let selmon_id = globals.selmon;

        let mut showtags = if let Some(mon) = globals.monitors.get(selmon_id) {
            mon.showtags
        } else {
            0
        };

        let mut show_bool = showtags != 0;
        ctrl_toggle(&mut show_bool, arg.ui);
        showtags = if show_bool { 1 } else { 0 };

        (selmon_id, showtags)
    };

    {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(selmon_id) {
            mon.showtags = new_showtags;
        }
    }

    let tagwidth = get_tag_width();
    let globals = get_globals_mut();
    globals.tags.width = tagwidth;

    if let Some(mon) = globals.monitors.get_mut(selmon_id) {
        draw_bar(mon);
    }
}

pub fn hide_window(_arg: &Arg) {
    let sel_win = {
        let globals = get_globals();
        globals.monitors.get(globals.selmon).and_then(|m| m.sel)
    };

    let Some(win) = sel_win else { return };

    crate::client::hide(win);
}

pub fn unhide_all(_arg: &Arg) {
    let clients: Vec<x11rb::protocol::xproto::Window> = {
        let globals = get_globals();
        globals.clients.keys().copied().collect()
    };

    for win in clients {
        crate::client::show(win);
    }
}

pub fn redraw_win(_arg: &Arg) {
    let monitors: Vec<usize> = {
        let globals = get_globals();
        globals
            .monitors
            .iter()
            .enumerate()
            .map(|(i, _)| i)
            .collect()
    };

    for i in monitors {
        let globals = get_globals_mut();
        if let Some(mon) = globals.monitors.get_mut(i) {
            draw_bar(mon);
        }
    }
}
