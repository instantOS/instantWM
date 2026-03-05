use crate::bar::draw_bar;
use crate::client::resize;
use crate::contexts::{CoreCtx, X11Ctx};
use crate::keyboard::grab_keys_x11;
use crate::tags::get_tag_width;
use crate::types::*;

pub fn ctrl_toggle(value: &mut bool, action: ToggleAction) {
    action.apply(value);
}

pub fn toggle_alt_tag(core: &mut CoreCtx, x11: &X11Ctx, action: ToggleAction) {
    let new_value = {
        let mut showalttag = core.g.tags.show_alt;
        ctrl_toggle(&mut showalttag, action);
        showalttag
    };

    core.g.tags.show_alt = new_value;

    let monitors: Vec<usize> = core.g.monitors.iter().enumerate().map(|(i, _)| i).collect();

    for i in monitors {
        draw_bar(core, x11, i);
    }

    let tagwidth = get_tag_width(core);
    core.g.tags.width = tagwidth;
}

pub fn alt_tab_free(core: &mut CoreCtx, x11: &X11Ctx, action: ToggleAction) {
    ctrl_toggle(&mut core.g.tags.prefix, action);
    grab_keys_x11(core, x11);
}

pub fn toggle_sticky(core: &mut CoreCtx, win: WindowId) {
    let monitor_id = if let Some(client) = core.g.clients.get_mut(&win) {
        client.issticky = !client.issticky;
        client.monitor_id
    } else {
        return;
    };

    if let Some(mid) = monitor_id {
        crate::layouts::arrange(core, Some(mid));
    }
}

pub fn toggle_prefix(core: &mut CoreCtx, x11: &X11Ctx) {
    core.g.tags.prefix = !core.g.tags.prefix;

    let selmon_id = core.g.selected_monitor_id();
    draw_bar(core, x11, selmon_id);
}

pub fn toggle_animated(core: &mut CoreCtx, action: ToggleAction) {
    ctrl_toggle(&mut core.g.animated, action);
}

pub fn set_border_width(core: &mut CoreCtx, win: WindowId, width: i32) {
    let (old_bw, _mon_id) = {
        if let Some(c) = core.g.clients.get(&win) {
            (c.border_width, c.monitor_id)
        } else {
            return;
        }
    };

    let new_bw = width;
    let d = old_bw - new_bw;

    {
        if let Some(client) = core.g.clients.get_mut(&win) {
            client.border_width = new_bw;
        }
    }

    let geo = {
        if let Some(c) = core.g.clients.get(&win) {
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

    resize(core, win, &geo, false);
}

pub fn toggle_focus_follows_mouse(core: &mut CoreCtx, action: ToggleAction) {
    ctrl_toggle(&mut core.g.focusfollowsmouse, action);
}

pub fn toggle_focus_follows_float_mouse(core: &mut CoreCtx, action: ToggleAction) {
    ctrl_toggle(&mut core.g.focusfollowsfloatmouse, action);
}

pub fn toggle_double_draw(core: &mut CoreCtx) {
    core.g.doubledraw = !core.g.doubledraw;
}

pub fn toggle_locked(core: &mut CoreCtx, x11: &X11Ctx, win: WindowId) {
    let _mon_id = {
        if let Some(client) = core.g.clients.get_mut(&win) {
            client.islocked = !client.islocked;
            client.monitor_id
        } else {
            return;
        }
    };

    let selmon_id = core.g.selected_monitor_id();
    draw_bar(core, x11, selmon_id);
}

pub fn toggle_show_tags(core: &mut CoreCtx, x11: &X11Ctx, action: ToggleAction) {
    let (selmon_id, new_showtags) = {
        let selmon_id = core.g.selected_monitor_id();

        let showtags = core.g.selected_monitor().showtags;

        let mut show_bool = showtags != 0;
        ctrl_toggle(&mut show_bool, action);
        let new_showtags = if show_bool { 1 } else { 0 };

        (selmon_id, new_showtags)
    };

    core.g.selected_monitor_mut().showtags = new_showtags;

    let tagwidth = get_tag_width(core);
    core.g.tags.width = tagwidth;

    draw_bar(core, x11, selmon_id);
}

pub fn hide_window(ctx: &mut crate::contexts::WmCtx, win: WindowId) {
    crate::client::hide(ctx, win);
}

pub fn unhide_all(ctx: &mut crate::contexts::WmCtx) {
    let clients: Vec<WindowId> = ctx.g().clients.keys().copied().collect();

    for win in clients {
        crate::client::show(ctx, win);
    }
}

pub fn redraw_win(core: &mut CoreCtx, x11: &X11Ctx) {
    let monitors: Vec<usize> = core.g.monitors.iter().enumerate().map(|(i, _)| i).collect();

    for i in monitors {
        draw_bar(core, x11, i);
    }
}
