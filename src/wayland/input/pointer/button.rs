//! Pointer button handling.

use smithay::backend::input::{InputBackend, PointerButtonEvent};
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::PointerHandle;
use smithay::utils::{Point, SERIAL_COUNTER};

use crate::backend::Backend;
use crate::backend::wayland::compositor::{
    KeyboardFocusTarget, PointerFocusTarget, WaylandRuntime, WaylandState,
};
use crate::types::MouseButton;
use crate::wayland::common::modifiers_to_x11_mask;

use crate::wayland::input::bar::{
    dispatch_wayland_bar_click, update_wayland_bar_hit_state, wayland_button_to_wm_button,
};
use crate::wayland::input::pointer::drag::{
    wayland_hover_resize_drag_begin, wayland_hover_resize_drag_finish,
};

/// Handle pointer button events.
pub fn handle_pointer_button<B: InputBackend>(
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandRuntime>,
    keyboard_handle: &KeyboardHandle<WaylandRuntime>,
    event: impl PointerButtonEvent<B>,
    pointer_location: Point<f64, smithay::utils::Logical>,
) {
    let serial = SERIAL_COUNTER.next_serial();
    let root_x = pointer_location.x.round() as i32;
    let root_y = pointer_location.y.round() as i32;
    let wm_button = wayland_button_to_wm_button(event.button_code()).and_then(MouseButton::from_u8);

    if event.state() == smithay::backend::input::ButtonState::Pressed {
        // Bar interactions must short-circuit the generic surface click path.
        if let Some(pos) = update_wayland_bar_hit_state(&mut state.wm, root_x, root_y, true) {
            let clean_state = crate::util::clean_mask(
                modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                0,
            );
            dispatch_wayland_bar_click(
                &mut state.wm,
                pos,
                event.button_code(),
                root_x,
                root_y,
                clean_state,
            );
            pointer_handle.frame(WaylandRuntime::from_state_mut(state));
            return;
        }

        if let Some(btn) = wm_button {
            let ctx = state.wm.ctx();
            if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx
                && wayland_hover_resize_drag_begin(&mut ctx, root_x, root_y, btn)
            {
                return;
            }
        }

        // Resolve the window directly under the pointer via Smithay's surface hit-test.
        let clicked_win = find_hovered_window(&state.wm, state, pointer_location);

        // Update focus before dispatching client button bindings.
        if let Some((layer_surface, location)) = state.layer_surface_under_pointer(pointer_location)
        {
            keyboard_handle.set_focus(
                WaylandRuntime::from_state_mut(state),
                Some(KeyboardFocusTarget::WlSurface(layer_surface.clone())),
                serial,
            );
            let focus = Some((
                PointerFocusTarget::WlSurface(layer_surface),
                location.to_f64(),
            ));
            let motion = smithay::input::pointer::MotionEvent {
                location: pointer_location,
                serial,
                time: event.time_msec(),
            };
            pointer_handle.motion(WaylandRuntime::from_state_mut(state), focus, &motion);
            pointer_handle.frame(WaylandRuntime::from_state_mut(state));
        } else if let Some(win) = clicked_win {
            let mut ctx = state.wm.ctx();
            crate::focus::focus_soft(&mut ctx, Some(win));
        } else {
            let mut ctx = state.wm.ctx();
            crate::focus::focus_soft(&mut ctx, None);
        }

        let mut consumed = false;
        if let Some(btn) = wm_button {
            let clean_state = crate::util::clean_mask(
                modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                0,
            );
            consumed =
                dispatch_wayland_client_button(&mut state.wm, btn, root_x, root_y, clean_state);
        }

        if !consumed {
            let button = smithay::input::pointer::ButtonEvent {
                serial,
                time: event.time_msec(),
                button: event.button_code(),
                state: event.state(),
            };
            pointer_handle.button(WaylandRuntime::from_state_mut(state), &button);
        }

        let maybe_close = if !consumed {
            let core = crate::contexts::CoreCtx::new(
                &mut state.wm.g,
                &mut state.wm.running,
                &mut state.wm.bar,
                &mut state.wm.focus,
            );
            let mon = core.globals().selected_monitor().clone();
            let local_x = root_x - mon.work_rect.x;

            match &mut state.wm.backend {
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
            }
        } else {
            false
        };
        if maybe_close {
            if let Backend::Wayland(data) = &mut state.wm.backend {
                data.wayland_systray_menu = None;
            }
            state.wm.bar.mark_dirty();
        }
    } else if event.state() == smithay::backend::input::ButtonState::Released {
        if let Some(btn) = wm_button {
            let ctx = state.wm.ctx();
            if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx
                && wayland_hover_resize_drag_finish(&mut ctx, btn)
            {
                return;
            }
        }

        let released_btn = wm_button;
        let is_wm_drag = (state.wm.g.drag.interactive.active
            && released_btn == Some(state.wm.g.drag.interactive.button))
            || (state.wm.g.drag.tag.active && released_btn == Some(state.wm.g.drag.tag.button));

        if !is_wm_drag {
            let button = smithay::input::pointer::ButtonEvent {
                serial,
                time: event.time_msec(),
                button: event.button_code(),
                state: event.state(),
            };
            pointer_handle.button(WaylandRuntime::from_state_mut(state), &button);
        }

        if state.wm.g.drag.tag.active && released_btn == Some(state.wm.g.drag.tag.button) {
            let mod_state = modifiers_to_x11_mask(&keyboard_handle.modifier_state());
            let mut ctx = state.wm.ctx();
            crate::mouse::drag_tag_finish(&mut ctx, mod_state);
        }

        if state.wm.g.drag.interactive.active
            && released_btn == Some(state.wm.g.drag.interactive.button)
        {
            let mut ctx = state.wm.ctx();
            crate::mouse::title_drag_finish(&mut ctx);
        }
    }

    pointer_handle.frame(WaylandRuntime::from_state_mut(state));
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Find the window under the pointer.
pub fn find_hovered_window(
    wm: &crate::wm::Wm,
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
    wm: &crate::wm::Wm,
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

/// Dispatch client button event.
fn dispatch_wayland_client_button(
    wm: &mut crate::wm::Wm,
    btn: MouseButton,
    root_x: i32,
    root_y: i32,
    clean_state: u32,
) -> bool {
    let buttons = wm.g.cfg.buttons.clone();
    for b in &buttons {
        if !b.matches(crate::types::BarPosition::ClientWin) || b.button != btn {
            continue;
        }
        if crate::util::clean_mask(b.mask, 0) != clean_state {
            continue;
        }
        let mut ctx = wm.ctx();
        (b.action)(
            &mut ctx,
            crate::types::ButtonArg {
                pos: crate::types::BarPosition::ClientWin,
                btn: b.button,
                rx: root_x,
                ry: root_y,
            },
        );
        return true;
    }
    false
}
