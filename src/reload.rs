use crate::config;

pub fn reload_config(ctx: &mut crate::contexts::WmCtx<'_>) -> Result<(), String> {
    let cfg = config::load_config(ctx.backend_kind())?;
    let previous_status_command = ctx.core().config().status_command.clone();

    ctx.core_mut().state_mut().apply_config(cfg)?;
    {
        let core = ctx.core_mut().state_mut();
        core.behavior
            .normalize_current_mode(&core.config.bindings.modes);
    }
    ctx.core_mut()
        .pending_work_mut()
        .queue_monitor_config_apply();
    ctx.core_mut().pending_work_mut().queue_input_config_apply();
    ctx.core_mut()
        .pending_work_mut()
        .queue_cursor_config_apply();
    ctx.core_mut().bar.mark_dirty();

    crate::runtime::init_keyboard_layout(ctx);
    if previous_status_command != ctx.core().config().status_command {
        ctx.core_mut().start_status_sources();
    }

    // Backend-owned bar resources must track the new config (X11 DrawContext
    // rebuild and per-monitor bar metric resync). The choreography is owned by
    // `WmCtx::reinit_bar_resources` so runtime updates and full reloads cannot drift.
    ctx.reinit_bar_resources();

    // Per-backend config projection: X11 re-renders bars/status from the new
    // DrawContext and refreshes its passive grabs; Wayland needs nothing
    // beyond `reinit_bar_resources` above.
    {
        ctx.refresh_bar_content();
        ctx.refresh_status_content();
        ctx.update_ewmh_desktop_props();
        ctx.refresh_key_grabs();
        crate::focus::focus(ctx, None);
    }

    // Re-run `exec` commands (but not `exec_once`) on reload.
    crate::startup::autostart::run_exec_commands(&ctx.core().config().exec);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestWm as Wm;

    use crate::config::ModeConfig;

    #[test]
    fn reload_marks_dirty_flags_for_wayland() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());

        wm.with_ctx(reload_config).unwrap();

        assert!(wm.work.monitor_config);
        assert!(wm.work.input_config);
        assert!(wm.work.cursor_config);
    }

    #[test]
    fn reload_resynchronizes_monitor_bar_height_on_wayland() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        let id = wm
            .core
            .model
            .monitors
            .push(crate::types::Monitor::new_with_values());

        wm.with_ctx(reload_config).unwrap();

        let metrics = wm.core.config.bar_metrics();
        let monitor = wm.core.model.monitor(id).unwrap();
        assert_eq!(monitor.bar_height, metrics.height);
        assert_eq!(monitor.horizontal_padding, metrics.horizontal_padding);
    }

    #[test]
    fn reload_does_not_replace_backend_derived_display_state() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.derived.display.width = 3440;
        wm.core.derived.display.height = 1440;

        wm.with_ctx(reload_config).unwrap();

        assert_eq!(wm.core.derived.display.width, 3440);
        assert_eq!(wm.core.derived.display.height, 1440);
    }

    #[test]
    fn normalize_current_mode_resets_missing_mode_to_default() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.behavior.current_mode =
            crate::core_state::ActiveWmMode::Named("resize".to_string());

        let core = &mut wm.core;
        core.behavior
            .normalize_current_mode(&core.config.bindings.modes);

        assert_eq!(
            wm.core.behavior.current_mode,
            crate::core_state::ActiveWmMode::Default
        );
    }

    #[test]
    fn normalize_current_mode_preserves_existing_mode() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.behavior.current_mode =
            crate::core_state::ActiveWmMode::Named("resize".to_string());
        wm.core
            .config
            .bindings
            .modes
            .insert("resize".to_string(), ModeConfig::default());

        let core = &mut wm.core;
        core.behavior
            .normalize_current_mode(&core.config.bindings.modes);

        assert_eq!(
            wm.core.behavior.current_mode,
            crate::core_state::ActiveWmMode::Named("resize".to_string())
        );
    }
}
