//! Pointer axis (scroll) handling.

use smithay::backend::input::{InputBackend, PointerAxisEvent};
use smithay::input::keyboard::KeyboardHandle;
use smithay::input::pointer::PointerHandle;
use smithay::utils::Point;

use crate::backend::wayland::compositor::WaylandState;
use crate::types::Point as RootPoint;
use crate::util::clean_mask;
use crate::wayland::common::modifiers_to_x11_mask;
use crate::wm::Wm;

use crate::wayland::input::bar::{handle_wayland_bar_scroll, update_wayland_bar_hit_state};

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

/// Internal helper for handling pointer axis from raw values.
pub fn handle_pointer_axis_raw(
    wm: &mut Wm,
    state: &mut WaylandState,
    pointer: &PointerHandle<WaylandState>,
    keyboard: &KeyboardHandle<WaylandState>,
    source: smithay::backend::input::AxisSource,
    horizontal: f64,
    vertical: f64,
    time: u32,
    pointer_location: Point<f64, smithay::utils::Logical>,
) {
    let scroll_factor = resolve_scroll_factor(&wm.g.cfg.input);
    let horizontal = horizontal * scroll_factor;
    let vertical = vertical * scroll_factor;

    let root = RootPoint::new(
        pointer_location.x.round() as i32,
        pointer_location.y.round() as i32,
    );

    // Check if the pointer is in the bar area; if so, dispatch bar scroll.
    let delta = vertical; // bar scroll uses vertical axis
    if delta.abs() > f64::EPSILON
        && let Some(pos) = update_wayland_bar_hit_state(wm, root, true)
    {
        let clean_state = clean_mask(modifiers_to_x11_mask(&keyboard.modifier_state()), 0);
        handle_wayland_bar_scroll(wm, pos, delta, root, clean_state);
        pointer.frame(state);
        return;
    }

    update_wayland_bar_hit_state(wm, root, false);

    let mut frame = smithay::input::pointer::AxisFrame::new(time).source(source);
    if horizontal.abs() >= f64::EPSILON {
        frame = frame.value(smithay::backend::input::Axis::Horizontal, horizontal);
    }
    if vertical.abs() >= f64::EPSILON {
        frame = frame.value(smithay::backend::input::Axis::Vertical, vertical);
    }
    pointer.axis(state, frame);
    pointer.frame(state);
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
            } else if matches!(event.source(), smithay::backend::input::AxisSource::Finger) {
                // smithay expects the compositor to emit axis_stop for touchpad-style
                // finger scrolling when libinput ends the zero-terminated sequence.
                frame = frame.stop(axis);
                has_axis_content = true;
            }
        }
    }

    let scroll_delta = event
        .amount_v120(smithay::backend::input::Axis::Vertical)
        .or_else(|| event.amount(smithay::backend::input::Axis::Vertical));
    if let Some(delta) = scroll_delta.filter(|d| *d != 0.0) {
        let root = RootPoint::new(
            pointer_location.x.round() as i32,
            pointer_location.y.round() as i32,
        );
        if let Some(pos) = update_wayland_bar_hit_state(wm, root, true) {
            let clean_state = crate::util::clean_mask(
                modifiers_to_x11_mask(&keyboard_handle.modifier_state()),
                0,
            );
            handle_wayland_bar_scroll(wm, pos, delta, root, clean_state);
        }
    }

    if has_axis_content {
        pointer_handle.axis(state, frame);
        pointer_handle.frame(state);
    }
}
