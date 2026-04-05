//! Keyboard input handling for Wayland compositor.

use smithay::backend::input::{InputBackend, KeyboardKeyEvent};
use smithay::input::keyboard::{FilterResult, KeyboardHandle};
use smithay::wayland::compositor::with_states;
use smithay::wayland::shell::wlr_layer::{KeyboardInteractivity, LayerSurfaceCachedState};

use crate::backend::wayland::compositor::{KeyboardFocusTarget, WaylandState};
use crate::wayland::common::modifiers_to_x11_mask;
use crate::wm::Wm;

use smithay::utils::SERIAL_COUNTER;

/// Handle keyboard events.
pub fn handle_keyboard<B: InputBackend>(
    wm: &mut Wm,
    state: &mut WaylandState,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: impl KeyboardKeyEvent<B>,
) {
    let serial = SERIAL_COUNTER.next_serial();
    let wm_shortcuts_allowed = match keyboard_handle.current_focus() {
        None => true,
        Some(KeyboardFocusTarget::Window(ref w)) => {
            // Use the unified window classifier to determine if shortcuts
            // should be suppressed. This handles all overlay types consistently.
            !state.should_suppress_shortcuts_for(w)
        }
        // Layer shell surfaces (e.g. rofi, dmenu) that request exclusive keyboard
        // interactivity must receive all input — suppress WM shortcuts for them.
        // Non-exclusive surfaces (e.g. the bar) still allow WM shortcuts.
        Some(KeyboardFocusTarget::WlSurface(ref s)) => {
            let interactivity = with_states(s, |states| {
                states
                    .cached_state
                    .get::<LayerSurfaceCachedState>()
                    .current()
                    .keyboard_interactivity
            });
            interactivity != KeyboardInteractivity::Exclusive
        }
        Some(KeyboardFocusTarget::Popup(_)) => false,
    };
    let key_code = event.key_code();
    let key_state = event.state();
    keyboard_handle.input(
        state,
        key_code,
        key_state,
        serial,
        event.time_msec(),
        |_data, modifiers, keysym| {
            if key_state == smithay::backend::input::KeyState::Released {
                return FilterResult::Forward;
            }
            if wm_shortcuts_allowed {
                let mod_mask = modifiers_to_x11_mask(modifiers);
                let ctx = wm.ctx();
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
