use crate::backend::Backend;
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
    pos: BarPosition,
    button_code: u32,
    root: Point,
    clean_state: u32,
) {
    let Some(button) = MouseButton::from_wayland_code(button_code) else {
        return;
    };

    if matches!(pos, BarPosition::SystrayMenuItem(_)) {
        let BarPosition::SystrayMenuItem(idx) = pos else {
            return;
        };
        let Backend::Wayland(data) = &mut wm.backend else {
            return;
        };
        if button == MouseButton::Left
            && let Some(entry) = data
                .wayland_systray_menu
                .as_ref()
                .and_then(|menu| menu.entries.get(idx))
            && entry.enabled
            && !entry.separator
            && let Some(runtime) = data.wayland_systray_runtime.as_ref()
        {
            runtime.dispatch_menu_action(entry.action);
        }
        return;
    }

    close_systray_menu(wm);

    if matches!(pos, BarPosition::SystrayItem(_)) {
        let BarPosition::SystrayItem(idx) = pos else {
            return;
        };
        // Destructure backend to avoid multiple mutable borrows
        let Backend::Wayland(data) = &mut wm.backend else {
            return;
        };
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
    run_bar_bindings(wayland_ctx, pos, button, root, clean_state);
}

/// Close the bar-hosted DBusMenu, returning whether a menu was open.
pub fn close_systray_menu(wm: &mut Wm) -> bool {
    let Backend::Wayland(data) = &mut wm.backend else {
        return false;
    };
    if data.wayland_systray_menu.take().is_none() {
        return false;
    }
    if let Some(runtime) = data.wayland_systray_runtime.as_ref() {
        runtime.close_menu();
    }
    wm.bar.mark_dirty();
    true
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
    run_bar_bindings(wayland_ctx, pos, button, root, clean_state);
}

fn run_bar_bindings(
    ctx: &mut WmCtxWayland<'_>,
    pos: BarPosition,
    btn: MouseButton,
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
            root,
            clean_state,
        },
        0,
        crate::mouse::bindings::MatchPolicy::All,
    );
}
