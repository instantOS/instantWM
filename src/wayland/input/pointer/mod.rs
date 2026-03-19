//! Pointer input handling for Wayland compositor.
//!
//! This module handles all pointer-related input events:
//! - Motion (absolute and relative)
//! - Button clicks
//! - Axis/scroll events
//! - Drag operations (title drag, tag drag, resize drag)

pub mod axis;
pub mod button;
pub mod drag;
pub mod motion;

// Re-export for convenience
pub use axis::handle_pointer_axis;
pub use button::handle_pointer_button;
pub use motion::{
    handle_pointer_motion, motion_event_from_libinput_absolute,
    motion_event_from_libinput_relative, motion_event_from_winit,
};
