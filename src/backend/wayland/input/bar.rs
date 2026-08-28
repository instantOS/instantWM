use crate::contexts::WmCtxWayland;
use crate::types::*;
use crate::wm::Wm;

pub fn update_bar_hit_state(
    wm: &mut Wm,
    root: Point,
    reset_start_menu: bool,
) -> Option<BarPosition> {
    let mut ctx = wm.ctx();
    crate::bar::update_hover(&mut ctx, root, reset_start_menu, true)
}

/// Close the bar-hosted DBusMenu, returning whether a menu was open.
pub fn close_systray_menu(wm: &mut Wm) -> bool {
    crate::systray::close_menu(&mut core_ctx(wm))
}

fn core_ctx(wm: &mut Wm) -> crate::contexts::CoreCtx<'_> {
    crate::contexts::CoreCtx::new(
        &mut wm.core,
        &mut wm.work,
        &mut wm.running,
        &mut wm.bar,
        &mut wm.focus,
    )
}

pub fn handle_bar_scroll(wm: &mut Wm, pos: BarPosition, delta: f64, root: Point, clean_state: u32) {
    let button = if delta > 0.0 {
        MouseButton::ScrollUp
    } else {
        MouseButton::ScrollDown
    };
    let mut ctx = wm.ctx();
    let crate::contexts::WmCtx::Wayland(ref mut wayland_ctx) = ctx else {
        return;
    };
    run_bar_bindings(
        wayland_ctx,
        pos,
        button,
        InteractionSource::Pointer,
        root,
        clean_state,
    );
}

fn run_bar_bindings(
    ctx: &mut WmCtxWayland<'_>,
    pos: BarPosition,
    btn: MouseButton,
    source: InteractionSource,
    root: Point,
    clean_state: u32,
) {
    let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx.reborrow());
    crate::mouse::bindings::run_first_matching(
        &mut wm_ctx,
        crate::mouse::bindings::ButtonBindingEvent {
            target: ButtonTarget::Bar(pos),
            window: None,
            button: btn,
            source,
            root,
            clean_state,
            time_msec: 0,
        },
        0,
    );
}
