//! DRM/libinput-specific input handling.

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, Event, InputEvent, PointerAxisEvent, PointerButtonEvent,
    PointerMotionEvent,
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
use crate::wm::Wm;

fn configure_device(
    device: &mut smithay::reexports::input::Device,
    input_config: &std::collections::HashMap<String, InputConfig>,
) {
    use smithay::reexports::input::DeviceCapability;

    let is_touchpad = device.has_capability(DeviceCapability::Gesture);
    let is_pointer = device.has_capability(DeviceCapability::Pointer);

    let config_key = if is_touchpad {
        "type:touchpad"
    } else if is_pointer {
        "type:pointer"
    } else {
        "type:keyboard"
    };

    let default_config = InputConfig::default();
    let config = input_config
        .get(config_key)
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

/// Re-apply input configuration to all tracked devices.
pub fn reconfigure_all_devices(
    devices: &mut [smithay::reexports::input::Device],
    input_config: &std::collections::HashMap<String, InputConfig>,
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
) -> bool {
    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();
    use crate::backend::wayland::commands::WmCommand;

    match event {
        InputEvent::DeviceAdded { mut device } => {
            configure_device(&mut device, &wm.g.cfg.input);
            state.runtime.tracked_devices.push(device);
            false
        }
        InputEvent::DeviceRemoved { device } => {
            state.runtime.tracked_devices.retain(|d| d != &device);
            false
        }
        InputEvent::Keyboard { event } => {
            // Keep keyboard synchronous for now
            handle_keyboard::<LibinputInputBackend>(wm, state, &keyboard_handle, event);
            true
        }
        InputEvent::PointerMotion { event } => {
            let dx = event.delta_x();
            let dy = event.delta_y();
            let current = state.runtime.pointer_location;
            state.runtime.pointer_location = smithay::utils::Point::from((
                (current.x + dx).clamp(0.0, total_w as f64),
                (current.y + dy).clamp(0.0, total_h as f64),
            ));
            state.push_command(WmCommand::PointerMotion {
                time_msec: event.time_msec(),
            });
            true
        }
        InputEvent::PointerMotionAbsolute { event } => {
            let x = event.x_transformed(total_w);
            let y = event.y_transformed(total_h);
            state.runtime.pointer_location = smithay::utils::Point::from((x, y));
            state.push_command(WmCommand::PointerMotion {
                time_msec: event.time_msec(),
            });
            true
        }
        InputEvent::PointerButton { event } => {
            state.push_command(WmCommand::PointerButton {
                button: event.button_code(),
                state: event.state(),
                time_msec: event.time_msec(),
            });
            true
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
            true
        }
        InputEvent::GesturePinchBegin { event } => {
            let smithay_event = GesturePinchBeginEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                fingers: event.fingers(),
            };
            pointer_handle.gesture_pinch_begin(state, &smithay_event);
            pointer_handle.frame(state);
            true
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
            true
        }
        InputEvent::GesturePinchEnd { event } => {
            let smithay_event = GesturePinchEndEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                cancelled: event.cancelled(),
            };
            pointer_handle.gesture_pinch_end(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        InputEvent::GestureSwipeBegin { event } => {
            let smithay_event = GestureSwipeBeginEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                fingers: event.fingers(),
            };
            pointer_handle.gesture_swipe_begin(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        InputEvent::GestureSwipeUpdate { event } => {
            let smithay_event = GestureSwipeUpdateEvent {
                time: event.time_msec(),
                delta: event.delta(),
            };
            pointer_handle.gesture_swipe_update(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        InputEvent::GestureSwipeEnd { event } => {
            let smithay_event = GestureSwipeEndEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                cancelled: event.cancelled(),
            };
            pointer_handle.gesture_swipe_end(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        InputEvent::GestureHoldBegin { event } => {
            let smithay_event = GestureHoldBeginEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                fingers: event.fingers(),
            };
            pointer_handle.gesture_hold_begin(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        InputEvent::GestureHoldEnd { event } => {
            let smithay_event = GestureHoldEndEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time_msec(),
                cancelled: event.cancelled(),
            };
            pointer_handle.gesture_hold_end(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        _ => false,
    }
}
