use crate::backend::wayland::compositor::WaylandState;
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

pub fn handle_bar_click(
    wm: &mut Wm,
    state: &mut WaylandState,
    pos: BarPosition,
    button_code: u32,
    source: InteractionSource,
    root: Point,
    clean_state: u32,
) {
    let Some(button) = MouseButton::from_wayland_code(button_code) else {
        return;
    };

    if let BarPosition::SystrayMenuItem(idx) = pos {
        state.dismiss_native_systray_menu();
        // Non-left presses on a hosted menu entry are consumed but inert.
        if button == MouseButton::Left {
            let mut core = core_ctx(wm);
            crate::systray::activate_menu_entry(&mut core, idx);
        }
        return;
    }

    crate::systray::close_menu(&mut core_ctx(wm));

    if let BarPosition::SystrayItem(idx) = pos {
        // Right-pressing an item whose native menu is open toggles it closed
        // instead of re-requesting it.
        let toggled_closed = match (&wm.bar.systray_host.tray.items.get(idx), button) {
            (Some(item), MouseButton::Right) => {
                state.native_systray_menu_matches(&item.service, &item.path)
            }
            _ => false,
        };
        state.dismiss_native_systray_menu();
        if !toggled_closed {
            let mut core = core_ctx(wm);
            crate::systray::press_icon(&mut core, idx, button, root);
        }
        return;
    }

    state.dismiss_native_systray_menu();

    if pos == BarPosition::StatusText {
        let mut ctx = wm.ctx();
        crate::bar::handle_status_text_click(&mut ctx, root, button.to_x11_detail(), clean_state);
        return;
    }

    let mut ctx = wm.ctx();
    let crate::contexts::WmCtx::Wayland(ref mut wayland_ctx) = ctx else {
        return;
    };
    run_bar_bindings(wayland_ctx, pos, button, source, root, clean_state);
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
    crate::mouse::bindings::run_matching(
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
        crate::mouse::bindings::MatchPolicy::All,
    );
}
