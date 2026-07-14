//! Pointer button handling.

use smithay::backend::input::ButtonState;
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::{ButtonEvent, MotionEvent, PointerHandle};
use smithay::utils::{Point, SERIAL_COUNTER};

use crate::backend::wayland::compositor::{KeyboardFocusTarget, PointerFocusTarget, WaylandState};
use crate::mouse::pointer::PointerRegion;
use crate::types::{MouseButton, Point as RootPoint};
use crate::wayland::common::modifiers_to_x11_mask;
use crate::wm::Wm;

use crate::wayland::input::bar::handle_bar_click;
use crate::wayland::input::pointer::drag::{hover_resize_drag_begin, hover_resize_drag_finish};

/// Internal helper for handling pointer button from raw values.
pub fn handle_pointer_button_raw(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer: &PointerHandle<WaylandState>,
    keyboard: &KeyboardHandle<WaylandState>,
    button: u32,
    btn_state: ButtonState,
    time: u32,
    pointer_location: Point<f64, smithay::utils::Logical>,
) {
    let serial = SERIAL_COUNTER.next_serial();
    let root = RootPoint::new(
        pointer_location.x.round() as i32,
        pointer_location.y.round() as i32,
    );
    let wm_button = MouseButton::from_wayland_code(button);

    let button = ButtonPress {
        serial,
        time,
        button_code: button,
        state: btn_state,
        root,
        wm_button,
        pointer_location,
    };

    if state.is_locked() {
        forward_button(state, pointer, button);
        pointer.frame(state);
        return;
    }

    let handled = match button.state {
        ButtonState::Pressed => handle_button_press(wm, state, pointer, keyboard, button),
        ButtonState::Released => handle_button_release(wm, state, pointer, keyboard, button),
    };
    if handled {
        return;
    }

    pointer.frame(state);
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
    // Layer-shell surfaces are compositor-level UI and must win over every
    // WM-owned pointer region.  In particular, notifications commonly use the
    // overlay layer and may intentionally cover the built-in bar.  Classifying
    // the bar/sidebar first would make the WM consume the press even though
    // pointer motion had already focused the layer surface.
    if let Some((layer_surface, location)) =
        state.layer_surface_under_pointer(button.pointer_location)
    {
        focus_layer_button_target(
            state,
            pointer_handle,
            keyboard_handle,
            button,
            layer_surface,
            location,
        );
        forward_button(state, pointer_handle, button);
        return false;
    }

    let clicked_win = state.logical_window_under_pointer(button.pointer_location);
    let pointer_region = {
        let mut ctx = wm.ctx();
        crate::mouse::pointer::button_region_at(ctx.core_mut(), button.root, clicked_win)
    };

    match pointer_region {
        PointerRegion::Bar { pos, .. } => {
            handle_bar_click(
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
                let _ = consume_pointer_binding(
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

    focus_button_target(wm, clicked_win);

    let consumed = button.wm_button.is_some_and(|btn| {
        consume_pointer_binding(
            wm,
            pointer_region,
            btn,
            button.root,
            clean_modifier_state(keyboard_handle),
        )
    });

    if !consumed {
        forward_button(state, pointer_handle, button);
        close_bar_systray_menu(wm, state);
    }

    false
}

fn close_bar_systray_menu(wm: &mut Wm, state: &mut WaylandState) {
    if crate::wayland::input::bar::close_systray_menu(wm) {
        state.request_bar_redraw();
    }
}

fn begin_hover_resize_drag(wm: &mut Wm, button: ButtonPress) -> bool {
    let Some(btn) = button.wm_button else {
        return false;
    };
    let ctx = wm.ctx();
    if let crate::contexts::WmCtx::Wayland(mut ctx) = ctx {
        hover_resize_drag_begin(&mut ctx, button.root, btn)
    } else {
        false
    }
}

fn focus_button_target(wm: &mut Wm, clicked_win: Option<crate::types::WindowId>) {
    let mut ctx = wm.ctx();
    if let Some(win) = clicked_win {
        crate::focus::select_monitor_for_client(&mut ctx, win);
        crate::focus::focus(&mut ctx, Some(win));
    } else {
        crate::focus::focus(&mut ctx, None);
    }
}

fn focus_layer_button_target(
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    button: ButtonPress,
    layer_surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    location: Point<i32, smithay::utils::Logical>,
) {
    if crate::backend::wayland::compositor::layer_shell::layer_surface_accepts_keyboard_focus(
        &layer_surface,
    ) {
        keyboard_handle.set_focus(
            state,
            Some(KeyboardFocusTarget::WlSurface(layer_surface.clone())),
            button.serial,
        );
    }
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

    if wm.core.drag.tag.active && button.wm_button == Some(wm.core.drag.tag.button) {
        let mod_state = modifiers_to_x11_mask(&keyboard_handle.modifier_state());
        let mut ctx = wm.ctx();
        crate::mouse::drag_tag_finish(&mut ctx, mod_state);
    }

    if wm.core.drag.interactive.active && button.wm_button == Some(wm.core.drag.interactive.button)
    {
        let mut ctx = wm.ctx();
        crate::mouse::title_drag_finish(&mut ctx);
    }

    if wm.core.drag.gesture.active
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
        hover_resize_drag_finish(&mut ctx, btn)
    } else {
        false
    }
}

fn is_wm_drag_release(wm: &Wm, released_btn: Option<MouseButton>) -> bool {
    (wm.core.drag.interactive.active && released_btn == Some(wm.core.drag.interactive.button))
        || (wm.core.drag.tag.active && released_btn == Some(wm.core.drag.tag.button))
        || (wm.core.drag.gesture.active && released_btn == Some(wm.core.drag.gesture.button))
}

fn consume_pointer_binding(
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
