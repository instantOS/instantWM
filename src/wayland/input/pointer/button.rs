//! Pointer button handling.

use smithay::backend::input::{ButtonState, InputBackend, PointerButtonEvent};
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::{ButtonEvent, MotionEvent, PointerHandle};
use smithay::utils::{Point, SERIAL_COUNTER};

use crate::backend::Backend;
use crate::backend::wayland::compositor::{KeyboardFocusTarget, PointerFocusTarget, WaylandState};
use crate::mouse::pointer::PointerRegion;
use crate::types::{MouseButton, Point as RootPoint};
use crate::wayland::common::modifiers_to_x11_mask;
use crate::wm::Wm;

use crate::wayland::input::bar::{handle_wayland_bar_click, wayland_button_to_mouse_button};
use crate::wayland::input::pointer::drag::{
    wayland_hover_resize_drag_begin, wayland_hover_resize_drag_finish,
};

/// Handle pointer button events.
pub fn handle_pointer_button<B: InputBackend>(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: impl PointerButtonEvent<B>,
    pointer_location: Point<f64, smithay::utils::Logical>,
) {
    let serial = SERIAL_COUNTER.next_serial();
    let root = RootPoint::new(
        pointer_location.x.round() as i32,
        pointer_location.y.round() as i32,
    );
    let wm_button = wayland_button_to_mouse_button(event.button_code());

    let button = ButtonPress {
        serial,
        time: event.time_msec(),
        button_code: event.button_code(),
        state: event.state(),
        root,
        wm_button,
        pointer_location,
    };

    if state.is_locked() {
        forward_button(state, pointer_handle, button);
        pointer_handle.frame(state);
        return;
    }

    let handled = match button.state {
        ButtonState::Pressed => {
            handle_button_press(wm, state, pointer_handle, keyboard_handle, button)
        }
        ButtonState::Released => {
            handle_button_release(wm, state, pointer_handle, keyboard_handle, button)
        }
    };
    if handled {
        return;
    }

    pointer_handle.frame(state);
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct ButtonPress {
    serial: smithay::utils::Serial,
    time: u32,
    button_code: u32,
    state: ButtonState,
    root: RootPoint,
    wm_button: Option<MouseButton>,
    pointer_location: Point<f64, smithay::utils::Logical>,
}

fn forward_button(
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    button: ButtonPress,
) {
    pointer_handle.button(
        state,
        &ButtonEvent {
            serial: button.serial,
            time: button.time,
            button: button.button_code,
            state: button.state,
        },
    );
}

fn clean_modifier_state(keyboard_handle: &KeyboardHandle<WaylandState>) -> u32 {
    crate::util::clean_mask(modifiers_to_x11_mask(&keyboard_handle.modifier_state()), 0)
}

fn handle_button_press(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    button: ButtonPress,
) -> bool {
    let clicked_win = find_hovered_window(wm, state, button.pointer_location);
    let pointer_region = {
        let mut ctx = wm.ctx();
        crate::mouse::pointer::button_region_at(ctx.core_mut(), button.root, clicked_win)
    };

    match pointer_region {
        PointerRegion::Bar { pos, .. } => {
            handle_wayland_bar_click(
                wm,
                pos,
                button.button_code,
                button.root,
                clean_modifier_state(keyboard_handle),
            );
            pointer_handle.frame(state);
            return true;
        }
        PointerRegion::Sidebar(_) => {
            if let Some(btn) = button.wm_button {
                let _ = consume_wayland_pointer_binding(
                    wm,
                    pointer_region,
                    btn,
                    button.root,
                    clean_modifier_state(keyboard_handle),
                );
            }
            pointer_handle.frame(state);
            return true;
        }
        PointerRegion::Client(_) | PointerRegion::Root { .. } => {}
    }

    if begin_hover_resize_drag(wm, button) {
        return true;
    }

    let on_layer_surface = focus_button_target(
        wm,
        state,
        pointer_handle,
        keyboard_handle,
        button,
        clicked_win,
    );

    let consumed = !on_layer_surface
        && button.wm_button.is_some_and(|btn| {
            consume_wayland_pointer_binding(
                wm,
                pointer_region,
                btn,
                button.root,
                clean_modifier_state(keyboard_handle),
            )
        });

    if !consumed {
        forward_button(state, pointer_handle, button);
        close_wayland_systray_menu_if_outside(wm, state, button.root.x);
    }

    false
}

fn begin_hover_resize_drag(wm: &mut Wm, button: ButtonPress) -> bool {
    let Some(btn) = button.wm_button else {
        return false;
    };
    let ctx = wm.ctx();
    if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx {
        wayland_hover_resize_drag_begin(&mut ctx, button.root, btn)
    } else {
        false
    }
}

fn focus_button_target(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    button: ButtonPress,
    clicked_win: Option<crate::types::WindowId>,
) -> bool {
    if let Some((layer_surface, location)) =
        state.layer_surface_under_pointer(button.pointer_location)
    {
        keyboard_handle.set_focus(
            state,
            Some(KeyboardFocusTarget::WlSurface(layer_surface.clone())),
            button.serial,
        );
        let focus = Some((
            PointerFocusTarget::WlSurface(layer_surface),
            location.to_f64(),
        ));
        let motion = MotionEvent {
            location: button.pointer_location,
            serial: button.serial,
            time: button.time,
        };
        pointer_handle.motion(state, focus, &motion);
        pointer_handle.frame(state);
        return true;
    }

    let mut ctx = wm.ctx();
    if let Some(win) = clicked_win {
        crate::focus::select_monitor_for_client(&mut ctx, win);
        crate::focus::focus_soft(&mut ctx, Some(win));
    } else {
        crate::focus::focus_soft(&mut ctx, None);
    }
    false
}

fn close_wayland_systray_menu_if_outside(wm: &mut Wm, state: &mut WaylandState, root_x: i32) {
    let core =
        crate::contexts::CoreCtx::new(&mut wm.g, &mut wm.running, &mut wm.bar, &mut wm.focus);
    let mon = core.globals().selected_monitor().clone();
    let local_x = root_x - mon.work_rect.x;

    let should_close = match &mut wm.backend {
        Backend::Wayland(data) => {
            data.wayland_systray_menu.as_ref().is_some()
                && crate::systray::wayland::hit_test_wayland_systray_menu_item(
                    &core,
                    &data.wayland_systray,
                    data.wayland_systray_menu.as_ref(),
                    &mon,
                    local_x,
                )
                .is_none()
        }
        Backend::X11(_) => false,
    };

    if should_close {
        if let Backend::Wayland(data) = &mut wm.backend {
            data.wayland_systray_menu = None;
        }
        state.request_bar_redraw();
    }
}

fn handle_button_release(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    button: ButtonPress,
) -> bool {
    if finish_hover_resize_drag(wm, button) {
        return true;
    }

    if !is_wm_drag_release(wm, button.wm_button) {
        forward_button(state, pointer_handle, button);
    }

    if wm.g.drag.tag.active && button.wm_button == Some(wm.g.drag.tag.button) {
        let mod_state = modifiers_to_x11_mask(&keyboard_handle.modifier_state());
        let mut ctx = wm.ctx();
        crate::mouse::drag_tag_finish(&mut ctx, mod_state);
    }

    if wm.g.drag.interactive.active && button.wm_button == Some(wm.g.drag.interactive.button) {
        let mut ctx = wm.ctx();
        crate::mouse::title_drag_finish(&mut ctx);
    }

    if wm.g.drag.gesture.active
        && let Some(btn) = button.wm_button
    {
        let mut ctx = wm.ctx();
        let _ = crate::mouse::finish_sidebar_gesture(&mut ctx, btn);
    }

    false
}

fn finish_hover_resize_drag(wm: &mut Wm, button: ButtonPress) -> bool {
    let Some(btn) = button.wm_button else {
        return false;
    };
    let ctx = wm.ctx();
    if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx {
        wayland_hover_resize_drag_finish(&mut ctx, btn)
    } else {
        false
    }
}

fn is_wm_drag_release(wm: &Wm, released_btn: Option<MouseButton>) -> bool {
    (wm.g.drag.interactive.active && released_btn == Some(wm.g.drag.interactive.button))
        || (wm.g.drag.tag.active && released_btn == Some(wm.g.drag.tag.button))
        || (wm.g.drag.gesture.active && released_btn == Some(wm.g.drag.gesture.button))
}

/// Find the window under the pointer.
pub fn find_hovered_window(
    wm: &Wm,
    state: &WaylandState,
    pointer_location: Point<f64, smithay::utils::Logical>,
) -> Option<crate::types::WindowId> {
    if let Some((surface, _)) = state.layer_surface_under_pointer(pointer_location) {
        return find_hovered_window_for_surface(wm, &surface);
    }
    state.logical_window_under_pointer(pointer_location)
}

/// Find hovered window for a surface.
fn find_hovered_window_for_surface(
    wm: &Wm,
    surface: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
) -> Option<crate::types::WindowId> {
    use smithay::wayland::compositor::with_states;

    if let Some(win) = with_states(surface, |states| {
        states
            .data_map
            .get::<crate::backend::wayland::compositor::WindowIdMarker>()
            .map(|marker| marker.id)
    }) {
        return Some(win);
    }

    let backend = match &wm.backend {
        crate::backend::Backend::Wayland(data) => &data.backend,
        _ => return None,
    };

    backend
        .with_state(|state| state.window_id_for_surface(surface))
        .flatten()
}

fn consume_wayland_pointer_binding(
    wm: &mut Wm,
    region: PointerRegion,
    btn: MouseButton,
    root: RootPoint,
    clean_state: u32,
) -> bool {
    let clicked_win = match region {
        PointerRegion::Client(win) => Some(win),
        _ => None,
    };
    let target = region.to_button_target();
    let mut ctx = wm.ctx();
    crate::mouse::bindings::consume_one(
        &mut ctx,
        crate::mouse::bindings::ButtonBindingEvent {
            target,
            window: clicked_win,
            button: btn,
            root,
            clean_state,
        },
        0,
    )
}
