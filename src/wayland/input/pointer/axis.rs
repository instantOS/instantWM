//! Pointer axis (scroll) handling.

use smithay::backend::input::{InputBackend, PointerAxisEvent};
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::PointerHandle;
use smithay::utils::Point;

use crate::backend::wayland::compositor::WaylandState;
use crate::config::config_toml::ToggleSetting;
use crate::types::MouseButton;
use crate::wayland::common::modifiers_to_x11_mask;
use crate::wm::Wm;

use crate::wayland::input::bar::{
    dispatch_wayland_bar_scroll, update_wayland_bar_hit_state, wayland_button_to_wm_button,
};
use smithay::utils::SERIAL_COUNTER;

/// Resolve scroll factor from config.
fn resolve_scroll_factor(input_config: &crate::config::config_toml::InputConfig) -> f64 {
    input_config.scroll_factor.unwrap_or(1.0)
}

/// Resolve natural scroll from config.
fn resolve_natural_scroll(input_config: &crate::config::config_toml::InputConfig) -> bool {
    input_config
        .natural_scroll
        .unwrap_or(ToggleSetting::Disabled)
        == ToggleSetting::Enabled
}

/// Handle pointer axis (scroll) events.
pub fn handle_pointer_axis<B: InputBackend>(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer_handle: &PointerHandle<WaylandState>,
    keyboard_handle: &KeyboardHandle<WaylandState>,
    event: impl PointerAxisEvent<B>,
    pointer_location: Point<f64, smithay::utils::Logical>,
) {
    // Get input config with fallback to default ("*")
    let input_config = wm.g.cfg.input.get("*").cloned().unwrap_or_default();
    let scroll_factor = resolve_scroll_factor(&input_config);
    let natural_scroll = resolve_natural_scroll(&input_config);

    // Negate scroll factor when natural scroll is enabled to flip the direction
    let direction_modifier = if natural_scroll { -1.0 } else { 1.0 };
    let effective_factor = scroll_factor * direction_modifier;

    let mut frame = smithay::input::pointer::AxisFrame::new(event.time_msec());
    frame = frame.source(event.source());

    for axis in [
        smithay::backend::input::Axis::Horizontal,
        smithay::backend::input::Axis::Vertical,
    ] {
        if let Some(amount) = event.amount(axis) {
            if amount.abs() >= f64::EPSILON {
                frame = frame.relative_direction(axis, event.relative_direction(axis));
                frame = frame.value(axis, amount * effective_factor);
                if let Some(steps) = event.amount_v120(axis) {
                    frame = frame.v120(axis, (steps as f64 * effective_factor) as i32);
                }
            } else if event.source() == smithay::backend::input::AxisSource::Finger {
                frame = frame.stop(axis);
            }
        }
    }

    let scroll_delta = event
        .amount_v120(smithay::backend::input::Axis::Vertical)
        .map(|s| s as f64)
        .or_else(|| event.amount(smithay::backend::input::Axis::Vertical));
    if let Some(delta) = scroll_delta.filter(|d| *d != 0.0) {
        let root_x = pointer_location.x.round() as i32;
        let root_y = pointer_location.y.round() as i32;
        if let Some(pos) = update_wayland_bar_hit_state(wm, root_x, root_y, true) {
            let clean_state = modifiers_to_x11_mask(&keyboard_handle.modifier_state());
            dispatch_wayland_bar_scroll(wm, pos, delta, root_x, root_y, clean_state);
        }
    }

    pointer_handle.axis(state, frame);
    pointer_handle.frame(state);
}
