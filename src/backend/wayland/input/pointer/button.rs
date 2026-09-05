//! Pointer button handling.

use smithay::backend::input::ButtonState;
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::{ButtonEvent, MotionEvent, PointerHandle};
use smithay::utils::{Point, SERIAL_COUNTER};

use crate::backend::wayland::commands::PointerButtonCommand;
use crate::backend::wayland::compositor::layer_shell::LayerFocusRequest;
use crate::backend::wayland::compositor::{PointerFocusTarget, WaylandState};
use crate::backend::wayland::input::modifiers_to_x11_mask;
use crate::types::{MouseButton, Point as RootPoint};
use crate::wm::Wm;

#[derive(Debug, Clone, Copy)]
pub(crate) struct PointerButtonInput {
    pub event: PointerButtonCommand,
    pub location: Point<f64, smithay::utils::Logical>,
}

pub(crate) fn handle_pointer_button(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer: &PointerHandle<WaylandState>,
    keyboard: &KeyboardHandle<WaylandState>,
    input: PointerButtonInput,
) {
    let serial = SERIAL_COUNTER.next_serial();
    let root = RootPoint::from_f64_round(input.location.x, input.location.y);
    let wm_button = MouseButton::from_wayland_code(input.event.code);

    let button = ButtonPress {
        serial,
        time: input.event.time_msec,
        button_code: input.event.code,
        state: input.event.state,
        root,
        wm_button,
        pointer_location: input.location,
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
        state.dismiss_native_systray_menu();
        focus_layer_button_target(state, pointer_handle, button, layer_surface, location);
        forward_button(state, pointer_handle, button);
        return false;
    }

    let clicked_win = state.logical_window_under_pointer(button.pointer_location);
    if state.is_pointer_over_overlay(button.pointer_location) {
        if !state
            .active_systray_menu()
            .is_some_and(|active| Some(active.win) == clicked_win)
        {
            state.dismiss_native_systray_menu();
        }
        forward_button(state, pointer_handle, button);
        return false;
    }

    let clean_modifiers = clean_modifier_state(keyboard_handle);
    let input = crate::mouse::press::PressInput {
        root: button.root,
        button: button.wm_button,
        raw_button: button.wm_button.map(|b| b.to_x11_detail()).unwrap_or(0),
        modifiers: clean_modifiers,
        clicked_window: clicked_win,
        source: crate::types::InteractionSource::Pointer,
        time_msec: button.time,
    };

    let outcome = {
        let mut ctx = wm.ctx();
        crate::mouse::press::dispatch_press_policy(&mut ctx, input)
    };

    let toggled_native_menu = match outcome {
        crate::mouse::press::PressOutcome::SystrayIconPress {
            index,
            button: MouseButton::Right,
            ..
        } => wm
            .bar
            .systray_host
            .tray
            .items
            .get(index)
            .is_some_and(|item| state.native_systray_menu_matches(&item.service, &item.path)),
        _ => false,
    };
    state.dismiss_native_systray_menu();

    match outcome {
        crate::mouse::press::PressOutcome::CapturedInteraction { .. } => {
            pointer_handle.frame(state);
            true
        }
        crate::mouse::press::PressOutcome::Consumed => {
            pointer_handle.frame(state);
            true
        }
        crate::mouse::press::PressOutcome::SystrayIconPress {
            index,
            button,
            root,
        } => {
            if !toggled_native_menu {
                let mut ctx = wm.ctx();
                crate::systray::press_icon(ctx.core_mut(), index, button, root);
            }
            pointer_handle.frame(state);
            true
        }
        crate::mouse::press::PressOutcome::ReplayToClient { .. } => {
            forward_button(state, pointer_handle, button);
            close_bar_systray_menu(wm, state);
            false
        }
    }
}

fn close_bar_systray_menu(wm: &mut Wm, state: &mut WaylandState) {
    if crate::backend::wayland::input::bar::close_systray_menu(wm) {
        state.request_bar_redraw();
    }
}

fn focus_layer_button_target(
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    button: ButtonPress,
    layer_surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    location: Point<i32, smithay::utils::Logical>,
) {
    state.focus_layer_keyboard(
        &layer_surface,
        button.serial,
        LayerFocusRequest::UserInteraction,
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
    crate::backend::wayland::input::pointer::motion::dispatch_smithay_pointer_motion(
        state,
        pointer_handle,
        focus,
        &motion,
    );
    pointer_handle.frame(state);
}

fn handle_button_release(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    button: ButtonPress,
) -> bool {
    let was_captured = is_wm_drag_release(wm, button.wm_button);
    if !was_captured {
        forward_button(state, pointer_handle, button);
    }
    if let Some(btn) = button.wm_button {
        let occupied = state
            .layer_surface_under_pointer(button.pointer_location)
            .is_some()
            || state.is_pointer_over_overlay(button.pointer_location)
            || state
                .logical_window_under_pointer(button.pointer_location)
                .is_some();
        let hover_target = (!occupied)
            .then(|| crate::mouse::pointer::sidebar_target_at(&wm.core.model, button.root))
            .flatten();
        let mut ctx = wm.ctx();
        let outcome = crate::mouse::interaction::handle(
            &mut ctx,
            crate::mouse::interaction::InteractionEvent::pointer_end(
                button.root,
                btn,
                clean_modifier_state(keyboard_handle),
                hover_target,
                button.time,
            ),
        );
        if outcome.captured() {
            pointer_handle.frame(state);
        }
        return outcome.captured();
    }
    was_captured
}

fn is_wm_drag_release(wm: &Wm, released_btn: Option<MouseButton>) -> bool {
    wm.core.interaction.drag.captured_source() == Some(crate::types::InteractionSource::Pointer)
        && wm.core.interaction.drag.captured_button() == released_btn
}
