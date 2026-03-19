use crate::backend::Backend;
use crate::config;
use crate::contexts::WmCtx;
use crate::wm::Wm;

pub fn reload_config(wm: &mut Wm) -> Result<(), String> {
    let cfg = config::init_config();

    crate::globals::apply_config(&mut wm.g, &cfg);
    crate::globals::apply_tags_config(&mut wm.g, &cfg);
    normalize_current_mode(wm);
    wm.g.dirty.monitor_config = true;
    wm.g.dirty.input_config = true;
    wm.bar.mark_dirty();

    match &wm.backend {
        Backend::X11(_) => reload_x11(wm),
        Backend::Wayland(_) => reload_wayland(wm),
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

fn reload_x11(wm: &mut Wm) {
    crate::startup::x11::init_drw_and_schemes(wm);

    let ctx = wm.ctx();
    if let WmCtx::X11(mut x11_ctx) = ctx {
        crate::keyboard_layout::init_keyboard_layout(&mut WmCtx::X11(x11_ctx.reborrow()));
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
        crate::bar::x11::draw_bars_x11(
            &mut x11_ctx.core,
            x11_ctx.x11_runtime,
            x11_ctx.systray.as_deref(),
        );
    }
}

fn reload_wayland(wm: &mut Wm) {
    let mut ctx = wm.ctx();
    crate::keyboard_layout::init_keyboard_layout(&mut ctx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::wayland::WaylandBackend;
    use crate::backend::Backend as WmBackend;
    use crate::config::ModeConfig;

    #[test]
    fn reload_marks_dirty_flags_for_wayland() {
        let mut wm = Wm::new(WmBackend::new_wayland(WaylandBackend::new()));

        reload_config(&mut wm).unwrap();

        assert!(wm.g.dirty.monitor_config);
        assert!(wm.g.dirty.input_config);
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
