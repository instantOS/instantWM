//! Pointer axis (scroll) handling.

use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::PointerHandle;
use smithay::utils::Point;

use crate::backend::wayland::commands::PointerAxisCommand;
use crate::backend::wayland::compositor::WaylandState;
use crate::types::Point as RootPoint;
use crate::util::clean_mask;
use crate::wayland::common::modifiers_to_x11_mask;
use crate::wm::Wm;

use crate::wayland::input::bar::{handle_bar_scroll, update_bar_hit_state};

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

#[derive(Debug, Clone, Copy)]
pub(crate) struct PointerAxisInput {
    pub event: PointerAxisCommand,
    pub location: Point<f64, smithay::utils::Logical>,
}

pub(crate) fn handle_pointer_axis(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer: &PointerHandle<WaylandState>,
    keyboard: &KeyboardHandle<WaylandState>,
    input: PointerAxisInput,
) {
    let scroll_factor = resolve_scroll_factor(&wm.core.config.input);

    let root = RootPoint::from_f64_round(input.location.x, input.location.y);

    // Check if the pointer is in the bar area; if so, dispatch bar scroll.
    let scroll_delta = input.event.vertical.v120.or(input.event.vertical.amount);
    if let Some(delta) = scroll_delta.filter(|d| *d != 0.0)
        && let Some(pos) = update_bar_hit_state(wm, root, true)
    {
        let clean_state = clean_mask(modifiers_to_x11_mask(&keyboard.modifier_state()), 0);
        handle_bar_scroll(wm, pos, delta, root, clean_state);
    }

    let mut frame =
        smithay::input::pointer::AxisFrame::new(input.event.time_msec).source(input.event.source);
    let mut has_axis_content = false;

    for (axis, axis_input) in [
        (
            smithay::backend::input::Axis::Horizontal,
            input.event.horizontal,
        ),
        (
            smithay::backend::input::Axis::Vertical,
            input.event.vertical,
        ),
    ] {
        if let Some(amount) = axis_input.amount {
            if amount.abs() >= f64::EPSILON {
                frame = frame.relative_direction(axis, axis_input.relative_direction);
                frame = frame.value(axis, amount * scroll_factor);
                has_axis_content = true;
                if let Some(steps) = axis_input.v120 {
                    frame = frame.v120(axis, (steps * scroll_factor) as i32);
                }
            } else if matches!(
                input.event.source,
                smithay::backend::input::AxisSource::Finger
            ) {
                // Finger scrolling must send axis_stop when libinput ends the sequence.
                frame = frame.stop(axis);
                has_axis_content = true;
            }
        }
    }

    if has_axis_content {
        pointer.axis(state, frame);
        pointer.frame(state);
    }
}
