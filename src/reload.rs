use crate::backend::Backend;
use crate::config;
use crate::contexts::WmCtx;
use crate::wm::Wm;

pub fn reload_config(wm: &mut Wm) -> Result<(), String> {
    let cfg = config::init_config();

    crate::globals::apply_config(&mut wm.g, &cfg);
    crate::globals::apply_tags_config(&mut wm.g, &cfg);
    wm.g.monitor_config_dirty = true;
    wm.g.input_config_dirty = true;
    wm.bar.mark_dirty();

    match &wm.backend {
        Backend::X11(_) => reload_x11(wm),
        Backend::Wayland(_) => reload_wayland(wm),
    }

    Ok(())
}

fn reload_x11(wm: &mut Wm) {
    crate::startup::x11::reload_runtime_config(wm);

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
        crate::bar::draw_bars_x11(
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
    use crate::backend::Backend as WmBackend;
    use crate::backend::wayland::WaylandBackend;

    #[test]
    fn reload_marks_dirty_flags_for_wayland() {
        let mut wm = Wm::new(WmBackend::Wayland(WaylandBackend::new()));

        reload_config(&mut wm).unwrap();

        assert!(wm.g.monitor_config_dirty);
        assert!(wm.g.input_config_dirty);
    }
}
