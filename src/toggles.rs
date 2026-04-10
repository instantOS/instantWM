use crate::client::manager::ClientManager;
use crate::contexts::WmCtx;
use crate::globals::WmBehavior;
use crate::layouts::arrange;
use crate::tags::get_tag_width;
use crate::types::*;

pub fn ctrl_toggle(value: &mut bool, action: ToggleAction) {
    action.apply(value);
}

fn toggled_bool(current: bool, action: ToggleAction) -> bool {
    let mut next = current;
    ctrl_toggle(&mut next, action);
    next
}

fn showtags_from_visible(visible: bool) -> u32 {
    if visible { 1 } else { 0 }
}

fn toggle_mode_name(current: &str, name: &str) -> String {
    if current == name {
        "default".to_string()
    } else {
        name.to_string()
    }
}

pub fn toggle_alt_tag(ctx: &mut WmCtx, action: ToggleAction) {
    let new_value = toggled_bool(ctx.core().globals().tags.show_alternative_names, action);

    ctx.core_mut().globals_mut().tags.show_alternative_names = new_value;

    let tagwidth = get_tag_width(ctx.core());
    ctx.core_mut().globals_mut().tags.width = tagwidth;
    ctx.request_bar_update(None);
}

pub fn toggle_sticky(ctx: &mut WmCtx, win: WindowId) {
    let monitor_id = if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
        client.issticky = !client.issticky;
        client.monitor_id
    } else {
        return;
    };
    arrange(ctx, Some(monitor_id));
}

pub fn toggle_animated(behavior: &mut WmBehavior, action: ToggleAction) {
    ctrl_toggle(&mut behavior.animated, action);
}

pub fn set_border_width(clients: &mut ClientManager, win: WindowId, width: i32) {
    let new_bw = width;

    let geo = {
        if let Some(client) = clients.get_mut(&win) {
            let old_bw = client.border_width;
            let d = old_bw - new_bw;
            client.border_width = new_bw;

            Rect {
                x: client.geo.x,
                y: client.geo.y,
                w: client.geo.w + 2 * d,
                h: client.geo.h + 2 * d,
            }
        } else {
            return;
        }
    };

    clients.update_geometry(win, geo);
}

pub fn set_special_next(behavior: &mut WmBehavior, value: SpecialNext) {
    behavior.specialnext = value;
}

pub fn toggle_focus_follows_mouse(behavior: &mut WmBehavior, action: ToggleAction) {
    ctrl_toggle(&mut behavior.focus_follows_mouse, action);
}

pub fn toggle_focus_follows_float_mouse(behavior: &mut WmBehavior, action: ToggleAction) {
    ctrl_toggle(&mut behavior.focus_follows_float_mouse, action);
}

pub fn toggle_double_draw(behavior: &mut WmBehavior) {
    behavior.double_draw = !behavior.double_draw;
}

pub fn toggle_locked(ctx: &mut WmCtx, win: WindowId) {
    if let Some(client) = ctx.core_mut().globals_mut().clients.get_mut(&win) {
        client.is_locked = !client.is_locked;
    } else {
        return;
    }

    let selmon_id = ctx.core().globals().selected_monitor_id();
    ctx.request_bar_update(Some(selmon_id));
}

pub fn toggle_show_tags(ctx: &mut WmCtx, action: ToggleAction) {
    let (selmon_id, new_showtags) = {
        let selmon_id = ctx.core().globals().selected_monitor_id();

        let showtags = ctx.core().globals().selected_monitor().showtags;
        let new_showtags = showtags_from_visible(toggled_bool(showtags != 0, action));

        (selmon_id, new_showtags)
    };

    ctx.core_mut().globals_mut().selected_monitor_mut().showtags = new_showtags;

    let tagwidth = get_tag_width(ctx.core());
    ctx.core_mut().globals_mut().tags.width = tagwidth;

    ctx.request_bar_update(Some(selmon_id));
}

pub fn unhide_all(ctx: &mut crate::contexts::WmCtx) {
    let clients: Vec<WindowId> = ctx.core().globals().clients.keys().copied().collect();

    for win in clients {
        let should_unhide = ctx
            .core()
            .globals()
            .clients
            .get(&win)
            .is_some_and(|c| c.is_hidden && !c.is_scratchpad());
        if should_unhide {
            crate::client::show(ctx, win);
        }
    }
}

pub fn toggle_mode(ctx: &mut WmCtx, name: &str) {
    let mode = toggle_mode_name(ctx.current_mode(), name);
    ctx.set_current_mode(mode);
    if let WmCtx::X11(x11) = ctx {
        crate::keyboard::grab_keys_x11(&x11.core, &x11.x11, x11.x11_runtime);
    }
    let selmon_id = ctx.core().globals().selected_monitor_id();
    ctx.request_bar_update(Some(selmon_id));
}

pub fn toggle_bar(ctx: &mut WmCtx) {
    let animated = ctx.core().globals().behavior.animated;
    let client_count = ctx.core().globals().clients.len() as i32;
    let mut tmp_no_anim = false;
    if animated && client_count > 6 {
        ctx.core_mut().globals_mut().behavior.animated = false;
        tmp_no_anim = true;
    }

    let bar_height = ctx.core().globals().cfg.bar_height;
    let selmon = ctx.core_mut().globals_mut().selected_monitor_mut();
    selmon.pertag_state().showbar = !selmon.pertag_state().showbar;

    selmon.update_bar_position(bar_height);

    let selmon_idx = ctx.core().globals().selected_monitor_id();

    match ctx {
        WmCtx::X11(x11) => {
            if let Some(m) = x11.core.globals().monitors.get(selmon_idx).cloned() {
                crate::bar::x11::resize_bar_win(
                    &x11.core,
                    &x11.x11,
                    &*x11.x11_runtime,
                    x11.systray.as_deref(),
                    &m,
                );
            }
            x11.core.bar.mark_dirty();
        }
        WmCtx::Wayland(_) => {
            ctx.request_bar_update(Some(selmon_idx));
        }
    }

    if tmp_no_anim {
        ctx.core_mut().globals_mut().behavior.animated = true;
    }
}

#[cfg(test)]
mod tests {
    use super::{showtags_from_visible, toggle_mode_name, toggled_bool};
    use crate::types::ToggleAction;

    #[test]
    fn toggled_bool_applies_toggle_action() {
        assert!(!toggled_bool(true, ToggleAction::Toggle));
        assert!(toggled_bool(false, ToggleAction::Toggle));
        assert!(toggled_bool(false, ToggleAction::SetTrue));
        assert!(!toggled_bool(true, ToggleAction::SetFalse));
    }

    #[test]
    fn showtags_from_visible_is_stable() {
        assert_eq!(showtags_from_visible(true), 1);
        assert_eq!(showtags_from_visible(false), 0);
    }

    #[test]
    fn toggle_mode_name_toggles_back_to_default() {
        assert_eq!(toggle_mode_name("default", "resize"), "resize");
        assert_eq!(toggle_mode_name("resize", "resize"), "default");
    }
}
