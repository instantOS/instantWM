use crate::backend::x11::X11BackendRef;
use crate::contexts::{CoreCtx, WmCtx};
use crate::globals::X11RuntimeConfig;
use crate::keyboard::grab_keys_x11;
use crate::tags::get_tag_width;
use crate::types::*;

pub fn ctrl_toggle(value: &mut bool, action: ToggleAction) {
    action.apply(value);
}

pub fn toggle_alt_tag(ctx: &mut WmCtx, action: ToggleAction) {
    let new_value = {
        let mut showalttag = ctx.g().tags.show_alternative_names;
        ctrl_toggle(&mut showalttag, action);
        showalttag
    };

    ctx.g_mut().tags.show_alternative_names = new_value;

    let tagwidth = get_tag_width(ctx.core());
    ctx.g_mut().tags.width = tagwidth;
    ctx.request_bar_update(None);
}

pub fn alt_tab_free(ctx: &mut WmCtx, action: ToggleAction) {
    if let WmCtx::X11(x11) = ctx {
        ctrl_toggle(&mut x11.core.g.tags.prefix, action);
        grab_keys_x11(&mut x11.core, &x11.x11, x11.x11_runtime);
    } else {
        let mut prefix = ctx.g().tags.prefix;
        ctrl_toggle(&mut prefix, action);
        ctx.g_mut().tags.prefix = prefix;
    }
}

pub fn toggle_sticky(core: &mut CoreCtx, win: WindowId) {
    let monitor_id = if let Some(client) = core.g.clients.get_mut(&win) {
        client.issticky = !client.issticky;
        client.monitor_id
    } else {
        return;
    };

    let _ = monitor_id;
}

pub fn toggle_prefix(ctx: &mut WmCtx) {
    let next = !ctx.g().tags.prefix;
    ctx.g_mut().tags.prefix = next;

    let selmon_id = ctx.g().selected_monitor_id();
    ctx.request_bar_update(Some(selmon_id));
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

    core.g.clients.update_geometry(win, geo);
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

pub fn toggle_locked(ctx: &mut WmCtx, win: WindowId) {
    let _mon_id = {
        if let Some(client) = ctx.g_mut().clients.get_mut(&win) {
            client.islocked = !client.islocked;
            client.monitor_id
        } else {
            return;
        }
    };

    let selmon_id = ctx.g().selected_monitor_id();
    ctx.request_bar_update(Some(selmon_id));
}

//TODO: could this be named better?
//TODO: what does this do?
pub fn toggle_show_tags(ctx: &mut WmCtx, action: ToggleAction) {
    let (selmon_id, new_showtags) = {
        let selmon_id = ctx.g().selected_monitor_id();

        let showtags = ctx.g().selected_monitor().showtags;

        let mut show_bool = showtags != 0;
        ctrl_toggle(&mut show_bool, action);
        let new_showtags = if show_bool { 1 } else { 0 };

        (selmon_id, new_showtags)
    };

    ctx.g_mut().selected_monitor_mut().showtags = new_showtags;

    let tagwidth = get_tag_width(ctx.core());
    ctx.g_mut().tags.width = tagwidth;

    ctx.request_bar_update(Some(selmon_id));
}

pub fn unhide_all(ctx: &mut crate::contexts::WmCtx) {
    let clients: Vec<WindowId> = ctx.g().clients.keys().copied().collect();

    for win in clients {
        crate::client::show(ctx, win);
    }
}

pub fn redraw_win(ctx: &mut WmCtx) {
    ctx.request_bar_update(None);
}

pub fn toggle_bar(ctx: &mut WmCtx) {
    if let WmCtx::X11(x11) = ctx {
        crate::bar::x11::toggle_bar(
            &mut x11.core,
            &x11.x11,
            x11.x11_runtime,
            x11.systray.as_deref(),
        );
        return;
    }

    let animated = ctx.g().animated;
    let client_count = ctx.g().clients.len() as i32;
    let mut tmp_no_anim = false;
    if animated && client_count > 6 {
        ctx.g_mut().animated = false;
        tmp_no_anim = true;
    }

    let bar_height = ctx.g().cfg.bar_height;
    let selmon = ctx.g_mut().selected_monitor_mut();
    selmon.showbar = !selmon.showbar;

    let current_tag = selmon.current_tag;
    if current_tag > 0 && current_tag <= selmon.tags.len() {
        selmon.tags[current_tag - 1].showbar = selmon.showbar;
    }

    selmon.update_bar_position(bar_height);

    let selmon_idx = ctx.g().selected_monitor_id();
    ctx.request_bar_update(Some(selmon_idx));

    if tmp_no_anim {
        ctx.g_mut().animated = true;
    }
}
