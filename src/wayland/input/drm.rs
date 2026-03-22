//! DRM/libinput-specific input handling.

use smithay::backend::input::InputEvent;
use smithay::backend::libinput::{LibinputInputBackend, PointerScrollAxis};
use smithay::input::pointer::{
    GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
    GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
};
use smithay::reexports::input::event::gesture::{
    GestureEndEvent, GestureEventCoordinates, GestureEventTrait, GesturePinchEventTrait,
};
use smithay::reexports::input::{Event as LibinputRawEvent, event, event::EventTrait};
use smithay::utils::{Point, SERIAL_COUNTER};

use crate::backend::wayland::compositor::WaylandState;
use crate::wayland::input::{
    handle_keyboard, handle_pointer_axis, handle_pointer_button, handle_pointer_motion,
    motion_event_from_libinput_absolute, motion_event_from_libinput_relative,
};

use crate::config::config_toml::{AccelProfile, ToggleSetting};

pub fn raw_event_to_input_event(
    event: LibinputRawEvent,
) -> Option<InputEvent<LibinputInputBackend>> {
    use event::{DeviceEvent, GestureEvent, keyboard::KeyboardEvent, pointer::PointerEvent};
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
        LibinputRawEvent::Gesture(GestureEvent::Pinch(
            event::gesture::GesturePinchEvent::Begin(e),
        )) => InputEvent::GesturePinchBegin { event: e },
        LibinputRawEvent::Gesture(GestureEvent::Pinch(
            event::gesture::GesturePinchEvent::Update(e),
        )) => InputEvent::GesturePinchUpdate { event: e },
        LibinputRawEvent::Gesture(GestureEvent::Pinch(event::gesture::GesturePinchEvent::End(
            e,
        ))) => InputEvent::GesturePinchEnd { event: e },
        LibinputRawEvent::Gesture(GestureEvent::Swipe(
            event::gesture::GestureSwipeEvent::Begin(e),
        )) => InputEvent::GestureSwipeBegin { event: e },
        LibinputRawEvent::Gesture(GestureEvent::Swipe(
            event::gesture::GestureSwipeEvent::Update(e),
        )) => InputEvent::GestureSwipeUpdate { event: e },
        LibinputRawEvent::Gesture(GestureEvent::Swipe(event::gesture::GestureSwipeEvent::End(
            e,
        ))) => InputEvent::GestureSwipeEnd { event: e },
        LibinputRawEvent::Gesture(GestureEvent::Hold(event::gesture::GestureHoldEvent::Begin(
            e,
        ))) => InputEvent::GestureHoldBegin { event: e },
        LibinputRawEvent::Gesture(GestureEvent::Hold(event::gesture::GestureHoldEvent::End(e))) => {
            InputEvent::GestureHoldEnd { event: e }
        }
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
    total_w: i32,
    total_h: i32,
) -> bool {
    let keyboard_handle = state.keyboard.clone();
    let pointer_handle = state.pointer.clone();

    match event {
        InputEvent::DeviceAdded { mut device } => {
            configure_device(&mut device, &state.wm.g.cfg.input);
            state.tracked_devices.push(device);
            false
        }
        InputEvent::DeviceRemoved { device } => {
            state.tracked_devices.retain(|d| d != &device);
            false
        }
        InputEvent::Keyboard { event } => {
            handle_keyboard::<LibinputInputBackend>(state, &keyboard_handle, event);
            true
        }
        InputEvent::PointerMotion { event } => {
            let motion_event = motion_event_from_libinput_relative(event);
            handle_pointer_motion(state, &pointer_handle, &keyboard_handle, motion_event);
            true
        }
        InputEvent::PointerMotionAbsolute { event } => {
            let motion_event = motion_event_from_libinput_absolute(event, total_w, total_h);
            handle_pointer_motion(state, &pointer_handle, &keyboard_handle, motion_event);
            true
        }
        InputEvent::PointerButton { event } => {
            handle_pointer_button::<LibinputInputBackend>(
                state,
                &pointer_handle,
                &keyboard_handle,
                event,
                state.pointer_location,
            );
            true
        }
        InputEvent::PointerAxis { event } => {
            handle_pointer_axis::<LibinputInputBackend>(
                state,
                &pointer_handle,
                &keyboard_handle,
                event,
                state.pointer_location,
            );
            true
        }
        InputEvent::GesturePinchBegin { event } => {
            let smithay_event = GesturePinchBeginEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time(),
                fingers: event.finger_count() as u32,
            };
            pointer_handle.gesture_pinch_begin(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        InputEvent::GesturePinchUpdate { event } => {
            let smithay_event = GesturePinchUpdateEvent {
                time: event.time(),
                delta: Point::from((event.dx(), event.dy())),
                scale: event.scale(),
                rotation: event.angle_delta(),
            };
            pointer_handle.gesture_pinch_update(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        InputEvent::GesturePinchEnd { event } => {
            let smithay_event = GesturePinchEndEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time(),
                cancelled: event.cancelled(),
            };
            pointer_handle.gesture_pinch_end(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        InputEvent::GestureSwipeBegin { event } => {
            let smithay_event = GestureSwipeBeginEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time(),
                fingers: event.finger_count() as u32,
            };
            pointer_handle.gesture_swipe_begin(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        InputEvent::GestureSwipeUpdate { event } => {
            let smithay_event = GestureSwipeUpdateEvent {
                time: event.time(),
                delta: Point::from((event.dx(), event.dy())),
            };
            pointer_handle.gesture_swipe_update(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        InputEvent::GestureSwipeEnd { event } => {
            let smithay_event = GestureSwipeEndEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time(),
                cancelled: event.cancelled(),
            };
            pointer_handle.gesture_swipe_end(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        InputEvent::GestureHoldBegin { event } => {
            let smithay_event = GestureHoldBeginEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time(),
                fingers: event.finger_count() as u32,
            };
            pointer_handle.gesture_hold_begin(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        InputEvent::GestureHoldEnd { event } => {
            let smithay_event = GestureHoldEndEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time: event.time(),
                cancelled: event.cancelled(),
            };
            pointer_handle.gesture_hold_end(state, &smithay_event);
            pointer_handle.frame(state);
            true
        }
        _ => false,
    }
}
