//! DRM/libinput-specific input handling.

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, Device as InputDevice, Event, InputEvent, PointerAxisEvent,
    PointerButtonEvent, PointerMotionEvent, TouchEvent,
};
use smithay::backend::input::{
    GestureBeginEvent as GestureBeginTrait, GestureEndEvent as GestureEndTrait,
    GesturePinchUpdateEvent as GesturePinchUpdateTrait,
    GestureSwipeUpdateEvent as GestureSwipeUpdateTrait,
};
use smithay::backend::libinput::LibinputInputBackend;
use smithay::input::pointer::{
    GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
    GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
};

use smithay::utils::SERIAL_COUNTER;

use crate::backend::wayland::compositor::WaylandState;
use crate::config::config_toml::InputConfig;
use crate::config::config_toml::{AccelProfile, ToggleSetting};
use crate::wayland::input::handle_keyboard;
use crate::wayland::input::touch::{
    NormalizedTouchPosition, TouchMappingTarget, TouchPointEvent, handle_touch_cancel,
    handle_touch_down, handle_touch_frame, handle_touch_motion, handle_touch_up,
};
use crate::wm::Wm;
use std::collections::HashMap;

/// Compositor-side work caused directly by a libinput event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibinputEventOutcome {
    Ignored,
    Activity,
    PointerMoved,
}

fn configure_device(
    device: &mut smithay::reexports::input::Device,
    input_config: &HashMap<String, InputConfig>,
) {
    let default_config = InputConfig::default();
    let config = input_config
        .get(device_type_key(device))
        .or_else(|| input_config.get("*"))
        .unwrap_or(&default_config);

    if let Some(tap) = config.tap {
        let _ = device.config_tap_set_enabled(tap == ToggleSetting::Enabled);
    }

    if let Some(natural_scroll) = config.natural_scroll {
        let _ = device
            .config_scroll_set_natural_scroll_enabled(natural_scroll == ToggleSetting::Enabled);
    }

    if let Some(accel_profile) = config.accel_profile {
        let profile = match accel_profile {
            AccelProfile::Flat => smithay::reexports::input::AccelProfile::Flat,
            AccelProfile::Adaptive => smithay::reexports::input::AccelProfile::Adaptive,
        };
        let _ = device.config_accel_set_profile(profile);
    }

    if let Some(pointer_accel) = config.pointer_accel {
        let _ = device.config_accel_set_speed(pointer_accel.clamp(-1.0, 1.0));
    }

    if let Some(left_handed) = config.left_handed {
        let _ = device.config_left_handed_set(left_handed == ToggleSetting::Enabled);
    }

    // scroll_factor is applied at the compositor level in the axis handler,
    // not via libinput. Nothing to do here for it.
}

fn device_type_key(device: &smithay::reexports::input::Device) -> &'static str {
    use smithay::reexports::input::DeviceCapability;

    if device.has_capability(DeviceCapability::Gesture) {
        "type:touchpad"
    } else if device.has_capability(DeviceCapability::Touch) {
        "type:touch"
    } else if device.has_capability(DeviceCapability::Pointer) {
        "type:pointer"
    } else {
        "type:keyboard"
    }
}

fn resolve_touch_output<'a>(
    device: &smithay::reexports::input::Device,
    input_config: &'a HashMap<String, InputConfig>,
) -> Option<&'a str> {
    // The backend id is the libinput sysname (for example `event12`). Device
    // names are also accepted because they are more readable in static config.
    // Type and wildcard selectors provide predictable fallbacks.
    resolve_touch_output_keys(
        &InputDevice::id(device),
        &InputDevice::name(device),
        device_type_key(device),
        input_config,
    )
}

fn resolve_touch_output_keys<'a>(
    id: &str,
    name: &str,
    type_key: &str,
    input_config: &'a HashMap<String, InputConfig>,
) -> Option<&'a str> {
    [id, name, type_key, "*"].into_iter().find_map(|key| {
        input_config
            .get(key)
            .and_then(|config| config.map_to_output.as_deref())
    })
}

fn touch_mapping_for_device(
    device: &smithay::reexports::input::Device,
    input_config: &HashMap<String, InputConfig>,
) -> TouchMappingTarget {
    if let Some(output) = resolve_touch_output(device, input_config) {
        return TouchMappingTarget::configured(output);
    }

    // `LIBINPUT_OUTPUT_NAME` is an optional udev hint. Honour it when the
    // hardware/administrator provides one; otherwise use the complete layout.
    device
        .output_name()
        .map(|name| TouchMappingTarget::Output(name.into_owned()))
        .unwrap_or(TouchMappingTarget::Layout)
}

fn normalized_touch_position<B, E>(event: &E) -> Option<NormalizedTouchPosition>
where
    B: smithay::backend::input::InputBackend,
    E: AbsolutePositionEvent<B>,
{
    NormalizedTouchPosition::new(event.x_transformed(1), event.y_transformed(1))
}

/// Re-apply input configuration to all tracked devices.
pub fn reconfigure_all_devices(
    devices: &mut [smithay::reexports::input::Device],
    input_config: &HashMap<String, InputConfig>,
) {
    for device in devices.iter_mut() {
        configure_device(device, input_config);
    }
}

pub fn dispatch_libinput_event(
    event: InputEvent<LibinputInputBackend>,
    state: &mut WaylandState,
    wm: &mut Wm,
    total_w: i32,
    total_h: i32,
) -> LibinputEventOutcome {
    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();
    use crate::backend::wayland::commands::{PointerMotionCommand, WmCommand};

    match event {
        InputEvent::DeviceAdded { mut device } => {
            configure_device(&mut device, &wm.core.config.input);
            state.runtime.tracked_devices.push(device);
            LibinputEventOutcome::Ignored
        }
        InputEvent::DeviceRemoved { device } => {
            use smithay::reexports::input::DeviceCapability;

            let removed_pointer = device.has_capability(DeviceCapability::Pointer);
            let removed_touch = device.has_capability(DeviceCapability::Touch);
            state.runtime.tracked_devices.retain(|d| d != &device);
            if removed_pointer {
                state.push_command(WmCommand::CancelInteractiveDrag(
                    crate::core_state::DragCancelReason::InputDeviceRemoved,
                ));
            }
            if removed_touch {
                handle_touch_cancel(state);
            }
            LibinputEventOutcome::Ignored
        }
        InputEvent::Keyboard { event } => {
            // Keep keyboard synchronous for now
            handle_keyboard::<LibinputInputBackend>(wm, state, &keyboard_handle, event);
            LibinputEventOutcome::Activity
        }
        InputEvent::PointerMotion { event } => {
            state.push_command(WmCommand::PointerMotion(PointerMotionCommand::Relative {
                dx: event.delta_x(),
                dy: event.delta_y(),
                dx_unaccel: event.delta_x_unaccel(),
                dy_unaccel: event.delta_y_unaccel(),
                time_msec: event.time_msec(),
                time_usec: event.time(),
            }));
            LibinputEventOutcome::PointerMoved
        }
        InputEvent::PointerMotionAbsolute { event } => {
            let x = event.x_transformed(total_w);
            let y = event.y_transformed(total_h);
            state.push_command(WmCommand::PointerMotion(PointerMotionCommand::Absolute {
                x,
                y,
                time_msec: event.time_msec(),
            }));
            LibinputEventOutcome::PointerMoved
        }
        InputEvent::PointerButton { event } => {
            state.push_command(WmCommand::PointerButton {
                button: event.button_code(),
                state: event.state(),
                time_msec: event.time_msec(),
            });
            LibinputEventOutcome::Activity
        }
        InputEvent::PointerAxis { event } => {
            state.push_command(WmCommand::PointerAxis {
                source: event.source(),
                horizontal: event.amount(Axis::Horizontal),
                vertical: event.amount(Axis::Vertical),
                horizontal_v120: event.amount_v120(Axis::Horizontal),
                vertical_v120: event.amount_v120(Axis::Vertical),
                horizontal_relative_direction: event.relative_direction(Axis::Horizontal),
                vertical_relative_direction: event.relative_direction(Axis::Vertical),
                time_msec: event.time_msec(),
            });
            LibinputEventOutcome::Activity
        }
        InputEvent::GesturePinchBegin { event } => {
            let smithay_event = GesturePinchBeginEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                fingers: event.fingers(),
            };
            pointer_handle.gesture_pinch_begin(state, &smithay_event);
            pointer_handle.frame(state);
            LibinputEventOutcome::Activity
        }
        InputEvent::GesturePinchUpdate { event } => {
            let smithay_event = GesturePinchUpdateEvent {
                time: event.time_msec(),
                delta: event.delta(),
                scale: event.scale(),
                rotation: event.rotation(),
            };
            pointer_handle.gesture_pinch_update(state, &smithay_event);
            pointer_handle.frame(state);
            LibinputEventOutcome::Activity
        }
        InputEvent::GesturePinchEnd { event } => {
            let smithay_event = GesturePinchEndEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                cancelled: event.cancelled(),
            };
            pointer_handle.gesture_pinch_end(state, &smithay_event);
            pointer_handle.frame(state);
            LibinputEventOutcome::Activity
        }
        InputEvent::GestureSwipeBegin { event } => {
            let smithay_event = GestureSwipeBeginEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                fingers: event.fingers(),
            };
            pointer_handle.gesture_swipe_begin(state, &smithay_event);
            pointer_handle.frame(state);
            LibinputEventOutcome::Activity
        }
        InputEvent::GestureSwipeUpdate { event } => {
            let smithay_event = GestureSwipeUpdateEvent {
                time: event.time_msec(),
                delta: event.delta(),
            };
            pointer_handle.gesture_swipe_update(state, &smithay_event);
            pointer_handle.frame(state);
            LibinputEventOutcome::Activity
        }
        InputEvent::GestureSwipeEnd { event } => {
            let smithay_event = GestureSwipeEndEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                cancelled: event.cancelled(),
            };
            pointer_handle.gesture_swipe_end(state, &smithay_event);
            pointer_handle.frame(state);
            LibinputEventOutcome::Activity
        }
        InputEvent::GestureHoldBegin { event } => {
            let smithay_event = GestureHoldBeginEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                fingers: event.fingers(),
            };
            pointer_handle.gesture_hold_begin(state, &smithay_event);
            pointer_handle.frame(state);
            LibinputEventOutcome::Activity
        }
        InputEvent::GestureHoldEnd { event } => {
            let smithay_event = GestureHoldEndEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                cancelled: event.cancelled(),
            };
            pointer_handle.gesture_hold_end(state, &smithay_event);
            pointer_handle.frame(state);
            LibinputEventOutcome::Activity
        }
        InputEvent::TouchDown { event } => {
            let mapping = touch_mapping_for_device(&event.device(), &wm.core.config.input);
            let position = normalized_touch_position::<LibinputInputBackend, _>(&event);
            if let Some(position) = position {
                handle_touch_down(
                    wm,
                    state,
                    TouchPointEvent {
                        slot: event.slot(),
                        position,
                        time_msec: event.time_msec(),
                    },
                    &mapping,
                );
            }
            LibinputEventOutcome::Activity
        }
        InputEvent::TouchMotion { event } => {
            let mapping = touch_mapping_for_device(&event.device(), &wm.core.config.input);
            let position = normalized_touch_position::<LibinputInputBackend, _>(&event);
            if let Some(position) = position {
                handle_touch_motion(
                    state,
                    TouchPointEvent {
                        slot: event.slot(),
                        position,
                        time_msec: event.time_msec(),
                    },
                    &mapping,
                );
            }
            LibinputEventOutcome::Activity
        }
        InputEvent::TouchUp { event } => {
            handle_touch_up(state, event.slot(), event.time_msec());
            LibinputEventOutcome::Activity
        }
        InputEvent::TouchFrame { .. } => {
            handle_touch_frame(state);
            LibinputEventOutcome::Activity
        }
        InputEvent::TouchCancel { .. } => {
            handle_touch_cancel(state);
            LibinputEventOutcome::Activity
        }
        _ => LibinputEventOutcome::Ignored,
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_touch_output_keys;
    use crate::config::config_toml::InputConfig;
    use std::collections::HashMap;

    fn config(output: &str) -> InputConfig {
        InputConfig {
            map_to_output: Some(output.into()),
            ..InputConfig::default()
        }
    }

    #[test]
    fn device_config_precedence_is_id_name_type_then_wildcard() {
        let configs = HashMap::from([
            ("*".into(), config("wildcard")),
            ("type:touch".into(), config("type")),
            ("Touchscreen".into(), config("name")),
            ("event12".into(), config("id")),
        ]);

        let resolved =
            resolve_touch_output_keys("event12", "Touchscreen", "type:touch", &configs).unwrap();
        assert_eq!(resolved, "id");

        let resolved =
            resolve_touch_output_keys("event99", "Touchscreen", "type:touch", &configs).unwrap();
        assert_eq!(resolved, "name");

        let resolved =
            resolve_touch_output_keys("event99", "Other", "type:touch", &configs).unwrap();
        assert_eq!(resolved, "type");

        let resolved =
            resolve_touch_output_keys("event99", "Other", "type:switch", &configs).unwrap();
        assert_eq!(resolved, "wildcard");
    }

    #[test]
    fn unset_specific_mapping_falls_back_to_type_mapping() {
        let configs = HashMap::from([
            ("event12".into(), InputConfig::default()),
            ("type:touch".into(), config("eDP-1")),
        ]);
        assert_eq!(
            resolve_touch_output_keys("event12", "Touchscreen", "type:touch", &configs),
            Some("eDP-1")
        );
    }
}
