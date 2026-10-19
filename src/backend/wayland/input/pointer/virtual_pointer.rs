//! `zwlr_virtual_pointer_v1` devices.
//!
//! A virtual pointer is an ordinary pointer device created by a client, so its
//! events carry the same payloads as physical ones and take the shared command
//! path; only absolute motion needs translating, because the protocol sends a
//! normalized position instead of a screen coordinate.
//!
//! Like synthetic keyboard input, these events deliberately do not count as
//! user activity: an automation tool must not keep the screen awake.

use smithay::backend::input::{
    AbsolutePositionEvent, Axis, Event, InputEvent, PointerAxisEvent, PointerButtonEvent,
    PointerMotionEvent,
};
use smithay::wayland::virtual_pointer::VirtualPointerBackend;

use crate::backend::wayland::commands::{
    PointerAxis, PointerAxisCommand, PointerButtonCommand, PointerMotionCommand, WmCommand,
};
use crate::backend::wayland::compositor::WaylandState;

/// Handle an event from a `zwlr_virtual_pointer_v1` device.
pub fn handle_virtual_pointer_event(
    state: &mut WaylandState,
    event: InputEvent<VirtualPointerBackend>,
) {
    match event {
        InputEvent::PointerMotion { event } => {
            state
                .native
                .push_command(WmCommand::PointerMotion(PointerMotionCommand::Relative {
                    dx: event.delta_x(),
                    dy: event.delta_y(),
                    dx_unaccel: event.delta_x_unaccel(),
                    dy_unaccel: event.delta_y_unaccel(),
                    time: event.time(),
                }));
        }
        InputEvent::PointerMotionAbsolute { event } => {
            // The position is normalized against the client's own extents, so
            // map it onto the output the client targeted; a client that named
            // no output covers the whole layout, like a physical absolute
            // device. Without a mapped output there is no region to map into.
            let region = event
                .device()
                .output()
                .and_then(|output| state.native.space.output_geometry(&output))
                .or_else(|| super::motion::layout_bounds_from_space(state));
            let Some(region) = region else {
                return;
            };
            let x = region.loc.x as f64 + event.x_transformed(region.size.w.max(1));
            let y = region.loc.y as f64 + event.y_transformed(region.size.h.max(1));
            state
                .native
                .push_command(WmCommand::PointerMotion(PointerMotionCommand::Absolute {
                    x,
                    y,
                    time: event.time(),
                }));
        }
        InputEvent::PointerButton { event } => {
            state
                .native
                .push_command(WmCommand::PointerButton(PointerButtonCommand {
                    code: event.button_code(),
                    state: event.state(),
                    time: event.time(),
                }));
        }
        InputEvent::PointerAxis { event } => {
            let time = event.time();
            state
                .native
                .push_command(WmCommand::PointerAxis(PointerAxisCommand {
                    source: event.source(),
                    horizontal: PointerAxis {
                        amount: event.amount(Axis::Horizontal),
                        v120: event.amount_v120(Axis::Horizontal),
                        relative_direction: event.relative_direction(Axis::Horizontal),
                    },
                    vertical: PointerAxis {
                        amount: event.amount(Axis::Vertical),
                        v120: event.amount_v120(Axis::Vertical),
                        relative_direction: event.relative_direction(Axis::Vertical),
                    },
                    time,
                }));
        }
        // A pointer that goes away takes its held buttons with it, exactly
        // like an unplugged mouse, so an interactive drag must not stay stuck.
        InputEvent::DeviceRemoved { .. } => {
            state.native.push_command(WmCommand::CancelInteractiveDrag(
                crate::core_state::DragCancelReason::InputDeviceRemoved,
            ));
        }
        InputEvent::DeviceAdded { .. } => {}
        _ => {}
    }
}
