//! Pointer axis (scroll) handling.

use smithay::backend::input::{InputBackend, PointerAxisEvent};
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::PointerHandle;
use smithay::utils::Point;

use crate::backend::wayland::compositor::WaylandState;
use crate::config::config_toml::ToggleSetting;
use crate::wayland::common::modifiers_to_x11_mask;
use crate::wm::Wm;

use crate::wayland::input::bar::{dispatch_wayland_bar_scroll, update_wayland_bar_hit_state};

/// Resolve the effective scroll factor from input configuration.
///
/// Checks `type:pointer`, `type:touchpad`, then `*` (wildcard) entries,
/// returning the first `scroll_factor` found, or `1.0` if none is set.
fn resolve_scroll_factor(
    input_config: &std::collections::HashMap<String, crate::config::config_toml::InputConfig>,
) -> f64 {
    for key in &["type:pointer", "type:touchpad", "*"] {
        if let Some(cfg) = input_config.get(*key)
            && let Some(factor) = cfg.scroll_factor
        {
            return factor.max(0.0);
        }
    }
    1.0
}

/// Resolve the effective natural scroll setting from input configuration.
///
/// Checks `type:pointer`, `type:touchpad`, then `*` (wildcard) entries,
/// returning whether natural scroll is enabled, or `false` if none is set.
fn resolve_natural_scroll(
    input_config: &std::collections::HashMap<String, crate::config::config_toml::InputConfig>,
) -> bool {
    for key in &["type:pointer", "type:touchpad", "*"] {
        if let Some(cfg) = input_config.get(*key)
            && let Some(natural_scroll) = cfg.natural_scroll
        {
            return natural_scroll == ToggleSetting::Enabled;
        }
    }
    false
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
    let scroll_factor = resolve_scroll_factor(&wm.g.cfg.input);
    let natural_scroll = resolve_natural_scroll(&wm.g.cfg.input);

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
                    frame = frame.v120(axis, (steps * effective_factor) as i32);
                }
            } else if event.source() == smithay::backend::input::AxisSource::Finger {
                frame = frame.stop(axis);
            }
        }
    }

    let scroll_delta = event
        .amount_v120(smithay::backend::input::Axis::Vertical)
        .or_else(|| event.amount(smithay::backend::input::Axis::Vertical));
    if let Some(delta) = scroll_delta.filter(|d| *d != 0.0) {
        let root_x = pointer_location.x.round() as i32;
        let root_y = pointer_location.y.round() as i32;
        if let Some(pos) = update_wayland_bar_hit_state(wm, root_x, root_y, true) {
            let clean_state = crate::util::clean_mask(
                modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                0,
            );
            dispatch_wayland_bar_scroll(wm, pos, delta, root_x, root_y, clean_state);
        }
    }

    pointer_handle.axis(state, frame);
    pointer_handle.frame(state);
}
