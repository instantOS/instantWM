//! Keyboard input handling for Wayland compositor.

use smithay::backend::input::{InputBackend, KeyboardKeyEvent};
use smithay::input::keyboard::{FilterResult, KeyboardHandle};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, LayerSurfaceCachedState};

use crate::backend::wayland::compositor::{KeyboardFocusTarget, WaylandState};
use crate::wayland::common::modifiers_to_x11_mask;

use smithay::utils::SERIAL_COUNTER;
use smithay::wayland::keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat;

/// Handle keyboard events.
pub fn handle_keyboard<B: InputBackend>(
    state: &mut WaylandState,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: impl KeyboardKeyEvent<B>,
) {
    let serial = SERIAL_COUNTER.next_serial();
    let wm_shortcuts_allowed = if keyboard_handle.is_grabbed() {
        false
    } else {
        if state.seat.keyboard_shortcuts_inhibited() {
            false
        } else {
            match keyboard_handle.current_focus() {
                None => true,
                Some(KeyboardFocusTarget::Window(_)) => true,
                Some(KeyboardFocusTarget::WlSurface(ref s)) => {
                    let is_exclusive = with_states(s, |states| {
                        if states.cached_state.has::<LayerSurfaceCachedState>() {
                            states
                                .cached_state
                                .get::<LayerSurfaceCachedState>()
                                .current()
                                .keyboard_interactivity
                                == KeyboardInteractivity::Exclusive
                        } else {
                            false
                        }
                    });
                    !is_exclusive
                }
                Some(KeyboardFocusTarget::Popup(_)) => false,
            }
        }
    };
    let key_code = event.key_code();
    let key_state = event.state();
    keyboard_handle.input(
        state,
        key_code,
        key_state,
        serial,
        event.time_msec(),
        |data: &mut WaylandState, modifiers, keysym| {
            if key_state == smithay::backend::input::KeyState::Released {
                return FilterResult::Forward;
            }
            if wm_shortcuts_allowed {
                let mod_mask = modifiers_to_x11_mask(modifiers);
                let ctx = data.wm.ctx();
                let crate::contexts::WmCtx::Wayland(ctx) = ctx else {
                    return FilterResult::Forward;
                };
                let mut wm_ctx = crate::contexts::WmCtx::Wayland(ctx);
                if crate::keyboard::handle_keysym(
                    &mut wm_ctx,
                    keysym.raw_syms().first().map_or(0, |ks| ks.raw()),
                    mod_mask,
                ) {
                    return FilterResult::Intercept(());
                }
            }
            FilterResult::Forward
        },
    );
}
