use crate::bar::draw_bar;
use crate::client::resize;
use crate::contexts::WmCtx;
use crate::keyboard::grab_keys;
use crate::tags::get_tag_width;
use crate::types::*;

pub fn ctrl_toggle(value: &mut bool, action: ToggleAction) {
    action.apply(value);
}

pub fn toggle_alt_tag(ctx: &mut WmCtx, action: ToggleAction) {
    let new_value = {
        let mut showalttag = ctx.g.tags.show_alt;
        ctrl_toggle(&mut showalttag, action);
        showalttag
    };

    ctx.g.tags.show_alt = new_value;

    let monitors: Vec<usize> = ctx.g.monitors.iter().enumerate().map(|(i, _)| i).collect();

    for i in monitors {
        draw_bar(ctx, i);
    }

    let tagwidth = get_tag_width(ctx);
    ctx.g.tags.width = tagwidth;
}

pub fn alt_tab_free(ctx: &mut WmCtx, action: ToggleAction) {
    ctrl_toggle(&mut ctx.g.tags.prefix, action);
    grab_keys(ctx);
}

pub fn toggle_sticky(ctx: &mut WmCtx, win: WindowId) {
    let monitor_id = if let Some(client) = ctx.g.clients.get_mut(&win) {
        client.issticky = !client.issticky;
        client.monitor_id
    } else {
        return;
    };

    if let Some(mid) = monitor_id {
        crate::layouts::arrange(ctx, Some(mid));
    }
}

pub fn toggle_prefix(ctx: &mut WmCtx) {
    ctx.g.tags.prefix = !ctx.g.tags.prefix;

    let selmon_id = ctx.g.selected_monitor_id();
    draw_bar(ctx, selmon_id);
}

pub fn toggle_animated(ctx: &mut WmCtx, action: ToggleAction) {
    ctrl_toggle(&mut ctx.g.animated, action);
}

pub fn set_border_width(ctx: &mut WmCtx, win: WindowId, width: i32) {
    let (old_bw, _mon_id) = {
        if let Some(c) = ctx.g.clients.get(&win) {
            (c.border_width, c.monitor_id)
        } else {
            return;
        }
    };

    let new_bw = width;
    let d = old_bw - new_bw;

    {
        if let Some(client) = ctx.g.clients.get_mut(&win) {
            client.border_width = new_bw;
        }
    }

    let geo = {
        if let Some(c) = ctx.g.clients.get(&win) {
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

    resize(ctx, win, &geo, false);
}

pub fn toggle_focus_follows_mouse(ctx: &mut WmCtx, action: ToggleAction) {
    ctrl_toggle(&mut ctx.g.focusfollowsmouse, action);
}

pub fn toggle_focus_follows_float_mouse(ctx: &mut WmCtx, action: ToggleAction) {
    ctrl_toggle(&mut ctx.g.focusfollowsfloatmouse, action);
}

pub fn toggle_double_draw(ctx: &mut WmCtx) {
    ctx.g.doubledraw = !ctx.g.doubledraw;
}

pub fn toggle_locked(ctx: &mut WmCtx, win: WindowId) {
    let _mon_id = {
        if let Some(client) = ctx.g.clients.get_mut(&win) {
            client.islocked = !client.islocked;
            client.monitor_id
        } else {
            return;
        }
    };

    let selmon_id = ctx.g.selected_monitor_id();
    draw_bar(ctx, selmon_id);
}

pub fn toggle_show_tags(ctx: &mut WmCtx, action: ToggleAction) {
    let (selmon_id, new_showtags) = {
        let selmon_id = ctx.g.selected_monitor_id();

        let showtags = if let Some(mon) = ctx.g.selected_monitor() {
            mon.showtags
        } else {
            0
        };

        let mut show_bool = showtags != 0;
        ctrl_toggle(&mut show_bool, action);
        let new_showtags = if show_bool { 1 } else { 0 };

        (selmon_id, new_showtags)
    };

    if let Some(mon) = ctx.g.selected_monitor_mut() {
        mon.showtags = new_showtags;
    }

    let tagwidth = get_tag_width(ctx);
    ctx.g.tags.width = tagwidth;

    draw_bar(ctx, selmon_id);
}

pub fn hide_window(ctx: &mut WmCtx, win: WindowId) {
    crate::client::hide(ctx, win);
}

pub fn unhide_all(ctx: &mut WmCtx) {
    let clients: Vec<WindowId> = ctx.g.clients.keys().copied().collect();

    for win in clients {
        crate::client::show(ctx, win);
    }
}

pub fn redraw_win(ctx: &mut WmCtx) {
    let monitors: Vec<usize> = ctx.g.monitors.iter().enumerate().map(|(i, _)| i).collect();

    for i in monitors {
        draw_bar(ctx, i);
    }
}
