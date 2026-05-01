use crate::backend::Backend;
use crate::contexts::WmCtxWayland;
use crate::types::*;
use crate::wm::Wm;

pub fn update_wayland_bar_hit_state(
    wm: &mut Wm,
    root: Point,
    reset_start_menu: bool,
) -> Option<BarPosition> {
    let mut ctx = wm.ctx();
    crate::bar::update_hover(&mut ctx, root, reset_start_menu, true)
}

pub fn handle_wayland_bar_click(
    wm: &mut Wm,
    pos: BarPosition,
    button_code: u32,
    root: Point,
    clean_state: u32,
) {
    let Some(button) = wayland_button_to_mouse_button(button_code) else {
        return;
    };

    if matches!(pos, BarPosition::SystrayItem(_)) {
        let BarPosition::SystrayItem(idx) = pos else {
            return;
        };
        // Destructure backend to avoid multiple mutable borrows
        let Backend::Wayland(data) = &mut wm.backend else {
            return;
        };
        if data.wayland_systray_runtime.as_ref().is_some() {
            if let Some(runtime) = data.wayland_systray_runtime.as_ref() {
                let target = data
                    .wayland_systray
                    .items
                    .get(idx)
                    .map(|it| (it.service.clone(), it.path.clone()));
                if let Some((service, path)) = target {
                    runtime.dispatch_click_item(service, path, button, root);
                }
            }
            data.wayland_systray_menu = None;
        }
        return;
    }

    if matches!(pos, BarPosition::SystrayMenuItem(_)) {
        let BarPosition::SystrayMenuItem(idx) = pos else {
            return;
        };
        let Backend::Wayland(data) = &mut wm.backend else {
            return;
        };
        if data.wayland_systray_runtime.as_ref().is_some() {
            if let Some(runtime) = data.wayland_systray_runtime.as_ref() {
                let target = data.wayland_systray_menu.as_ref().and_then(|menu| {
                    menu.items
                        .get(idx)
                        .map(|it| (menu.service.clone(), menu.path.clone(), it.id, it.enabled))
                });
                if let Some((service, path, id, enabled)) = target
                    && enabled
                {
                    runtime.dispatch_menu_click_item(service, path, id);
                }
            }
            data.wayland_systray_menu = None;
        }
        return;
    }

    if pos == BarPosition::StatusText {
        let mut ctx = wm.ctx();
        crate::bar::handle_status_text_click(&mut ctx, root, button.to_x11_detail(), clean_state);
        return;
    }

    let mut ctx = wm.ctx();
    let crate::contexts::WmCtx::Wayland(ref mut wayland_ctx) = ctx else {
        return;
    };
    run_wayland_bar_bindings(wayland_ctx, pos, button, root, clean_state);
}

pub fn handle_wayland_bar_scroll(
    wm: &mut Wm,
    pos: BarPosition,
    delta: f64,
    root: Point,
    clean_state: u32,
) {
    let button = if delta > 0.0 {
        MouseButton::ScrollUp
    } else {
        MouseButton::ScrollDown
    };
    let mut ctx = wm.ctx();
    let crate::contexts::WmCtx::Wayland(ref mut wayland_ctx) = ctx else {
        return;
    };
    run_wayland_bar_bindings(wayland_ctx, pos, button, root, clean_state);
}

fn run_wayland_bar_bindings(
    ctx: &mut WmCtxWayland<'_>,
    pos: BarPosition,
    btn: MouseButton,
    root: Point,
    clean_state: u32,
) {
    let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx.reborrow());
    crate::mouse::bindings::run_all(
        &mut wm_ctx,
        crate::mouse::bindings::ButtonBindingEvent {
            target: ButtonTarget::Bar(pos),
            window: None,
            button: btn,
            root,
            clean_state,
        },
        0,
    );
}

/// Linux evdev button codes (from `<linux/input-event-codes.h>`).
///
/// BTN_LEFT   = 0x110 — primary mouse button.
/// BTN_RIGHT  = 0x111 — secondary mouse button.
/// BTN_MIDDLE = 0x112 — middle / scroll-wheel click.
///
/// The WM uses 1-indexed button numbers matching the X11 convention so that
/// the same button-binding table works on both backends.
const BTN_LEFT: u32 = 0x110;
const BTN_MIDDLE: u32 = 0x112;
const BTN_RIGHT: u32 = 0x111;

pub fn wayland_button_to_mouse_button(code: u32) -> Option<MouseButton> {
    match code {
        BTN_LEFT => Some(MouseButton::Left),
        BTN_MIDDLE => Some(MouseButton::Middle),
        BTN_RIGHT => Some(MouseButton::Right),
        _ => None,
    }
}
