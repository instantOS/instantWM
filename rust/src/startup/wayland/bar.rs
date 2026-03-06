use crate::bar::{bar_position_at_x, bar_position_to_gesture};
use crate::types::*;
use crate::wm::Wm;

pub(super) fn update_wayland_bar_hit_state(
    wm: &mut Wm,
    root_x: i32,
    root_y: i32,
    reset_start_menu: bool,
) -> Option<BarPosition> {
    let rect = Rect {
        x: root_x,
        y: root_y,
        w: 1,
        h: 1,
    };
    let mid = crate::types::find_monitor_by_rect(wm.g.monitors.monitors(), &rect)?;
    let mut ctx = wm.ctx();
    if mid != ctx.g.selected_monitor_id() {
        ctx.g.monitors.set_sel_idx(mid);
    }

    let bar_h = ctx.g.cfg.bar_height.max(1);
    let mon = ctx.g.selected_monitor();
    let in_bar = mon.showbar && root_y >= mon.bar_y && root_y < mon.bar_y + bar_h;
    if !in_bar {
        let had_hover = mon.gesture != crate::types::Gesture::None;
        if had_hover {
            crate::bar::reset_bar_wayland(ctx.core_mut());
        }
        return None;
    }

    let mon = ctx.g.selected_monitor();
    let local_x = root_x - mon.work_rect.x;
    let pos = bar_position_at_x(&mon, ctx.core(), local_x);
    if reset_start_menu && pos == BarPosition::StartMenu {
        crate::bar::reset_bar_wayland(ctx.core_mut());
    }

    let gesture = if pos == BarPosition::StatusText {
        ctx.g.selected_monitor().gesture
    } else {
        bar_position_to_gesture(pos)
    };
    ctx.g.selected_monitor_mut().gesture = gesture;

    Some(pos)
}

pub(super) fn dispatch_wayland_bar_click(
    wm: &mut Wm,
    pos: BarPosition,
    button_code: u32,
    root_x: i32,
    root_y: i32,
    clean_state: u32,
) {
    let Some(button_code) = wayland_button_to_wm_button(button_code) else {
        return;
    };
    let Some(button) = MouseButton::from_u8(button_code) else {
        return;
    };

    if matches!(pos, BarPosition::SystrayItem(_)) {
        if let Some(runtime) = wm.wayland_systray.as_ref() {
            let BarPosition::SystrayItem(idx) = pos else {
                return;
            };
            let target =
                wm.g.wayland_systray
                    .items
                    .get(idx)
                    .map(|it| (it.service.clone(), it.path.clone()));
            if let Some((service, path)) = target {
                runtime.dispatch_click_item(service, path, button, root_x, root_y);
            }
        }
        wm.g.wayland_systray_menu = None;
        return;
    }

    if matches!(pos, BarPosition::SystrayMenuItem(_)) {
        if let Some(runtime) = wm.wayland_systray.as_ref() {
            let BarPosition::SystrayMenuItem(idx) = pos else {
                return;
            };
            let target = wm.g.wayland_systray_menu.as_ref().and_then(|menu| {
                menu.items
                    .get(idx)
                    .map(|it| (menu.service.clone(), menu.path.clone(), it.id, it.enabled))
            });
            if let Some((service, path, id, enabled)) = target {
                if enabled {
                    runtime.dispatch_menu_click_item(service, path, id);
                }
            }
        }
        wm.g.wayland_systray_menu = None;
        return;
    }

    let mut ctx = wm.ctx();
    dispatch_wayland_bar_button(&mut ctx, pos, button, root_x, root_y, clean_state);
}

pub(super) fn dispatch_wayland_bar_scroll(
    wm: &mut Wm,
    pos: BarPosition,
    delta: f64,
    root_x: i32,
    root_y: i32,
    clean_state: u32,
) {
    let button = if delta > 0.0 {
        MouseButton::ScrollUp
    } else {
        MouseButton::ScrollDown
    };
    let mut ctx = wm.ctx();
    dispatch_wayland_bar_button(&mut ctx, pos, button, root_x, root_y, clean_state);
}

fn dispatch_wayland_bar_button(
    ctx: &mut crate::contexts::WmCtx<'_>,
    pos: BarPosition,
    btn: MouseButton,
    root_x: i32,
    root_y: i32,
    clean_state: u32,
) {
    let numlockmask = ctx.g.x11.numlockmask;
    let buttons = ctx.g.cfg.buttons.clone();
    for b in &buttons {
        if !b.matches(pos) || b.button != btn {
            continue;
        }
        if crate::util::clean_mask(b.mask, numlockmask) != clean_state {
            continue;
        }
        (b.action)(
            ctx,
            ButtonArg {
                pos,
                btn: b.button,
                rx: root_x,
                ry: root_y,
            },
        );
    }
}

pub(super) fn wayland_button_to_wm_button(code: u32) -> Option<u8> {
    match code {
        0x110 => Some(1),
        0x112 => Some(2),
        0x111 => Some(3),
        _ => None,
    }
}
