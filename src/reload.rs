use crate::config;
use crate::wm::Wm;

pub fn reload_config(wm: &mut Wm) -> Result<(), String> {
    let cfg = config::load_config(wm.backend.kind())?;
    let previous_status_command = wm.core.config.status_command.clone();

    crate::core_state::apply_config(&mut wm.core, cfg);
    wm.core
        .behavior
        .normalize_current_mode(&wm.core.config.bindings.modes);
    wm.work.queue_monitor_config_apply();
    wm.work.queue_input_config_apply();
    wm.work.queue_cursor_config_apply();
    wm.bar.mark_dirty();

    crate::runtime::init_keyboard_layout(wm);
    crate::bar::status::reload_status_command(
        previous_status_command.as_deref(),
        wm.core.config.status_command.as_deref(),
    );

    // Backend-owned bar resources must track the new config (X11 DrawContext
    // rebuild, Wayland bar-metric recompute). The choreography is owned by
    // `Wm::reinit_bar_resources` so runtime updates and full reloads cannot drift.
    wm.reinit_bar_resources();

    // Per-backend config projection: X11 re-renders bars/status from the new
    // DrawContext and refreshes its passive grabs; Wayland needs nothing
    // beyond `reinit_bar_resources` above.
    {
        let mut ctx = wm.ctx();
        ctx.refresh_bar_content();
        ctx.refresh_status_content();
        ctx.update_ewmh_desktop_props();
        ctx.refresh_key_grabs();
        crate::focus::focus(&mut ctx, None);
    }

    // Re-run `exec` commands (but not `exec_once`) on reload.
    crate::startup::autostart::run_exec_commands(&wm.core.config.exec);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::Backend as WmBackend;
    use crate::backend::wayland::WaylandBackend;
    use crate::config::ModeConfig;

    #[test]
    fn reload_marks_dirty_flags_for_wayland() {
        let mut wm = Wm::new(WmBackend::new_wayland(WaylandBackend::new()));

        reload_config(&mut wm).unwrap();

        assert!(wm.work.monitor_config);
        assert!(wm.work.input_config);
        assert!(wm.work.cursor_config);
    }

    #[test]
    fn reload_sets_bar_height_on_wayland() {
        let mut wm = Wm::new(WmBackend::new_wayland(WaylandBackend::new()));

        reload_config(&mut wm).unwrap();

        assert!(
            wm.core.derived.bar_height > 0,
            "bar_height should be computed from font metrics, got {}",
            wm.core.derived.bar_height
        );
        assert!(
            wm.core.derived.bar_horizontal_padding > 0,
            "horizontal_padding should be set from font height, got {}",
            wm.core.derived.bar_horizontal_padding
        );
    }

    #[test]
    fn reload_does_not_replace_backend_derived_display_state() {
        let mut wm = Wm::new(WmBackend::new_wayland(WaylandBackend::new()));
        wm.core.derived.display.width = 3440;
        wm.core.derived.display.height = 1440;

        reload_config(&mut wm).unwrap();

        assert_eq!(wm.core.derived.display.width, 3440);
        assert_eq!(wm.core.derived.display.height, 1440);
    }

    #[test]
    fn normalize_current_mode_resets_missing_mode_to_default() {
        let mut wm = Wm::new(WmBackend::new_wayland(WaylandBackend::new()));
        wm.core.behavior.current_mode =
            crate::core_state::ActiveWmMode::Named("resize".to_string());

        wm.core
            .behavior
            .normalize_current_mode(&wm.core.config.bindings.modes);

        assert_eq!(
            wm.core.behavior.current_mode,
            crate::core_state::ActiveWmMode::Default
        );
    }

    #[test]
    fn normalize_current_mode_preserves_existing_mode() {
        let mut wm = Wm::new(WmBackend::new_wayland(WaylandBackend::new()));
        wm.core.behavior.current_mode =
            crate::core_state::ActiveWmMode::Named("resize".to_string());
        wm.core
            .config
            .bindings
            .modes
            .insert("resize".to_string(), ModeConfig::default());

        wm.core
            .behavior
            .normalize_current_mode(&wm.core.config.bindings.modes);

        assert_eq!(
            wm.core.behavior.current_mode,
            crate::core_state::ActiveWmMode::Named("resize".to_string())
        );
    }
}
