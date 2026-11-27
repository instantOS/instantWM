use crate::config;
use crate::contexts::WmCtx;
use crate::core_state::EffectiveConfig;

fn record_config_error(ctx: &mut WmCtx<'_>, error: String) -> String {
    ctx.core_mut().state.config_errors = vec![error.clone()];
    error
}

pub fn reload_config(ctx: &mut WmCtx<'_>) -> Result<(), String> {
    let backend = ctx.backend_kind();
    reload_config_with(ctx, || config::load_config(backend))
}

/// Install a caller-supplied configuration, recording load and apply failures
/// for `instantwmctl status` while keeping the active configuration.
fn reload_config_with(
    ctx: &mut WmCtx<'_>,
    load: impl FnOnce() -> Result<EffectiveConfig, String>,
) -> Result<(), String> {
    let cfg = load().map_err(|error| record_config_error(ctx, error))?;
    apply_reload(ctx, cfg)
}

fn apply_reload(ctx: &mut WmCtx<'_>, cfg: EffectiveConfig) -> Result<(), String> {
    let previous_status_command = ctx.core().state.config.status_command.clone();

    ctx.core_mut()
        .state
        .apply_config(cfg)
        .map_err(|error| record_config_error(ctx, error))?;
    ctx.core_mut().state.config_errors.clear();
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

    // Layout reads the logical metrics immediately. Native resources and bar
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

    fn reload_defaults(ctx: &mut WmCtx<'_>) -> Result<(), String> {
        apply_reload(ctx, config::default_config(ctx.backend_kind()))
    }

    #[test]
    fn reload_marks_dirty_flags_for_wayland() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());

        wm.with_ctx(reload_defaults).unwrap();

        assert!(wm.core.work.monitor_config);
        assert!(wm.core.work.input_config);
        assert!(wm.core.work.cursor_config);
    }

    #[test]
    fn reload_resynchronizes_monitor_bar_height_on_wayland() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        let mut monitor = crate::types::Monitor::new_with_values();
        monitor.bar_height = 1;
        monitor.horizontal_padding = 0;
        let id = wm.core.state.model.monitors.push(monitor);

        wm.with_ctx(reload_defaults).unwrap();

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

        wm.with_ctx(reload_defaults).unwrap();

        assert_eq!(wm.core.state.derived.display.width, 3440);
        assert_eq!(wm.core.state.derived.display.height, 1440);
    }

    #[test]
    fn reload_clears_previous_config_errors_after_success() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.config_errors = vec!["previous config error".to_string()];
        let next = crate::config::default_config(crate::backend::BackendKind::Wayland);

        wm.with_ctx(|ctx| reload_config_with(ctx, || Ok(next)))
            .unwrap();

        assert!(wm.core.state.config_errors.is_empty());
    }

    #[test]
    fn failed_load_retains_the_active_config_and_records_the_error() {
        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        let previous_tags = wm.core.state.config.tags.count;
        let error = "layout.inner_gap must be non-negative".to_string();

        let result = wm.with_ctx(|ctx| reload_config_with(ctx, || Err(error.clone())));

        assert_eq!(result, Err(error.clone()));
        assert_eq!(wm.core.state.config.tags.count, previous_tags);
        assert_eq!(wm.core.state.config_errors, vec![error]);
        assert!(!wm.core.work.monitor_config);
    }

    #[test]
    fn failed_apply_records_the_error_for_status() {
        use crate::test_support::MonitorBuilder;
        use crate::types::TagMask;

        let mut wm = Wm::new(crate::backend::WaylandBackendData::default());
        wm.core.state.model.monitors.push(
            MonitorBuilder::new()
                .tag_count(20)
                .selected_tags(TagMask::single(20).unwrap())
                .build(),
        );
        let mut next = crate::config::default_config(crate::backend::BackendKind::Wayland);
        next.tags.count = 1;

        let result = wm.with_ctx(|ctx| reload_config_with(ctx, || Ok(next)));

        let error = result.unwrap_err();
        assert!(error.contains("cannot reduce tags.count to 1"), "{error}");
        assert_eq!(wm.core.state.config.tags.count, 20);
        assert_eq!(wm.core.state.config_errors, vec![error]);
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
