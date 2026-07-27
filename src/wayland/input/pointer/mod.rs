//! Pointer input handling for Wayland compositor.
//!
//! This module handles all pointer-related input events:
//! - Motion (absolute and relative)
//! - Button clicks
//! - Axis/scroll events
//! - Drag operations (title drag, tag drag, resize drag)

use smithay::input::pointer::PointerHandle;
use smithay::utils::{Point, SERIAL_COUNTER};

use crate::backend::wayland::compositor::{PointerFocusTarget, WaylandState};

pub mod axis;
pub mod button;
mod constraints;
pub mod drag;
pub mod motion;

/// Remove pointer focus when touchscreen input becomes active.
///
/// Ordering invariant:
///
/// 1. Native touch-down clears `wl_pointer` focus.
/// 2. The next client-bound physical pointer event restores it.
///
/// Do not move the leave to the pointer-return path. Firefox otherwise retains
/// mixed mouse/touch state; Excalidraw exposes that by treating later trackpad
/// clicks as touchscreen taps and immediately creating a fixed-size arrow.
/// wlroots/Sway implements the same ordering by clearing pointer focus while
/// hiding the cursor in its touch-down handler.
pub(super) fn clear_focus_for_touch(
    pointer: &PointerHandle<WaylandState>,
    state: &mut WaylandState,
    location: Point<f64, smithay::utils::Logical>,
    time_msec: u32,
) {
    if pointer.current_focus().is_none() {
        return;
    }
    state.runtime.pointer_focus_cleared_by_touch = true;
    pointer.motion(
        state,
        None,
        &smithay::input::pointer::MotionEvent {
            location,
            serial: SERIAL_COUNTER.next_serial(),
            time: time_msec,
        },
    );
    pointer.frame(state);
}

/// Restore focus before button/axis input that has no preceding pointer motion.
pub(super) fn restore_focus_after_touch(
    pointer: &PointerHandle<WaylandState>,
    state: &mut WaylandState,
    location: Point<f64, smithay::utils::Logical>,
    time_msec: u32,
) {
    state.runtime.cursor_hidden_by_touch = false;
    if !state.runtime.pointer_focus_cleared_by_touch {
        return;
    }
    let contents = state.contents_under_pointer(location);
    let focus = contents
        .surface
        .map(|(surface, origin)| (PointerFocusTarget::WlSurface(surface), origin.to_f64()));
    if focus.is_none() {
        return;
    }
    state.runtime.pointer_focus_cleared_by_touch = false;
    pointer.motion(
        state,
        focus,
        &smithay::input::pointer::MotionEvent {
            location,
            serial: SERIAL_COUNTER.next_serial(),
            time: time_msec,
        },
    );
}
