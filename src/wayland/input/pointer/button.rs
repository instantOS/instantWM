//! Pointer button handling.

use crate::actions::execute_button_action;
use smithay::backend::input::{InputBackend, PointerButtonEvent};
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::PointerHandle;
use smithay::utils::{Point, SERIAL_COUNTER};

use crate::backend::Backend;
use crate::backend::wayland::compositor::{KeyboardFocusTarget, PointerFocusTarget, WaylandState};
use crate::types::MouseButton;
use crate::wayland::common::modifiers_to_x11_mask;
use crate::wm::Wm;

use crate::wayland::input::bar::{dispatch_wayland_bar_click, wayland_button_to_mouse_button};
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
    let root_x = pointer_location.x.round() as i32;
    let root_y = pointer_location.y.round() as i32;
    let wm_button = wayland_button_to_mouse_button(event.button_code());

    // When the session is locked, just forward the raw button event to the lock surface.
    if state.is_locked() {
        let button = smithay::input::pointer::ButtonEvent {
            serial,
            time: event.time_msec(),
            button: event.button_code(),
            state: event.state(),
        };
        pointer_handle.button(state, &button);
        pointer_handle.frame(state);
        return;
    }

    if event.state() == smithay::backend::input::ButtonState::Pressed {
        let clicked_win = find_hovered_window(wm, state, pointer_location);
        let pointer_region = {
            let mut ctx = wm.ctx();
            crate::mouse::pointer::button_region_at(ctx.core_mut(), root_x, root_y, clicked_win)
        };

        // Bar clicks have their own status/systray handling and must not fall
        // through to generic surface focus.
        if let crate::mouse::pointer::PointerRegion::Bar { pos, .. } = pointer_region {
            let clean_state = crate::util::clean_mask(
                modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                0,
            );
            dispatch_wayland_bar_click(wm, pos, event.button_code(), root_x, root_y, clean_state);
            pointer_handle.frame(state);
            return;
        }

        // Sidebar is WM-owned but not a bar target.
        if matches!(
            pointer_region,
            crate::mouse::pointer::PointerRegion::Sidebar(_)
        ) {
            if let Some(btn) = wm_button {
                let clean_state = crate::util::clean_mask(
                    modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                    0,
                );
                let _ = dispatch_wayland_pointer_button(
                    wm,
                    pointer_region,
                    btn,
                    root_x,
                    root_y,
                    clean_state,
                );
            }
            pointer_handle.frame(state);
            return;
        }

        if let Some(btn) = wm_button {
            let ctx = wm.ctx();
            if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx
                && wayland_hover_resize_drag_begin(&mut ctx, root_x, root_y, btn)
            {
                return;
            }
        }

        // Update focus before dispatching client button bindings.
        let on_layer_surface = if let Some((layer_surface, location)) =
            state.layer_surface_under_pointer(pointer_location)
        {
            keyboard_handle.set_focus(
                state,
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
            pointer_handle.motion(state, focus, &motion);
            pointer_handle.frame(state);
            true
        } else if let Some(win) = clicked_win {
            let mut ctx = wm.ctx();
            crate::focus::select_monitor_for_client(&mut ctx, win);
            crate::focus::focus_soft(&mut ctx, Some(win));
            false
        } else {
            let mut ctx = wm.ctx();
            crate::focus::focus_soft(&mut ctx, None);
            false
        };

        // When a non-WM layer surface (notification, launcher like fuzzel) is
        // under the pointer, forward the click to that surface and skip WM
        // button bindings so we don't treat it as a root-window click.
        let mut consumed = false;
        if !on_layer_surface && let Some(btn) = wm_button {
            let clean_state = crate::util::clean_mask(
                modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                0,
            );
            consumed = dispatch_wayland_pointer_button(
                wm,
                pointer_region,
                btn,
                root_x,
                root_y,
                clean_state,
            );
        }

        if !consumed {
            let button = smithay::input::pointer::ButtonEvent {
                serial,
                time: event.time_msec(),
                button: event.button_code(),
                state: event.state(),
            };
            pointer_handle.button(state, &button);
        }

        let maybe_close = if !consumed {
            let core = crate::contexts::CoreCtx::new(
                &mut wm.g,
                &mut wm.running,
                &mut wm.bar,
                &mut wm.focus,
            );
            let mon = core.globals().selected_monitor().clone();
            let local_x = root_x - mon.work_rect.x;

            // Check if we should close - only Wayland has systray menu

            match &mut wm.backend {
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
            // Only Wayland has a systray menu to close
            if let Backend::Wayland(data) = &mut wm.backend {
                data.wayland_systray_menu = None;
            }
            state.request_bar_redraw();
        }
    } else if event.state() == smithay::backend::input::ButtonState::Released {
        if let Some(btn) = wm_button {
            let ctx = wm.ctx();
            if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx
                && wayland_hover_resize_drag_finish(&mut ctx, btn)
            {
                return;
            }
        }

        let released_btn = wm_button;
        let is_wm_drag = (wm.g.drag.interactive.active
            && released_btn == Some(wm.g.drag.interactive.button))
            || (wm.g.drag.tag.active && released_btn == Some(wm.g.drag.tag.button))
            || (wm.g.drag.gesture.active && released_btn == Some(wm.g.drag.gesture.button));

        if !is_wm_drag {
            let button = smithay::input::pointer::ButtonEvent {
                serial,
                time: event.time_msec(),
                button: event.button_code(),
                state: event.state(),
            };
            pointer_handle.button(state, &button);
        }

        if wm.g.drag.tag.active && released_btn == Some(wm.g.drag.tag.button) {
            let mod_state = modifiers_to_x11_mask(&keyboard_handle.modifier_state());
            let mut ctx = wm.ctx();
            crate::mouse::drag_tag_finish(&mut ctx, mod_state);
        }

        if wm.g.drag.interactive.active && released_btn == Some(wm.g.drag.interactive.button) {
            let mut ctx = wm.ctx();
            crate::mouse::title_drag_finish(&mut ctx);
        }

        if wm.g.drag.gesture.active
            && let Some(btn) = released_btn
        {
            let mut ctx = wm.ctx();
            let _ = crate::mouse::finish_sidebar_gesture(&mut ctx, btn);
        }
    }

    pointer_handle.frame(state);
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

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

/// Dispatch client button event.
fn dispatch_wayland_pointer_button(
    wm: &mut Wm,
    region: crate::mouse::pointer::PointerRegion,
    btn: MouseButton,
    root_x: i32,
    root_y: i32,
    clean_state: u32,
) -> bool {
    let clicked_win = match region {
        crate::mouse::pointer::PointerRegion::Client(win) => Some(win),
        _ => None,
    };
    let target = region.to_button_target();
    let buttons = wm.g.cfg.buttons.clone();
    for b in &buttons {
        if !b.matches(target) || b.button != btn {
            continue;
        }
        if crate::util::clean_mask(b.mask, 0) != clean_state {
            continue;
        }
        let mut ctx = wm.ctx();
        execute_button_action(
            &mut ctx,
            &b.action,
            crate::types::ButtonArg {
                target,
                window: clicked_win,
                btn: b.button,
                rx: root_x,
                ry: root_y,
            },
        );
        return true;
    }
    false
}
