use crate::backend::Backend;
use crate::config;
use crate::contexts::WmCtx;
use crate::wm::Wm;

pub fn reload_config(wm: &mut Wm) -> Result<(), String> {
    let cfg = config::init_config(wm.backend.kind());

    crate::globals::apply_config(&mut wm.g, &cfg);
    crate::globals::apply_tags_config(&mut wm.g, &cfg);
    normalize_current_mode(wm);
    wm.g.queue_monitor_config_apply();
    wm.g.queue_input_config_apply();
    wm.bar.mark_dirty();

    crate::runtime::init_keyboard_layout(wm);

    if matches!(&wm.backend, Backend::X11(_)) {
        reload_x11(wm);
    }
    if let Backend::Wayland(data) = &mut wm.backend {
        reload_wayland(&mut wm.g, data, &cfg);
    }

    Ok(())
}

fn normalize_current_mode(wm: &mut Wm) {
    if wm.g.behavior.current_mode == "default" {
        return;
    }

    if !wm.g.cfg.modes.contains_key(&wm.g.behavior.current_mode) {
        wm.g.behavior.current_mode = "default".to_string();
    }
}

fn reload_wayland(
    g: &mut crate::globals::Globals,
    data: &mut crate::backend::WaylandBackendData,
    cfg: &config::Config,
) {
    use crate::wayland::common::{wayland_font_height_from_size, wayland_font_size_from_config};
    use crate::types::{CLOSE_BUTTON_DETAIL, CLOSE_BUTTON_WIDTH};

    let font_size = wayland_font_size_from_config(&cfg.fonts);
    let font_height = wayland_font_height_from_size(font_size);

    data.bar_painter.set_font_size(font_size);

    let min_bar_height = CLOSE_BUTTON_WIDTH + CLOSE_BUTTON_DETAIL + 2;
    g.cfg.bar_height = (if cfg.bar_height > 0 {
        font_height + cfg.bar_height
    } else {
        font_height + 12
    })
    .max(min_bar_height);
    g.cfg.horizontal_padding = font_height;
}

fn reload_x11(wm: &mut Wm) {
    crate::startup::x11::init_drw_and_schemes(wm);

    let ctx = wm.ctx();
    if let WmCtx::X11(mut x11_ctx) = ctx {
        crate::bar::x11::update_bars(
            &mut x11_ctx.core,
            &x11_ctx.x11,
            x11_ctx.x11_runtime,
            x11_ctx.systray.as_deref(),
        );
        crate::bar::x11::update_status(
            &mut x11_ctx.core,
            &x11_ctx.x11,
            x11_ctx.x11_runtime,
            x11_ctx.systray.as_deref_mut(),
        );
        crate::keyboard::grab_keys_x11(&x11_ctx.core, &x11_ctx.x11, x11_ctx.x11_runtime);
        crate::focus::focus_soft_x11(&mut x11_ctx.core, &x11_ctx.x11, x11_ctx.x11_runtime, None);
    }
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

        assert!(wm.g.pending.monitor_config);
        assert!(wm.g.pending.input_config);
    }

    #[test]
    fn reload_sets_bar_height_on_wayland() {
        let mut wm = Wm::new(WmBackend::new_wayland(WaylandBackend::new()));

        reload_config(&mut wm).unwrap();

        assert!(
            wm.g.cfg.bar_height > 0,
            "bar_height should be computed from font metrics, got {}",
            wm.g.cfg.bar_height
        );
        assert!(
            wm.g.cfg.horizontal_padding > 0,
            "horizontal_padding should be set from font height, got {}",
            wm.g.cfg.horizontal_padding
        );
    }

    #[test]
    fn normalize_current_mode_resets_missing_mode_to_default() {
        let mut wm = Wm::new(WmBackend::new_wayland(WaylandBackend::new()));
        wm.g.behavior.current_mode = "resize".to_string();

        normalize_current_mode(&mut wm);

        assert_eq!(wm.g.behavior.current_mode, "default");
    }

    #[test]
    fn normalize_current_mode_preserves_existing_mode() {
        let mut wm = Wm::new(WmBackend::new_wayland(WaylandBackend::new()));
        wm.g.behavior.current_mode = "resize".to_string();
        wm.g.cfg
            .modes
            .insert("resize".to_string(), ModeConfig::default());

        normalize_current_mode(&mut wm);

        assert_eq!(wm.g.behavior.current_mode, "resize");
    }
}
