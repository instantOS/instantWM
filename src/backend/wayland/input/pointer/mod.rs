//! Pointer input handling for Wayland compositor.
//!
//! This module handles all pointer-related input events:
//! - Motion (absolute and relative)
//! - Button clicks
//! - Axis/scroll events
//! - Drag operations (title drag, tag drag, resize drag)

pub mod axis;
pub mod button;
mod constraints;
pub mod drag;
pub mod motion;
mod virtual_pointer;

#[cfg(test)]
mod virtual_pointer_tests;

pub(crate) use constraints::{activate_for_focus, new_constraint_should_activate};
pub use virtual_pointer::handle_virtual_pointer_event;

#[cfg(test)]
mod protocol_tests;
