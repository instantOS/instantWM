use crate::bar::bar_position_to_gesture;
use crate::bar::status::emit_i3bar_status_click;
use crate::contexts::WmCtxWayland;
use crate::types::*;
use crate::wm::Wm;

pub fn update_wayland_bar_hit_state(
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
            crate::bar::reset_bar_common(ctx.core_mut());
        }
        return None;
    }

    let mon = ctx.g.selected_monitor();
    let local_x = root_x - mon.work_rect.x;
    let pos = mon.bar_position_at_x(ctx.core(), local_x);
    if reset_start_menu && pos == BarPosition::StartMenu {
        crate::bar::reset_bar_common(ctx.core_mut());
    }

    let old_gesture = ctx.g.selected_monitor().gesture;
    let gesture = if pos == BarPosition::StatusText {
        old_gesture
    } else {
        bar_position_to_gesture(pos)
    };
    if old_gesture != gesture {
        ctx.g.selected_monitor_mut().gesture = gesture;
        ctx.request_bar_update(Some(mid));
    }

    Some(pos)
}

pub fn dispatch_wayland_bar_click(
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
        if let Some(runtime) = wm.wayland_systray_runtime.as_ref() {
            let BarPosition::SystrayItem(idx) = pos else {
                return;
            };
            let target = wm
                .wayland_systray
                .items
                .get(idx)
                .map(|it| (it.service.clone(), it.path.clone()));
            if let Some((service, path)) = target {
                runtime.dispatch_click_item(service, path, button, root_x, root_y);
            }
        }
        wm.wayland_systray_menu = None;
        return;
    }

    if matches!(pos, BarPosition::SystrayMenuItem(_)) {
        if let Some(runtime) = wm.wayland_systray_runtime.as_ref() {
            let BarPosition::SystrayMenuItem(idx) = pos else {
                return;
            };
            let target = wm.wayland_systray_menu.as_ref().and_then(|menu| {
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
        wm.wayland_systray_menu = None;
        return;
    }

    if pos == BarPosition::StatusText {
        let selmon = wm.g.selected_monitor().clone();
        let local_x = root_x - selmon.work_rect.x;
        let parsed = wm
            .bar
            .parsed_status_for_text(&wm.g.bar_runtime.status_text)
            .clone();
        let click_targets = wm
            .bar
            .monitor_hit_cache(selmon.id())
            .map(|h| h.status_click_targets.as_slice())
            .unwrap_or(&[]);
        emit_i3bar_status_click(
            &parsed,
            click_targets,
            local_x,
            root_y - selmon.bar_y,
            button_code,
            wm.g.cfg.bar_height,
            clean_state,
        );
    }

    let mut ctx = wm.ctx();
    let crate::contexts::WmCtx::Wayland(ref mut wayland_ctx) = ctx else {
        return;
    };
    dispatch_wayland_bar_button(wayland_ctx, pos, button, root_x, root_y, clean_state);
}

pub fn dispatch_wayland_bar_scroll(
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
    let crate::contexts::WmCtx::Wayland(ref mut wayland_ctx) = ctx else {
        return;
    };
    dispatch_wayland_bar_button(wayland_ctx, pos, button, root_x, root_y, clean_state);
}

fn dispatch_wayland_bar_button(
    ctx: &mut WmCtxWayland<'_>,
    pos: BarPosition,
    btn: MouseButton,
    root_x: i32,
    root_y: i32,
    clean_state: u32,
) {
    // numlockmask is X11-specific; on Wayland modifier state comes pre-cleaned
    // by the compositor, so we treat it as 0.
    const NUMLOCKMASK: u32 = 0;
    let buttons = ctx.core.g.cfg.buttons.clone();
    for b in &buttons {
        if !b.matches(pos) || b.button != btn {
            continue;
        }
        if crate::util::clean_mask(b.mask, NUMLOCKMASK) != clean_state {
            continue;
        }
        let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx.reborrow());
        (b.action)(
            &mut wm_ctx,
            ButtonArg {
                pos,
                btn: b.button,
                rx: root_x,
                ry: root_y,
            },
        );
    }
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

pub fn wayland_button_to_wm_button(code: u32) -> Option<u8> {
    match code {
        BTN_LEFT => Some(1),
        BTN_MIDDLE => Some(2),
        BTN_RIGHT => Some(3),
        _ => None,
    }
}
