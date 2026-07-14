use crate::contexts::WmCtx;
use crate::tags::get_tag_width;
use crate::types::*;

fn toggled_bool(current: bool, action: ToggleAction) -> bool {
    let mut next = current;
    action.apply(&mut next);
    next
}

fn toggle_mode_name(current: &str, name: &str) -> String {
    if current == name {
        "default".to_string()
    } else {
        name.to_string()
    }
}

pub fn toggle_alt_tag(ctx: &mut WmCtx, action: ToggleAction) {
    let new_value = toggled_bool(ctx.core().model().tags.show_alternative_names, action);

    ctx.core_mut().model_mut().tags.show_alternative_names = new_value;

    let tagwidth = get_tag_width(ctx.core());
    ctx.core_mut().model_mut().tags.width = tagwidth;
    ctx.request_bar_update();
}

pub fn toggle_sticky(ctx: &mut WmCtx, win: WindowId) {
    let monitor_id = if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
        client.is_sticky = !client.is_sticky;
        client.monitor_id
    } else {
        return;
    };
    ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
}

pub fn toggle_locked(ctx: &mut WmCtx, win: WindowId) {
    if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
        client.is_locked = !client.is_locked;
    } else {
        return;
    }

    ctx.request_bar_update();
}

pub fn toggle_show_tags(ctx: &mut WmCtx, action: ToggleAction) {
    let (_selmon_id, new_showtags) = {
        let selmon_id = ctx.core().model().selected_monitor_id();

        let showtags = ctx.core().model().selected_monitor().showtags;
        let new_showtags = toggled_bool(showtags, action);

        (selmon_id, new_showtags)
    };

    ctx.core_mut().model_mut().selected_monitor_mut().showtags = new_showtags;

    let tagwidth = get_tag_width(ctx.core());
    ctx.core_mut().model_mut().tags.width = tagwidth;

    ctx.request_bar_update();
}

pub fn unhide_all(ctx: &mut crate::contexts::WmCtx) {
    let clients_to_unhide: Vec<WindowId> = ctx
        .core()
        .state()
        .model
        .clients
        .iter()
        .filter(|(_, c)| c.is_hidden && !c.is_scratchpad())
        .map(|(win, _)| *win)
        .collect();

    for win in clients_to_unhide {
        crate::client::show_window(ctx, win);
    }
}

pub fn toggle_mode(ctx: &mut WmCtx, name: &str) {
    let mode = toggle_mode_name(ctx.current_mode(), name);
    // Overview exit is handled by `exit_overview` (which updates
    // `current_mode` directly) rather than `set_current_mode` to avoid
    // calling `handle_mode_transition` a second time — the exit logic
    // runs inside `exit_overview` itself.
    if name == crate::overview::OVERVIEW_MODE_NAME && mode == "default" {
        crate::overview::exit_overview(ctx, crate::overview::ExitMode::RestorePrevious);
    } else {
        ctx.set_current_mode(mode);
    }
    if let WmCtx::X11(x11) = ctx {
        crate::backend::x11::keyboard::grab_keys(x11.core.state(), &x11.x11, x11.x11_runtime);
    }
}

pub fn toggle_bar(ctx: &mut WmCtx) {
    let animated = ctx.core().behavior().animated;
    let client_count = ctx.core().model().clients.len() as i32;
    let mut tmp_no_anim = false;
    if animated && client_count > 6 {
        ctx.core_mut().behavior_mut().animated = false;
        tmp_no_anim = true;
    }

    let bar_height = ctx.core().config().derived.bar_height;
    let selected_monitor = ctx.core_mut().model_mut().selected_monitor_mut();
    selected_monitor.per_tag_state().show_bar = !selected_monitor.per_tag_state().show_bar;
    selected_monitor.show_bar = selected_monitor.per_tag_state().show_bar;

    selected_monitor.update_bar_position(bar_height);

    let selmon_idx = ctx.core().model().selected_monitor_id();

    match ctx {
        WmCtx::X11(x11) => {
            if let Some(m) = x11.core.model().monitors.get(selmon_idx).cloned() {
                crate::backend::x11::bar::resize_bar_win(
                    x11.core.state(),
                    &x11.x11,
                    &*x11.x11_runtime,
                    x11.xembed_tray.as_deref(),
                    &m,
                );
            }
            x11.core.bar.mark_dirty();
        }
        WmCtx::Wayland(_) => {
            ctx.request_bar_update();
        }
    }

    ctx.core_mut().queue_layout_for_monitor_urgent(selmon_idx);

    if tmp_no_anim {
        ctx.core_mut().behavior_mut().animated = true;
    }
}

#[cfg(test)]
mod tests {
    use super::{toggle_mode_name, toggled_bool};
    use crate::types::ToggleAction;

    #[test]
    fn toggled_bool_applies_toggle_action() {
        assert!(!toggled_bool(true, ToggleAction::Toggle));
        assert!(toggled_bool(false, ToggleAction::Toggle));
        assert!(toggled_bool(false, ToggleAction::SetTrue));
        assert!(!toggled_bool(true, ToggleAction::SetFalse));
    }

    #[test]
    fn toggle_mode_name_toggles_back_to_default() {
        assert_eq!(toggle_mode_name("default", "resize"), "resize");
        assert_eq!(toggle_mode_name("resize", "resize"), "default");
    }
}
