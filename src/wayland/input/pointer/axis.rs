//! Pointer axis (scroll) handling.

use smithay::backend::input::{InputBackend, PointerAxisEvent};
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::PointerHandle;
use smithay::utils::Point;

use crate::backend::wayland::compositor::WaylandState;
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

    let mut frame = smithay::input::pointer::AxisFrame::new(event.time_msec());
    frame = frame.source(event.source());
    let mut has_axis_content = false;

    for axis in [
        smithay::backend::input::Axis::Horizontal,
        smithay::backend::input::Axis::Vertical,
    ] {
        if let Some(amount) = event.amount(axis) {
            if amount.abs() >= f64::EPSILON {
                let scaled_amount = amount * scroll_factor;
                frame = frame.relative_direction(axis, event.relative_direction(axis));
                frame = frame.value(axis, scaled_amount);
                has_axis_content = true;
                if let Some(steps) = event.amount_v120(axis) {
                    frame = frame.v120(axis, (steps * scroll_factor) as i32);
                }
            } else if matches!(
                event.source(),
                smithay::backend::input::AxisSource::Finger
                    | smithay::backend::input::AxisSource::Continuous
            ) {
                frame = frame.stop(axis);
                has_axis_content = true;
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

    if has_axis_content {
        pointer_handle.axis(state, frame);
        pointer_handle.frame(state);
    }
}
