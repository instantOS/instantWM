use smithay::backend::input::InputEvent;
use smithay::backend::libinput::{LibinputInputBackend, PointerScrollAxis};
use smithay::reexports::input::{event, event::EventTrait, Event as LibinputRawEvent};

use crate::backend::wayland::compositor::WaylandState;
use crate::startup::wayland::input::{
    handle_keyboard, handle_pointer_axis, handle_pointer_button, handle_pointer_motion_absolute,
    handle_pointer_motion_relative,
};
use crate::wm::Wm;

use crate::config::config_toml::{AccelProfile, ToggleSetting};

pub fn raw_event_to_input_event(
    event: LibinputRawEvent,
) -> Option<InputEvent<LibinputInputBackend>> {
    use event::{keyboard::KeyboardEvent, pointer::PointerEvent, DeviceEvent};
    Some(match event {
        LibinputRawEvent::Keyboard(KeyboardEvent::Key(e)) => InputEvent::Keyboard { event: e },
        LibinputRawEvent::Pointer(PointerEvent::Motion(e)) => {
            InputEvent::PointerMotion { event: e }
        }
        LibinputRawEvent::Pointer(PointerEvent::MotionAbsolute(e)) => {
            InputEvent::PointerMotionAbsolute { event: e }
        }
        LibinputRawEvent::Pointer(PointerEvent::Button(e)) => {
            InputEvent::PointerButton { event: e }
        }
        LibinputRawEvent::Pointer(PointerEvent::ScrollWheel(e)) => InputEvent::PointerAxis {
            event: PointerScrollAxis::Wheel(e),
        },
        LibinputRawEvent::Pointer(PointerEvent::ScrollFinger(e)) => InputEvent::PointerAxis {
            event: PointerScrollAxis::Finger(e),
        },
        LibinputRawEvent::Pointer(PointerEvent::ScrollContinuous(e)) => InputEvent::PointerAxis {
            event: PointerScrollAxis::Continuous(e),
        },
        LibinputRawEvent::Device(DeviceEvent::Added(e)) => InputEvent::DeviceAdded {
            device: EventTrait::device(&e),
        },
        LibinputRawEvent::Device(DeviceEvent::Removed(e)) => InputEvent::DeviceRemoved {
            device: EventTrait::device(&e),
        },
        _ => return None,
    })
}

pub fn configure_device(
    device: &mut smithay::reexports::input::Device,
    input_config: &std::collections::HashMap<String, crate::config::config_toml::InputConfig>,
) {
    use smithay::reexports::input::DeviceCapability;

    let is_touchpad = device.has_capability(DeviceCapability::Gesture); // rough check for touchpad
    let is_pointer = device.has_capability(DeviceCapability::Pointer);

    let config_key = if is_touchpad {
        "type:touchpad"
    } else if is_pointer {
        "type:pointer"
    } else {
        "type:keyboard"
    };

    if let Some(config) = input_config
        .get(config_key)
        .or_else(|| input_config.get("*"))
    {
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

        // scroll_factor is applied at the compositor level in the axis handler,
        // not via libinput. Nothing to do here for it.
    }
}

/// Re-apply input configuration to all tracked devices.
pub fn reconfigure_all_devices(
    devices: &mut [smithay::reexports::input::Device],
    input_config: &std::collections::HashMap<String, crate::config::config_toml::InputConfig>,
) {
    for device in devices.iter_mut() {
        configure_device(device, input_config);
    }
}

pub fn dispatch_libinput_event(
    event: InputEvent<LibinputInputBackend>,
    state: &mut WaylandState,
    wm: &mut Wm,
    pointer_location: &mut smithay::utils::Point<f64, smithay::utils::Logical>,
    total_w: i32,
    total_h: i32,
) -> bool {
    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();

    match event {
        InputEvent::DeviceAdded { mut device } => {
            crate::startup::drm::input::configure_device(&mut device, &wm.g.cfg.input);
            state.tracked_devices.push(device);
            false
        }
        InputEvent::DeviceRemoved { device } => {
            state.tracked_devices.retain(|d| d != &device);
            false
        }
        InputEvent::Keyboard { event } => {
            handle_keyboard::<LibinputInputBackend>(wm, state, &keyboard_handle, event);
            true
        }
        InputEvent::PointerMotion { event } => {
            handle_pointer_motion_relative::<LibinputInputBackend>(
                wm,
                state,
                &pointer_handle,
                &keyboard_handle,
                event,
                pointer_location,
                total_w,
                total_h,
            );
            true
        }
        InputEvent::PointerMotionAbsolute { event } => {
            handle_pointer_motion_absolute::<LibinputInputBackend>(
                wm,
                state,
                &pointer_handle,
                &keyboard_handle,
                event,
                pointer_location,
                total_w,
                total_h,
            );
            true
        }
        InputEvent::PointerButton { event } => {
            handle_pointer_button::<LibinputInputBackend>(
                wm,
                state,
                &pointer_handle,
                &keyboard_handle,
                event,
                *pointer_location,
            );
            true
        }
        InputEvent::PointerAxis { event } => {
            handle_pointer_axis::<LibinputInputBackend>(
                wm,
                state,
                &pointer_handle,
                &keyboard_handle,
                event,
                *pointer_location,
            );
            true
        }
        _ => false,
    }
}
