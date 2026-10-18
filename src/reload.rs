use crate::config;
use crate::contexts::WmCtx;

pub fn reload_config(ctx: &mut WmCtx<'_>) -> Result<(), String> {
    let cfg = config::load_config(ctx.backend_kind())?;
    let previous_status_command = ctx.core().state.config.status_command.clone();

    ctx.core_mut().state.apply_config(cfg)?;
    {
        let core = &mut ctx.core_mut().state;
        core.behavior
            .normalize_current_mode(&core.config.bindings.modes);
    }
    ctx.core_mut().work.queue_monitor_config_apply();
    ctx.core_mut().work.queue_input_config_apply();
    ctx.core_mut().work.queue_cursor_config_apply();

    crate::keyboard_layout::init_keyboard_layout(ctx);
    if previous_status_command != ctx.core().state.config.status_command {
        ctx.core_mut().start_status_sources();
    }

    // Layout reads the scaled metrics immediately. Native resources and bar
    // surfaces are projected once after the tick's monitor/layout work.
    crate::monitor::resync_monitor_ui_metrics(&mut ctx.core_mut().state);
    ctx.core_mut()
        .bar
        .invalidate(crate::bar::BarChange::Appearance);

    ctx.update_ewmh_desktop_props();
    ctx.refresh_key_grabs();
    crate::focus::focus(ctx, None);

    // Re-run `exec` commands (but not `exec_once`) on reload.
    crate::startup::autostart::run_exec_commands(&ctx.core().state.config.exec);

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

        assert!(wm.core.work.monitor_config);
        assert!(wm.core.work.input_config);
        assert!(wm.core.work.cursor_config);
    }

    #[test]
    fn reload_resynchronizes_monitor_bar_height_on_wayland() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        let id = wm
            .core
            .state
            .model
            .monitors
            .push(crate::types::Monitor::new_with_values());

        wm.with_ctx(reload_config).unwrap();

        let metrics = wm.core.state.config.bar_metrics();
        let monitor = wm.core.state.model.monitor(id).unwrap();
        assert_eq!(monitor.bar_height, metrics.height);
        assert_eq!(monitor.horizontal_padding, metrics.horizontal_padding);
    }

    #[test]
    fn reload_does_not_replace_backend_derived_display_state() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.derived.display.width = 3440;
        wm.core.state.derived.display.height = 1440;

        wm.with_ctx(reload_config).unwrap();

        assert_eq!(wm.core.state.derived.display.width, 3440);
        assert_eq!(wm.core.state.derived.display.height, 1440);
    }

    #[test]
    fn normalize_current_mode_resets_missing_mode_to_default() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.behavior.current_mode =
            crate::core_state::ActiveWmMode::Named("resize".to_string());

        let core = &mut wm.core.state;
        core.behavior
            .normalize_current_mode(&core.config.bindings.modes);

        assert_eq!(
            wm.core.state.behavior.current_mode,
            crate::core_state::ActiveWmMode::Default
        );
    }

    #[test]
    fn normalize_current_mode_preserves_existing_mode() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.behavior.current_mode =
            crate::core_state::ActiveWmMode::Named("resize".to_string());
        wm.core
            .state
            .config
            .bindings
            .modes
            .insert("resize".to_string(), ModeConfig::default());

        let core = &mut wm.core.state;
        core.behavior
            .normalize_current_mode(&core.config.bindings.modes);

        assert_eq!(
            wm.core.state.behavior.current_mode,
            crate::core_state::ActiveWmMode::Named("resize".to_string())
        );
    }
}
