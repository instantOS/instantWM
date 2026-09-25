use crate::contexts::WmCtx;
use crate::core_state::ActiveWmMode;
use crate::types::*;

fn toggle_mode_name(current: &ActiveWmMode, name: &str) -> ActiveWmMode {
    if current.as_str() == name {
        ActiveWmMode::Default
    } else {
        ActiveWmMode::from_name(name)
    }
}

pub fn toggle_sticky(ctx: &mut WmCtx, win: WindowId) {
    let monitor_id = if let Some(client) = ctx.core_mut().model_mut().client_mut(win) {
        if client.is_scratchpad() {
            return;
        }
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
    if name == crate::core_state::TREE_PLACEMENT_MODE_NAME {
        if matches!(ctx.current_mode(), ActiveWmMode::TreePlacement(_)) {
            ctx.reset_mode();
        }
        return;
    }
    let next_mode = toggle_mode_name(ctx.current_mode(), name);
    // Overview exit is handled by `exit_overview` (which updates
    // `current_mode` directly) rather than `set_current_mode` to avoid
    // calling `handle_mode_transition` a second time — the exit logic
    // runs inside `exit_overview` itself.
    if name == crate::overview::OVERVIEW_MODE_NAME && next_mode == ActiveWmMode::Default {
        crate::overview::exit_overview(ctx, crate::overview::ExitMode::RestorePrevious);
    } else {
        ctx.set_current_mode(next_mode);
    }
}

pub fn toggle_bar(ctx: &mut WmCtx) {
    let selmon_idx = {
        let selected_monitor = ctx.core_mut().model_mut().expect_selected_monitor_mut();
        let current = selected_monitor.show_bar_for_mask(selected_monitor.selected_tags());
        // A per-view session override; the configured default is untouched.
        selected_monitor.per_tag_state().show_bar = Some(!current);
        ctx.core().model().selected_monitor_id()
    };

    ctx.refresh_top_bars();

    ctx.core_mut().queue_layout_for_monitor_urgent(selmon_idx);
}

/// Set the bottom bar visibility everywhere.
///
/// The bottom bar state is a single global setting, so toggling it applies
/// to every monitor and survives tag switches.
///
/// Shared by the hotkey toggle and the IPC toggle command (which can also
/// force on/off).
pub fn set_bottom_bar_shown(ctx: &mut WmCtx, shown: bool) {
    let changed_monitors: Vec<MonitorId> = ctx
        .core_mut()
        .model_mut()
        .monitors_iter_mut()
        .filter(|(_, monitor)| monitor.show_bottom_bar != shown)
        .map(|(monitor_id, _)| monitor_id)
        .collect();
    if changed_monitors.is_empty() {
        return;
    }

    for monitor in ctx.core_mut().model_mut().monitors_iter_all_mut() {
        monitor.show_bottom_bar = shown;
    }

    for monitor_id in changed_monitors {
        ctx.refresh_monitor_bottom_bar(monitor_id);

        ctx.core_mut().queue_layout_for_monitor_urgent(monitor_id);
    }
}

#[cfg(test)]
mod tests {
    use super::{set_bottom_bar_shown, toggle_mode_name, unhide_all};
    use crate::backend::{Backend, wayland::WaylandBackend};
    use crate::core_state::ActiveWmMode;
    use crate::types::{Client, Monitor, TagMask, WindowId};
    use crate::wm::Wm;

    /// A WM with one output whose configured bar visibility is `show`.
    fn wm_with_bar(show: bool) -> Wm {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let monitor_id = wm.core.model.monitors.push(Monitor::new_with_values());
        wm.core.model.monitors.set_selected(monitor_id);
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .bar_default_show = show;
        wm
    }

    fn bar_visible(wm: &Wm) -> bool {
        wm.core.model.expect_selected_monitor().shows_bar()
    }

    #[test]
    fn toggle_bar_is_a_per_view_override_that_leaves_the_default_alone() {
        let mut wm = wm_with_bar(true);

        super::toggle_bar(&mut wm.ctx());
        assert!(!bar_visible(&wm));
        // The configured default is untouched; only the view overrides it.
        assert!(wm.core.model.expect_selected_monitor().bar_default_show);

        super::toggle_bar(&mut wm.ctx());
        assert!(bar_visible(&wm));
    }

    #[test]
    fn a_bar_override_applies_only_to_the_view_it_was_set_on() {
        let mut wm = wm_with_bar(true);
        wm.core
            .model
            .expect_selected_monitor_mut()
            .set_selected_tags(TagMask::single(1).unwrap());

        super::toggle_bar(&mut wm.ctx());
        assert!(!bar_visible(&wm));

        // Switching to another view falls back to the configured value.
        wm.core
            .model
            .expect_selected_monitor_mut()
            .set_selected_tags(TagMask::single(2).unwrap());
        assert!(bar_visible(&wm));

        // …and coming back restores the override.
        wm.core
            .model
            .expect_selected_monitor_mut()
            .set_selected_tags(TagMask::single(1).unwrap());
        assert!(!bar_visible(&wm));
    }

    #[test]
    fn reloading_restores_the_configured_bar_visibility() {
        let mut wm = wm_with_bar(true);
        super::toggle_bar(&mut wm.ctx());
        assert!(!bar_visible(&wm));

        // A reload is "restore every configured value", overrides included.
        let config = wm.core.config.clone();
        wm.core.apply_config(config).unwrap();
        assert!(bar_visible(&wm));
        assert!(
            wm.core
                .model
                .expect_selected_monitor()
                .per_tag()
                .unwrap()
                .show_bar
                .is_none(),
            "reload drops the session override entirely"
        );
    }

    #[test]
    fn toggle_mode_name_toggles_back_to_default() {
        assert_eq!(
            toggle_mode_name(&ActiveWmMode::Default, "resize"),
            ActiveWmMode::Named("resize".to_string())
        );
        assert_eq!(
            toggle_mode_name(&ActiveWmMode::Named("resize".to_string()), "resize"),
            ActiveWmMode::Default
        );
    }

    #[test]
    fn unhide_all_reveals_hidden_windows_without_moving_focus() {
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));
        let monitor_id = wm.core.model.monitors.push(Monitor::default());
        wm.core.model.monitors.set_selected(monitor_id);

        let focused = WindowId(1);
        let hidden = WindowId(2);
        let also_hidden = WindowId(3);
        for (win, is_hidden) in [(focused, false), (hidden, true), (also_hidden, true)] {
            wm.core.model.insert_client(Client {
                win,
                monitor_id,
                is_hidden,
                ..Client::default()
            });
            wm.core
                .model
                .monitor_mut(monitor_id)
                .unwrap()
                .clients
                .push(win);
        }
        wm.core
            .model
            .monitor_mut(monitor_id)
            .unwrap()
            .set_selected(Some(focused));

        unhide_all(&mut wm.ctx());

        assert!(!wm.core.model.client(hidden).unwrap().is_hidden);
        assert!(!wm.core.model.client(also_hidden).unwrap().is_hidden);
        assert_eq!(wm.core.model.selected_win(), Some(focused));
    }

    #[test]
    fn set_bottom_bar_shown_applies_to_every_monitor_and_tag() {
        // The bottom bar is one global session setting. A toggle must reach
        // every monitor and survive tag switches; historically it was stored
        // per selected monitor and per tag mask, so other monitors kept the
        // bar and switching tags resurrected it.
        let mut wm = Wm::new(Backend::new_wayland(WaylandBackend::new()));

        let first = wm.core.model.monitors.push(Monitor::default());
        let second = wm.core.model.monitors.push(Monitor::default());
        for monitor in wm.core.model.monitors_iter_all_mut() {
            monitor.show_bottom_bar = true;
        }
        wm.core.model.set_selected_monitor(second);

        let tag_a = TagMask::single(1).unwrap();
        let tag_b = TagMask::single(2).unwrap();
        // Seed per-tag state on two different tags of both monitors so a
        // per-tag-scoped implementation would leave stale entries behind.
        for id in [first, second] {
            let monitor = wm.core.model.monitor_mut(id).unwrap();
            monitor.set_selected_tags_with_history(tag_a);
            monitor.per_tag_state();
            monitor.set_selected_tags_with_history(tag_b);
            monitor.per_tag_state();
            monitor.set_selected_tags_with_history(tag_a);
            assert!(monitor.shows_bottom_bar());
        }

        // Toggle off while viewing tag_a of the second monitor.
        set_bottom_bar_shown(&mut wm.ctx(), false);

        let all_hidden = |wm: &Wm| {
            [first, second]
                .iter()
                .all(|id| !wm.core.model.monitor(*id).unwrap().shows_bottom_bar())
        };
        assert!(all_hidden(&wm));

        // Switching tags must not resurrect the bar.
        for id in [first, second] {
            let monitor = wm.core.model.monitor_mut(id).unwrap();
            monitor.set_selected_tags_with_history(tag_b);
            assert!(!monitor.shows_bottom_bar());
        }
        assert!(all_hidden(&wm));

        // Toggling back on re-enables it everywhere again.
        set_bottom_bar_shown(&mut wm.ctx(), true);
        assert!(
            [first, second]
                .iter()
                .all(|id| wm.core.model.monitor(*id).unwrap().shows_bottom_bar())
        );
    }
}
